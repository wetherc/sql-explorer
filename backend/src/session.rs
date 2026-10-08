//! The sessions of one open connection.
//!
//! Each editor tab holds one session of its own. The statements of one tab
//! keep their temporary tables, their `SET` options, and their transactions,
//! because they run on one server session. The statements of two tabs run at
//! the same time, because each tab has its own session.

use crate::db::drivers::{CancelHandle, DatabaseDriver};
use crate::state::HEALTH_CHECK_AFTER;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, OwnedMutexGuard};

/// The key of the session that a request without a tab uses. The name starts
/// with a sign that no tab identifier carries, so no tab can take this slot.
pub const DEFAULT_SESSION: &str = "@default";

/// The time after which the periodic sweep closes an idle tab session.
///
/// A closed session loses its temporary tables and its `SET` options, so the
/// limit is long. A tab that a user leaves over lunch keeps its session.
pub const SESSION_IDLE_REAP: Duration = Duration::from_secs(60 * 60);

/// The time the idle reaper gives one session to say whether it is inside
/// an open transaction.
pub const TRANSACTION_PROBE_LIMIT: Duration = Duration::from_secs(5);

/// The time a tab session must stand idle before a new tab can close it to
/// make room at the cap. A tab that ran a statement a moment ago keeps its
/// temporary tables and its `SET` options.
pub const EVICT_IDLE_AFTER: Duration = Duration::from_secs(5 * 60);

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
    /// True when the driver can pause a read at the row limit. See
    /// `DatabaseDriver::pauses_reads`.
    pub pauses_reads: bool,
    /// The moment the session last answered.
    last_ok: Mutex<Instant>,
    /// The moment a request last took the session.
    last_used: Mutex<Instant>,
    /// One check at a time for each session.
    pub health: Mutex<()>,
    /// True when a limit dropped an exchange of the session in the middle of
    /// a message. A request that waited for the driver at that moment must
    /// not send on it, because it would read the rest of the old answer.
    broken: AtomicBool,
    /// True when this session took the place of a session that stopped
    /// answering, until a run on it tells the tab.
    replaced: AtomicBool,
}

/// What one run tells the tab about the session it ran on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionReport {
    /// True when the session of the tab closed and a new one took its
    /// place, so the temporary tables, the open transaction and the `SET`
    /// options of the old session are gone.
    pub reset: bool,
}

impl Session {
    pub fn new(driver: Box<dyn DatabaseDriver>) -> Self {
        let cancel_handle = driver.cancel_handle();
        let needs_ping = driver.needs_ping();
        let keeps_connection_after_stop = driver.keeps_connection_after_stop();
        let pauses_reads = driver.pauses_reads();
        Self {
            driver: Arc::new(Mutex::new(driver)),
            cancel_handle,
            needs_ping,
            keeps_connection_after_stop,
            pauses_reads,
            last_ok: Mutex::new(Instant::now()),
            last_used: Mutex::new(Instant::now()),
            health: Mutex::new(()),
            broken: AtomicBool::new(false),
            replaced: AtomicBool::new(false),
        }
    }

    /// Records that this session takes the place of a session that stopped
    /// answering.
    pub fn mark_replacement(&self) {
        self.replaced.store(true, Ordering::SeqCst);
    }

    /// True once after [`Session::mark_replacement`], so one run alone
    /// tells the tab.
    pub fn take_replaced(&self) -> bool {
        self.replaced.swap(false, Ordering::SeqCst)
    }

    /// Records that nothing can be sent on the session again.
    pub fn mark_broken(&self) {
        self.broken.store(true, Ordering::SeqCst);
    }

    /// True when a limit dropped an exchange of the session.
    pub fn is_broken(&self) -> bool {
        self.broken.load(Ordering::SeqCst)
    }

    /// Records that the session answered.
    pub async fn mark_ok(&self) {
        *self.last_ok.lock().await = Instant::now();
        self.touch().await;
    }

    /// Records that a request took the session.
    pub async fn touch(&self) {
        *self.last_used.lock().await = Instant::now();
    }

