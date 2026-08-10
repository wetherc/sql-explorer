//! The records the application keeps about the statements the user ran and
//! the statements the user saved.

use serde::{Deserialize, Serialize};

/// The number of entries the history keeps. An older entry is dropped.
pub const HISTORY_LIMIT: usize = 500;

/// One statement the user ran.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: String,
    pub connection_id: String,
    pub connection_name: String,
    pub query: String,
    /// The moment the statement ran, as text in the RFC 3339 form.
    pub ran_at: String,
    pub elapsed_ms: u64,
    pub row_count: usize,
    pub succeeded: bool,
    #[serde(default)]
    pub error: Option<String>,
}

/// One statement the user saved under a name.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SavedQuery {
    pub id: String,
    pub name: String,
    pub query: String,
    #[serde(default)]
    pub connection_id: Option<String>,
    #[serde(default)]
    pub folder: Option<String>,
    pub updated_at: String,
}

/// Adds an entry to the front of the history and drops the entries above
/// the limit. An entry that repeats the statement at the front replaces it,
/// so that a statement the user ran twice does not fill the list.
pub fn push_entry(history: &mut Vec<HistoryEntry>, entry: HistoryEntry) {
    if let Some(first) = history.first() {
        if first.query == entry.query && first.connection_id == entry.connection_id {
            history.remove(0);
        }
    }
    history.insert(0, entry);
    history.truncate(HISTORY_LIMIT);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, query: &str) -> HistoryEntry {
        HistoryEntry {
            id: id.into(),
            connection_id: "c1".into(),
            connection_name: "Server".into(),
            query: query.into(),
            ran_at: "2026-08-10T00:00:00Z".into(),
            elapsed_ms: 5,
            row_count: 1,
            succeeded: true,
            error: None,
        }
    }

    #[test]
    fn a_new_entry_goes_to_the_front() {
        let mut history = vec![entry("1", "SELECT 1")];
        push_entry(&mut history, entry("2", "SELECT 2"));
        assert_eq!(history[0].id, "2");
        assert_eq!(history[1].id, "1");
    }

    #[test]
    fn a_repeated_statement_replaces_the_one_at_the_front() {
        let mut history = vec![entry("1", "SELECT 1")];
        push_entry(&mut history, entry("2", "SELECT 1"));
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].id, "2");
    }

    #[test]
    fn a_repeated_statement_on_another_connection_is_a_new_entry() {
        let mut history = vec![entry("1", "SELECT 1")];
        let mut next = entry("2", "SELECT 1");
        next.connection_id = "c2".into();
        push_entry(&mut history, next);
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn the_history_stops_at_the_limit() {
        let mut history: Vec<HistoryEntry> = Vec::new();
        for index in 0..(HISTORY_LIMIT + 10) {
            push_entry(
                &mut history,
                entry(&index.to_string(), &format!("SELECT {index}")),
            );
        }
        assert_eq!(history.len(), HISTORY_LIMIT);
        assert_eq!(history[0].id, (HISTORY_LIMIT + 9).to_string());
    }

    #[test]
    fn an_entry_that_failed_carries_the_reason() {
        let mut failed = entry("1", "SELECT bad");
        failed.succeeded = false;
        failed.error = Some("column not found".into());
        let text = serde_json::to_string(&failed).unwrap();
        assert!(text.contains("connectionId"));
        assert_eq!(serde_json::from_str::<HistoryEntry>(&text).unwrap(), failed);
    }

    #[test]
    fn a_saved_query_round_trips_through_json() {
        let query = SavedQuery {
            id: "q1".into(),
            name: "Daily count".into(),
            query: "SELECT COUNT(*) FROM t".into(),
            connection_id: Some("c1".into()),
            folder: Some("Reports".into()),
            updated_at: "2026-08-10T00:00:00Z".into(),
        };
        let text = serde_json::to_string(&query).unwrap();
        assert_eq!(serde_json::from_str::<SavedQuery>(&text).unwrap(), query);

        let minimal: SavedQuery =
            serde_json::from_str(r#"{"id":"a","name":"b","query":"c","updatedAt":"d"}"#).unwrap();
        assert_eq!(minimal.connection_id, None);
        assert_eq!(minimal.folder, None);
    }
}
