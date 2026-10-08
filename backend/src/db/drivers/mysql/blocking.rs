//! The report of the sessions that block others on MySQL and MariaDB.
//!
//! A session waits either for a row lock of InnoDB or for a metadata lock,
//! such as the lock that an `ALTER TABLE` needs. MySQL 8 gives the row
//! locks in `performance_schema.data_lock_waits`. MariaDB and MySQL 5.7 give
//! them in `information_schema.INNODB_LOCK_WAITS`. The metadata locks show
//! in `performance_schema.metadata_locks` alone, so they need the
//! performance schema and its instrument of metadata locks.

use super::MysqlDriver;
use crate::db::blocking::{
    count, label, statement_text, BlockingReport, BlockingSession, OpenTransaction, ROW_LIMIT,
};
use crate::error::{Error, Result};
use mysql_async::prelude::*;
use mysql_async::Row;
use std::collections::HashSet;

/// The errors of a missing privilege: a command, a table or a database
/// that the user may not read.
const DENIED: [u16; 4] = [1044, 1142, 1143, 1227];

/// The note of a server whose performance schema is off.
const NO_PERFORMANCE_SCHEMA: &str =
    "performance_schema is off on this server, so metadata lock waits, such as a wait for an \
     ALTER TABLE, don't show.";

/// The note of a server that does not record metadata locks.
const NO_MDL_INSTRUMENT: &str =
    "The wait/lock/metadata/sql/mdl instrument is off on this server, so metadata lock waits, \
     such as a wait for an ALTER TABLE, don't show.";

/// What the report needs to know about the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ServerFacts {
    /// True when the server gives the row locks in `performance_schema`, as
    /// MySQL 8 and later do.
    lock_tables_in_performance_schema: bool,
    /// True when the performance schema is on.
    performance_schema: bool,
}

impl ServerFacts {
    /// Reads the facts from the text of `VERSION()` and the value of
    /// `@@performance_schema`.
    fn of(version: &str, performance_schema: bool) -> Self {
        let major: u32 = version
            .split('.')
            .next()
            .and_then(|major| major.parse().ok())
            .unwrap_or(0);
        Self {
            lock_tables_in_performance_schema: !version.contains("MariaDB") && major >= 8,
            performance_schema,
        }
    }
}

/// The last statement of a thread of the session `id_column`. A session
/// that is idle inside a transaction runs no statement, and this gives the
/// statement that it ran last. Without the performance schema the text is
/// NULL.
fn last_statement(facts: ServerFacts, id_column: &str) -> String {
    if facts.performance_schema {
        format!(
            "(SELECT s.SQL_TEXT FROM performance_schema.threads AS th \
               JOIN performance_schema.events_statements_current AS s \
                 ON s.THREAD_ID = th.THREAD_ID \
               WHERE th.PROCESSLIST_ID = {id_column} LIMIT 1)"
        )
    } else {
        "NULL".to_string()
    }
}

/// One row for each wait for a row lock of InnoDB.
fn row_waits_query(facts: ServerFacts) -> String {
    let last = last_statement(facts, "b.trx_mysql_thread_id");
    let (waits, requesting, blocking, locks, lock_id, object, mode) =
        if facts.lock_tables_in_performance_schema {
            (
                "performance_schema.data_lock_waits",
                "REQUESTING_ENGINE_TRANSACTION_ID",
                "BLOCKING_ENGINE_TRANSACTION_ID",
                "performance_schema.data_locks",
                "l.ENGINE_LOCK_ID = w.REQUESTING_ENGINE_LOCK_ID",
                "CONCAT(l.OBJECT_SCHEMA, '.', l.OBJECT_NAME)",
                "l.LOCK_MODE",
            )
        } else {
            (
                "information_schema.INNODB_LOCK_WAITS",
                "requesting_trx_id",
                "blocking_trx_id",
                "information_schema.INNODB_LOCKS",
                "l.lock_id = w.requested_lock_id",
                "l.lock_table",
                "l.lock_mode",
            )
        };
    format!(
        "SELECT r.trx_mysql_thread_id, r.trx_query, \
           TIMESTAMPDIFF(MICROSECOND, r.trx_wait_started, NOW(6)) DIV 1000, \
           b.trx_mysql_thread_id, p.USER, p.HOST, p.COMMAND, COALESCE(b.trx_query, {last}), \
           {object}, {mode} \
         FROM {waits} AS w \
         JOIN information_schema.INNODB_TRX AS r ON r.trx_id = w.{requesting} \
         JOIN information_schema.INNODB_TRX AS b ON b.trx_id = w.{blocking} \
         LEFT JOIN {locks} AS l ON {lock_id} \
         LEFT JOIN information_schema.PROCESSLIST AS p ON p.ID = b.trx_mysql_thread_id \
         ORDER BY 3 DESC \
         LIMIT {ROW_LIMIT}"
    )
}