    /// Moves the moment of the last answer and of the last use into the
    /// past, for a test.
    #[cfg(test)]
    pub async fn age(&self, by: Duration) {
        *self.last_ok.lock().await = Instant::now() - by;
        *self.last_used.lock().await = Instant::now() - by;
    }

    /// True when the session stood idle long enough that it should be
    /// checked before it is used again.
    pub async fn needs_check(&self) -> bool {
        self.last_ok.lock().await.elapsed() >= HEALTH_CHECK_AFTER
    }

    /// True when no request took the session within the given time.
    pub async fn idle_past(&self, threshold: Duration) -> bool {
        self.last_used.lock().await.elapsed() >= threshold
    }

    /// The moment a request last took the session.
    async fn used_at(&self) -> Instant {
        *self.last_used.lock().await
    }

    /// True when the session can close: its driver is free, and it is not
    /// inside an open transaction. A probe that fails or passes
    /// [`TRANSACTION_PROBE_LIMIT`] also lets it close, because the session
    /// then does not answer.
    async fn can_close(&self) -> bool {
        let probe = {
            let Ok(mut driver) = self.driver.try_lock() else {
                return false;
            };
            tokio::time::timeout(TRANSACTION_PROBE_LIMIT, driver.holds_open_transaction()).await
        };
        !matches!(probe, Ok(Ok(true)))
    }
}

/// The sessions of one connection, keyed by the tab that holds each one.
pub struct SessionPool {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    /// The opens of new sessions that run now. The map is never locked
    /// across an await.
    opening: std::sync::Mutex<Opening>,
    cap: usize,
}

/// The opens of new sessions that run now.
#[derive(Default)]
struct Opening {
    /// One lock for each key whose session opens, so two requests of one
    /// tab open one session and not two. A tab does not wait for the open
    /// of another tab, which can take the full connect time of the server.
    locks: HashMap<String, Arc<Mutex<()>>>,
    /// The tab sessions that opens have taken a place for but not yet put
    /// into the pool. They count against the cap.
    reserved: usize,
}

/// The right of one request to open the session of one key.
///
/// The ticket keeps the lock of the key until it drops. A place that
/// [`OpenTicket::reserve`] took goes back when the ticket drops without an
/// insert, for example when the open fails or the user presses Stop.
pub struct OpenTicket<'a> {
    pool: &'a SessionPool,
    key: String,
    lock: Arc<Mutex<()>>,
    guard: Option<OwnedMutexGuard<()>>,
    reserved: bool,
}

impl OpenTicket<'_> {
    /// Takes a place for the new session under the cap. Returns false when
    /// the tab sessions and the opens that run now fill the cap. The default
    /// session does not count against the cap, so its ticket always gets a
    /// place.
    pub async fn reserve(&mut self) -> bool {
        if self.reserved || self.key == DEFAULT_SESSION {
            return true;
        }
        let sessions = self.pool.sessions.lock().await;
        let tabs = tab_keys(&sessions).count();
        let mut opening = self.pool.opening();
        if tabs + opening.reserved >= self.pool.cap {
            return false;
        }
        opening.reserved += 1;
        self.reserved = true;
        true
    }

    /// Puts the new session into the pool, gives back the place that the
    /// ticket took, and returns the session. The two steps are one step for
    /// [`OpenTicket::reserve`], so the session never counts twice.
    pub async fn insert(mut self, session: Session) -> Arc<Session> {
        let held = Arc::new(session);
        let mut sessions = self.pool.sessions.lock().await;
        sessions.insert(self.key.clone(), held.clone());
        self.unreserve();
        held
    }

    fn unreserve(&mut self) {
        if std::mem::take(&mut self.reserved) {
            self.pool.opening().reserved -= 1;
        }
    }
}

