//! What the end of a run tells the tab about its session.

use super::release_broken;
use crate::session::{Session, SessionReport, TRANSACTION_PROBE_LIMIT};
use crate::state::{AppState, OpenConnection};
use std::sync::Arc;

/// Gives what the end of a run tells the tab about its session: whether a
/// new session took the place of the old one, and whether the session is
/// inside an open transaction.
///
/// The probe of the transaction costs a round trip on MS SQL Server,
/// PostgreSQL and MySQL, so it runs only after a run that can have changed
/// the state, as [`Session::needs_transaction_probe`] decides. The text of
/// the script is a cheaper signal than the probe, but a procedure, a
/// trigger or an error can open or end a transaction with no `BEGIN` or
/// `COMMIT` in the text, so the text alone decides only when no probe is
/// needed. Other runs report the state of the last probe.
///
/// A probe that passes [`TRANSACTION_PROBE_LIMIT`] is dropped in the middle
/// of an exchange, so the session then closes as after a stop.
pub(super) async fn session_after_run(
    state: &AppState,
    connection_id: &str,
    open: &OpenConnection,
    session_key: &str,
    session: &Arc<Session>,
    script: &str,
    failed: bool,
) -> SessionReport {
    let reset = session.take_replaced();
    if session.is_broken() {
        return SessionReport {
            reset,
            open_transaction: None,
        };
    }
    if !session.needs_transaction_probe(script, open.dialect, failed) {
        return SessionReport {
            reset,
            open_transaction: Some(session.in_transaction()),
        };
    }
    // Another request of the session, such as a statement of a second tab
    // on an SQLite database in memory, can keep the driver. The state is
    // then not known.
    let Ok(mut driver) = tokio::time::timeout(TRANSACTION_PROBE_LIMIT, session.driver.lock()).await
    else {
        return SessionReport {
            reset,
            open_transaction: None,
        };
    };
    let probe =
        tokio::time::timeout(TRANSACTION_PROBE_LIMIT, driver.holds_open_transaction()).await;
    drop(driver);
    match probe {
        Ok(Ok(open_now)) => {
            session.set_in_transaction(open_now);
            SessionReport {
                reset,
                open_transaction: Some(open_now),
            }
        }
        Ok(Err(error)) => {
            log::warn!(
                "The transaction of a session of '{connection_id}' could not be read: {error}"
            );
            SessionReport {
                reset,
                open_transaction: None,
            }
        }
        Err(_) => {
            log::warn!(
                "The transaction of a session of '{connection_id}' could not be read within {} \
                 seconds, so the session closes.",
                TRANSACTION_PROBE_LIMIT.as_secs()
            );
            let released = release_broken(state, connection_id, open, session_key, session).await;
            SessionReport {
                reset: reset || released,
                open_transaction: released.then_some(false),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{state_with_sqlite, temp_sqlite};
    use super::*;
    use crate::db::drivers::DatabaseDriver;
    use crate::db::{AppColumn, Database, DriverCapabilities, Schema, Table};
    use crate::error::{Error, Result};
    use crate::sql::Dialect;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// How the probe of the transaction answers.
    #[derive(Clone, Copy)]
    enum Probe {
        Open,
        Fails,
        Hangs,
    }

    /// A driver whose probe of the transaction a test selects, and which
    /// counts the probes.
    struct ProbeDriver {
        probe: Probe,
        probes: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl DatabaseDriver for ProbeDriver {
        fn capabilities(&self) -> DriverCapabilities {
            DriverCapabilities::default()
        }
        fn dialect(&self) -> Dialect {
            Dialect::Sqlite
        }
        async fn ping(&mut self) -> Result<()> {
            Ok(())
        }
        async fn holds_open_transaction(&mut self) -> Result<bool> {
            self.probes.fetch_add(1, Ordering::SeqCst);
            match self.probe {
                Probe::Open => Ok(true),
                Probe::Fails => Err(Error::Connection("gone".into())),
                Probe::Hangs => std::future::pending().await,
            }
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

    /// The open connection of a test, with the session of the tab `t1`.
    struct Tab {
        _dir: tempfile::TempDir,
        state: AppState,
        open: OpenConnection,
        session: Arc<Session>,
        probes: Arc<AtomicUsize>,
    }

    impl Tab {
        /// An open SQLite connection whose tab `t1` has a session with the
        /// given probe.
        async fn with(probe: Probe) -> Self {
            let (dir, descriptor) = temp_sqlite();
            let (_app, state) = state_with_sqlite(descriptor).await;
            let open = state.connection("s1").await.unwrap();
            let probes = Arc::new(AtomicUsize::new(0));
            let driver = ProbeDriver {
                probe,
                probes: probes.clone(),
            };
            let session = open
                .sessions
                .insert("t1", Session::new(Box::new(driver)))
                .await;
            Self {
                _dir: dir,
                state,
                open,
                session,
                probes,
            }
        }

        async fn report(&self, script: &str, failed: bool) -> SessionReport {
            session_after_run(
                &self.state,
                "s1",
                &self.open,
                "t1",
                &self.session,
                script,
                failed,
            )
            .await
        }

        fn probes(&self) -> usize {
            self.probes.load(Ordering::SeqCst)
        }
    }

    #[tokio::test]
    async fn a_read_reports_the_last_state_without_a_probe() {
        let tab = Tab::with(Probe::Open).await;
        tab.session.mark_replacement();
        assert_eq!(
            tab.report("SELECT 1", false).await,
            SessionReport {
                reset: true,
                open_transaction: Some(false),
            }
        );
        assert_eq!(tab.probes(), 0);
        // The mark of the new session goes with the first run alone.
        assert!(!tab.report("SELECT 1", false).await.reset);
    }

    #[tokio::test]
    async fn a_run_that_can_open_a_transaction_asks_the_driver() {
        let tab = Tab::with(Probe::Open).await;
        assert_eq!(
            tab.report("BEGIN", false).await.open_transaction,
            Some(true)
        );
        assert!(tab.session.in_transaction());
        assert_eq!(tab.probes(), 1);

        // A read keeps the state that the probe found.
        let read = tab.report("SELECT 1", false).await;
        assert_eq!(read.open_transaction, Some(true));
        assert_eq!(tab.probes(), 1);
    }

    #[tokio::test]
    async fn a_probe_that_fails_reports_no_state() {
        let tab = Tab::with(Probe::Fails).await;
        assert_eq!(tab.report("COMMIT", true).await.open_transaction, None);
        // The session stays, because the failure ended its exchange.
        assert!(!tab.session.is_broken());
        assert!(tab.open.sessions.get("t1").await.is_some());
    }

    #[tokio::test]
    async fn a_broken_session_gets_no_probe() {
        let tab = Tab::with(Probe::Open).await;
        tab.session.mark_broken();
        assert_eq!(tab.report("BEGIN", true).await.open_transaction, None);
        assert_eq!(tab.probes(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_busy_driver_gives_no_state() {
        let tab = Tab::with(Probe::Open).await;
        let _busy = tab.session.driver.lock().await;
        assert_eq!(tab.report("BEGIN", false).await.open_transaction, None);
        assert_eq!(tab.probes(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_probe_past_its_limit_closes_the_session() {
        let tab = Tab::with(Probe::Hangs).await;
        assert_eq!(
            tab.report("BEGIN", false).await,
            SessionReport {
                reset: true,
                open_transaction: Some(false),
            }
        );
        assert!(tab.session.is_broken());
        assert!(tab.open.sessions.get("t1").await.is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn a_probe_past_its_limit_in_a_closed_tab_reports_nothing() {
        let tab = Tab::with(Probe::Hangs).await;
        tab.open.sessions.release("t1").await;
        assert_eq!(tab.report("BEGIN", false).await, SessionReport::default());
        assert!(tab.session.is_broken());
    }
}
