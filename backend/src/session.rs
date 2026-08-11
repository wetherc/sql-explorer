//! The sessions of one open connection.
//!
//! Each editor tab holds one session of its own. The statements of one tab
//! keep their temporary tables, their `SET` options, and their transactions,
//! because they run on one server session. The statements of two tabs run at
//! the same time, because each tab has its own session.

// The command layer does not call this module yet.
#![allow(dead_code)]

use crate::db::drivers::{CancelHandle, DatabaseDriver};
use crate::state::HEALTH_CHECK_AFTER;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, MutexGuard};

/// The key of the session that a request without a tab uses. The name starts
/// with a sign that no tab identifier carries, so no tab can take this slot.
pub const DEFAULT_SESSION: &str = "@default";

/// The time after which an idle tab session closes.
pub const SESSION_IDLE_REAP: Duration = Duration::from_secs(600);

/// The largest number of tab sessions one connection opens when the record
/// of the connection names no other limit.
pub const DEFAULT_SESSION_CAP: usize = 6;

/// One server session of one connection.
pub struct Session {
    pub driver: Arc<Mutex<Box<dyn DatabaseDriver>>>,
    /// Stops a statement while the driver above is busy with it.
    pub cancel_handle: Option<Arc<dyn CancelHandle>>,
    /// True when an idle session must be checked before it is used.
    pub needs_ping: bool,
    /// True when the session stays fit for use after a limit stopped a
    /// statement.
    pub keeps_connection_after_stop: bool,
    /// The moment the session last answered.
    last_ok: Mutex<Instant>,
    /// One check at a time for each session.
    pub health: Mutex<()>,
}

impl Session {
    pub fn new(driver: Box<dyn DatabaseDriver>) -> Self {
        let cancel_handle = driver.cancel_handle();
        let needs_ping = driver.needs_ping();
        let keeps_connection_after_stop = driver.keeps_connection_after_stop();
        Self {
            driver: Arc::new(Mutex::new(driver)),
            cancel_handle,
            needs_ping,
            keeps_connection_after_stop,
            last_ok: Mutex::new(Instant::now()),
            health: Mutex::new(()),
        }
    }

    /// Records that the session answered.
    pub async fn mark_ok(&self) {
        *self.last_ok.lock().await = Instant::now();
    }

    /// Moves the moment of the last answer into the past, for a test.
    #[cfg(test)]
    pub async fn age(&self, by: Duration) {
        *self.last_ok.lock().await = Instant::now() - by;
    }

    /// True when the session stood idle long enough that it should be
    /// checked before it is used again.
    pub async fn needs_check(&self) -> bool {
        self.idle_past(HEALTH_CHECK_AFTER).await
    }

    /// True when the session gave no answer within the given time.
    async fn idle_past(&self, threshold: Duration) -> bool {
        self.last_ok.lock().await.elapsed() >= threshold
    }
}

/// The sessions of one connection, keyed by the tab that holds each one.
pub struct SessionPool {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    /// One new session opens at a time, so the count against the cap stays
    /// exact and two requests for one new tab open one session, not two.
    open_lock: Mutex<()>,
    cap: usize,
}

