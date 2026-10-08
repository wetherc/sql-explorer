//! The files of the settings and of the history.
//!
//! Every read is tolerant: one record that cannot be understood is written
//! to the log, left out and reported to the window, so a single damaged
//! entry does not stop the whole list from loading.

use crate::error::{Error, Result};
use crate::history::{push_entry, trim_history, HistoryEntry};
use crate::jsonfile;
use crate::storage::SavedConnection;
use serde::de::DeserializeOwned;
use serde_json::Value as JsonValue;
use std::path::PathBuf;
use tauri::{AppHandle, Runtime};

/// The file that holds the saved connections.
pub const CONNECTIONS_FILE: &str = "connections.json";
/// The file of the history. The file can also contain saved statements under
/// the key `saved`. No command reads or changes that key, so a write of the
/// history leaves those entries in place.
pub const QUERIES_FILE: &str = "queries.json";
/// The file that holds the open tabs.
pub const WORKSPACE_FILE: &str = "workspace.json";
/// The file that holds the folders and the single files the user accepted.
pub const FOLDERS_FILE: &str = "folders.json";

/// Reads one list of a file, changes it and writes it back, under the lock
/// of that file. Two runs that end together then cannot drop the entry of
/// each other.
fn edit_list<R: Runtime, T, F>(app: &AppHandle<R>, file: &str, key: &str, change: F) -> Result<()>
where
    T: DeserializeOwned + serde::Serialize,
    F: FnOnce(&mut Vec<T>),
{
    let path = settings_path(app, file)?;
    jsonfile::update(&path, |values| {
        let mut list: Vec<T> = parse_list(file, values.get(key).cloned());
        change(&mut list);
        values.insert(key.to_string(), serde_json::to_value(&list)?);
        Ok::<_, Error>(())
    })?
}

/// Sets one value of a file and writes the file.
fn set_value<R: Runtime>(
    app: &AppHandle<R>,
    file: &str,
    key: &str,
    value: JsonValue,
) -> Result<()> {
    let path = settings_path(app, file)?;
    jsonfile::update(&path, |values| {
        values.insert(key.to_string(), value);
    })
}

/// Reads one value of a file.
fn get_value<R: Runtime>(app: &AppHandle<R>, file: &str, key: &str) -> Result<Option<JsonValue>> {
    let path = settings_path(app, file)?;
    Ok(jsonfile::read(&path)?.remove(key))
}

const HISTORY_KEY: &str = "history";
const WORKSPACE_KEY: &str = "workspace";
const ROOTS_KEY: &str = "roots";
const GRANTS_KEY: &str = "files";

/// The path of one file of the settings.
///
/// The files live in the data folder of the application. A test names a
/// folder of its own, so no test reads or writes the files of the real
/// application.
#[cfg(not(test))]
fn settings_path<R: Runtime>(app: &AppHandle<R>, name: &str) -> Result<PathBuf> {
    use tauri::Manager;
    Ok(app.path().app_data_dir()?.join(name))
}

#[cfg(test)]
fn settings_path<R: Runtime>(_app: &AppHandle<R>, name: &str) -> Result<PathBuf> {
    Ok(tests::settings_folder().join(name))
}

/// Reports the records of a file that could not be understood.
fn note_dropped(file: &str, dropped: usize) {
    if dropped > 0 {
        let records = if dropped == 1 { "record" } else { "records" };
        jsonfile::note_problem(format!(
            "{dropped} {records} in {file} couldn't be read and {} left out.",
            if dropped == 1 { "was" } else { "were" }
        ));
    }
}

/// Reads every value of a file and drops the records that cannot be
/// understood.
fn parse_values<T: DeserializeOwned>(file: &str, values: Vec<(String, JsonValue)>) -> Vec<T> {
    let total = values.len();
    let parsed: Vec<T> = values
        .into_iter()
        .filter_map(|(key, value)| match serde_json::from_value::<T>(value) {
            Ok(parsed) => Some(parsed),
            Err(error) => {
                log::warn!("The stored record '{key}' was left out: {error}");
                None
            }
        })
        .collect();
    note_dropped(file, total - parsed.len());
    parsed
}

/// Reads a list out of one key of a file, and drops the entries that
/// cannot be understood.
fn parse_list<T: DeserializeOwned>(file: &str, value: Option<JsonValue>) -> Vec<T> {
    let Some(JsonValue::Array(items)) = value else {
        return Vec::new();
    };
    let total = items.len();
    let parsed: Vec<T> = items
        .into_iter()
        .filter_map(|item| match serde_json::from_value::<T>(item) {
            Ok(parsed) => Some(parsed),
            Err(error) => {
                log::warn!("A stored entry was left out: {error}");
                None
            }
        })
        .collect();
    note_dropped(file, total - parsed.len());
    parsed
}

pub fn read_connections<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<SavedConnection>> {
    let path = settings_path(app, CONNECTIONS_FILE)?;
    let values: Vec<(String, JsonValue)> = jsonfile::read(&path)?.into_iter().collect();
    let mut connections: Vec<SavedConnection> = parse_values(CONNECTIONS_FILE, values);
    connections
        .iter_mut()
        .for_each(SavedConnection::adopt_integrated_flag);
    connections.sort_by_key(|connection| connection.name.to_lowercase());
    Ok(connections)
}