/// One row for each pair of a pending metadata lock and a granted lock of
/// another session on the same object. The report keeps the pairs whose
/// lock types conflict, see [`mdl_conflicts`].
fn metadata_waits_query() -> String {
    format!(
        "SELECT wt.PROCESSLIST_ID, wt.PROCESSLIST_INFO, wt.PROCESSLIST_TIME * 1000, \
           gt.PROCESSLIST_ID, gt.PROCESSLIST_USER, gt.PROCESSLIST_HOST, gt.PROCESSLIST_COMMAND, \
           COALESCE(gt.PROCESSLIST_INFO, {last}), \
           CONCAT_WS('.', w.OBJECT_SCHEMA, w.OBJECT_NAME), w.LOCK_TYPE, g.LOCK_TYPE \
         FROM performance_schema.metadata_locks AS w \
         JOIN performance_schema.metadata_locks AS g \
           ON g.OBJECT_TYPE = w.OBJECT_TYPE AND g.OBJECT_SCHEMA <=> w.OBJECT_SCHEMA \
             AND g.OBJECT_NAME <=> w.OBJECT_NAME AND g.LOCK_STATUS = 'GRANTED' \
             AND g.OWNER_THREAD_ID <> w.OWNER_THREAD_ID \
         JOIN performance_schema.threads AS wt ON wt.THREAD_ID = w.OWNER_THREAD_ID \
         JOIN performance_schema.threads AS gt ON gt.THREAD_ID = g.OWNER_THREAD_ID \
         WHERE w.LOCK_STATUS = 'PENDING' AND wt.PROCESSLIST_ID IS NOT NULL \
           AND gt.PROCESSLIST_ID IS NOT NULL \
         ORDER BY 3 DESC \
         LIMIT {ROW_LIMIT}",
        last = last_statement(
            ServerFacts {
                lock_tables_in_performance_schema: true,
                performance_schema: true,
            },
            "gt.PROCESSLIST_ID"
        ),
    )
}

/// The other sessions with an open InnoDB transaction, the oldest first.
fn open_transactions_query(facts: ServerFacts) -> String {
    let last = last_statement(facts, "t.trx_mysql_thread_id");
    format!(
        "SELECT t.trx_mysql_thread_id, p.USER, p.HOST, p.COMMAND, \
           COALESCE(t.trx_query, {last}), TIMESTAMPDIFF(SECOND, t.trx_started, NOW()) \
         FROM information_schema.INNODB_TRX AS t \
         LEFT JOIN information_schema.PROCESSLIST AS p ON p.ID = t.trx_mysql_thread_id \
         WHERE t.trx_mysql_thread_id <> CONNECTION_ID() \
         ORDER BY t.trx_started \
         LIMIT {ROW_LIMIT}"
    )
}

/// The table locks of the metadata lock system in the order of the matrix
/// of `mdl.cc`.
const TABLE_LOCKS: [&str; 10] = [
    "SHARED",
    "SHARED_HIGH_PRIO",
    "SHARED_READ",
    "SHARED_WRITE",
    "SHARED_WRITE_LOW_PRIO",
    "SHARED_UPGRADABLE",
    "SHARED_READ_ONLY",
    "SHARED_NO_WRITE",
    "SHARED_NO_READ_WRITE",
    "EXCLUSIVE",
];

/// For each requested lock of [`TABLE_LOCKS`], a `+` for each granted lock
/// that it can share the object with.
const TABLE_MATRIX: [&str; 10] = [
    "+++++++++-",
    "+++++++++-",
    "++++++++--",
    "++++++----",
    "++++++----",
    "+++++-+---",
    "+++--+++--",
    "+++---+---",
    "++--------",
    "----------",
];

