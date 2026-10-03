//! The files that hold the settings, the history and the saved queries.
//!
//! Every read is tolerant: one record that cannot be understood is written
//! to the log and left out, so a single damaged entry does not stop the
//! whole list from loading.

use crate::error::{Error, Result};
use crate::history::{push_entry, trim_history, HistoryEntry, SavedQuery};
use crate::storage::SavedConnection;
use serde::de::DeserializeOwned;
use serde_json::Value as JsonValue;
use std::path::PathBuf;
use tauri::{AppHandle, Runtime};
use tauri_plugin_store::StoreExt;

/// The file that holds the saved connections.
pub const CONNECTIONS_FILE: &str = "connections.json";
/// The file that holds the history and the saved queries.
pub const QUERIES_FILE: &str = "queries.json";
/// The file that holds the open tabs.
pub const WORKSPACE_FILE: &str = "workspace.json";
/// The file that holds the folders and the single files the user accepted.
pub const FOLDERS_FILE: &str = "folders.json";

/// One change of the history or of the saved queries at a time.
///
/// Each change reads a list, changes it and writes it back. Two runs that end
/// together call the command on two threads, and without the lock the second
/// write drops the entry of the first.
static QUERIES_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Reads one list of the file of the queries, changes it and writes it back,
/// under the lock of that file.
fn edit_queries<R: Runtime, T, F>(app: &AppHandle<R>, key: &str, change: F) -> Result<()>
where
    T: DeserializeOwned + serde::Serialize,
    F: FnOnce(&mut Vec<T>),
{
    let _edit = QUERIES_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let store = app.store(settings_path(QUERIES_FILE))?;
    let mut list: Vec<T> = parse_list(store.get(key));
    change(&mut list);
    store.set(key, serde_json::to_value(&list)?);
    store.save()?;
    Ok(())
}

const HISTORY_KEY: &str = "history";
const SAVED_KEY: &str = "saved";
const WORKSPACE_KEY: &str = "workspace";
const ROOTS_KEY: &str = "roots";
const GRANTS_KEY: &str = "files";

/// The path of one file of the settings.
///
/// A relative path lands in the data folder of the application, which is what
/// the release build uses. A test names a folder of its own, so no test reads
/// or writes the files of the real application.
#[cfg(not(test))]
fn settings_path(name: &str) -> PathBuf {
    PathBuf::from(name)
}

#[cfg(test)]
fn settings_path(name: &str) -> PathBuf {
    tests::settings_folder().join(name)
}

/// Reads every value of a file and drops the records that cannot be
/// understood.
fn parse_values<T: DeserializeOwned>(values: Vec<(String, JsonValue)>) -> Vec<T> {
    values
        .into_iter()
        .filter_map(|(key, value)| match serde_json::from_value::<T>(value) {
            Ok(parsed) => Some(parsed),
            Err(error) => {
                log::warn!("The stored record '{key}' was left out: {error}");
                None
            }
        })
        .collect()
}

/// Reads a list out of one key of a file, and drops the entries that
/// cannot be understood.
fn parse_list<T: DeserializeOwned>(value: Option<JsonValue>) -> Vec<T> {
    let Some(JsonValue::Array(items)) = value else {
        return Vec::new();
    };
    items
        .into_iter()
        .filter_map(|item| match serde_json::from_value::<T>(item) {
            Ok(parsed) => Some(parsed),
            Err(error) => {
                log::warn!("A stored entry was left out: {error}");
                None
            }
        })
        .collect()
}

pub fn read_connections<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<SavedConnection>> {
    let store = app.store(settings_path(CONNECTIONS_FILE))?;
    let values: Vec<(String, JsonValue)> = store.entries();
    let mut connections: Vec<SavedConnection> = parse_values(values);
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
    let store = app.store(settings_path(CONNECTIONS_FILE))?;
    store.set(connection.id.clone(), serde_json::to_value(connection)?);
    store.save()?;
    Ok(())
}

pub fn delete_connection<R: Runtime>(app: &AppHandle<R>, id: &str) -> Result<()> {
    let store = app.store(settings_path(CONNECTIONS_FILE))?;
    store.delete(id);
    store.save()?;
    Ok(())
}

/// Reads the history. A file that holds more than the limits allow gives the
/// newer entries alone, and the next write of the file drops the others.
pub fn read_history<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<HistoryEntry>> {
    let store = app.store(settings_path(QUERIES_FILE))?;
    let mut history = parse_list(store.get(HISTORY_KEY));
    trim_history(&mut history);
    Ok(history)
}

/// Writes one entry to the history file. The caller keeps its own copy of the
/// list, so the function gives no list back. A large history then stays out of
/// the answer of each execution.
pub fn add_history<R: Runtime>(app: &AppHandle<R>, entry: HistoryEntry) -> Result<()> {
    edit_queries(app, HISTORY_KEY, |history: &mut Vec<HistoryEntry>| {
        push_entry(history, entry)
    })
}

pub fn clear_history<R: Runtime>(app: &AppHandle<R>) -> Result<()> {
    edit_queries(app, HISTORY_KEY, |history: &mut Vec<HistoryEntry>| {
        history.clear()
    })
}

pub fn read_saved_queries<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<SavedQuery>> {
    let store = app.store(settings_path(QUERIES_FILE))?;
    let mut queries: Vec<SavedQuery> = parse_list(store.get(SAVED_KEY));
    queries.sort_by_key(|query| query.name.to_lowercase());
    Ok(queries)
}

