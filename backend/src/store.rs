//! The files that hold the settings, the history and the saved queries.
//!
//! Every read is tolerant: one record that cannot be understood is written
//! to the log and left out, so a single damaged entry does not stop the
//! whole list from loading.

use crate::error::{Error, Result};
use crate::history::{push_entry, HistoryEntry, SavedQuery};
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

const HISTORY_KEY: &str = "history";
const SAVED_KEY: &str = "saved";
const WORKSPACE_KEY: &str = "workspace";

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
    let store = app.store(PathBuf::from(CONNECTIONS_FILE))?;
    let values: Vec<(String, JsonValue)> = store.entries();
    let mut connections: Vec<SavedConnection> = parse_values(values);
    connections.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(connections)
}

pub fn write_connection<R: Runtime>(
    app: &AppHandle<R>,
    connection: &SavedConnection,
) -> Result<()> {
    let store = app.store(PathBuf::from(CONNECTIONS_FILE))?;
    store.set(connection.id.clone(), serde_json::to_value(connection)?);
    store.save()?;
    Ok(())
}

pub fn delete_connection<R: Runtime>(app: &AppHandle<R>, id: &str) -> Result<()> {
    let store = app.store(PathBuf::from(CONNECTIONS_FILE))?;
    store.delete(id);
    store.save()?;
    Ok(())
}

pub fn read_history<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<HistoryEntry>> {
    let store = app.store(PathBuf::from(QUERIES_FILE))?;
    Ok(parse_list(store.get(HISTORY_KEY)))
}

pub fn add_history<R: Runtime>(
    app: &AppHandle<R>,
    entry: HistoryEntry,
) -> Result<Vec<HistoryEntry>> {
    let store = app.store(PathBuf::from(QUERIES_FILE))?;
    let mut history: Vec<HistoryEntry> = parse_list(store.get(HISTORY_KEY));
    push_entry(&mut history, entry);
    store.set(HISTORY_KEY, serde_json::to_value(&history)?);
    store.save()?;
    Ok(history)
}

pub fn clear_history<R: Runtime>(app: &AppHandle<R>) -> Result<()> {
    let store = app.store(PathBuf::from(QUERIES_FILE))?;
    store.set(HISTORY_KEY, JsonValue::Array(Vec::new()));
    store.save()?;
    Ok(())
}

pub fn read_saved_queries<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<SavedQuery>> {
    let store = app.store(PathBuf::from(QUERIES_FILE))?;
    let mut queries: Vec<SavedQuery> = parse_list(store.get(SAVED_KEY));
    queries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(queries)
}

pub fn write_saved_query<R: Runtime>(app: &AppHandle<R>, query: &SavedQuery) -> Result<()> {
    if query.id.trim().is_empty() {
        return Err(Error::Configuration(
            "A saved statement needs an identifier.".to_string(),
        ));
    }
    let store = app.store(PathBuf::from(QUERIES_FILE))?;
    let mut queries: Vec<SavedQuery> = parse_list(store.get(SAVED_KEY));
    match queries.iter_mut().find(|item| item.id == query.id) {
        Some(existing) => *existing = query.clone(),
        None => queries.push(query.clone()),
    }
    store.set(SAVED_KEY, serde_json::to_value(&queries)?);
    store.save()?;
    Ok(())
}

pub fn delete_saved_query<R: Runtime>(app: &AppHandle<R>, id: &str) -> Result<()> {
    let store = app.store(PathBuf::from(QUERIES_FILE))?;
    let mut queries: Vec<SavedQuery> = parse_list(store.get(SAVED_KEY));
    queries.retain(|item| item.id != id);
    store.set(SAVED_KEY, serde_json::to_value(&queries)?);
    store.save()?;
    Ok(())
}

pub fn read_workspace<R: Runtime>(app: &AppHandle<R>) -> Result<JsonValue> {
    let store = app.store(PathBuf::from(WORKSPACE_FILE))?;
    Ok(store.get(WORKSPACE_KEY).unwrap_or(JsonValue::Null))
}

pub fn write_workspace<R: Runtime>(app: &AppHandle<R>, workspace: JsonValue) -> Result<()> {
    let store = app.store(PathBuf::from(WORKSPACE_FILE))?;
    store.set(WORKSPACE_KEY, workspace);
    store.save()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::DbType;

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

    #[test]
    fn the_file_names_are_set() {
        assert_eq!(CONNECTIONS_FILE, "connections.json");
        assert_eq!(QUERIES_FILE, "queries.json");
        assert_eq!(WORKSPACE_FILE, "workspace.json");
    }
}