/// True when a requested metadata lock must wait for a granted one. The
/// locks of a scope, such as the global read lock, use
/// `INTENTION_EXCLUSIVE`, which shares the scope with itself alone. A lock
/// type that this table does not know counts as a conflict.
fn mdl_conflicts(requested: &str, granted: &str) -> bool {
    const INTENTION: &str = "INTENTION_EXCLUSIVE";
    if requested == INTENTION || granted == INTENTION {
        return !(requested == INTENTION && granted == INTENTION);
    }
    let index = |name: &str| TABLE_LOCKS.iter().position(|lock| *lock == name);
    match (index(requested), index(granted)) {
        (Some(row), Some(column)) => TABLE_MATRIX[row].as_bytes()[column] == b'-',
        _ => true,
    }
}

impl MysqlDriver {
    /// Reads the waits for locks and the open transactions of the server.
    /// A part that the user may not read adds a note in place of its rows.
    pub(super) async fn blocking_report(&mut self) -> Result<BlockingReport> {
        let conn = self.conn()?;
        let (version, performance_schema): (String, i64) = conn
            .query_first("SELECT VERSION(), @@performance_schema")
            .await?
            .unwrap_or_default();
        let facts = ServerFacts::of(&version, performance_schema == 1);
        let mut report = BlockingReport::default();

        if facts.lock_tables_in_performance_schema && !facts.performance_schema {
            report.notes.push(
                "performance_schema is off on this server, so row lock waits don't show."
                    .to_string(),
            );
        } else if let Some(rows) = allowed(
            conn.query(row_waits_query(facts)).await,
            "Row lock waits",
            &mut report,
        )? {
            report.sessions.extend(rows.iter().map(wait_row));
        }

        if !facts.performance_schema {
            report.notes.push(NO_PERFORMANCE_SCHEMA.to_string());
        } else {
            let enabled: Option<String> = allowed(
                conn.query_first(
                    "SELECT ENABLED FROM performance_schema.setup_instruments \
                     WHERE NAME = 'wait/lock/metadata/sql/mdl'",
                )
                .await,
                "Metadata lock waits",
                &mut report,
            )?
            .flatten();
            if enabled.as_deref() == Some("YES") {
                if let Some(rows) = allowed(
                    conn.query(metadata_waits_query()).await,
                    "Metadata lock waits",
                    &mut report,
                )? {
                    report.sessions.extend(metadata_waits(&rows));
                }
            } else if enabled.is_some() {
                report.notes.push(NO_MDL_INSTRUMENT.to_string());
            }
        }

        if let Some(rows) = allowed(
            conn.query(open_transactions_query(facts)).await,
            "Open transactions",
            &mut report,
        )? {
            report.open_transactions = rows.iter().map(open_row).collect();
        }
        report.sessions.truncate(ROW_LIMIT);
        Ok(report)
    }
}

/// Gives the result of a part of the report. A missing privilege gives
/// `None` and a note with the text of the server. Any other error stops the
/// report.
fn allowed<T>(
    result: std::result::Result<T, mysql_async::Error>,
    part: &str,
    report: &mut BlockingReport,
) -> Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(mysql_async::Error::Server(server)) if DENIED.contains(&server.code) => {
            report
                .notes
                .push(format!("{part} aren't shown: {}", server.message));
            Ok(None)
        }
        Err(error) => Err(Error::from(error)),
    }
}

/// The waits of the rows of [`metadata_waits_query`] whose locks conflict.
/// Two conflicting locks of one session on one object give one row.
fn metadata_waits(rows: &[Row]) -> Vec<BlockingSession> {
    let mut seen = HashSet::new();
    rows.iter()
        .filter(|row| {
            let requested = text(row, 9).unwrap_or_default();
            let granted = text(row, 10).unwrap_or_default();
            mdl_conflicts(&requested, &granted)
        })
        .map(wait_row)
        .filter(|wait| {
            seen.insert((
                wait.waiting_session,
                wait.blocking_session,
                wait.object.clone(),
            ))
        })
        .collect()
}

fn text(row: &Row, index: usize) -> Option<String> {
    let value: Option<String> = row.get_opt(index).and_then(|value| value.ok()).flatten();
    label(value.as_deref())
}

fn number(row: &Row, index: usize) -> Option<i64> {
    row.get_opt(index).and_then(|value| value.ok()).flatten()
}

fn statement(row: &Row, index: usize) -> Option<String> {
    let value: Option<String> = row.get_opt(index).and_then(|value| value.ok()).flatten();
    statement_text(value.as_deref())
}