pub fn write_connection<R: Runtime>(
    app: &AppHandle<R>,
    connection: &SavedConnection,
) -> Result<()> {
    set_value(
        app,
        CONNECTIONS_FILE,
        &connection.id,
        serde_json::to_value(connection)?,
    )
}

pub fn delete_connection<R: Runtime>(app: &AppHandle<R>, id: &str) -> Result<()> {
    let path = settings_path(app, CONNECTIONS_FILE)?;
    jsonfile::update(&path, |values| {
        values.remove(id);
    })
}

/// Reads the history. A file that holds more than the limits allow gives the
/// newer entries alone, and the next write of the file drops the others.
pub fn read_history<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<HistoryEntry>> {
    let mut history = parse_list(QUERIES_FILE, get_value(app, QUERIES_FILE, HISTORY_KEY)?);
    trim_history(&mut history);
    Ok(history)
}

/// Writes one entry to the history file. The caller keeps its own copy of the
/// list, so the function gives no list back. A large history then stays out of
/// the answer of each execution.
pub fn add_history<R: Runtime>(app: &AppHandle<R>, entry: HistoryEntry) -> Result<()> {
    edit_list(
        app,
        QUERIES_FILE,
        HISTORY_KEY,
        |history: &mut Vec<HistoryEntry>| push_entry(history, entry),
    )
}

pub fn clear_history<R: Runtime>(app: &AppHandle<R>) -> Result<()> {
    edit_list(
        app,
        QUERIES_FILE,
        HISTORY_KEY,
        |history: &mut Vec<HistoryEntry>| history.clear(),
    )
}

/// Reads the folders that the user accepted in an earlier session.
///
/// This file belongs to the backend. No command writes it with a path that
/// the interface chose, so a folder reaches the list only after the user
/// accepted it in a dialog of the operating system.
pub fn read_file_roots<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<String>> {
    Ok(parse_list(
        FOLDERS_FILE,
        get_value(app, FOLDERS_FILE, ROOTS_KEY)?,
    ))
}

/// Writes the folders that the user accepted.
pub fn write_file_roots<R: Runtime>(app: &AppHandle<R>, roots: &[String]) -> Result<()> {
    set_value(app, FOLDERS_FILE, ROOTS_KEY, serde_json::to_value(roots)?)
}

/// Reads the single files that the user accepted in an earlier session. The
/// same rule as for the folders applies: only a dialog of the operating
/// system adds a file to this list.
pub fn read_file_grants<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<String>> {
    Ok(parse_list(
        FOLDERS_FILE,
        get_value(app, FOLDERS_FILE, GRANTS_KEY)?,
    ))
}

/// Writes the single files that the user accepted.
pub fn write_file_grants<R: Runtime>(app: &AppHandle<R>, files: &[String]) -> Result<()> {
    set_value(app, FOLDERS_FILE, GRANTS_KEY, serde_json::to_value(files)?)
}

pub fn read_workspace<R: Runtime>(app: &AppHandle<R>) -> Result<JsonValue> {
    Ok(get_value(app, WORKSPACE_FILE, WORKSPACE_KEY)?.unwrap_or(JsonValue::Null))
}

pub fn write_workspace<R: Runtime>(app: &AppHandle<R>, workspace: JsonValue) -> Result<()> {
    set_value(app, WORKSPACE_FILE, WORKSPACE_KEY, workspace)
}

