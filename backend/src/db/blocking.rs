//! The report of the sessions that block other sessions with their locks.
//!
//! A read of the explorer that waits too long for a lock stops with
//! [`crate::error::Error::LockWait`]. The report then tells the user which
//! session keeps the lock. Each driver that can read the lock tables of its
//! server fills the report through
//! [`crate::db::drivers::DatabaseDriver::blocking_sessions`].

use serde::Serialize;

/// The most characters of a statement that a report gives. A long batch
/// would make the dialog of the report hard to read.
pub const STATEMENT_LIMIT: usize = 1000;

/// The most rows that each list of a report gives.
pub const ROW_LIMIT: usize = 200;

/// The sessions that wait for locks, and the sessions that keep locks.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockingReport {
    /// One row for each wait: a session that waits for a lock, and the
    /// session that keeps that lock. A session that waits for the locks of
    /// two sessions gives two rows.
    pub sessions: Vec<BlockingSession>,
    /// The other sessions that are inside a transaction and keep locks, the
    /// oldest transaction first. After a read of the explorer stopped, no
    /// session can wait for the lock that stopped it, but the session that
    /// keeps the lock is in this list.
    pub open_transactions: Vec<OpenTransaction>,
    /// The limits of the report, such as a missing permission or a part of
    /// the server that is off.
    pub notes: Vec<String>,
}

impl BlockingReport {
    /// Keeps the rows of one blocking session alone: the waits that it
    /// causes, and its own open transaction.
    pub fn blocked_by(mut self, session: u64) -> Self {
        self.sessions.retain(|row| row.blocking_session == session);
        self.open_transactions.retain(|row| row.session == session);
        self
    }
}

/// One wait for a lock.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockingSession {
    /// The session that waits.
    pub waiting_session: u64,
    /// The statement that waits.
    pub waiting_statement: Option<String>,
    /// How long the statement has waited, in milliseconds.
    pub wait_ms: Option<u64>,
    /// The session that keeps the lock.
    pub blocking_session: u64,
    pub blocking_login: Option<String>,
    /// The client computer of the blocking session.
    pub blocking_host: Option<String>,
    /// The program that opened the blocking session.
    pub blocking_program: Option<String>,
    /// The state of the blocking session as the server names it, such as
    /// `sleeping` or `idle in transaction`.
    pub blocking_status: Option<String>,
    /// The statement that the blocking session runs, or the last one it ran.
    pub blocking_statement: Option<String>,
    /// The object of the lock, such as a table.
    pub object: Option<String>,
    /// The lock that the waiting session asked for, as the server names it.
    pub lock_mode: Option<String>,
}

/// One session inside a transaction that keeps locks.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenTransaction {
    pub session: u64,
    pub login: Option<String>,
    pub host: Option<String>,
    pub program: Option<String>,
    pub status: Option<String>,
    /// The statement that the session runs, or the last one it ran.
    pub statement: Option<String>,
    /// How long the transaction has been open, in seconds.
    pub open_secs: Option<u64>,
}

/// Cleans a statement text for the report. A text with nothing but white
/// space gives `None`, and a long text is cut at [`STATEMENT_LIMIT`]
/// characters.
pub fn statement_text(text: Option<&str>) -> Option<String> {
    let text = text?.trim();
    if text.is_empty() {
        return None;
    }
    match text.char_indices().nth(STATEMENT_LIMIT) {
        Some((end, _)) => Some(format!("{}...", &text[..end])),
        None => Some(text.to_string()),
    }
}

/// Cleans a short text, such as a login or a host. Empty text gives `None`.
pub fn label(text: Option<&str>) -> Option<String> {
    text.map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

/// Gives a count of the server as a number of the report. A negative value,
/// such as the age of a transaction that started after the clock of the
/// query, gives `None`.
pub fn count(value: Option<i64>) -> Option<u64> {
    value.and_then(|value| u64::try_from(value).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait(waiting: u64, blocking: u64) -> BlockingSession {
        BlockingSession {
            waiting_session: waiting,
            blocking_session: blocking,
            ..BlockingSession::default()
        }
    }

    fn open(session: u64) -> OpenTransaction {
        OpenTransaction {
            session,
            ..OpenTransaction::default()
        }
    }

    #[test]
    fn the_filter_keeps_the_rows_of_one_blocking_session() {
        let report = BlockingReport {
            sessions: vec![wait(1, 7), wait(2, 8), wait(3, 7)],
            open_transactions: vec![open(7), open(8)],
            notes: vec!["note".into()],
        };
        let kept = report.blocked_by(7);
        assert_eq!(kept.sessions, vec![wait(1, 7), wait(3, 7)]);
        assert_eq!(kept.open_transactions, vec![open(7)]);
        assert_eq!(kept.notes, vec!["note".to_string()]);
    }

    #[test]
    fn a_statement_is_trimmed_and_cut() {
        assert_eq!(statement_text(None), None);
        assert_eq!(statement_text(Some("  \n ")), None);
        assert_eq!(
            statement_text(Some(" SELECT 1 ")).as_deref(),
            Some("SELECT 1")
        );
        let long = "é".repeat(STATEMENT_LIMIT + 5);
        let cut = statement_text(Some(&long)).unwrap();
        assert_eq!(cut.chars().count(), STATEMENT_LIMIT + 3);
        assert!(cut.ends_with("..."));
        let exact = "x".repeat(STATEMENT_LIMIT);
        assert_eq!(statement_text(Some(&exact)), Some(exact));
    }

    #[test]
    fn a_label_and_a_count_drop_empty_values() {
        assert_eq!(label(Some(" sa ")).as_deref(), Some("sa"));
        assert_eq!(label(Some("")), None);
        assert_eq!(label(None), None);
        assert_eq!(count(Some(5)), Some(5));
        assert_eq!(count(Some(-1)), None);
        assert_eq!(count(None), None);
    }

    #[test]
    fn the_report_uses_camel_case() {
        let report = BlockingReport {
            sessions: vec![wait(1, 2)],
            open_transactions: vec![open(2)],
            notes: Vec::new(),
        };
        let value = serde_json::to_value(report).unwrap();
        assert_eq!(value["sessions"][0]["waitingSession"], 1);
        assert_eq!(value["sessions"][0]["blockingSession"], 2);
        assert_eq!(value["openTransactions"][0]["session"], 2);
        assert!(value["openTransactions"][0]["openSecs"].is_null());
    }
}