fn wait_row(row: &Row) -> BlockingSession {
    BlockingSession {
        waiting_session: count(number(row, 0)).unwrap_or_default(),
        waiting_statement: statement(row, 1),
        wait_ms: count(number(row, 2)),
        blocking_session: count(number(row, 3)).unwrap_or_default(),
        blocking_login: text(row, 4),
        blocking_host: text(row, 5),
        blocking_program: None,
        blocking_status: text(row, 6),
        blocking_statement: statement(row, 7),
        object: text(row, 8),
        lock_mode: text(row, 9),
    }
}

fn open_row(row: &Row) -> OpenTransaction {
    OpenTransaction {
        session: count(number(row, 0)).unwrap_or_default(),
        login: text(row, 1),
        host: text(row, 2),
        program: None,
        status: text(row, 3),
        statement: statement(row, 4),
        open_secs: count(number(row, 5)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_selects_the_tables_of_the_row_locks() {
        let mysql = ServerFacts::of("8.4.11", true);
        assert!(mysql.lock_tables_in_performance_schema);
        assert!(row_waits_query(mysql).contains("data_lock_waits"));
        assert!(row_waits_query(mysql).contains("events_statements_current"));

        let maria = ServerFacts::of("11.4.12-MariaDB-ubu2404", false);
        assert!(!maria.lock_tables_in_performance_schema);
        assert!(row_waits_query(maria).contains("INNODB_LOCK_WAITS"));
        assert!(!row_waits_query(maria).contains("events_statements_current"));

        assert!(!ServerFacts::of("5.7.44", true).lock_tables_in_performance_schema);
        assert!(!ServerFacts::of("", true).lock_tables_in_performance_schema);
        assert!(open_transactions_query(maria).contains("CONNECTION_ID()"));
        assert!(metadata_waits_query().contains("'PENDING'"));
    }

    #[test]
    fn the_matrix_of_metadata_locks_follows_the_server() {
        // A read shares a table with another read and with an open
        // transaction that read it, but an ALTER TABLE waits for both.
        assert!(!mdl_conflicts("SHARED_READ", "SHARED_READ"));
        assert!(mdl_conflicts("EXCLUSIVE", "SHARED_READ"));
        assert!(mdl_conflicts("SHARED_READ", "EXCLUSIVE"));
        assert!(mdl_conflicts("SHARED_READ", "SHARED_NO_READ_WRITE"));
        assert!(!mdl_conflicts("SHARED_HIGH_PRIO", "SHARED_NO_READ_WRITE"));
        assert!(mdl_conflicts("SHARED_WRITE", "SHARED_READ_ONLY"));
        assert!(mdl_conflicts("SHARED_UPGRADABLE", "SHARED_UPGRADABLE"));
        assert!(!mdl_conflicts("SHARED_UPGRADABLE", "SHARED_WRITE"));
        // The locks of a scope.
        assert!(!mdl_conflicts("INTENTION_EXCLUSIVE", "INTENTION_EXCLUSIVE"));
        assert!(mdl_conflicts("INTENTION_EXCLUSIVE", "SHARED"));
        assert!(mdl_conflicts("SHARED", "INTENTION_EXCLUSIVE"));
        assert!(!mdl_conflicts("SHARED", "SHARED"));
        assert!(mdl_conflicts("UNKNOWN", "SHARED"));
        for (row, line) in TABLE_MATRIX.iter().enumerate() {
            assert_eq!(line.len(), TABLE_LOCKS.len(), "row {row}");
        }
    }

    #[test]
    fn a_missing_privilege_gives_a_note() {
        let mut report = BlockingReport::default();
        let denied = mysql_async::Error::Server(mysql_async::ServerError {
            code: 1227,
            message: "Access denied; you need the PROCESS privilege".into(),
            state: "42000".into(),
        });
        let rows: Option<()> = allowed(Err(denied), "Row lock waits", &mut report).unwrap();
        assert_eq!(rows, None);
        assert_eq!(
            report.notes,
            vec!["Row lock waits aren't shown: Access denied; you need the PROCESS privilege"]
        );

        let other = mysql_async::Error::Server(mysql_async::ServerError {
            code: 1146,
            message: "Table doesn't exist".into(),
            state: "42S02".into(),
        });
        assert!(allowed::<()>(Err(other), "Row lock waits", &mut report).is_err());
        assert_eq!(allowed(Ok(5), "x", &mut report).unwrap(), Some(5));
    }
}
