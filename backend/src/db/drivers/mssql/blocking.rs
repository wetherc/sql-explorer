//! The report of the sessions that block others on MS SQL Server.

use super::MssqlDriver;
use crate::db::blocking::{
    count, label, statement_text, BlockingReport, BlockingSession, OpenTransaction, ROW_LIMIT,
};
use crate::error::Result;
use tiberius::Row;

/// The note of a login that can't read the sessions of other logins.
const NO_SERVER_STATE: &str = "Your login can't see other sessions. It needs the VIEW \
     SERVER STATE permission (VIEW SERVER PERFORMANCE STATE on SQL Server 2022, or VIEW \
     DATABASE STATE on Azure SQL Database).";

/// Gives 1 when the login can read the sessions of every login. Without
/// this permission the views of the sessions show the own session alone,
/// with no error. `HAS_PERMS_BY_NAME` gives NULL for a permission that the
/// version does not know.
const PERMISSION_QUERY: &str = "SELECT CAST(CASE \
     WHEN HAS_PERMS_BY_NAME(NULL, NULL, 'VIEW SERVER STATE') = 1 THEN 1 \
     WHEN HAS_PERMS_BY_NAME(NULL, NULL, 'VIEW SERVER PERFORMANCE STATE') = 1 THEN 1 \
     WHEN SERVERPROPERTY('EngineEdition') = 5 \
          AND HAS_PERMS_BY_NAME(DB_NAME(), 'DATABASE', 'VIEW DATABASE STATE') = 1 THEN 1 \
     ELSE 0 END AS int)";

/// One row for each request that waits for another session. The object of
/// a lock on a key, a page or a row is found through `sys.partitions` of the
/// current database alone, and other waits give the resource text of the
/// server. A blocking session with no request is idle, so its statement is
/// the last one it sent.
fn waits_query() -> String {
    format!(
        "SELECT TOP ({ROW_LIMIT}) \
           CAST(r.session_id AS bigint), wt.text, CAST(r.wait_time AS bigint), \
           CAST(r.blocking_session_id AS bigint), s.login_name, s.host_name, s.program_name, \
           s.status, bt.text, \
           COALESCE(CASE \
             WHEN wl.resource_type = 'OBJECT' THEN \
               QUOTENAME(DB_NAME(wl.resource_database_id)) + '.' + \
               QUOTENAME(OBJECT_SCHEMA_NAME(CAST(wl.resource_associated_entity_id AS int), \
                 wl.resource_database_id)) + '.' + \
               QUOTENAME(OBJECT_NAME(CAST(wl.resource_associated_entity_id AS int), \
                 wl.resource_database_id)) \
             WHEN wp.object_id IS NOT NULL THEN \
               QUOTENAME(DB_NAME()) + '.' + QUOTENAME(OBJECT_SCHEMA_NAME(wp.object_id)) + '.' + \
               QUOTENAME(OBJECT_NAME(wp.object_id)) \
           END, NULLIF(r.wait_resource, '')), \
           COALESCE(wl.request_mode, r.wait_type) \
         FROM sys.dm_exec_requests AS r \
         JOIN sys.dm_exec_sessions AS s ON s.session_id = r.blocking_session_id \
         OUTER APPLY (SELECT TOP 1 br.sql_handle FROM sys.dm_exec_requests AS br \
           WHERE br.session_id = r.blocking_session_id) AS br \
         OUTER APPLY (SELECT TOP 1 c.most_recent_sql_handle FROM sys.dm_exec_connections AS c \
           WHERE c.session_id = r.blocking_session_id) AS bc \
         OUTER APPLY sys.dm_exec_sql_text(r.sql_handle) AS wt \
         OUTER APPLY sys.dm_exec_sql_text(COALESCE(br.sql_handle, bc.most_recent_sql_handle)) AS bt \
         OUTER APPLY (SELECT TOP 1 l.resource_type, l.request_mode, l.resource_database_id, \
             l.resource_associated_entity_id FROM sys.dm_tran_locks AS l \
           WHERE l.request_session_id = r.session_id AND l.request_status = 'WAIT') AS wl \
         OUTER APPLY (SELECT TOP 1 p.object_id FROM sys.partitions AS p \
           WHERE wl.resource_type IN ('KEY', 'PAGE', 'RID', 'HOBT') \
             AND wl.resource_database_id = DB_ID() \
             AND p.hobt_id = wl.resource_associated_entity_id) AS wp \
         WHERE r.blocking_session_id > 0 \
         ORDER BY r.wait_time DESC"
    )
}