impl SessionPool {
    pub fn new(cap: usize) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            open_lock: Mutex::new(()),
            cap,
        }
    }

    /// The largest number of tab sessions this pool opens.
    pub fn cap(&self) -> usize {
        self.cap
    }

    /// Takes the lock that serialises the opening of new sessions. The
    /// caller holds the guard from the count against the cap until the
    /// insert of the new session.
    pub async fn begin_open(&self) -> MutexGuard<'_, ()> {
        self.open_lock.lock().await
    }

    /// Returns the session of one key, when the pool holds one.
    pub async fn get(&self, key: &str) -> Option<Arc<Session>> {
        self.sessions.lock().await.get(key).cloned()
    }

    /// Puts a session into the pool and returns it. A session that already
    /// sat under the key goes.
    pub async fn insert(&self, key: &str, session: Session) -> Arc<Session> {
        let held = Arc::new(session);
        self.sessions
            .lock()
            .await
            .insert(key.to_string(), held.clone());
        held
    }

    /// Removes the session of one key. Returns true when one was present.
    /// A statement that still runs on the session completes, because the
    /// command that runs it holds its own reference.
    pub async fn release(&self, key: &str) -> bool {
        self.sessions.lock().await.remove(key).is_some()
    }

    /// The number of sessions that tabs hold. The default session does not
    /// count against the cap.
    pub async fn tab_count(&self) -> usize {
        self.sessions
            .lock()
            .await
            .keys()
            .filter(|key| *key != DEFAULT_SESSION)
            .count()
    }

    /// True when the tab sessions have reached the cap.
    pub async fn at_cap(&self) -> bool {
        self.tab_count().await >= self.cap
    }

    /// True when the pool holds no session at all.
    pub async fn is_empty(&self) -> bool {
        self.sessions.lock().await.is_empty()
    }

    /// Removes every tab session that stood idle past the limit. A session
    /// whose driver is busy stays, because a statement still runs on it. The
    /// default session stays, because the health check covers it.
    pub async fn reap_idle(&self) {
        let mut sessions = self.sessions.lock().await;
        let mut gone: Vec<String> = Vec::new();
        for (key, session) in sessions.iter() {
            if key == DEFAULT_SESSION {
                continue;
            }
            if !session.idle_past(SESSION_IDLE_REAP).await {
                continue;
            }
            if session.driver.try_lock().is_err() {
                continue;
            }
            gone.push(key.clone());
        }
        for key in &gone {
            sessions.remove(key);
        }
    }

    /// Removes every session.
    pub async fn clear(&self) {
        self.sessions.lock().await.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DriverCapabilities;
    use crate::db::{AppColumn, Database, ExecOptions, QueryParams, QueryResponse, Schema, Table};
    use crate::error::Result;
    use crate::sql::Dialect;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// A handle that records that it was asked to stop a statement.
    struct FlagCancel(Arc<AtomicBool>);

    #[async_trait]
    impl CancelHandle for FlagCancel {
        async fn cancel(&self) -> Result<()> {
            self.0.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    /// A driver whose flags a test selects.
    struct StubDriver {
        cancelled: Option<Arc<AtomicBool>>,
        needs_ping: bool,
        keeps_connection_after_stop: bool,
    }

    impl StubDriver {
        fn plain() -> Self {
            Self {
                cancelled: None,
                needs_ping: true,
                keeps_connection_after_stop: false,
            }
        }
    }

    #[async_trait]
    impl DatabaseDriver for StubDriver {
        fn capabilities(&self) -> DriverCapabilities {
            DriverCapabilities::default()
        }
        fn dialect(&self) -> Dialect {
            Dialect::Sqlite
        }
        fn needs_ping(&self) -> bool {
            self.needs_ping
        }
        fn keeps_connection_after_stop(&self) -> bool {
            self.keeps_connection_after_stop
        }
        fn cancel_handle(&self) -> Option<Arc<dyn CancelHandle>> {
            self.cancelled
                .as_ref()
                .map(|flag| Arc::new(FlagCancel(flag.clone())) as Arc<dyn CancelHandle>)
        }
        async fn ping(&mut self) -> Result<()> {
            Ok(())
        }
        async fn execute_query(
            &mut self,
            _query: &str,
            _params: Option<&QueryParams>,
            _options: &ExecOptions,
        ) -> Result<QueryResponse> {
            Ok(QueryResponse::default())
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

    fn pool() -> SessionPool {
        SessionPool::new(2)
    }

    #[tokio::test]
    async fn a_session_takes_the_flags_of_its_driver() {
        let session = Session::new(Box::new(StubDriver {
            cancelled: Some(Arc::new(AtomicBool::new(false))),
            needs_ping: false,
            keeps_connection_after_stop: true,
        }));
        assert!(session.cancel_handle.is_some());
        assert!(!session.needs_ping);
        assert!(session.keeps_connection_after_stop);

        let plain = Session::new(Box::new(StubDriver::plain()));
        assert!(plain.cancel_handle.is_none());
        assert!(plain.needs_ping);
        assert!(!plain.keeps_connection_after_stop);
    }

    #[tokio::test]
    async fn the_handle_of_a_session_stops_a_statement() {
        let flag = Arc::new(AtomicBool::new(false));
        let session = Session::new(Box::new(StubDriver {
            cancelled: Some(flag.clone()),
            needs_ping: true,
            keeps_connection_after_stop: false,
        }));
        session
            .cancel_handle
            .as_ref()
            .expect("the driver can cancel")
            .cancel()
            .await
            .unwrap();
        assert!(flag.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn a_session_that_just_answered_needs_no_check() {
        let session = Session::new(Box::new(StubDriver::plain()));
        assert!(!session.needs_check().await);
        session.age(HEALTH_CHECK_AFTER).await;
        assert!(session.needs_check().await);
        session.mark_ok().await;
        assert!(!session.needs_check().await);
    }

    #[tokio::test]
    async fn a_session_can_be_added_read_and_released() {
        let pool = pool();
        assert!(pool.get("t1").await.is_none());
        assert!(pool.is_empty().await);

        pool.insert("t1", Session::new(Box::new(StubDriver::plain())))
            .await;
        assert!(pool.get("t1").await.is_some());
        assert!(!pool.is_empty().await);

        assert!(pool.release("t1").await);
        assert!(!pool.release("t1").await);
        assert!(pool.get("t1").await.is_none());
    }

    #[tokio::test]
    async fn an_insert_replaces_the_session_of_the_key() {
        let pool = pool();
        let first = pool
            .insert("t1", Session::new(Box::new(StubDriver::plain())))
            .await;
        let second = pool
            .insert("t1", Session::new(Box::new(StubDriver::plain())))
            .await;
        assert!(!Arc::ptr_eq(&first, &second));
        let held = pool.get("t1").await.unwrap();
        assert!(Arc::ptr_eq(&held, &second));
        assert_eq!(pool.tab_count().await, 1);
    }

    #[tokio::test]
    async fn the_default_session_does_not_count_against_the_cap() {
        let pool = pool();
        pool.insert(DEFAULT_SESSION, Session::new(Box::new(StubDriver::plain())))
            .await;
        assert_eq!(pool.tab_count().await, 0);
        assert!(!pool.at_cap().await);

        pool.insert("t1", Session::new(Box::new(StubDriver::plain())))
            .await;
        assert!(!pool.at_cap().await);
        pool.insert("t2", Session::new(Box::new(StubDriver::plain())))
            .await;
        assert_eq!(pool.tab_count().await, 2);
        assert!(pool.at_cap().await);
        assert_eq!(pool.cap(), 2);
    }

    #[tokio::test]
    async fn the_reap_removes_only_an_idle_tab_session() {
        let pool = pool();
        let old = pool
            .insert("old", Session::new(Box::new(StubDriver::plain())))
            .await;
        old.age(SESSION_IDLE_REAP).await;
        pool.insert("fresh", Session::new(Box::new(StubDriver::plain())))
            .await;
        let default = pool
            .insert(DEFAULT_SESSION, Session::new(Box::new(StubDriver::plain())))
            .await;
        default.age(SESSION_IDLE_REAP).await;

        pool.reap_idle().await;

        assert!(pool.get("old").await.is_none());
        assert!(pool.get("fresh").await.is_some());
        assert!(pool.get(DEFAULT_SESSION).await.is_some());
    }

    #[tokio::test]
    async fn the_reap_leaves_a_session_whose_driver_is_busy() {
        let pool = pool();
        let busy = pool
            .insert("busy", Session::new(Box::new(StubDriver::plain())))
            .await;
        busy.age(SESSION_IDLE_REAP).await;

        let guard = busy.driver.lock().await;
        pool.reap_idle().await;
        drop(guard);

        assert!(pool.get("busy").await.is_some());
        pool.reap_idle().await;
        assert!(pool.get("busy").await.is_none());
    }

    #[tokio::test]
    async fn a_clear_removes_every_session() {
        let pool = pool();
        pool.insert(DEFAULT_SESSION, Session::new(Box::new(StubDriver::plain())))
            .await;
        pool.insert("t1", Session::new(Box::new(StubDriver::plain())))
            .await;
        pool.clear().await;
        assert!(pool.is_empty().await);
    }

    #[tokio::test]
    async fn one_session_opens_at_a_time() {
        let pool = Arc::new(pool());
        let guard = pool.begin_open().await;
        let waiting = tokio::time::timeout(Duration::from_millis(20), pool.begin_open()).await;
        assert!(waiting.is_err());
        drop(guard);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), pool.begin_open())
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn a_released_session_completes_the_statement_it_runs() {
        let pool = pool();
        let session = pool
            .insert("t1", Session::new(Box::new(StubDriver::plain())))
            .await;
        let mut guard = session.driver.lock().await;
        assert!(pool.release("t1").await);
        let response = guard
            .execute_query("SELECT 1", None, &ExecOptions::default())
            .await
            .unwrap();
        assert_eq!(response.results.len(), 0);
    }
}