/// Reads every file of the settings once and gives the problems found in
/// them. A damaged file is moved aside during the read, and the window shows
/// each problem once at start.
pub fn storage_problems<R: Runtime>(app: &AppHandle<R>) -> Vec<String> {
    let reads = [
        read_connections(app).err(),
        read_history(app).err(),
        read_workspace(app).err(),
        read_file_roots(app).err(),
        read_file_grants(app).err(),
    ];
    for error in reads.into_iter().flatten() {
        jsonfile::note_problem(error.to_string());
    }
    jsonfile::take_problems()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::DbType;

    /// The folder that holds the files of the settings while a test runs.
    ///
    /// Each thread takes a folder of its own, and one test runs on one
    /// thread, so two tests that run at the same time write no file in
    /// common.
    pub(super) fn settings_folder() -> PathBuf {
        thread_local! {
            static FOLDER: PathBuf = {
                let path = std::env::temp_dir().join(format!(
                    "sql-explorer-settings-{}-{:?}",
                    std::process::id(),
                    std::thread::current().id()
                ));
                let _ = std::fs::remove_dir_all(&path);
                std::fs::create_dir_all(&path).unwrap();
                path
            };
        }
        FOLDER.with(PathBuf::clone)
    }

    fn record(id: &str, name: &str) -> JsonValue {
        serde_json::json!({ "id": id, "name": name, "dbType": "sqlite" })
    }

    #[test]
    fn a_record_that_cannot_be_understood_is_left_out() {
        let values = vec![
            ("a".to_string(), record("a", "Alpha")),
            ("b".to_string(), serde_json::json!({ "broken": true })),
            ("c".to_string(), record("c", "Gamma")),
        ];
        let parsed: Vec<SavedConnection> = parse_values("t.json", values);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].id, "a");
        assert_eq!(parsed[0].db_type, DbType::Sqlite);
        assert_eq!(parsed[1].id, "c");
    }

    #[test]
    fn an_empty_file_gives_an_empty_list() {
        let parsed: Vec<SavedConnection> = parse_values("t.json", Vec::new());
        assert!(parsed.is_empty());
    }

    #[test]
    fn a_list_drops_only_the_entries_that_are_damaged() {
        let value = serde_json::json!([
            {
                "id": "1",
                "connectionId": "c",
                "connectionName": "n",
                "query": "SELECT 1",
                "ranAt": "t",
                "elapsedMs": 1,
                "rowCount": 1,
                "succeeded": true
            },
            { "nope": 1 }
        ]);
        let entries: Vec<HistoryEntry> = parse_list("t.json", Some(value));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "1");
    }

    #[test]
    fn a_value_that_is_not_a_list_gives_an_empty_list() {
        let entries: Vec<HistoryEntry> = parse_list("t.json", None);
        assert!(entries.is_empty());
        let entries: Vec<HistoryEntry> = parse_list("t.json", Some(serde_json::json!("text")));
        assert!(entries.is_empty());
    }

    fn app_with_store() -> tauri::App<tauri::test::MockRuntime> {
        tauri::test::mock_builder()
            .build(tauri::generate_context!())
            .unwrap()
    }

    fn entry(id: &str, query: &str) -> HistoryEntry {
        HistoryEntry {
            id: id.to_string(),
            connection_id: "c1".to_string(),
            connection_name: "Server".to_string(),
            query: query.to_string(),
            ran_at: "2026-01-01T00:00:00Z".to_string(),
            elapsed_ms: 1,
            row_count: 1,
            succeeded: true,
            error: None,
        }
    }

    #[test]
    fn the_history_takes_each_entry_and_clears() {
        let app = app_with_store();
        add_history(app.handle(), entry("1", "SELECT 1")).unwrap();
        add_history(app.handle(), entry("2", "SELECT 2")).unwrap();
        let ids: Vec<String> = read_history(app.handle())
            .unwrap()
            .into_iter()
            .map(|entry| entry.id)
            .collect();
        assert_eq!(ids, ["2", "1"]);

        clear_history(app.handle()).unwrap();
        assert!(read_history(app.handle()).unwrap().is_empty());
    }

    #[test]
    fn a_write_of_the_history_keeps_the_other_keys_of_its_file() {
        let app = app_with_store();
        let path = settings_path(app.handle(), QUERIES_FILE).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let saved = serde_json::json!([{ "id": "a", "name": "Daily", "query": "SELECT 1" }]);
        std::fs::write(&path, serde_json::json!({ "saved": saved }).to_string()).unwrap();

        add_history(app.handle(), entry("1", "SELECT 1")).unwrap();
        clear_history(app.handle()).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        let values: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(values["saved"], saved);
    }

    #[test]
    fn dropped_records_and_damaged_files_are_reported() {
        let _problems = crate::jsonfile::PROBLEM_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let app = app_with_store();
        let _: Vec<HistoryEntry> = parse_list("one.json", Some(serde_json::json!([{ "nope": 1 }])));
        let _: Vec<HistoryEntry> = parse_list(
            "two.json",
            Some(serde_json::json!([{ "nope": 1 }, { "nope": 2 }])),
        );
        std::fs::write(settings_folder().join(WORKSPACE_FILE), b"{").unwrap();
        // A folder in the place of a file gives an error that is reported too.
        std::fs::create_dir_all(settings_folder().join(FOLDERS_FILE)).unwrap();
        let problems = storage_problems(app.handle());
        assert!(problems
            .contains(&"1 record in one.json couldn't be read and was left out.".to_string()));
        assert!(problems
            .contains(&"2 records in two.json couldn't be read and were left out.".to_string()));
        assert!(problems
            .iter()
            .any(|text| text.starts_with("workspace.json couldn't be read")));
        assert_eq!(read_workspace(app.handle()).unwrap(), JsonValue::Null);
        assert!(problems.iter().any(|text| text.contains("folders.json")));
    }

    #[test]
    fn the_settings_round_trip() {
        let app = app_with_store();
        write_workspace(app.handle(), serde_json::json!({ "tabs": 2 })).unwrap();
        assert_eq!(read_workspace(app.handle()).unwrap()["tabs"], 2);
        write_file_grants(app.handle(), &["/a.sql".to_string()]).unwrap();
        assert_eq!(read_file_grants(app.handle()).unwrap(), ["/a.sql"]);
    }

    #[test]
    fn the_file_names_are_set() {
        assert_eq!(CONNECTIONS_FILE, "connections.json");
        assert_eq!(QUERIES_FILE, "queries.json");
        assert_eq!(WORKSPACE_FILE, "workspace.json");
        assert_eq!(FOLDERS_FILE, "folders.json");
    }
}
