//! The records the application keeps about the statements the user ran.

use serde::{Deserialize, Serialize};

/// The number of entries the history keeps. An older entry is dropped.
pub const HISTORY_LIMIT: usize = 500;

/// The most text that the statements and the error texts of the history hold
/// together, in UTF-16 units. The backend writes the whole file again after
/// each run and sends the whole list to the interface, so 500 runs of a script
/// of 1 MB would otherwise make a file of about 500 MB. The interface counts
/// the same units, so the two lists hold the same entries.
pub const HISTORY_TEXT_BUDGET: usize = 4 * 1024 * 1024;

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
    trim_history(history);
}

/// The text of one entry that counts against the budget of the history.
fn text_weight(entry: &HistoryEntry) -> usize {
    let error = entry.error.as_deref().unwrap_or_default();
    entry.query.encode_utf16().count() + error.encode_utf16().count()
}

/// Drops the entries above the limit, and the older entries whose text would
/// pass the budget. The newest entry stays also when its text alone passes
/// the budget, so the last run is always in the history.
pub fn trim_history(history: &mut Vec<HistoryEntry>) {
    history.truncate(HISTORY_LIMIT);
    let mut total = 0;
    let keep = history
        .iter()
        .position(|entry| {
            total += text_weight(entry);
            total > HISTORY_TEXT_BUDGET
        })
        .unwrap_or(history.len())
        .max(1);
    history.truncate(keep);
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
    fn the_history_stops_at_the_budget_of_its_text() {
        let quarter = "x".repeat(HISTORY_TEXT_BUDGET / 4);
        let mut history: Vec<HistoryEntry> = Vec::new();
        for index in 0..6 {
            push_entry(
                &mut history,
                entry(&index.to_string(), &format!("{index}{quarter}")),
            );
        }
        // Each entry holds a little more than a quarter of the budget, so
        // three entries fit.
        let ids: Vec<&str> = history.iter().map(|entry| entry.id.as_str()).collect();
        assert_eq!(ids, ["5", "4", "3"]);

        // The error text counts against the budget too.
        let mut failed = entry("6", "SELECT 1");
        failed.error = Some(quarter.repeat(3));
        push_entry(&mut history, failed);
        let ids: Vec<&str> = history.iter().map(|entry| entry.id.as_str()).collect();
        assert_eq!(ids, ["6"]);
    }

    #[test]
    fn the_newest_entry_stays_when_its_text_passes_the_budget() {
        let mut history = vec![entry("1", "SELECT 1")];
        let huge = "é".repeat(HISTORY_TEXT_BUDGET + 1);
        push_entry(&mut history, entry("2", &huge));
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].id, "2");

        let mut empty = Vec::new();
        trim_history(&mut empty);
        assert!(empty.is_empty());
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
}