/// The other user sessions inside a transaction that keep a lock on more
/// than a database, the oldest transaction first.
fn open_transactions_query() -> String {
    format!(
        "SELECT TOP ({ROW_LIMIT}) \
           CAST(s.session_id AS bigint), s.login_name, s.host_name, s.program_name, s.status, \
           t.text, CAST(DATEDIFF(SECOND, x.began, SYSDATETIME()) AS bigint) \
         FROM sys.dm_exec_sessions AS s \
         CROSS APPLY (SELECT MIN(a.transaction_begin_time) AS began \
           FROM sys.dm_tran_session_transactions AS st \
           JOIN sys.dm_tran_active_transactions AS a ON a.transaction_id = st.transaction_id \
           WHERE st.session_id = s.session_id) AS x \
         OUTER APPLY (SELECT TOP 1 c.most_recent_sql_handle FROM sys.dm_exec_connections AS c \
           WHERE c.session_id = s.session_id) AS c \
         OUTER APPLY sys.dm_exec_sql_text(c.most_recent_sql_handle) AS t \
         WHERE s.session_id <> @@SPID AND s.is_user_process = 1 AND x.began IS NOT NULL \
           AND EXISTS (SELECT 1 FROM sys.dm_tran_locks AS l \
             WHERE l.request_session_id = s.session_id AND l.request_status = 'GRANT' \
               AND l.resource_type <> 'DATABASE') \
         ORDER BY x.began"
    )
}

impl MssqlDriver {
    /// Reads the waits for locks and the open transactions of the server.
    pub(super) async fn blocking_report(&mut self) -> Result<BlockingReport> {
        let allowed = self
            .client
            .simple_query(PERMISSION_QUERY)
            .await?
            .into_row()
            .await?
            .and_then(|row| row.get::<i32, _>(0))
            == Some(1);
        if !allowed {
            return Ok(BlockingReport {
                notes: vec![NO_SERVER_STATE.to_string()],
                ..BlockingReport::default()
            });
        }
        let batch = format!("{};\n{};", waits_query(), open_transactions_query());
        let mut sets = self
            .client
            .simple_query(batch)
            .await?
            .into_results()
            .await?
            .into_iter();
        let waits = sets.next().unwrap_or_default();
        let open = sets.next().unwrap_or_default();
        Ok(BlockingReport {
            sessions: waits.iter().map(wait_row).collect(),
            open_transactions: open.iter().map(open_row).collect(),
            notes: Vec::new(),
        })
    }
}

fn text(row: &Row, index: usize) -> Option<&str> {
    row.try_get::<&str, _>(index).ok().flatten()
}

fn number(row: &Row, index: usize) -> Option<i64> {
    row.try_get::<i64, _>(index).ok().flatten()
}

fn wait_row(row: &Row) -> BlockingSession {
    BlockingSession {
        waiting_session: count(number(row, 0)).unwrap_or_default(),
        waiting_statement: statement_text(text(row, 1)),
        wait_ms: count(number(row, 2)),
        blocking_session: count(number(row, 3)).unwrap_or_default(),
        blocking_login: label(text(row, 4)),
        blocking_host: label(text(row, 5)),
        blocking_program: label(text(row, 6)),
        blocking_status: label(text(row, 7)),
        blocking_statement: statement_text(text(row, 8)),
        object: label(text(row, 9)),
        lock_mode: label(text(row, 10)),
    }
}

fn open_row(row: &Row) -> OpenTransaction {
    OpenTransaction {
        session: count(number(row, 0)).unwrap_or_default(),
        login: label(text(row, 1)),
        host: label(text(row, 2)),
        program: label(text(row, 3)),
        status: label(text(row, 4)),
        statement: statement_text(text(row, 5)),
        open_secs: count(number(row, 6)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_queries_name_the_row_limit() {
        assert!(waits_query().contains(&format!("TOP ({ROW_LIMIT})")));
        assert!(open_transactions_query().contains("@@SPID"));
    }
}