impl Drop for OpenTicket<'_> {
    fn drop(&mut self) {
        self.unreserve();
        drop(self.guard.take());
        // The map, this ticket, and each request that waits keep one
        // reference to the lock. When no request waits, the entry goes.
        let mut opening = self.pool.opening();
        let unused = Arc::strong_count(&self.lock) <= 2;
        if unused
            && opening
                .locks
                .get(&self.key)
                .is_some_and(|lock| Arc::ptr_eq(lock, &self.lock))
        {
            opening.locks.remove(&self.key);
        }
    }
}

/// The keys of the tab sessions, without the default session.
fn tab_keys(sessions: &HashMap<String, Arc<Session>>) -> impl Iterator<Item = &String> {
    sessions.keys().filter(|key| *key != DEFAULT_SESSION)
}

impl SessionPool {
    pub fn new(cap: usize) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            opening: std::sync::Mutex::new(Opening::default()),
            cap,
        }
    }

    /// The record of the opens. No code panics while it keeps this lock, so
    /// the record of a poisoned lock is still valid.
    fn opening(&self) -> std::sync::MutexGuard<'_, Opening> {
        self.opening
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Builds a pool that already holds one session. The connect command
    /// uses this form, because the connection opens with one driver.
    pub fn with_session(cap: usize, key: &str, session: Session) -> Self {
        let pool = Self::new(cap);
        pool.sessions
            .try_lock()
            .expect("the pool is new and nothing locks it")
            .insert(key.to_string(), Arc::new(session));
        pool
    }

    /// The largest number of tab sessions this pool opens.
    pub fn cap(&self) -> usize {
        self.cap
    }

    /// Waits until no other request opens the session of the key, and
    /// returns the ticket for the open. Requests for other keys do not wait.
    /// The caller looks for the session again after the wait, because the
    /// request before it can have opened it.
    pub async fn begin_open(&self, key: &str) -> OpenTicket<'_> {
        let lock = {
            let mut opening = self.opening();
            // A wait that a Stop dropped leaves an entry that no request
            // uses.
            opening.locks.retain(|_, lock| Arc::strong_count(lock) > 1);
            opening.locks.entry(key.to_string()).or_default().clone()
        };
        let guard = lock.clone().lock_owned().await;
        OpenTicket {
            pool: self,
            key: key.to_string(),
            lock,
            guard: Some(guard),
            reserved: false,
        }
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
    #[cfg(test)]
    pub async fn tab_count(&self) -> usize {
        tab_keys(&*self.sessions.lock().await).count()
    }

    /// True when the pool holds no session at all.
    pub async fn is_empty(&self) -> bool {
        self.sessions.lock().await.is_empty()
    }

    /// Removes every tab session that stood idle past the limit. A session
    /// whose driver is busy stays, because a statement still runs on it. A
    /// session inside an open transaction stays, because the close would
    /// roll back the work of the transaction. The default session stays,
    /// because the health check covers it.
    ///
    /// The probe of the transaction goes to the server, so the map is free
    /// while it runs. A probe that fails or passes [`TRANSACTION_PROBE_LIMIT`]
    /// removes the session, because the session then does not answer.
    pub async fn reap_idle(&self) {
        for (key, session) in self.tab_sessions().await {
            if session.idle_past(SESSION_IDLE_REAP).await && session.can_close().await {
                self.remove_same(&key, &session).await;
            }
        }
    }

    /// Closes the tab session that a request took least recently and that
    /// can close, to make room at the cap. Returns true when one closed.
    ///
    /// Only a session that stood idle for [`EVICT_IDLE_AFTER`] and that no
    /// request has in use can go. A session that is busy or inside an open
    /// transaction stays too, so a cap that only such sessions fill leaves
    /// the pool full.
    pub async fn evict_least_recent(&self) -> bool {
        let mut idle = Vec::new();
        for (key, session) in self.tab_sessions().await {
            if session.idle_past(EVICT_IDLE_AFTER).await {
                idle.push((session.used_at().await, key, session));
            }
        }
        idle.sort_by_key(|(used, _, _)| *used);
        for (_, key, session) in idle {
            if Arc::strong_count(&session) == 2
                && session.can_close().await
                && self.remove_unused(&key, &session).await
            {
                return true;
            }
        }
        false
    }

    /// The tab sessions of the pool, without the default session.
    async fn tab_sessions(&self) -> Vec<(String, Arc<Session>)> {
        self.sessions
            .lock()
            .await
            .iter()
            .filter(|(key, _)| *key != DEFAULT_SESSION)
            .map(|(key, session)| (key.clone(), session.clone()))
            .collect()
    }

    /// Removes the session of a key when it is still the given one. A new
    /// session that took the slot during a probe stays.
    async fn remove_same(&self, key: &str, session: &Arc<Session>) -> bool {
        self.remove_if(key, session, |_| true).await
    }

    /// Removes the session of a key when it is still the given one and only
    /// the pool and the caller keep a reference to it. A request that took
    /// the session during the probe keeps it in the pool.
    async fn remove_unused(&self, key: &str, session: &Arc<Session>) -> bool {
        self.remove_if(key, session, |current| Arc::strong_count(current) == 2)
            .await
    }

    async fn remove_if(
        &self,
        key: &str,
        session: &Arc<Session>,
        allowed: impl Fn(&Arc<Session>) -> bool,
    ) -> bool {
        let mut sessions = self.sessions.lock().await;
        if sessions
            .get(key)
            .is_some_and(|current| Arc::ptr_eq(current, session) && allowed(current))
        {
            sessions.remove(key);
            return true;
        }
        false
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
    use tokio::sync::Notify;

    /// A handle that records that it was asked to stop a statement.
    struct FlagCancel(Arc<AtomicBool>);

    #[async_trait]
    impl CancelHandle for FlagCancel {
        async fn cancel(&self) -> Result<()> {
            self.0.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    /// How the probe of an open transaction answers.
    enum Probe {
        Idle,
        Open,
        Fails,
        Hangs,
        /// Answers idle once the test sends the signal.
        Waits(Arc<Notify>),
    }

    /// A driver whose flags a test selects.
    struct StubDriver {
        cancelled: Option<Arc<AtomicBool>>,
        needs_ping: bool,
        keeps_connection_after_stop: bool,
        probe: Probe,
    }

    impl StubDriver {
        fn plain() -> Self {
            Self::probing(Probe::Idle)
        }

        fn probing(probe: Probe) -> Self {
            Self {
                cancelled: None,
                needs_ping: true,
                keeps_connection_after_stop: false,
                probe,
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
        async fn holds_open_transaction(&mut self) -> Result<bool> {
            match &self.probe {
                Probe::Idle => Ok(false),
                Probe::Open => Ok(true),
                Probe::Fails => Err(crate::error::Error::Connection("gone".into())),
                Probe::Hangs => std::future::pending().await,
                Probe::Waits(signal) => {
                    signal.notified().await;
                    Ok(false)
                }
            }
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
            probe: Probe::Idle,
        }));
        assert!(session.cancel_handle.is_some());
        assert!(!session.needs_ping);
        assert!(session.keeps_connection_after_stop);

        let plain = Session::new(Box::new(StubDriver::plain()));
        assert!(!plain.is_broken());
        plain.mark_broken();
        assert!(plain.is_broken());
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
            probe: Probe::Idle,
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
    async fn a_replacement_tells_one_run_alone() {
        let session = Session::new(Box::new(StubDriver::plain()));
        assert!(!session.take_replaced());
        session.mark_replacement();
        assert!(session.take_replaced());
        assert!(!session.take_replaced());
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
        pool.insert("t1", Session::new(Box::new(StubDriver::plain())))
            .await;
        assert!(pool.begin_open("t2").await.reserve().await);
        pool.insert("t2", Session::new(Box::new(StubDriver::plain())))
            .await;
        assert_eq!(pool.tab_count().await, 2);
        assert!(!pool.begin_open("t3").await.reserve().await);
        assert!(pool.begin_open(DEFAULT_SESSION).await.reserve().await);
        assert_eq!(pool.cap(), 2);
    }

    #[tokio::test]
    async fn an_open_that_runs_counts_against_the_cap() {
        let pool = pool();
        let mut first = pool.begin_open("t1").await;
        let mut second = pool.begin_open("t2").await;
        assert!(first.reserve().await);
        // A second reserve of one ticket takes no second place.
        assert!(first.reserve().await);
        assert!(second.reserve().await);
        let mut third = pool.begin_open("t3").await;
        assert!(!third.reserve().await);

        // The insert moves the place of the ticket into the pool, so the
        // count stays the same.
        first
            .insert(Session::new(Box::new(StubDriver::plain())))
            .await;
        assert!(!third.reserve().await);

        // A ticket that drops without an insert gives its place back.
        drop(second);
        assert!(third.reserve().await);
        assert_eq!(pool.tab_count().await, 1);
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

    /// Puts a session with the probe into the pool, idle past the limit.
    async fn idle_session(pool: &SessionPool, key: &str, probe: Probe) -> Arc<Session> {
        let session = pool
            .insert(key, Session::new(Box::new(StubDriver::probing(probe))))
            .await;
        session.age(SESSION_IDLE_REAP).await;
        session
    }

    #[tokio::test]
    async fn the_reap_keeps_a_session_inside_a_transaction() {
        let pool = pool();
        idle_session(&pool, "open", Probe::Open).await;
        idle_session(&pool, "failed", Probe::Fails).await;

        pool.reap_idle().await;

        assert!(pool.get("open").await.is_some());
        // A probe that fails means that the session does not answer.
        assert!(pool.get("failed").await.is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn the_reap_removes_a_session_whose_probe_does_not_answer() {
        let pool = pool();
        idle_session(&pool, "silent", Probe::Hangs).await;

        pool.reap_idle().await;

        assert!(pool.get("silent").await.is_none());
    }

    #[tokio::test]
    async fn the_reap_leaves_a_session_that_took_the_slot_during_the_probe() {
        let pool = Arc::new(pool());
        let signal = Arc::new(Notify::new());
        idle_session(&pool, "t1", Probe::Waits(signal.clone())).await;

        let sweep = tokio::spawn({
            let pool = pool.clone();
            async move { pool.reap_idle().await }
        });
        tokio::task::yield_now().await;
        let fresh = pool
            .insert("t1", Session::new(Box::new(StubDriver::plain())))
            .await;
        signal.notify_one();
        sweep.await.unwrap();

        let kept = pool.get("t1").await.unwrap();
        assert!(Arc::ptr_eq(&kept, &fresh));
    }

    #[tokio::test]
    async fn the_reap_keeps_a_session_that_a_request_took_lately() {
        let pool = pool();
        let session = idle_session(&pool, "t1", Probe::Idle).await;
        session.touch().await;
        pool.reap_idle().await;
        assert!(pool.get("t1").await.is_some());
    }

    #[tokio::test]
    async fn at_the_cap_the_least_recent_session_that_can_close_goes() {
        let pool = pool();
        let oldest = idle_session(&pool, "oldest", Probe::Open).await;
        oldest.age(EVICT_IDLE_AFTER * 4).await;
        drop(oldest);
        let busy = pool
            .insert("busy", Session::new(Box::new(StubDriver::plain())))
            .await;
        busy.age(EVICT_IDLE_AFTER * 3).await;
        let older = pool
            .insert("older", Session::new(Box::new(StubDriver::plain())))
            .await;
        older.age(EVICT_IDLE_AFTER * 2).await;
        drop(older);
        let newer = pool
            .insert("newer", Session::new(Box::new(StubDriver::plain())))
            .await;
        newer.age(EVICT_IDLE_AFTER).await;
        drop(newer);
        pool.insert(DEFAULT_SESSION, Session::new(Box::new(StubDriver::plain())))
            .await;

        let guard = busy.driver.lock().await;
        assert!(pool.evict_least_recent().await);
        drop(guard);

        // The session inside a transaction and the busy one stay.
        assert!(pool.get("oldest").await.is_some());
        assert!(pool.get("busy").await.is_some());
        assert!(pool.get("older").await.is_none());
        assert!(pool.get("newer").await.is_some());
    }

    #[tokio::test]
    async fn the_cap_keeps_a_session_that_a_request_took_lately() {
        let pool = pool();
        let session = idle_session(&pool, "t1", Probe::Idle).await;
        session.touch().await;
        drop(session);
        assert!(!pool.evict_least_recent().await);

        // Just under the limit is still too recent.
        let session = pool.get("t1").await.unwrap();
        session.age(EVICT_IDLE_AFTER - Duration::from_secs(5)).await;
        drop(session);
        assert!(!pool.evict_least_recent().await);
        assert!(pool.get("t1").await.is_some());
    }

    #[tokio::test]
    async fn the_cap_keeps_an_idle_session_that_a_request_has_in_use() {
        let pool = pool();
        let in_use = idle_session(&pool, "t1", Probe::Idle).await;
        assert!(!pool.evict_least_recent().await);
        assert!(pool.get("t1").await.is_some());

        // A request that takes the session during the probe keeps it too.
        let request = in_use.clone();
        assert!(!pool.remove_unused("t1", &in_use).await);
        drop(request);
        drop(in_use);
        assert!(pool.evict_least_recent().await);
        assert!(pool.get("t1").await.is_none());
    }

    #[tokio::test]
    async fn a_cap_full_of_open_transactions_evicts_nothing() {
        let pool = pool();
        idle_session(&pool, "open", Probe::Open).await;
        assert!(!pool.evict_least_recent().await);
        assert!(pool.get("open").await.is_some());
        assert!(
            !pool
                .remove_same(
                    "gone",
                    &Arc::new(Session::new(Box::new(StubDriver::plain())))
                )
                .await
        );
    }

    #[tokio::test]
    async fn a_pool_can_start_with_one_session() {
        let pool = SessionPool::with_session(
            2,
            DEFAULT_SESSION,
            Session::new(Box::new(StubDriver::plain())),
        );
        assert!(pool.get(DEFAULT_SESSION).await.is_some());
        assert_eq!(pool.tab_count().await, 0);
        assert_eq!(pool.cap(), 2);
    }

    #[tokio::test]
    async fn one_open_at_a_time_runs_for_each_key() {
        let pool = Arc::new(pool());
        let ticket = pool.begin_open("t1").await;
        let waiting = tokio::time::timeout(Duration::from_millis(20), pool.begin_open("t1")).await;
        assert!(waiting.is_err());

        // The open of another tab does not wait for the first one.
        let other = tokio::time::timeout(Duration::from_millis(20), pool.begin_open("t2")).await;
        assert!(other.is_ok());
        drop(other);

        let queued = tokio::spawn({
            let pool = pool.clone();
            async move {
                pool.begin_open("t1").await;
            }
        });
        tokio::task::yield_now().await;
        drop(ticket);
        queued.await.unwrap();
        assert!(pool.opening().locks.is_empty());
    }

    #[tokio::test]
    async fn a_wait_that_a_stop_dropped_leaves_no_lock() {
        let pool = pool();
        let ticket = pool.begin_open("t1").await;
        let dropped = tokio::time::timeout(Duration::from_millis(5), pool.begin_open("t1")).await;
        assert!(dropped.is_err());
        drop(ticket);
        assert!(pool.opening().locks.is_empty());

        // A ticket whose entry a later open replaced leaves the new entry.
        let mut stale = pool.begin_open("t2").await;
        let replaced = Arc::new(Mutex::new(()));
        pool.opening().locks.insert("t2".into(), replaced.clone());
        stale.lock = Arc::new(Mutex::new(()));
        drop(stale);
        assert!(Arc::ptr_eq(&pool.opening().locks["t2"], &replaced));

        // The next open takes away the entry that no request uses.
        drop(replaced);
        drop(pool.begin_open("t3").await);
        assert!(pool.opening().locks.is_empty());
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
