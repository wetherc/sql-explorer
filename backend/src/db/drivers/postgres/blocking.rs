//! The report of the sessions that block others on PostgreSQL.

use super::PostgresDriver;
use crate::db::blocking::{
    count, label, statement_text, BlockingReport, BlockingSession, OpenTransaction, ROW_LIMIT,
};
use crate::error::Result;
use tokio_postgres::Row;

/// The text that `pg_stat_activity` gives in place of the statement of a
/// session of another role, for a role without the right to read it.
const HIDDEN_STATEMENT: &str = "<insufficient privilege>";

/// The note of a role that can't read the activity of other roles.
const NO_READ_ALL_STATS: &str = "Statements, states and times of other users' sessions are \
     hidden. Superusers and members of pg_read_all_stats can see them.";

/// Gives true when the role can read the activity of every session.
const PRIVILEGE_QUERY: &str = "SELECT rolsuper OR pg_has_role(current_user, \
     'pg_read_all_stats', 'MEMBER') FROM pg_roles WHERE rolname = current_user";

/// The first version whose `pg_locks` has the start of each wait.
const WAIT_START_VERSION: i32 = 140_000;

/// One row for each pair of a waiting session and a session that it waits
/// for. The waits come from `pg_locks`, which every role can read in full,
/// and `pg_stat_activity` adds the details that the role can see.
///
/// The object is the relation of the lock that the session waits for, or
/// the relation of the row lock that it keeps while it waits for the
/// transaction of another session. A relation of another database has no
/// name in this database, so it gives no object.
fn waits_query(wait_start: bool) -> String {
    let wait_ms = if wait_start {
        "(SELECT (EXTRACT(EPOCH FROM clock_timestamp() - l.waitstart) * 1000)::bigint \
           FROM pg_locks AS l WHERE l.pid = w.pid AND NOT l.granted \
             AND l.waitstart IS NOT NULL LIMIT 1)"
    } else {
        "NULL::bigint"
    };
    format!(
        "SELECT w.pid::bigint, wa.query, {wait_ms}, \
           bp.pid::bigint, b.usename::text, b.client_addr::text, b.application_name, b.state, \
           b.query, \
           (SELECT l.relation::regclass::text FROM pg_locks AS l \
             WHERE l.pid = w.pid AND l.relation IS NOT NULL \
               AND l.database = (SELECT oid FROM pg_database WHERE datname = current_database()) \
               AND (NOT l.granted OR l.locktype = 'tuple') \
             ORDER BY l.granted LIMIT 1), \
           (SELECT l.mode FROM pg_locks AS l WHERE l.pid = w.pid AND NOT l.granted LIMIT 1) \
         FROM (SELECT DISTINCT pid FROM pg_locks WHERE NOT granted AND pid IS NOT NULL) AS w \
         CROSS JOIN LATERAL unnest(pg_blocking_pids(w.pid)) AS bp(pid) \
         LEFT JOIN pg_stat_activity AS wa ON wa.pid = w.pid \
         LEFT JOIN pg_stat_activity AS b ON b.pid = bp.pid \
         ORDER BY 3 DESC NULLS LAST, 1 \
         LIMIT {ROW_LIMIT}"
    )
}

/// The other sessions that keep a lock on a relation, a row, an object or
/// an advisory key, the oldest transaction first. A lock of a relation
/// stays until the end of the transaction that took it. A role that can't
/// read the activity of a session sees no start of its transaction, so
/// those sessions come last.
fn open_transactions_query() -> String {
    format!(
        "SELECT h.pid::bigint, a.usename::text, a.client_addr::text, a.application_name, \
           a.state, a.query, EXTRACT(EPOCH FROM clock_timestamp() - a.xact_start)::bigint \
         FROM (SELECT DISTINCT pid FROM pg_locks WHERE granted AND pid IS NOT NULL \
             AND locktype IN ('relation', 'tuple', 'object', 'advisory')) AS h \
         JOIN pg_stat_activity AS a ON a.pid = h.pid \
         WHERE h.pid <> pg_backend_pid() AND a.usename IS NOT NULL \
           AND (a.backend_type = 'client backend' OR a.backend_type IS NULL) \
         ORDER BY a.xact_start NULLS LAST, 1 \
         LIMIT {ROW_LIMIT}"
    )
}

impl PostgresDriver {
    /// Reads the waits for locks and the open transactions of the server.
    pub(super) async fn blocking_report(&mut self) -> Result<BlockingReport> {
        let version: i32 = self
            .client
            .query_one("SELECT current_setting('server_version_num')::int", &[])
            .await?
            .get(0);
        let privileged: Option<bool> = self.client.query_one(PRIVILEGE_QUERY, &[]).await?.get(0);
        let waits = self
            .client
            .query(&waits_query(version >= WAIT_START_VERSION), &[])
            .await?;
        let open = self.client.query(&open_transactions_query(), &[]).await?;
        let mut report = BlockingReport {
            sessions: waits.iter().map(wait_row).collect(),
            open_transactions: open.iter().map(open_row).collect(),
            notes: Vec::new(),
        };
        if privileged != Some(true) {
            report.notes.push(NO_READ_ALL_STATS.to_string());
        }
        Ok(report)
    }
}

/// Gives a statement of the report. The mark of a hidden statement gives
/// `None`, and the report then has a note.
fn statement(row: &Row, index: usize) -> Option<String> {
    statement_text(
        row.get::<_, Option<&str>>(index)
            .filter(|text| *text != HIDDEN_STATEMENT),
    )
}

fn text(row: &Row, index: usize) -> Option<String> {
    label(row.get(index))
}

fn wait_row(row: &Row) -> BlockingSession {
    BlockingSession {
        waiting_session: count(row.get(0)).unwrap_or_default(),
        waiting_statement: statement(row, 1),
        wait_ms: count(row.get(2)),
        blocking_session: count(row.get(3)).unwrap_or_default(),
        blocking_login: text(row, 4),
        blocking_host: text(row, 5),
        blocking_program: text(row, 6),
        blocking_status: text(row, 7),
        blocking_statement: statement(row, 8),
        object: text(row, 9),
        lock_mode: text(row, 10),
    }
}

fn open_row(row: &Row) -> OpenTransaction {
    OpenTransaction {
        session: count(row.get(0)).unwrap_or_default(),
        login: text(row, 1),
        host: text(row, 2),
        program: text(row, 3),
        status: text(row, 4),
        statement: statement(row, 5),
        open_secs: count(row.get(6)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_old_server_gives_no_wait_time() {
        assert!(waits_query(true).contains("l.waitstart"));
        assert!(!waits_query(false).contains("waitstart"));
        assert!(open_transactions_query().contains(&format!("LIMIT {ROW_LIMIT}")));
    }
}