pub fn write_saved_query<R: Runtime>(app: &AppHandle<R>, query: &SavedQuery) -> Result<()> {
    if query.id.trim().is_empty() {
        return Err(Error::Configuration(
            "A saved statement needs an ID.".to_string(),
        ));
    }
    edit_queries(
        app,
        SAVED_KEY,
        |queries: &mut Vec<SavedQuery>| match queries.iter_mut().find(|item| item.id == query.id) {
            Some(existing) => *existing = query.clone(),
            None => queries.push(query.clone()),
        },
    )
}

pub fn delete_saved_query<R: Runtime>(app: &AppHandle<R>, id: &str) -> Result<()> {
    edit_queries(app, SAVED_KEY, |queries: &mut Vec<SavedQuery>| {
        queries.retain(|item| item.id != id)
    })
}

/// Reads the folders that the user accepted in an earlier session.
///
/// This file belongs to the backend. No command writes it with a path that
/// the interface chose, so a folder reaches the list only after the user
/// accepted it in a dialog of the operating system.
pub fn read_file_roots<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<String>> {
    let store = app.store(settings_path(FOLDERS_FILE))?;
    Ok(parse_list(store.get(ROOTS_KEY)))
}

/// Writes the folders that the user accepted.
pub fn write_file_roots<R: Runtime>(app: &AppHandle<R>, roots: &[String]) -> Result<()> {
    let store = app.store(settings_path(FOLDERS_FILE))?;
    store.set(ROOTS_KEY, serde_json::to_value(roots)?);
    store.save()?;
    Ok(())
}

/// Reads the single files that the user accepted in an earlier session. The
/// same rule as for the folders applies: only a dialog of the operating
/// system adds a file to this list.
pub fn read_file_grants<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<String>> {
    let store = app.store(settings_path(FOLDERS_FILE))?;
    Ok(parse_list(store.get(GRANTS_KEY)))
}

/// Writes the single files that the user accepted.
pub fn write_file_grants<R: Runtime>(app: &AppHandle<R>, files: &[String]) -> Result<()> {
    let store = app.store(settings_path(FOLDERS_FILE))?;
    store.set(GRANTS_KEY, serde_json::to_value(files)?);
    store.save()?;
    Ok(())
}

pub fn read_workspace<R: Runtime>(app: &AppHandle<R>) -> Result<JsonValue> {
    let store = app.store(settings_path(WORKSPACE_FILE))?;
    Ok(store.get(WORKSPACE_KEY).unwrap_or(JsonValue::Null))
}

pub fn write_workspace<R: Runtime>(app: &AppHandle<R>, workspace: JsonValue) -> Result<()> {
    let store = app.store(settings_path(WORKSPACE_FILE))?;
    store.set(WORKSPACE_KEY, workspace);
    store.save()?;
    Ok(())
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
                    "sql-explorer-settings-{:?}",
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
        let parsed: Vec<SavedConnection> = parse_values(values);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].id, "a");
        assert_eq!(parsed[0].db_type, DbType::Sqlite);
        assert_eq!(parsed[1].id, "c");
    }

    #[test]
    fn an_empty_file_gives_an_empty_list() {
        let parsed: Vec<SavedConnection> = parse_values(Vec::new());
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
        let entries: Vec<HistoryEntry> = parse_list(Some(value));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "1");
    }

    #[test]
    fn a_value_that_is_not_a_list_gives_an_empty_list() {
        let entries: Vec<HistoryEntry> = parse_list(None);
        assert!(entries.is_empty());
        let entries: Vec<HistoryEntry> = parse_list(Some(serde_json::json!("text")));
        assert!(entries.is_empty());
    }

    fn app_with_store() -> tauri::App<tauri::test::MockRuntime> {
        tauri::test::mock_builder()
            .plugin(tauri_plugin_store::Builder::default().build())
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

    fn saved(id: &str, name: &str) -> SavedQuery {
        SavedQuery {
            id: id.to_string(),
            name: name.to_string(),
            query: "SELECT 1".to_string(),
            connection_id: None,
            folder: None,
            updated_at: "2026-01-01T00:00:00Z".to_string(),
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
    fn a_saved_query_is_added_replaced_and_deleted() {
        let app = app_with_store();
        write_saved_query(app.handle(), &saved("a", "Beta")).unwrap();
        write_saved_query(app.handle(), &saved("b", "Alpha")).unwrap();
        write_saved_query(app.handle(), &saved("a", "Gamma")).unwrap();
        let names: Vec<String> = read_saved_queries(app.handle())
            .unwrap()
            .into_iter()
            .map(|query| query.name)
            .collect();
        assert_eq!(names, ["Alpha", "Gamma"]);

        delete_saved_query(app.handle(), "b").unwrap();
        assert_eq!(read_saved_queries(app.handle()).unwrap().len(), 1);
        assert!(write_saved_query(app.handle(), &saved(" ", "Blank")).is_err());
    }

    #[test]
    fn the_file_names_are_set() {
        assert_eq!(CONNECTIONS_FILE, "connections.json");
        assert_eq!(QUERIES_FILE, "queries.json");
        assert_eq!(WORKSPACE_FILE, "workspace.json");
        assert_eq!(FOLDERS_FILE, "folders.json");
    }
}
