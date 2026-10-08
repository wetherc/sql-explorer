//! The command that lists the sessions that block others with their locks.

use super::{metadata_read, CatalogRead};
use crate::db::blocking::BlockingReport;
use crate::error::Result;
use crate::state::AppState;
use tauri::{AppHandle, Runtime};

/// Lists the sessions of the server that wait for locks and the sessions
/// that keep locks. With `blocked_by`, the report keeps the rows of that one
/// blocking session alone.
///
/// The read runs on the background driver of the catalog, so it does not
/// wait behind a statement of a tab. That driver ends each wait for a lock
/// after a short limit, and the tables of the locks take no locks of their
/// own.
#[tauri::command]
pub async fn blocking_sessions<R: Runtime>(
    app: AppHandle<R>,
    connection_id: String,
    blocked_by: Option<u64>,
    state: tauri::State<'_, AppState>,
) -> Result<BlockingReport> {
    let read = metadata_read(&app, &state, &connection_id).await?;
    report(&read, blocked_by).await
}

/// Reads the report on the session of the read and applies the filter.
async fn report(read: &CatalogRead<'_>, blocked_by: Option<u64>) -> Result<BlockingReport> {
    let mut guard = read.lock().await?;
    let report = read.run(guard.blocking_sessions()).await?;
    Ok(match blocked_by {
        Some(session) => report.blocked_by(session),
        None => report,
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::{state_with_sqlite, temp_sqlite};
    use super::*;
    use crate::db::blocking::{BlockingSession, OpenTransaction};
    use crate::db::drivers::DatabaseDriver;
    use crate::db::{AppColumn, Database, DriverCapabilities, Schema, Table};
    use crate::session::Session;
    use crate::sql::Dialect;
    use std::sync::Arc;
    use std::time::Duration;

    /// A driver whose server has one session that blocks two others.
    struct LockedDriver;

    #[async_trait::async_trait]
    impl DatabaseDriver for LockedDriver {
        fn capabilities(&self) -> DriverCapabilities {
            DriverCapabilities::default()
        }
        fn dialect(&self) -> Dialect {
            Dialect::Postgres
        }
        async fn ping(&mut self) -> Result<()> {
            Ok(())
        }
        async fn blocking_sessions(&mut self) -> Result<BlockingReport> {
            let wait = |waiting, blocking| BlockingSession {
                waiting_session: waiting,
                blocking_session: blocking,
                ..BlockingSession::default()
            };
            Ok(BlockingReport {
                sessions: vec![wait(11, 7), wait(12, 7), wait(13, 9)],
                open_transactions: vec![
                    OpenTransaction {
                        session: 7,
                        ..OpenTransaction::default()
                    },
                    OpenTransaction {
                        session: 9,
                        ..OpenTransaction::default()
                    },
                ],
                notes: Vec::new(),
            })
        }
        async fn list_databases(&mut self) -> Result<Vec<Database>> {
            Ok(Vec::new())
        }
        async fn list_schemas(&mut self, _database: &str) -> Result<Vec<Schema>> {
            Ok(Vec::new())
        }
        async fn list_tables(
            &mut self,
            _database: &str,
            _schema: Option<&str>,
        ) -> Result<Vec<Table>> {
            Ok(Vec::new())
        }
        async fn list_columns(
            &mut self,
            _database: &str,
            _schema: Option<&str>,
            _table: &str,
        ) -> Result<Vec<AppColumn>> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn the_report_keeps_every_row_or_the_rows_of_one_blocker() {
        let (_dir, descriptor) = temp_sqlite();
        let (_app, state) = state_with_sqlite(descriptor).await;
        let session = Arc::new(Session::new(Box::new(LockedDriver)));
        let read = CatalogRead::new(&state, "s1", session, Duration::from_secs(5));

        let all = report(&read, None).await.unwrap();
        assert_eq!(all.sessions.len(), 3);
        assert_eq!(all.open_transactions.len(), 2);

        let one = report(&read, Some(7)).await.unwrap();
        let waiting: Vec<u64> = one.sessions.iter().map(|row| row.waiting_session).collect();
        assert_eq!(waiting, vec![11, 12]);
        assert_eq!(one.open_transactions.len(), 1);
    }

    #[tokio::test]
    async fn a_driver_without_lock_tables_gives_its_refusal() {
        let (_dir, descriptor) = temp_sqlite();
        let (_app, state) = state_with_sqlite(descriptor.clone()).await;
        let driver = super::super::open_driver(&descriptor).await.unwrap();
        let read = CatalogRead::new(
            &state,
            "s1",
            Arc::new(Session::new(driver)),
            Duration::from_secs(5),
        );
        let error = report(&read, None).await.unwrap_err();
        assert_eq!(error.category(), crate::error::ErrorCategory::Unsupported);
    }
}
