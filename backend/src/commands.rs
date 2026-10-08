//! The commands the user interface calls.

use crate::db::columnar::ChunkSink;
use crate::db::drivers::{
    athena::AthenaDriver, mssql, mssql::MssqlDriver, mysql, mysql::MysqlDriver, postgres,
    postgres::PostgresDriver, sqlite::SqliteDriver,
};
use crate::db::{
    self, drivers::CancelHandle, drivers::DatabaseDriver, AppColumn, Constraint, CreateQuery,
    Database, ExecOptions, IndexInfo, ObjectType, PartitionList, PlanMode, QueryParams,
    QueryResponse, RelationType, Routine, ScheduledEvent, Schema, SchemaSnapshot, Table,
    TableDetails, Trigger,
};
use crate::error::{Error, Result};
use crate::files;
use crate::history::HistoryEntry;
use crate::script::{self, ScriptStatement};
use crate::secrets::{self, SecretStore};
use crate::session::{Session, DEFAULT_SESSION};
use crate::sql::ParamValues;
use crate::state::{
    AppState, BackgroundRole, ConnectionHealth, ConnectionInfo, ConnectionStatusEvent,
    OpenConnection, CONNECTION_STATUS_EVENT,
};
use crate::storage::{AwsCredentialSource, DbType, SavedConnection};
use crate::store;
use std::sync::Arc;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Emitter, Runtime};
use tokio_util::sync::CancellationToken;

pub mod run_file;

/// Opens the driver that belongs to the engine of the record.
pub async fn open_driver(connection: &SavedConnection) -> Result<Box<dyn DatabaseDriver>> {
    connection.validate().map_err(Error::Configuration)?;
    match connection.db_type {
        DbType::Mssql => MssqlDriver::connect(connection).await,
        DbType::Mysql => MysqlDriver::connect(connection).await,
        DbType::Postgres => PostgresDriver::connect(connection).await,
        DbType::Sqlite => SqliteDriver::connect(connection).await,
        DbType::Athena => AthenaDriver::connect(connection).await,
    }
}

/// Sends the state of a connection to the user interface.
fn announce<R: Runtime>(
    app: &AppHandle<R>,
    connection_id: &str,
    health: ConnectionHealth,
    message: Option<String>,
) {
    let event = ConnectionStatusEvent {
        connection_id: connection_id.to_string(),
        health,
        message,
    };
    if let Err(error) = app.emit(CONNECTION_STATUS_EVENT, event) {
        log::warn!("The connection state could not be sent: {error}");
    }
}

/// Ends a reopen of one session that failed.
///
/// The session goes. When it was the last session of the connection, the
/// connection goes too and the window hears that it is disconnected. When
/// other sessions remain, the connection stays open, so the window hears
/// that it is connected. A connection that a connect or a disconnect
/// replaced in the meantime is left alone, because that command sent its own
/// state.
///
/// The window shows the reason of a state that has one as a notice. A caller
/// that also returns the error to the window gives no reason, because the
/// window then reports the same failure twice.
async fn reopen_failed<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    connection_id: &str,
    open: &OpenConnection,
    session_key: &str,
    reason: Option<String>,
) {
    let current = state.connection(connection_id).await;
    if !current.is_ok_and(|current| Arc::ptr_eq(&current.sessions, &open.sessions)) {
        return;
    }
    open.sessions.release(session_key).await;
    let health = if open.sessions.is_empty().await
        && state.remove_if_same(connection_id, &open.sessions).await
    {
        ConnectionHealth::Disconnected
    } else {
        ConnectionHealth::Connected
    };
    announce(app, connection_id, health, reason);
}

/// Runs work on the secret store on a blocking thread. A call to the
/// keychain can wait for a prompt of the operating system, and on an async
/// thread that wait would stop the other commands.
async fn with_store<T, F>(state: &AppState, work: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(&dyn SecretStore) -> Result<T> + Send + 'static,
{
    let store = Arc::clone(&state.secrets);
    off_thread(move || work(store.as_ref())).await
}

/// Fills the secrets of a record from the secret store, unless the caller
/// already gave them. A connection holds a password and, for Athena, a
/// secret access key and a session token.
async fn with_secrets(state: &AppState, connection: SavedConnection) -> Result<SavedConnection> {
    with_store(state, move |store| fill_secrets(store, connection)).await
}

/// Reads the secrets that a record does not give out of one store.
fn fill_secrets(
    store: &dyn SecretStore,
    mut connection: SavedConnection,
) -> Result<SavedConnection> {
    if connection.password.is_none() {
        connection.password = store.get(&connection.id)?;
    }
    // Only Athena uses the keys of AWS, so other engines skip two reads of
    // the keychain.
    if connection.db_type != DbType::Athena {
        return Ok(connection);
    }
    if connection.aws_secret_access_key.is_none() {
        connection.aws_secret_access_key = store.get(&secrets::aws_secret_key(&connection.id))?;
    }
    if connection.aws_session_token.is_none() {
        connection.aws_session_token = store.get(&secrets::aws_token_key(&connection.id))?;
    }
    Ok(connection)
}

/// Writes one secret of a connection, or takes it away.
///
/// The three fields follow one rule: a text keeps the secret, an empty text
/// takes it away, and an absent field leaves the store as it stands. The
/// form sends an absent field when the user did not touch it, so a saved
/// secret survives an edit of the other fields. Returns true when the store
/// holds the secret after the call.
fn store_secret(store: &dyn SecretStore, key: &str, value: Option<&str>) -> Result<bool> {
    match value {
        Some(text) if !text.is_empty() => {
            store.set(key, text)?;
            Ok(true)
        }
        Some(_) => {
            store.delete(key)?;
            Ok(false)
        }
        None => Ok(store.get(key)?.is_some()),
    }
}

/// Refuses a connection string that gives a password. The settings file
/// keeps the connection string as plain text, and the keychain keeps the
/// password of the Password box. A string that cannot be read is refused
/// too, because the connection cannot open with it.
fn refuse_password_in_string(connection: &SavedConnection) -> Result<()> {
    let Some(url) = connection.options.connection_url.as_deref() else {
        return Ok(());
    };
    let found = match connection.db_type {
        DbType::Mssql => mssql::string_has_password(url)?,
        DbType::Postgres => postgres::string_has_password(url)?,
        DbType::Mysql => mysql::string_has_password(url)?,
        DbType::Athena | DbType::Sqlite => false,
    };
    if found {
        return Err(Error::Configuration(
            "Remove the password from the connection string and enter it in the Password field. \
             Connection strings are saved as plain text in the settings file, while passwords \
             go in the system keychain."
                .to_string(),
        ));
    }
    Ok(())
}

/// The saved record of one connection, out of the file of connections. The
/// read runs on a blocking thread, because the first read of the file goes
/// to the disk.
async fn saved_record<R: Runtime>(app: &AppHandle<R>, id: &str) -> Result<Option<SavedConnection>> {
    let id = id.to_owned();
    with_app(app, move |app| {
        Ok(store::read_connections(app)?
            .into_iter()
            .find(|record| record.id == id))
    })
    .await
}

/// Opens a connection that the file of connections holds.
///
/// The command takes the identifier alone and reads every other field out of
/// that file. A stored secret therefore always goes to the server that the
/// user saved beside it, and a caller cannot pair a stored secret with a
/// server of its own choice.
#[tauri::command]
pub async fn connect<R: Runtime>(
    app: AppHandle<R>,
    connection_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<ConnectionInfo> {
    let id = connection_id;
    let Some(record) = saved_record(&app, &id).await? else {
        return Err(Error::Configuration(format!(
            "No saved connection has the ID '{id}'."
        )));
    };
    let full = with_secrets(&state, record).await?;

    match open_driver(&full).await {
        Ok(driver) => {
            // A connection that is still open under the identifier goes
            // first, with its statements and its background drivers.
            if state.remove(&id).await {
                stop_requests(state.take_requests_of(&id).await).await;
            }
            let info = state.insert(&id, OpenConnection::new(full, driver)).await;
            announce(&app, &id, ConnectionHealth::Connected, None);
            log::info!("The connection '{id}' is open.");
            Ok(info)
        }
        Err(error) => {
            // The window shows the connection as closed, so an older
            // connection under the identifier goes too. The command returns
            // the error, and the window reports it there, so the state goes
            // out with no reason.
            if state.remove(&id).await {
                stop_requests(state.take_requests_of(&id).await).await;
            }
            announce(&app, &id, ConnectionHealth::Disconnected, None);
            Err(error)
        }
    }
}

/// Fills the secrets of a record that a test names.
///
/// A test carries the form as the user filled it in, so the record can differ
/// from the saved one. A stored secret belongs to the server that the user
/// saved beside it, so the keychain answers only while every other field of
/// the record still matches the saved record. A record that names another
/// server must carry its own secrets, and the message asks the user for
/// them.
async fn with_secrets_for_test<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    connection: SavedConnection,
) -> Result<SavedConnection> {
    let Some(saved) = saved_record(app, &connection.id).await? else {
        return Ok(connection);
    };
    if saved.without_secrets() == connection.without_secrets() {
        return with_secrets(state, connection).await;
    }
    let held = [
        (
            connection.password.is_none(),
            connection.id.clone(),
            "password",
        ),
        (
            connection.aws_secret_access_key.is_none(),
            secrets::aws_secret_key(&connection.id),
            "secret access key",
        ),
        (
            connection.aws_session_token.is_none(),
            secrets::aws_token_key(&connection.id),
            "session token",
        ),
    ];
    let stored = with_store(state, move |store| {
        for (absent, key, name) in held {
            if absent && store.get(&key)?.is_some() {
                return Ok(Some(name));
            }
        }
        Ok(None)
    })
    .await?;
    match stored {
        Some(name) => Err(Error::Configuration(format!(
            "These settings differ from the saved connection, so the stored {name} can't be used. Enter the {name} to test your changes."
        ))),
        None => Ok(connection),
    }
}

/// Opens a connection, confirms that it answers, and closes it again.
#[tauri::command]
pub async fn test_connection<R: Runtime>(
    app: AppHandle<R>,
    connection: SavedConnection,
    state: tauri::State<'_, AppState>,
) -> Result<String> {
    let full = with_secrets_for_test(&app, &state, connection).await?;
    let mut driver = open_driver(&full).await?;
    driver.ping().await?;
    Ok("Connection successful.".to_string())
}

#[tauri::command]
pub async fn disconnect<R: Runtime>(
    app: AppHandle<R>,
    connection_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<()> {
    if state.remove(&connection_id).await {
        // A statement that still runs would pass its limit later and open a
        // session of a connection that is closed.
        stop_requests(state.take_requests_of(&connection_id).await).await;
        announce(&app, &connection_id, ConnectionHealth::Disconnected, None);
        log::info!("The connection '{connection_id}' is closed.");
    }
    Ok(())
}

/// Releases the session of one tab. The interface calls this when a tab
/// closes or moves to another connection. A statement that still runs on
/// the session completes, and the session then goes.
#[tauri::command]
pub async fn release_session(
    connection_id: String,
    tab_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<()> {
    if let Ok(open) = state.connection(&connection_id).await {
        if open.sessions.release(&tab_id).await {
            log::info!("The session of tab '{tab_id}' on '{connection_id}' is released.");
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn list_active_connections(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ConnectionInfo>> {
    let connections = state.connections.lock().await;
    Ok(connections
        .iter()
        .map(|(id, open)| ConnectionInfo {
            connection_id: id.clone(),
            capabilities: open.capabilities,
            dialect: open.dialect,
        })
        .collect())
}

/// Returns one healthy session of a connection for one tab, and the key the
/// session sits under.
///
/// A tab that already holds a session gets it back after a health check. A
/// tab without a session gets a new one, up to the cap of the pool. At the
/// cap, the sessions that other tabs left idle go first.
///
/// A Stop ends only the waits and the open of a new driver. A dropped open
/// leaves no session behind. A ping or a probe of a transaction always runs
/// to its end, because a drop in the middle of the exchange would leave a
/// session in the pool that reads the wrong answer next.
async fn session_for<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    connection_id: &str,
    tab_id: Option<&str>,
    token: &CancellationToken,
) -> Result<(OpenConnection, Arc<Session>, String)> {
    let open = state.connection(connection_id).await?;
    let key = open.session_key(tab_id);

    if let Some(session) = open.sessions.get(&key).await {
        let session =
            ensure_session_healthy(app, state, connection_id, &open, &key, session, token).await?;
        session.touch().await;
        return Ok((open, session, key));
    }

    // Two requests of one tab open one session. The open of another tab
    // does not wait here, because the ticket locks one key alone.
    let pool = open.sessions.clone();
    let mut ticket = unless_stopped(async { Ok(pool.begin_open(&key).await) }, token).await?;
    if let Some(session) = pool.get(&key).await {
        session.touch().await;
        return Ok((open, session, key));
    }
    if !ticket.reserve().await {
        pool.evict_least_recent().await;
        if !ticket.reserve().await {
            return Err(Error::Invalid(format!(
                "This connection is already using {} sessions. Close a tab or raise Max \
                 sessions in the connection settings.",
                pool.cap()
            )));
        }
    }

    let driver = open_until_stopped(state, &open, token).await?;
    let session = ticket.insert(Session::new(driver)).await;
    Ok((open, session, key))
}

/// Opens a new driver for a connection until the user presses Stop. An open
/// can wait for the full connect time of a server that does not answer, and
/// the Stop then ends the wait.
async fn open_until_stopped(
    state: &AppState,
    open: &OpenConnection,
    token: &CancellationToken,
) -> Result<Box<dyn DatabaseDriver>> {
    // The box stops the nesting of the future type here. Without it, the
    // layout of `session_for` passes the query depth limit of the compiler.
    unless_stopped(
        Box::pin(async {
            let full = with_secrets(state, open.descriptor.clone()).await?;
            open_driver(&full).await
        }),
        token,
    )
    .await
}

/// Confirms that a session that stood idle still answers, and opens a new
/// one in its place when it does not.
async fn ensure_session_healthy<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    connection_id: &str,
    open: &OpenConnection,
    key: &str,
    session: Arc<Session>,
    token: &CancellationToken,
) -> Result<Arc<Session>> {
    if !session.needs_ping || !session.needs_check().await {
        return Ok(session);
    }

    // One check at a time for each session. A command that waited here may
    // find that the first check put a new session into the slot, and then
    // takes that one, so two commands never open the same session twice.
    let health = session.clone();
    let _guard = health.health.lock().await;
    if let Some(current) = open.sessions.get(key).await {
        if !Arc::ptr_eq(&current, &session) {
            return Ok(current);
        }
    }
    if !session.needs_check().await {
        return Ok(session);
    }

    // A Stop ends the wait for the driver, and the ping then runs to its
    // end.
    let healthy = {
        let mut driver = unless_stopped(async { Ok(session.driver.lock().await) }, token).await?;
        answers_ping(driver.as_mut()).await
    };
    if healthy {
        session.mark_ok().await;
        return Ok(session);
    }

    announce(app, connection_id, ConnectionHealth::Reconnecting, None);
    log::warn!("A session of '{connection_id}' stopped answering. Opening it again.");

    match open_until_stopped(state, open, token).await {
        // The session that does not answer stays in its slot, so the next
        // request checks it again.
        Err(Error::Cancelled) => Err(Error::Cancelled),
        Ok(driver) => {
            let replacement = open.sessions.insert(key, Session::new(driver)).await;
            // The background drivers share the fate of the session that
            // stopped answering, so the next metadata read opens new ones.
            state.clear_background(connection_id).await;
            announce(app, connection_id, ConnectionHealth::Connected, None);
            Ok(replacement)
        }
        Err(error) => {
            // The request that asked for the session returns the error, and
            // the window reports it there.
            reopen_failed(app, state, connection_id, open, key, None).await;
            Err(error)
        }
    }
}

/// Gives the open connection for a read of the metadata.
///
/// The read sends no ping on the default session. A read of the catalog
/// runs on the background driver, which has a check of its own. A
/// connection with one session alone is a SQLite database in memory, which
/// has no network to lose, and a ping there waits behind the statement that
/// keeps the driver, with no limit.
async fn ensure_healthy<R: Runtime>(
    _app: &AppHandle<R>,
    state: &AppState,
    connection_id: &str,
) -> Result<OpenConnection> {
    state.connection(connection_id).await
}

/// Starts a metadata read. The read goes to a second connection when one can
/// open, so that the tree of the explorer does not wait behind a statement of
/// the user. A SQLite database in memory has one session alone, and the read
/// runs on it.
async fn metadata_read<'a, R: Runtime>(
    app: &AppHandle<R>,
    state: &'a AppState,
    connection_id: &'a str,
) -> Result<CatalogRead<'a>> {
    let open = ensure_healthy(app, state, connection_id).await?;
    catalog_read(state, connection_id, &open, BackgroundRole::Catalog).await
}

/// Starts a read of the catalog on the background driver of one role. The
/// limit of the read starts before the driver is found, so the health check
/// and the open of a new driver count against it.
async fn catalog_read<'a>(
    state: &'a AppState,
    connection_id: &'a str,
    open: &OpenConnection,
    role: BackgroundRole,
) -> Result<CatalogRead<'a>> {
    let deadline = tokio::time::Instant::now() + CATALOG_LIMIT;
    let session = background_session(state, connection_id, open, role, deadline).await?;
    Ok(CatalogRead::until(
        state,
        connection_id,
        session,
        CATALOG_LIMIT,
        deadline,
    ))
}

/// The longest time that a ping of a driver that stood idle takes. A server
/// that closed the connection without a word gives no answer, and the ping
/// then waits until the operating system gives up on the socket, which can
/// take many minutes. A ping past this limit counts as a driver that stopped
/// answering, so the caller opens a new one.
pub const PING_LIMIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Sends a ping, and gives true when the driver answers within
/// [`PING_LIMIT`].
async fn answers_ping(driver: &mut dyn DatabaseDriver) -> bool {
    matches!(
        tokio::time::timeout(PING_LIMIT, driver.ping()).await,
        Ok(Ok(()))
    )
}

/// Waits for the work until the deadline, and gives the timeout error of a
/// read of the catalog when the deadline comes first.
async fn before_deadline<T>(
    deadline: tokio::time::Instant,
    work: impl std::future::Future<Output = T>,
) -> Result<T> {
    tokio::time::timeout_at(deadline, work)
        .await
        .map_err(|_| Error::Timeout(CATALOG_LIMIT.as_secs()))
}

/// The longest time that one read of the catalog takes, the wait for the
/// driver included.
///
/// A read of the catalog can wait behind a lock of the server, for example
/// the lock of a change of a table that another session did not commit.
/// Without a limit, such a read keeps the driver of the metadata reads, and
/// every later read of the explorer, of the completions and of the scripts of
/// that connection waits behind it.
pub const CATALOG_LIMIT: std::time::Duration = std::time::Duration::from_secs(60);

/// The longest time that one statement of a background driver waits for a
/// lock of the server. The server then ends the statement with an error.
///
/// A read of the catalog on a server that is not busy ends in much less than
/// one second. A lock that stays for 5 seconds is almost always a transaction
/// that another session did not end, and that lock can stay for hours. A
/// read that waits for it would keep the driver for the full
/// [`CATALOG_LIMIT`], and every other read of the tree on that connection
/// would wait behind it. With this limit, the read of the locked object fails
/// and the other reads continue. A short change of a table, such as an
/// `ALTER TABLE` that ends in less than 5 seconds, does not stop a read.
///
/// The limit applies to each wait for a lock, so a read that waits for some
/// locks can take longer in total. [`CATALOG_LIMIT`] still stops it.
pub const LOCK_WAIT_LIMIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Sets [`LOCK_WAIT_LIMIT`] on a new background driver. The sessions of the
/// tabs never get it, so a statement of the user waits for a lock as the
/// server decides. A server that refuses the setting does not stop the read,
/// because the read then waits for locks up to [`CATALOG_LIMIT`].
async fn limit_lock_waits(driver: &mut dyn DatabaseDriver, connection_id: &str) {
    if let Err(error) = driver.limit_lock_waits(LOCK_WAIT_LIMIT).await {
        log::warn!(
            "The lock wait limit of the catalog reads of '{connection_id}' could not be set, \
             so a read can wait for a lock until the end of its time limit: {error}"
        );
    }
}

/// One read of the catalog on one session, under the limit of its time.
struct CatalogRead<'a> {
    state: &'a AppState,
    connection_id: &'a str,
    session: Arc<Session>,
    limit: std::time::Duration,
    deadline: tokio::time::Instant,
}

impl<'a> CatalogRead<'a> {
    fn new(
        state: &'a AppState,
        connection_id: &'a str,
        session: Arc<Session>,
        limit: std::time::Duration,
    ) -> Self {
        let deadline = tokio::time::Instant::now() + limit;
        Self::until(state, connection_id, session, limit, deadline)
    }

    /// A read whose limit started before this call, at `deadline - limit`.
    fn until(
        state: &'a AppState,
        connection_id: &'a str,
        session: Arc<Session>,
        limit: std::time::Duration,
        deadline: tokio::time::Instant,
    ) -> Self {
        Self {
            state,
            connection_id,
            session,
            limit,
            deadline,
        }
    }

    fn timeout(&self) -> Error {
        Error::Timeout(self.limit.as_secs())
    }

    /// Takes the driver of the session. When another exchange keeps the
    /// driver until the deadline, the read fails and the session stays,
    /// because that exchange has a limit of its own.
    async fn lock(&self) -> Result<tokio::sync::MutexGuard<'_, Box<dyn DatabaseDriver>>> {
        let guard = tokio::time::timeout_at(self.deadline, self.session.driver.lock())
            .await
            .map_err(|_| self.timeout())?;
        if self.session.is_broken() {
            return Err(Error::Connection(
                "A read before this one passed its time limit and closed the connection. Try \
                 again."
                    .to_string(),
            ));
        }
        Ok(guard)
    }

    /// Runs the read until the deadline. A read that passes the deadline is
    /// dropped in the middle of an exchange, so the server is asked to stop
    /// the statement and the session goes. The next read then opens a new
    /// session.
    async fn run<T>(&self, read: impl std::future::Future<Output = Result<T>>) -> Result<T> {
        match tokio::time::timeout_at(self.deadline, read).await {
            Ok(result) => result.map_err(Error::name_lock_wait),
            Err(_) => {
                self.discard().await;
                Err(self.timeout())
            }
        }
    }

    async fn discard(&self) {
        if let Some(handle) = self.session.cancel_handle.clone() {
            if let Err(error) = handle.cancel().await {
                log::warn!("The server did not stop the read of the catalog: {error}");
            }
        }
        if self.session.keeps_connection_after_stop {
            return;
        }
        let open = self.state.connection(self.connection_id).await.ok();
        // A database in memory exists in its one session alone. A new
        // session opens an empty database, so the session stays.
        if open.as_ref().is_some_and(|open| open.single_session) {
            log::warn!(
                "A read of the catalog of '{}' passed its limit. The session stays, because \
                 it keeps the only copy of the database.",
                self.connection_id
            );
            return;
        }
        log::warn!(
            "A read of the catalog of '{}' passed its limit, so its session closes.",
            self.connection_id
        );
        self.session.mark_broken();
        // The background driver of the other role stays, because its reads
        // do not use this session.
        if self
            .state
            .drop_background(self.connection_id, &self.session)
            .await
        {
            return;
        }
        // The read ran on the default session, because no second connection
        // could open. The next command opens a new default session.
        if let Some(open) = open {
            if let Some(current) = open.sessions.get(DEFAULT_SESSION).await {
                if Arc::ptr_eq(&current, &self.session) {
                    open.sessions.release(DEFAULT_SESSION).await;
                }
            }
        }
    }
}

/// Waits for the number of seconds, or forever when the number is zero.
async fn until_the_limit(timeout_secs: u64) {
    if timeout_secs == 0 {
        std::future::pending::<()>().await
    } else {
        tokio::time::sleep(std::time::Duration::from_secs(timeout_secs)).await
    }
}

/// How one exchange with a server ended.
pub enum Bounded<T> {
    /// The driver answered, with a result or with an error of its own.
    Answered(Result<T>),
    /// A limit ended the exchange before the driver answered.
    Stopped(Error),
}

/// How long the Stop button waits for the driver to report the error that the
/// server sends it. A stop reaches the server on a channel of its own, and the
/// statement then fails through the connection in the time of one round trip.
pub const STOP_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

/// The time the Stop button gives one session to report the failure that
/// the server sends it. A session with no way to ask the server to stop
/// gets none, because no such failure is coming.
fn stop_grace(session: &Session) -> std::time::Duration {
    if session.cancel_handle.is_some() {
        STOP_GRACE
    } else {
        std::time::Duration::ZERO
    }
}

/// Waits for the Stop button, and then for the driver to answer.
///
/// The stop already reached the server on a channel of its own, so the server
/// ends the statement and the driver reports that failure through the
/// connection. Waiting for it leaves the connection in a known place, and the
/// connection stays open. A driver that says nothing in this time is dropped
/// in the middle of a message, and the connection then goes.
async fn stopped_by_the_user(token: &CancellationToken, grace: std::time::Duration) {
    token.cancelled().await;
    if !grace.is_zero() {
        tokio::time::sleep(grace).await;
    }
}

/// Runs the work before a statement, such as the open of a session, until
/// the user presses Stop. An open of a session can wait for the full connect
/// time of a server that does not answer, and the Stop then ends the wait.
async fn unless_stopped<T>(
    work: impl std::future::Future<Output = Result<T>>,
    token: &CancellationToken,
) -> Result<T> {
    tokio::select! {
        result = work => result,
        () = token.cancelled() => Err(Error::Cancelled),
    }
}

/// Takes the driver of a session for one request, and then gives the
/// request the handle that stops its statement.
///
/// Another request of the same session can keep the driver, for example a
/// statement of a second tab on the one session of an SQLite database in
/// memory. A Stop during the wait ends the wait alone. The handle comes only
/// with the driver, because before that it stops the statement of the other
/// request.
async fn driver_for_request<'s>(
    state: &AppState,
    request_id: &str,
    session: &'s Session,
    token: &CancellationToken,
) -> Result<tokio::sync::MutexGuard<'s, Box<dyn DatabaseDriver>>> {
    let guard = tokio::select! {
        guard = session.driver.lock() => guard,
        () = token.cancelled() => return Err(Error::Cancelled),
    };
    if token.is_cancelled()
        || !state
            .arm_request(request_id, session.cancel_handle.clone())
            .await
    {
        return Err(Error::Cancelled);
    }
    if session.is_broken() {
        return Err(Error::Connection(
            "The session closed after a stop while this statement waited for it. Run the \
             statement again."
                .to_string(),
        ));
    }
    Ok(guard)
}

/// Runs one exchange with a server under the two limits that apply to it: the
/// Stop button of the user, and the time limit of the connection.
///
/// A limit that ends the exchange drops it in the middle of a message, so the
/// answer says which of the two ended it and the caller then closes the
/// connection. An error that the driver itself reports leaves the connection
/// open.
///
/// `grace` is the time the Stop button gives the driver to report the failure
/// that the server sends it. It is zero for a connection that has no way to
/// ask the server to stop, because no such failure is coming.
///
/// At the time limit, `cancel` asks the server to stop the statement, and the
/// work stays in use for `grace` more. An MS SQL driver sends its stop only
/// while a task reads the answer, so a drop at once would leave the
/// statement running on the server after the socket closes. A statement
/// that ends inside the grace gives the timeout error as an answer, and the
/// connection stays open.
pub async fn run_bounded<T, F>(
    work: F,
    token: &CancellationToken,
    timeout_secs: u64,
    grace: std::time::Duration,
    cancel: Option<Arc<dyn CancelHandle>>,
) -> Bounded<T>
where
    F: std::future::Future<Output = Result<T>>,
{
    tokio::pin!(work);
    let limit = tokio::select! {
        result = &mut work => return Bounded::Answered(stopped_answer(result, token)),
        () = stopped_by_the_user(token, grace) => return Bounded::Stopped(Error::Cancelled),
        () = until_the_limit(timeout_secs) => Error::Timeout(timeout_secs),
    };
    let Some(handle) = cancel.filter(|_| !grace.is_zero()) else {
        return Bounded::Stopped(limit);
    };
    if let Err(error) = handle.cancel().await {
        log::warn!("The server did not stop the statement at its time limit: {error}");
    }
    tokio::select! {
        result = &mut work => Bounded::Answered(match result {
            Err(error) if error.is_stop_reply() => Err(limit),
            other => other,
        }),
        () = tokio::time::sleep(grace) => Bounded::Stopped(limit),
    }
}

/// Gives the error of a statement that the user stopped as `Cancelled`.
///
/// The server reports a stop as an error of its own, for example PostgreSQL
/// SQLSTATE 57014. The same codes also come from a `statement_timeout` or a
/// `KILL` of another user, so the code alone cannot tell a stop from a failure.
/// The token of the request can, because only the Stop button sets it.
///
/// Only the reply of the engine to a stop, or a loss of the connection,
/// becomes `Cancelled`. A different failure that came at the same moment,
/// such as a syntax error or a deadlock, stays as it is, so the user sees
/// the real reason.
fn stopped_answer<T>(result: Result<T>, token: &CancellationToken) -> Result<T> {
    match result {
        Err(error)
            if token.is_cancelled()
                && !matches!(error, Error::Cancelled)
                && error.is_stop_reply() =>
        {
            log::debug!("The stopped statement ended with: {error}");
            Err(Error::Cancelled)
        }
        other => other,
    }
}

/// Puts the values of the named parameters into one statement.
///
/// The text keeps the placeholders of the dialect and the values travel bound,
/// so a value never becomes part of the statement. Athena binds no value, so
/// its parameters reach the service as literals of SQL.
///
/// A statement that contains no name is left as it stands and gets no
/// parameter, which keeps a script of more than one statement working.
pub fn prepare_parameters(
    query: &str,
    dialect: crate::sql::Dialect,
    values: Option<&ParamValues>,
) -> Result<(String, Option<QueryParams>)> {
    let empty = ParamValues::new();
    let values = values.unwrap_or(&empty);

    if dialect == crate::sql::Dialect::Athena {
        let names = crate::sql::find_parameters(query, dialect);
        if names.is_empty() {
            return Ok((query.to_string(), None));
        }
        let text = crate::sql::inline_parameters(query, dialect, values)
            .map_err(|name| missing_parameter(&name))?;
        return Ok((text, None));
    }

    let prepared = crate::sql::rewrite_parameters(query, dialect);
    if prepared.order.is_empty() {
        return Ok((query.to_string(), None));
    }
    let mut bound: QueryParams = Vec::new();
    for name in &prepared.order {
        let value = values.get(name).ok_or_else(|| missing_parameter(name))?;
        bound.push(db::QueryParam {
            value: value.clone(),
        });
    }
    Ok((prepared.sql, Some(bound)))
}

/// Moves the place of an error in the text that ran into the text that the
/// window sent.
///
/// The rewrite of the parameters changes a name such as `:id` into a
/// placeholder such as `$1`, so a column after a parameter does not match the
/// text of the user. The rewrite keeps the lines, so the line stays. The
/// column stays when the line is the same in both texts up to the column, and
/// is 1 otherwise.
fn in_sent_text<T>(outcome: Bounded<T>, sent: &str, ran: &str) -> Bounded<T> {
    let Bounded::Answered(Err(Error::Located {
        inner,
        line,
        column,
    })) = outcome
    else {
        return outcome;
    };
    let before = |text: &str| -> Option<String> {
        let line = text.lines().nth(line.checked_sub(1)? as usize)?;
        Some(
            line.chars()
                .take(column.saturating_sub(1) as usize)
                .collect(),
        )
    };
    let column = match sent == ran || before(sent) == before(ran) {
        true => column,
        false => 1,
    };
    Bounded::Answered(Err(Error::Located {
        inner,
        line,
        column,
    }))
}

/// The message for a parameter that the statement names and the request left
/// out.
fn missing_parameter(name: &str) -> Error {
    Error::Invalid(format!("Parameter ':{name}' needs a value."))
}

/// Lists the names of the parameters of a statement. The interface asks for a
/// value for each name before it runs the statement.
///
/// The lexer reads the whole script, so the work runs on a blocking thread.
/// A command that is not async runs on the main thread, and a long script
/// would then stop the window until the lexer ends.
#[tauri::command]
pub async fn query_parameters(query: String, dialect: crate::sql::Dialect) -> Result<Vec<String>> {
    off_thread(move || Ok(crate::sql::find_parameters(&query, dialect))).await
}

/// What one execution carries.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteRequest {
    pub connection_id: String,
    pub request_id: String,
    pub query: String,
    /// The tab that runs the statement. A request without a tab runs on the
    /// default session.
    #[serde(default)]
    pub tab_id: Option<String>,
    #[serde(default)]
    pub query_params: Option<ParamValues>,
    #[serde(default)]
    pub options: Option<ExecOptions>,
}

/// Runs a script and sends its rows to the window as binary chunks.
///
/// The rows travel on the channel while the read runs, so neither side keeps
/// the whole answer. The command itself gives no rows back: the last frame of
/// the channel carries the messages of the server and the numbers of the run.
#[tauri::command]
pub async fn execute_query<R: Runtime>(
    app: AppHandle<R>,
    request: ExecuteRequest,
    state: tauri::State<'_, AppState>,
    on_chunk: Channel<InvokeResponseBody>,
) -> Result<()> {
    let ExecuteRequest {
        connection_id,
        request_id,
        query,
        tab_id,
        query_params,
        options,
    } = request;
    let started = std::time::Instant::now();
    // The record goes in first, so a Stop while the session opens still
    // reaches the run.
    let token = state.start_request(&request_id, &connection_id).await;
    let prepared = async {
        let (open, session, key) =
            session_for(&app, &state, &connection_id, tab_id.as_deref(), &token).await?;
        let options = options.unwrap_or_else(|| open.descriptor.exec_options());
        let (query, bound) = prepare_parameters(&query, open.dialect, query_params.as_ref())?;
        Ok::<_, Error>((open, session, key, options, query, bound))
    }
    .await;
    let (open, session, key, options, ran, bound) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            state.end_request(&request_id).await;
            // The window waits for the end frame of every run, so a run that
            // fails before it starts sends one too.
            let _ = ChunkSink::new(on_chunk, 0).fail(started.elapsed().as_millis() as u64);
            return Err(error);
        }
    };

    let mut sink = ChunkSink::new(on_chunk, options.max_rows);
    let outcome = match driver_for_request(&state, &request_id, &session, &token).await {
        Ok(mut guard) => {
            run_bounded(
                guard.execute_stream(&ran, bound.as_ref(), &options, &mut sink),
                &token,
                options.timeout_secs,
                stop_grace(&session),
                session.cancel_handle.clone(),
            )
            .await
        }
        Err(error) => Bounded::Answered(Err(error)),
    };
    let outcome = in_sent_text(outcome, &query, &ran);

    state.end_request(&request_id).await;
    // A set that the row limit cut can stay in the registry, also when a
    // later statement of the script failed, because the grid shows that set.
    let kept = state
        .kept
        .keep(&request_id, &connection_id, sink.take_kept());
    sink.announce_kept(kept);
    match finish_run(&state, &connection_id, &open, &key, &session, outcome).await {
        Ok(summary) => sink.finish(summary),
        Err(error) => {
            // The messages that the server sent before the failure still
            // reach the window. A failure of the channel itself gives way to
            // the error of the run.
            let _ = sink.fail(started.elapsed().as_millis() as u64);
            Err(error)
        }
    }
}

/// What one request for a plan carries.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanRequest {
    pub connection_id: String,
    pub request_id: String,
    pub query: String,
    pub mode: PlanMode,
    /// The tab that asks for the plan. A request without a tab runs on the
    /// default session.
    #[serde(default)]
    pub tab_id: Option<String>,
    #[serde(default)]
    pub query_params: Option<ParamValues>,
    #[serde(default)]
    pub options: Option<ExecOptions>,
}

/// Reads the plan of one statement and gives it back as a result set.
#[tauri::command]
pub async fn explain_query<R: Runtime>(
    app: AppHandle<R>,
    request: PlanRequest,
    state: tauri::State<'_, AppState>,
) -> Result<QueryResponse> {
    let PlanRequest {
        connection_id,
        request_id,
        query,
        mode,
        tab_id,
        query_params,
        options,
    } = request;
    let token = state.start_request(&request_id, &connection_id).await;
    let prepared = async {
        let (open, session, key) =
            session_for(&app, &state, &connection_id, tab_id.as_deref(), &token).await?;
        let options = options.unwrap_or_else(|| open.descriptor.exec_options());
        // A plan needs the values of the parameters, because the plan of a
        // statement depends on the values it contains.
        let (query, bound) = prepare_parameters(&query, open.dialect, query_params.as_ref())?;
        Ok::<_, Error>((open, session, key, options, query, bound))
    }
    .await;
    let (open, session, key, options, ran, bound) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            state.end_request(&request_id).await;
            return Err(error);
        }
    };

    let outcome = match driver_for_request(&state, &request_id, &session, &token).await {
        Ok(mut guard) => {
            run_bounded(
                guard.explain(&ran, bound.as_ref(), mode, &options),
                &token,
                options.timeout_secs,
                stop_grace(&session),
                session.cancel_handle.clone(),
            )
            .await
        }
        Err(error) => Bounded::Answered(Err(error)),
    };
    let outcome = in_sent_text(outcome, &query, &ran);

    state.end_request(&request_id).await;
    finish_run(&state, &connection_id, &open, &key, &session, outcome).await
}

/// Closes the accounts of one exchange. A limit that ended the exchange asks
/// the server to stop the statement, and the session then goes unless the
/// driver reports that it stays fit for use.
async fn finish_run<T>(
    state: &AppState,
    connection_id: &str,
    open: &OpenConnection,
    session_key: &str,
    session: &Arc<Session>,
    outcome: Bounded<T>,
) -> Result<T> {
    match outcome {
        Bounded::Answered(Ok(value)) => {
            session.mark_ok().await;
            Ok(value)
        }
        Bounded::Answered(Err(error)) => Err(error),
        Bounded::Stopped(error) => {
            // The server was already asked to stop the statement: by the
            // Stop button through the record of the request, or by
            // `run_bounded` at the time limit.
            if session.keeps_connection_after_stop {
                return Err(error);
            }
            // The exchange was dropped in the middle of a message, so nothing
            // can be sent on this session again. The session leaves its slot,
            // and the next request of the tab opens a new one. The command
            // returns at once and does not wait for that open, which can take
            // the full connect time of a server that does not answer. The
            // other sessions of the connection stay as they are.
            session.mark_broken();
            if still_in_use(state, connection_id, open, session_key, session).await {
                open.sessions.release(session_key).await;
                log::info!(
                    "A session of '{connection_id}' closed after a stop. The next request opens \
                     a new one."
                );
            }
            Err(error)
        }
    }
}

/// True when the connection is still open with the same pool of sessions.
async fn still_open(state: &AppState, connection_id: &str, open: &OpenConnection) -> bool {
    state
        .connection(connection_id)
        .await
        .is_ok_and(|current| Arc::ptr_eq(&current.sessions, &open.sessions))
}

/// True when the connection is still open and the tab still holds the
/// session. A disconnect or a closed tab during a long stop leaves nothing to
/// open again.
async fn still_in_use(
    state: &AppState,
    connection_id: &str,
    open: &OpenConnection,
    session_key: &str,
    session: &Arc<Session>,
) -> bool {
    if !still_open(state, connection_id, open).await {
        return false;
    }
    match open.sessions.get(session_key).await {
        Some(held) => Arc::ptr_eq(&held, session),
        None => false,
    }
}

/// Asks the server to stop a statement, and stops waiting for it.
///
/// The record of the statement carries the handle of the session that runs
/// it, so the stop reaches the correct session. The identifier of the
/// connection stays in the call for older callers, but the lookup does not
/// need it.
/// Asks the server to stop each statement, and stops waiting for it.
async fn stop_requests(requests: Vec<crate::state::RunningRequest>) {
    // The stops go out at the same time, and each one waits for its server
    // for STOP_GRACE at most. A stop of PostgreSQL or MySQL opens a new
    // connection, so on a server that does not answer, eight stops one after
    // the other would keep a disconnect waiting for eight connect times.
    futures_util::future::join_all(requests.into_iter().map(|request| async move {
        // The handle does not need the lock of the driver, so it works
        // while the statement runs.
        if let Some(handle) = request.cancel_handle {
            match tokio::time::timeout(STOP_GRACE, handle.cancel()).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => log::warn!("The server did not stop the statement: {error}"),
                Err(_) => log::warn!(
                    "The server did not answer the stop within {} seconds.",
                    STOP_GRACE.as_secs()
                ),
            }
        }
        request.token.cancel();
    }))
    .await;
}

#[tauri::command]
pub async fn cancel_query(
    #[allow(unused_variables)] connection_id: String,
    request_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<()> {
    stop_requests(state.take_request(&request_id).await.into_iter().collect()).await;
    Ok(())
}

#[tauri::command]
pub async fn list_databases<R: Runtime>(
    app: AppHandle<R>,
    connection_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Database>> {
    let read = metadata_read(&app, &state, &connection_id).await?;
    let mut guard = read.lock().await?;
    read.run(guard.list_databases()).await
}

#[tauri::command]
pub async fn list_schemas<R: Runtime>(
    app: AppHandle<R>,
    connection_id: String,
    database: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Schema>> {
    let read = metadata_read(&app, &state, &connection_id).await?;
    let mut guard = read.lock().await?;
    read.run(guard.list_schemas(&database)).await
}

/// Names one schema of one connection. The commands that list the relations
/// or the routines of a schema take this request.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaScope {
    pub connection_id: String,
    pub database: String,
    #[serde(default)]
    pub schema_name: Option<String>,
}

/// Names one relation of one connection. The commands that list the parts of
/// a relation take this request.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableScope {
    pub connection_id: String,
    pub database: String,
    #[serde(default)]
    pub schema_name: Option<String>,
    pub table_name: String,
}

#[tauri::command]
pub async fn list_tables<R: Runtime>(
    app: AppHandle<R>,
    request: SchemaScope,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Table>> {
    let read = metadata_read(&app, &state, &request.connection_id).await?;
    let mut guard = read.lock().await?;
    read.run(guard.list_tables(&request.database, request.schema_name.as_deref()))
        .await
}

#[tauri::command]
pub async fn list_columns<R: Runtime>(
    app: AppHandle<R>,
    request: TableScope,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<AppColumn>> {
    let read = metadata_read(&app, &state, &request.connection_id).await?;
    let mut guard = read.lock().await?;
    read.run(guard.list_columns(
        &request.database,
        request.schema_name.as_deref(),
        &request.table_name,
    ))
    .await
}

#[tauri::command]
pub async fn list_routines<R: Runtime>(
    app: AppHandle<R>,
    request: SchemaScope,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Routine>> {
    let read = metadata_read(&app, &state, &request.connection_id).await?;
    let mut guard = read.lock().await?;
    read.run(guard.list_routines(&request.database, request.schema_name.as_deref()))
        .await
}

#[tauri::command]
pub async fn list_indexes<R: Runtime>(
    app: AppHandle<R>,
    request: TableScope,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<IndexInfo>> {
    let read = metadata_read(&app, &state, &request.connection_id).await?;
    let mut guard = read.lock().await?;
    read.run(guard.list_indexes(
        &request.database,
        request.schema_name.as_deref(),
        &request.table_name,
    ))
    .await
}

#[tauri::command]
pub async fn list_constraints<R: Runtime>(
    app: AppHandle<R>,
    request: TableScope,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Constraint>> {
    let read = metadata_read(&app, &state, &request.connection_id).await?;
    let mut guard = read.lock().await?;
    read.run(guard.list_constraints(
        &request.database,
        request.schema_name.as_deref(),
        &request.table_name,
    ))
    .await
}

#[tauri::command]
pub async fn list_triggers<R: Runtime>(
    app: AppHandle<R>,
    request: TableScope,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Trigger>> {
    let read = metadata_read(&app, &state, &request.connection_id).await?;
    let mut guard = read.lock().await?;
    read.run(guard.list_triggers(
        &request.database,
        request.schema_name.as_deref(),
        &request.table_name,
    ))
    .await
}

#[tauri::command]
pub async fn list_events<R: Runtime>(
    app: AppHandle<R>,
    request: SchemaScope,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ScheduledEvent>> {
    let read = metadata_read(&app, &state, &request.connection_id).await?;
    let mut guard = read.lock().await?;
    read.run(guard.list_events(&request.database, request.schema_name.as_deref()))
        .await
}

#[tauri::command]
pub async fn list_partitions<R: Runtime>(
    app: AppHandle<R>,
    request: TableScope,
    state: tauri::State<'_, AppState>,
) -> Result<PartitionList> {
    let read = metadata_read(&app, &state, &request.connection_id).await?;
    let mut guard = read.lock().await?;
    read.run(guard.list_partitions(
        &request.database,
        request.schema_name.as_deref(),
        &request.table_name,
    ))
    .await
}

/// Collects everything the properties dialog shows about one relation: the
/// facts, the columns, the indexes and the constraints.
///
/// The four lists travel together, so the dialog opens with one call and the
/// lock of the driver is taken once.
#[tauri::command]
pub async fn table_details<R: Runtime>(
    app: AppHandle<R>,
    request: TableScope,
    state: tauri::State<'_, AppState>,
) -> Result<TableDetails> {
    let read = metadata_read(&app, &state, &request.connection_id).await?;
    let database = &request.database;
    let schema = request.schema_name.as_deref();
    let table = &request.table_name;
    let mut guard = read.lock().await?;

    // The limit applies to the four reads together, because the dialog
    // waits for all four.
    read.run(async {
        Ok(TableDetails {
            facts: guard.table_facts(database, schema, table).await?,
            columns: guard.list_columns(database, schema, table).await?,
            indexes: guard.list_indexes(database, schema, table).await?,
            constraints: guard.list_constraints(database, schema, table).await?,
        })
    })
    .await
}

/// The number of columns a snapshot keeps when the caller names no bound.
pub const DEFAULT_SNAPSHOT_COLUMNS: usize = 20_000;

/// What a read of one schema carries.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotRequest {
    pub connection_id: String,
    pub database: String,
    #[serde(default)]
    pub max_columns: Option<usize>,
    #[serde(default)]
    pub own_connection: Option<bool>,
}

/// Reads every relation and every column of one database, for the
/// completions of the editor.
///
/// The read runs on a second driver of the same record, so that it never
/// waits behind a statement of the user and no statement of the user waits
/// behind it. The reads of the tree use a different driver, because a read
/// of a large schema can keep its driver for up to [`CATALOG_LIMIT`]. A
/// caller that asks for the one session, and a second driver that cannot
/// open, put the read on the session of the user instead.
#[tauri::command]
pub async fn schema_snapshot<R: Runtime>(
    app: AppHandle<R>,
    request: SnapshotRequest,
    state: tauri::State<'_, AppState>,
) -> Result<SchemaSnapshot> {
    let open = ensure_healthy(&app, &state, &request.connection_id).await?;
    let limit = request
        .max_columns
        .unwrap_or(DEFAULT_SNAPSHOT_COLUMNS)
        .max(1);

    let read = match request.own_connection.unwrap_or(true) {
        true => {
            catalog_read(
                &state,
                &request.connection_id,
                &open,
                BackgroundRole::Snapshot,
            )
            .await?
        }
        false => CatalogRead::new(
            &state,
            &request.connection_id,
            open.default_session().await?,
            CATALOG_LIMIT,
        ),
    };
    let mut guard = read.lock().await?;
    read.run(guard.schema_snapshot(&request.database, limit))
        .await
}

/// Confirms that a background driver that stood idle still answers. A driver
/// that gives no answer must go, because the server closed its side.
///
/// The wait for the driver ends at the deadline of the read, because another
/// read can keep the driver until its own deadline. The ping has
/// [`PING_LIMIT`] of its own, so it can end a little after the deadline. A
/// ping that the deadline cut in the middle of its exchange would leave the
/// driver in its slot with half an answer still on the connection.
async fn background_answers(
    session: &Arc<Session>,
    deadline: tokio::time::Instant,
) -> Result<bool> {
    if !session.needs_ping || !session.needs_check().await {
        return Ok(true);
    }
    // One check at a time for each driver, so two reads send one ping.
    let _guard = before_deadline(deadline, session.health.lock()).await?;
    if !session.needs_check().await {
        return Ok(true);
    }
    let healthy = {
        let mut driver = before_deadline(deadline, session.driver.lock()).await?;
        answers_ping(driver.as_mut()).await
    };
    if healthy {
        session.mark_ok().await;
    }
    Ok(healthy)
}

/// Returns the background session of a connection for one role, and opens
/// one when the connection has none or when the one it has stopped
/// answering. A driver that cannot open gives the default session, because a
/// snapshot that waits is better than no completions. The read of the
/// password and the open of a new driver end at the deadline. A new driver
/// gets [`LOCK_WAIT_LIMIT`] before its first read.
///
/// A connection with one session alone gives its default session and opens
/// no second connection. A second connection to a SQLite database in memory
/// opens a separate empty database, so the tree would show no table.
async fn background_session(
    state: &AppState,
    connection_id: &str,
    open: &OpenConnection,
    role: BackgroundRole,
    deadline: tokio::time::Instant,
) -> Result<Arc<Session>> {
    if open.single_session {
        return open.default_session().await;
    }
    if let Some(session) = state.background_session(connection_id, role).await {
        if background_answers(&session, deadline).await? {
            return Ok(session);
        }
        log::warn!("The {role:?} driver of '{connection_id}' stopped answering. Opening it again.");
        state.drop_background(connection_id, &session).await;
    }
    // One read opens the driver, and the reads that start at the same time
    // wait for it and then take it.
    let opening = state.background_open_lock(connection_id, role);
    let _opening = before_deadline(deadline, opening.lock()).await?;
    if let Some(session) = state.background_session(connection_id, role).await {
        return Ok(session);
    }
    let full = match before_deadline(deadline, with_secrets(state, open.descriptor.clone())).await?
    {
        Ok(full) => full,
        Err(error) => {
            log::warn!("The password of '{connection_id}' could not be read: {error}");
            return open.default_session().await;
        }
    };
    // The box stops the nesting of the future type here. Without it, the
    // layout of `metadata_read` passes the query depth limit of the compiler
    // in the build that measures coverage.
    let mut driver = match before_deadline(deadline, Box::pin(open_driver(&full))).await? {
        Ok(driver) => driver,
        Err(error) => {
            log::warn!(
                "A second connection for '{connection_id}' could not open, so the schema is \
                 read on the session of the user: {error}"
            );
            return open.default_session().await;
        }
    };
    // The limit goes on the driver before any read uses it. A deadline that
    // cuts the exchange drops the driver, which is not yet in its slot.
    before_deadline(
        deadline,
        Box::pin(limit_lock_waits(driver.as_mut(), connection_id)),
    )
    .await?;
    // A disconnect or a new connect during the open replaced the connection.
    // The driver then serves this read alone and closes, so the slot never
    // keeps a driver of a closed connection.
    if !still_open(state, connection_id, open).await {
        return Ok(Arc::new(Session::new(driver)));
    }
    Ok(state
        .set_background_driver(connection_id, role, driver)
        .await)
}

/// Builds one statement for an object of the tree: the CREATE text, or a
/// SELECT, an INSERT or an UPDATE built from the column list.
///
/// The CREATE text comes from the engine when the engine keeps it. An engine
/// that keeps no text, and an answer that holds nothing, give a draft built
/// from the columns.
#[tauri::command]
pub async fn script_object<R: Runtime>(
    app: AppHandle<R>,
    request: ScriptRequest,
    state: tauri::State<'_, AppState>,
) -> Result<String> {
    let ScriptRequest {
        connection_id,
        database,
        schema_name,
        table_name,
        parent_name,
        target,
        statement,
    } = request;

    let open = ensure_healthy(&app, &state, &connection_id).await?;
    let dialect = open.dialect;
    let name = dialect.qualified_name(database.as_deref(), schema_name.as_deref(), &table_name);

    // The work only reads the catalog, so it runs on the driver of the
    // metadata reads and leaves the sessions of the tabs free.
    let read = catalog_read(&state, &connection_id, &open, BackgroundRole::Catalog).await?;
    let mut guard = read.lock().await?;
    let relation = match target {
        ScriptTarget::Relation(relation) => relation,
        ScriptTarget::Object(object_type) => {
            let place = ObjectPlace {
                database: database.as_deref(),
                schema: schema_name.as_deref(),
                parent: parent_name.as_deref(),
                name: &table_name,
            };
            return read
                .run(object_script(guard.as_mut(), place, object_type, statement))
                .await;
        }
    };
    let (columns, from_engine) = read
        .run(async {
            let columns = guard
                .list_columns(
                    database.as_deref().unwrap_or_default(),
                    schema_name.as_deref(),
                    &table_name,
                )
                .await?;
            let from_engine = match statement {
                ScriptStatement::Create => match guard.create_query(
                    database.as_deref(),
                    schema_name.as_deref(),
                    &table_name,
                    relation,
                ) {
                    Some(query) => {
                        let response = guard
                            .execute_query(&query.sql, None, &ExecOptions::default())
                            .await?;
                        create_text_of(&response, &query)
                    }
                    None => None,
                },
                _ => None,
            };
            Ok((columns, from_engine))
        })
        .await?;

    drop(guard);
    script_text(dialect, &name, statement, &columns, from_engine)
}

/// What the user interface asks for when it wants the text of one object.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptRequest {
    pub connection_id: String,
    pub database: Option<String>,
    pub schema_name: Option<String>,
    /// The name of the object. A trigger and an event give their own name
    /// here, as a view and a synonym do.
    pub table_name: String,
    /// The relation of a trigger. Any other object has none.
    #[serde(default)]
    pub parent_name: Option<String>,
    /// The type of the object, which decides where the CREATE text lives.
    pub target: ScriptTarget,
    /// The statement the user asked for.
    pub statement: ScriptStatement,
}

/// The type of the object of a script. The interface sends one word. The
/// words of the relations and the words of a trigger and an event are all
/// different, so the two enums can share the field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(untagged)]
pub enum ScriptTarget {
    Relation(RelationType),
    Object(ObjectType),
}

/// Where a trigger or an event lives, and its name.
#[derive(Debug, Clone, Copy)]
struct ObjectPlace<'a> {
    database: Option<&'a str>,
    schema: Option<&'a str>,
    parent: Option<&'a str>,
    name: &'a str,
}

/// Reads the CREATE text of a trigger or an event from the engine. The
/// catalog of the columns says nothing about the body of such an object, so
/// no draft can take the place of the text. A SELECT, an INSERT and an
/// UPDATE do not apply to such an object.
async fn object_script(
    driver: &mut dyn DatabaseDriver,
    place: ObjectPlace<'_>,
    object_type: ObjectType,
    statement: ScriptStatement,
) -> Result<String> {
    if statement != ScriptStatement::Create {
        return Err(Error::Invalid(
            "Triggers and events can only be scripted as CREATE.".to_string(),
        ));
    }
    let no_text = || {
        Error::Invalid(format!(
            "The database returned no CREATE script for '{}'.",
            place.name
        ))
    };
    let query = driver
        .object_create_query(
            place.database,
            place.schema,
            place.parent,
            place.name,
            object_type,
        )
        .ok_or_else(no_text)?;
    let response = driver
        .execute_query(&query.sql, None, &ExecOptions::default())
        .await?;
    create_text_of(&response, &query).ok_or_else(no_text)
}

/// Selects the statement that the user asked for. The text of
/// the engine wins for the CREATE form, and a draft serves when there is no
/// such text.
fn script_text(
    dialect: crate::sql::Dialect,
    name: &str,
    statement: ScriptStatement,
    columns: &[AppColumn],
    from_engine: Option<String>,
) -> Result<String> {
    if statement == ScriptStatement::Select {
        return Ok(script::select_statement(dialect, name, columns));
    }
    if let (ScriptStatement::Create, Some(text)) = (statement, from_engine) {
        return Ok(text);
    }
    if columns.is_empty() {
        // The other forms are built from the columns, and a relation that
        // reports none gives no statement at all.
        return Err(Error::Invalid(
            "The object has no columns, so the statement can't be generated.".to_string(),
        ));
    }
    Ok(match statement {
        ScriptStatement::Insert => script::insert_statement(dialect, name, columns),
        ScriptStatement::Update => script::update_statement(dialect, name, columns),
        // The select form left this function above.
        ScriptStatement::Create | ScriptStatement::Select => {
            script::create_draft(dialect, name, columns)
        }
    })
}

/// Reads the CREATE text from the answer of its query. A query with a
/// terminator puts a text that the splitter would cut between `DELIMITER`
/// commands.
pub(crate) fn create_text_of(response: &QueryResponse, query: &CreateQuery) -> Option<String> {
    let text = text_of_column(response, query.column)?;
    Some(if query.delimited {
        crate::sql::within_delimiter(&text)
    } else {
        text
    })
}

/// Reads one column of every row as text and joins the lines. Athena gives
/// the CREATE text one line for each row, and the other engines give it in
/// one row. An answer that holds no text gives `None`.
fn text_of_column(response: &QueryResponse, column: usize) -> Option<String> {
    let lines: Vec<String> = response
        .results
        .iter()
        .flat_map(|set| set.rows.iter())
        .filter_map(|row| match row.get(column) {
            Some(serde_json::Value::String(text)) => Some(text.clone()),
            _ => None,
        })
        .collect();
    let text = lines.join("\n");
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Builds the statement that reads the first rows of one relation. The
/// backend builds it so that every name is quoted for the engine.
#[tauri::command]
pub async fn preview_query(
    request: PreviewRequest,
    state: tauri::State<'_, AppState>,
) -> Result<String> {
    let open = state.connection(&request.connection_id).await?;
    Ok(open.dialect.preview_query(
        request.database.as_deref(),
        request.schema_name.as_deref(),
        &request.table_name,
        request.limit.unwrap_or(1000),
    ))
}

/// What the preview of one relation carries.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewRequest {
    pub connection_id: String,
    #[serde(default)]
    pub database: Option<String>,
    #[serde(default)]
    pub schema_name: Option<String>,
    pub table_name: String,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Quotes a name for the engine of one connection, so that the user
/// interface can build a statement without knowing the rules.
#[tauri::command]
pub async fn quote_identifier(
    connection_id: String,
    name: String,
    state: tauri::State<'_, AppState>,
) -> Result<String> {
    let open = state.connection(&connection_id).await?;
    Ok(open.dialect.quote_identifier(&name))
}

// --- The saved connections ---

#[tauri::command]
pub async fn get_connections<R: Runtime>(app: AppHandle<R>) -> Result<Vec<SavedConnection>> {
    off_thread(move || store::read_connections(&app)).await
}

#[tauri::command]
pub async fn save_connection<R: Runtime>(
    app: AppHandle<R>,
    connection: SavedConnection,
    state: tauri::State<'_, AppState>,
) -> Result<()> {
    connection.validate().map_err(Error::Configuration)?;
    refuse_password_in_string(&connection)?;

    // A stored secret belongs to the server it was saved for. When the
    // record names another server and gives no new secret, the old secret
    // goes, so a changed host cannot receive the password of the old one.
    let moved = saved_record(&app, &connection.id)
        .await?
        .is_some_and(|saved| target_changed(&saved, &connection));
    let no_keys = connection.options.aws_credential_source != AwsCredentialSource::Keys;
    let writes = [
        (
            connection.id.clone(),
            kept(connection.password.as_deref(), moved).map(str::to_string),
        ),
        (
            secrets::aws_secret_key(&connection.id),
            kept(
                connection.aws_secret_access_key.as_deref(),
                moved || no_keys,
            )
            .map(str::to_string),
        ),
        (
            secrets::aws_token_key(&connection.id),
            kept(connection.aws_session_token.as_deref(), moved || no_keys).map(str::to_string),
        ),
    ];
    with_store(&state, move |store| {
        for (key, value) in &writes {
            store_secret(store, key, value.as_deref())?;
        }
        Ok(())
    })
    .await?;

    let record = connection.without_secrets();
    off_thread(move || store::write_connection(&app, &record)).await
}

/// The secret to store: the new one, or an empty text that takes the stored
/// one away when `drop` is set and the record gives no new secret.
fn kept(value: Option<&str>, drop: bool) -> Option<&str> {
    match value {
        None if drop => Some(""),
        value => value,
    }
}

/// True when a record names another server than the saved record, so a
/// secret of the saved record does not belong to it.
///
/// The fields compare without the spaces at their ends, and a blank field is
/// the same as a missing one, so a field that the form cleared names no new
/// server.
fn target_changed(saved: &SavedConnection, record: &SavedConnection) -> bool {
    use crate::db::drivers::non_empty;
    let target = |connection: &SavedConnection| {
        let options = &connection.options;
        (
            connection.db_type,
            connection.port,
            [
                non_empty(&connection.host).map(str::to_owned),
                non_empty(&connection.user).map(str::to_owned),
                non_empty(&options.instance_name).map(str::to_owned),
                non_empty(&options.connection_url).map(str::to_owned),
                non_empty(&options.file_path).map(str::to_owned),
                non_empty(&options.aws_region).map(str::to_owned),
                non_empty(&options.aws_access_key_id).map(str::to_owned),
            ],
        )
    };
    target(saved) != target(record)
}

#[tauri::command]
pub async fn delete_connection<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    state: tauri::State<'_, AppState>,
) -> Result<()> {
    close_deleted_connection(&app, &state, &id).await;
    off_thread(move || store::delete_connection(&app, &id)).await
}

/// Closes a connection that the user deletes: its statements stop, the
/// window hears that it is closed, and every key of it leaves the keychain.
/// A secret that stays in the keychain is written to the log, because the
/// delete of the record itself still goes ahead.
async fn close_deleted_connection<R: Runtime>(app: &AppHandle<R>, state: &AppState, id: &str) {
    if state.remove(id).await {
        stop_requests(state.take_requests_of(id).await).await;
        announce(app, id, ConnectionHealth::Disconnected, None);
    }
    let keys = [
        id.to_string(),
        secrets::aws_secret_key(id),
        secrets::aws_token_key(id),
    ];
    let failures = with_store(state, move |store| {
        Ok(keys
            .iter()
            .filter_map(|key| store.delete(key).err())
            .collect::<Vec<_>>())
    })
    .await
    .unwrap_or_else(|error| vec![error]);
    for error in failures {
        log::warn!("A secret of the deleted connection '{id}' stayed in the keychain: {error}");
    }
}

/// Runs blocking work on a thread of its own, so a slow disk or keychain
/// does not stop the async threads that serve the other commands.
///
/// A test runs the work in place, because each test thread has a folder of
/// settings of its own.
#[cfg(not(test))]
async fn off_thread<T, F>(work: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| Error::Storage(format!("The background work stopped: {error}")))?
}

#[cfg(test)]
async fn off_thread<T, F>(work: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    work()
}

/// Runs blocking work that needs the application, such as a read or a write
/// of a file of the settings, on a thread of its own.
async fn with_app<R: Runtime, T, F>(app: &AppHandle<R>, work: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(&AppHandle<R>) -> Result<T> + Send + 'static,
{
    let app = app.clone();
    off_thread(move || work(&app)).await
}

/// Reads the files of the settings and gives each problem found in them,
/// such as a damaged file that was moved aside. The window shows each one
/// once at start.
#[tauri::command]
pub async fn storage_problems<R: Runtime>(app: AppHandle<R>) -> Result<Vec<String>> {
    off_thread(move || Ok(store::storage_problems(&app))).await
}

// --- The query history and the saved queries ---

#[tauri::command]
pub async fn get_history<R: Runtime>(app: AppHandle<R>) -> Result<Vec<HistoryEntry>> {
    off_thread(move || store::read_history(&app)).await
}

#[tauri::command]
pub async fn add_history_entry<R: Runtime>(app: AppHandle<R>, entry: HistoryEntry) -> Result<()> {
    off_thread(move || store::add_history(&app, entry)).await
}

#[tauri::command]
pub async fn clear_history<R: Runtime>(app: AppHandle<R>) -> Result<()> {
    off_thread(move || store::clear_history(&app)).await
}

// --- The open tabs ---

#[tauri::command]
pub async fn get_workspace<R: Runtime>(app: AppHandle<R>) -> Result<serde_json::Value> {
    off_thread(move || store::read_workspace(&app)).await
}

#[tauri::command]
pub async fn save_workspace<R: Runtime>(
    app: AppHandle<R>,
    workspace: serde_json::Value,
) -> Result<()> {
    off_thread(move || store::write_workspace(&app, workspace)).await
}

/// The form a file export takes.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportFormat {
    Csv,
    Json,
    Xlsx,
}

impl ExportFormat {
    /// The name of the filter of the save dialog, and the extension of the
    /// file.
    fn file_type(self) -> (&'static str, &'static str) {
        match self {
            ExportFormat::Csv => ("CSV", "csv"),
            ExportFormat::Json => ("JSON", "json"),
            ExportFormat::Xlsx => ("Excel", "xlsx"),
        }
    }
}

/// What an export to a file needs to know. The backend asks the user for
/// the path itself, so the interface names only the file to suggest.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    pub connection_id: String,
    pub request_id: String,
    pub query: String,
    /// The file name that the save dialog suggests.
    pub default_name: String,
    pub format: ExportFormat,
    /// The row limit of the export, which is higher than the one of the view.
    pub max_rows: usize,
    /// The tab that runs the export. The export runs on the session of the
    /// tab, so it sees the temporary tables the tab created.
    #[serde(default)]
    pub tab_id: Option<String>,
    /// The values of the named parameters of the statement.
    #[serde(default)]
    pub query_params: Option<ParamValues>,
}

/// What one export wrote.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSummary {
    pub rows: usize,
    /// True when even the higher row limit of the export stopped the read.
    pub truncated: bool,
    /// The file the export wrote.
    pub path: String,
    /// True when the sheet of an xlsx file was full and rows were left out.
    pub sheet_full: bool,
    /// The number of text cells of an xlsx file that were cut at the bound
    /// of a cell.
    pub cut_cells: u64,
    /// A warning for the user about the content of the file.
    pub warning: Option<String>,
}

/// The warning for the text cells that an xlsx file cut.
fn cut_cells_warning(cut_cells: u64) -> Option<String> {
    match cut_cells {
        0 => None,
        1 => Some(
            "1 cell had more than 32,767 characters, the Excel limit for one cell, so its text was cut.".to_string(),
        ),
        count => Some(format!(
            "{count} cells had more than 32,767 characters, the Excel limit for one cell, so their text was cut."
        )),
    }
}

/// Asks the user for the path of a new file. Returns `None` when the user
/// closed the dialog without a choice. The backend opens the dialog itself,
/// so a command never writes to a path that the user did not accept.
async fn ask_save_path<R: Runtime>(
    app: &AppHandle<R>,
    default_name: &str,
    filter_label: &str,
    extension: &str,
    start_folder: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    use tauri_plugin_dialog::DialogExt;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let mut dialog = app
        .dialog()
        .file()
        .set_file_name(default_name)
        .add_filter(filter_label, &[extension]);
    // The dialog opens where the work of the user is, when the interface
    // knows such a folder.
    if let Some(folder) = start_folder {
        dialog = dialog.set_directory(folder);
    }
    dialog.save_file(move |path| {
        let _ = sender.send(path);
    });
    receiver
        .await
        .ok()
        .flatten()
        .and_then(|path| path.into_path().ok())
}

// --- The files of the user ---

/// The paths of a list of folders or files, as text for the interface and
/// for the record of the backend.
fn path_names(roots: &[std::path::PathBuf]) -> Vec<String> {
    roots
        .iter()
        .map(|root| root.to_string_lossy().to_string())
        .collect()
}

/// Records a folder that the user accepted in a dialog of the operating
/// system.
///
/// The folder goes into the state, which guards every later path, and into
/// the record of the backend, so the next session reaches the same folders.
/// A record that cannot be written costs the next session the folder alone,
/// so the dialog goes on.
async fn accept_folder<R: Runtime>(app: &AppHandle<R>, state: &AppState, root: std::path::PathBuf) {
    let _record = state.files_record.lock().await;
    state.add_file_root(root).await;
    let names = path_names(&state.file_roots().await);
    if let Err(error) = with_app(app, move |app| store::write_file_roots(app, &names)).await {
        log::warn!("The folders of the panel could not be written: {error}");
    }
}

/// Records a file that the user accepted in a dialog of the operating system.
///
/// The resolved path goes into the grants of the state and into the record
/// of the backend, so a later read or write of the same tab reaches that one
/// file after a restart as well. A file that is gone from the disk gives no
/// grant. A record that cannot be written costs the next session the grant
/// alone, so the dialog goes on.
async fn accept_file<R: Runtime>(app: &AppHandle<R>, state: &AppState, path: &std::path::Path) {
    let path = path.to_path_buf();
    let Ok(Some(file)) = off_thread(move || Ok(files::grant_for(&path))).await else {
        return;
    };
    let _record = state.files_record.lock().await;
    state.add_file_grant(file).await;
    let names = path_names(&state.file_grants().await);
    if let Err(error) = with_app(app, move |app| store::write_file_grants(app, &names)).await {
        log::warn!("The files that the user opened could not be written: {error}");
    }
}

/// Asks the user for a folder and records it as a root.
///
/// Every other file command refuses a path outside the roots, so this
/// command is the only way a folder becomes reachable. Returns the path, or
/// `None` when the user closed the dialog.
#[tauri::command]
pub async fn pick_folder<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>> {
    use tauri_plugin_dialog::DialogExt;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |path| {
        let _ = sender.send(path);
    });
    let Some(path) = receiver
        .await
        .ok()
        .flatten()
        .and_then(|path| path.into_path().ok())
    else {
        return Ok(None);
    };
    accept_folder(&app, &state, path.clone()).await;
    let opened = path.to_string_lossy().to_string();
    log::info!("Opened the folder '{opened}'.");
    Ok(Some(opened))
}

/// What the interface says about one command of the menu.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuCommandState {
    pub id: String,
    pub enabled: bool,
}

/// Turns the items of the menu on and off.
///
/// The state of a command lives in the interface, because the interface
/// holds the tabs and the connections that decide it. The interface sends
/// the state of every item it knows whenever one of them changes, and the
/// backend then sets the items of the menu that the operating system draws.
///
/// A state that names no item of the menu is dropped, so the interface can
/// send the whole list without a check of its own.
///
/// The whole change runs in one task on the main thread. A call to the menu
/// from an async thread waits for the main thread once for each call, and
/// the walk of the menu makes several calls for each item.
#[tauri::command]
pub async fn set_menu_commands<R: Runtime>(
    app: AppHandle<R>,
    states: Vec<MenuCommandState>,
) -> Result<()> {
    let states: Vec<(String, bool)> = states
        .into_iter()
        .filter(|state| crate::menu::names_a_command(&state.id))
        .map(|state| (state.id, state.enabled))
        .collect();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let _ = sender.send(crate::menu::set_commands_enabled(&handle, &states));
    })?;
    receiver
        .await
        .map_err(|_| Error::Anyhow(anyhow::anyhow!("The menu didn't respond.")))??;
    Ok(())
}

/// One file that the user opened through the dialog.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedFile {
    pub path: String,
    pub contents: String,
    /// The encoding of the file, which a later save of the tab keeps.
    pub encoding: files::TextEncoding,
}

/// The text of one file and the encoding of the file.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextFile {
    pub contents: String,
    pub encoding: files::TextEncoding,
}

/// Asks the user for one statement file and reads it.
///
/// The file becomes a grant, so a later save of the same tab reaches it. The
/// folder of the file stays out of reach, because the user accepted one file
/// and not the files beside it. Returns `None` when the user closed the
/// dialog.
#[tauri::command]
pub async fn open_statement_file<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
) -> Result<Option<OpenedFile>> {
    use tauri_plugin_dialog::DialogExt;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("Statement", &["sql", "txt"])
        .pick_file(move |path| {
            let _ = sender.send(path);
        });
    let Some(path) = receiver
        .await
        .ok()
        .flatten()
        .and_then(|path| path.into_path().ok())
    else {
        return Ok(None);
    };

    let read = path.clone();
    let (contents, encoding) = off_thread(move || files::read_text_file(&read)).await?;
    accept_file(&app, &state, &path).await;
    let opened = path.to_string_lossy().to_string();
    log::info!("Opened the file '{opened}'.");
    Ok(Some(OpenedFile {
        path: opened,
        contents,
        encoding,
    }))
}

/// The folders that the user accepted, for the panel of files.
///
/// The record of the backend holds this list. The interface reads it and
/// never writes it, so the interface cannot widen what the guard accepts.
/// A folder that is gone from the disk drops out of the list and out of the
/// record. The call also puts the single-file grants of the record into the
/// state, and a file that is gone drops out the same way.
#[tauri::command]
pub async fn file_roots<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<String>> {
    file_roots_for(&app, &state).await
}

async fn file_roots_for<R: Runtime>(app: &AppHandle<R>, state: &AppState) -> Result<Vec<String>> {
    let _record = state.files_record.lock().await;
    let (recorded, kept) = with_app(app, |app| {
        let recorded = store::read_file_roots(app)?;
        let kept: Vec<std::path::PathBuf> = recorded
            .iter()
            .filter_map(|path| files::root_from_record(path))
            .collect();
        Ok((recorded.len(), kept))
    })
    .await?;
    state.set_file_roots(kept.clone()).await;
    let names = path_names(&kept);
    if names.len() != recorded {
        let written = names.clone();
        with_app(app, move |app| store::write_file_roots(app, &written)).await?;
    }

    let (recorded, grants) = with_app(app, |app| {
        let recorded = store::read_file_grants(app)?;
        let grants: Vec<std::path::PathBuf> = recorded
            .iter()
            .filter_map(|path| files::grant_for(std::path::Path::new(path)))
            .collect();
        Ok((recorded, grants))
    })
    .await?;
    state.set_file_grants(grants.clone()).await;
    let granted = path_names(&grants);
    if granted != recorded {
        with_app(app, move |app| store::write_file_grants(app, &granted)).await?;
    }
    Ok(names)
}

/// Takes one folder out of the panel and out of the record. A path that
/// stands under the folder is outside every root after the call, so the
/// close of a folder ends the reach of the interface into it.
#[tauri::command]
pub async fn close_folder<R: Runtime>(
    app: AppHandle<R>,
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<()> {
    close_folder_for(&app, &path, &state).await
}

async fn close_folder_for<R: Runtime>(
    app: &AppHandle<R>,
    path: &str,
    state: &AppState,
) -> Result<()> {
    let _record = state.files_record.lock().await;
    state.remove_file_root(std::path::Path::new(path)).await;
    let names = path_names(&state.file_roots().await);
    with_app(app, move |app| store::write_file_roots(app, &names)).await?;
    log::info!("Closed the folder '{path}'.");
    Ok(())
}

/// Lists the entries of one folder inside the roots.
#[tauri::command]
pub async fn list_folder(
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<files::FolderEntry>> {
    let roots = state.file_roots().await;
    off_thread(move || {
        let target = files::path_inside_roots(std::path::Path::new(&path), &roots)?;
        files::read_folder(&target)
    })
    .await
}

/// Resolves a path that a read or a write of a file names, and refuses it
/// when it is neither a grant nor inside a root.
///
/// The check resolves the path and each root on every call, so a root that
/// the user renamed, or a root under a link that now points elsewhere, is
/// judged by the disk as it is at the time of the call. The resolution runs
/// on a blocking thread, because a slow or a network disk would otherwise
/// stop an async thread.
async fn accepted_path(path: &str, state: &AppState) -> Result<std::path::PathBuf> {
    let roots = state.file_roots().await;
    let grants = state.file_grants().await;
    let path = path.to_owned();
    off_thread(move || files::path_accepted(std::path::Path::new(&path), &roots, &grants)).await
}

/// Reads the text of one file that is a grant or inside the roots, and the
/// encoding of the file.
#[tauri::command]
pub async fn read_text_file(path: String, state: tauri::State<'_, AppState>) -> Result<TextFile> {
    let target = accepted_path(&path, &state).await?;
    let (contents, encoding) = off_thread(move || files::read_text_file(&target)).await?;
    Ok(TextFile { contents, encoding })
}

/// What a request to save the statement of a tab carries.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveStatementRequest {
    /// The file of the tab, when the tab has one.
    #[serde(default)]
    pub path: Option<String>,
    /// The file name that the save dialog suggests.
    pub default_name: String,
    /// The folder the dialog opens in, when the interface knows one.
    pub default_folder: Option<String>,
    pub contents: String,
    /// The encoding to write. A request without one writes UTF-8.
    #[serde(default)]
    pub encoding: Option<files::TextEncoding>,
}

/// The file that a save of a statement wrote.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedStatement {
    pub path: String,
    /// The encoding of the file. Text that the requested encoding cannot
    /// store is written as UTF-8 with a byte order mark.
    pub encoding: files::TextEncoding,
}

/// Writes the statement of a tab to its file, or asks the user for a path.
///
/// A tab whose file is a grant or inside a root writes that file at once. A
/// tab without a file, or with a file outside every grant and root, opens
/// the save dialog. A file is outside every grant and root when the user
/// opened it from a folder in the files panel and then closed the folder. The dialog then
/// starts at the name and the folder of that file, so the user confirms the
/// same file. The file that the dialog gives becomes a grant, so the next
/// save writes it at once. Returns the path and the encoding of the file, or
/// `None` when the user closed the dialog.
#[tauri::command]
pub async fn save_statement_file<R: Runtime>(
    app: AppHandle<R>,
    request: SaveStatementRequest,
    state: tauri::State<'_, AppState>,
) -> Result<Option<SavedStatement>> {
    if let Some(known) = request.path.as_deref() {
        if let Some(saved) =
            write_accepted(known, &request.contents, request.encoding, &state).await?
        {
            log::info!("Wrote the file '{}'.", saved.path);
            return Ok(Some(saved));
        }
    }
    let (name, folder) = dialog_start(&request);
    let Some(path) = ask_save_path(&app, &name, "SQL", "sql", folder.as_deref()).await else {
        return Ok(None);
    };
    let SaveStatementRequest {
        contents, encoding, ..
    } = request;
    let target = path.clone();
    let saved = off_thread(move || write_statement(&target, &contents, encoding)).await?;
    accept_file(&app, &state, &path).await;
    log::info!("Wrote the file '{}'.", saved.path);
    Ok(Some(saved))
}

/// Writes the statement to a file that is a grant or inside a root. Gives
/// `None` when the path is out of reach, so the caller asks the user for a
/// path. The result names the path as the tab gave it, because the tab
/// finds its file by that text.
async fn write_accepted(
    path: &str,
    contents: &str,
    encoding: Option<files::TextEncoding>,
    state: &AppState,
) -> Result<Option<SavedStatement>> {
    let Ok(target) = accepted_path(path, state).await else {
        return Ok(None);
    };
    let contents = contents.to_owned();
    let mut saved = off_thread(move || write_statement(&target, &contents, encoding)).await?;
    saved.path = path.to_owned();
    Ok(Some(saved))
}

/// The file name that the save dialog suggests and the folder it opens in.
/// A tab with a file suggests the name and the folder of that file.
fn dialog_start(request: &SaveStatementRequest) -> (String, Option<std::path::PathBuf>) {
    let known = request.path.as_deref().map(std::path::Path::new);
    let name = known
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| request.default_name.clone());
    let folder = known
        .and_then(|path| path.parent())
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(std::path::Path::to_path_buf)
        .or_else(|| {
            request
                .default_folder
                .as_ref()
                .map(std::path::PathBuf::from)
        });
    (name, folder)
}

/// Writes the statement of a tab to a path that the user chose.
fn write_statement(
    path: &std::path::Path,
    contents: &str,
    encoding: Option<files::TextEncoding>,
) -> Result<SavedStatement> {
    let encoding = encoding.unwrap_or(files::TextEncoding::Utf8);
    let used = files::write_text_as(path, contents, encoding)?;
    Ok(SavedStatement {
        path: path.to_string_lossy().to_string(),
        encoding: used,
    })
}

/// What a request to save one file carries. The content is text, or base64
/// text when the file is binary.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveFileRequest {
    /// The file name that the save dialog suggests.
    pub default_name: String,
    /// The label of the file type in the dialog.
    pub filter_label: String,
    /// The extension of the file type, without the period.
    pub extension: String,
    pub contents: String,
}

/// Asks the user for a path and writes text there. Returns the path, or
/// `None` when the user closed the dialog.
#[tauri::command]
pub async fn save_text_file<R: Runtime>(
    app: AppHandle<R>,
    request: SaveFileRequest,
) -> Result<Option<String>> {
    let SaveFileRequest {
        default_name,
        filter_label,
        extension,
        contents,
    } = request;
    let Some(path) = ask_save_path(&app, &default_name, &filter_label, &extension, None).await
    else {
        return Ok(None);
    };
    let target = path.clone();
    off_thread(move || files::write_bytes(&target, contents.as_bytes())).await?;
    let written = path.to_string_lossy().to_string();
    log::info!("Wrote the file '{written}'.");
    Ok(Some(written))
}

/// Asks the user for a path and writes bytes there. The bytes arrive as
/// base64 text, because the bridge carries no binary body beside the other
/// fields. Returns the path, or `None` when the user closed the dialog.
#[tauri::command]
pub async fn save_binary_file<R: Runtime>(
    app: AppHandle<R>,
    request: SaveFileRequest,
) -> Result<Option<String>> {
    let SaveFileRequest {
        default_name,
        filter_label,
        extension,
        contents,
    } = request;
    // The text of a large file is long, so the decode runs on a blocking
    // thread as well.
    let bytes = off_thread(move || decode_base64(&contents)).await?;
    let Some(path) = ask_save_path(&app, &default_name, &filter_label, &extension, None).await
    else {
        return Ok(None);
    };
    let target = path.clone();
    off_thread(move || files::write_bytes(&target, &bytes)).await?;
    let written = path.to_string_lossy().to_string();
    log::info!("Wrote the file '{written}'.");
    Ok(Some(written))
}

/// Reads the bytes out of base64 text.
fn decode_base64(text: &str) -> Result<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(text.as_bytes())
        .map_err(|error| Error::Invalid(format!("The file content is corrupt: {error}")))
}

/// Runs a statement again with a higher row limit and writes the rows
/// straight to a file as they arrive. A large result therefore never
/// passes through the user interface, and the backend keeps at most the
/// rows in the queue of the file writer.
///
/// The statement must only read, because an export runs it a second time.
#[tauri::command]
pub async fn export_query<R: Runtime>(
    app: AppHandle<R>,
    request: ExportRequest,
    state: tauri::State<'_, AppState>,
) -> Result<Option<ExportSummary>> {
    let ExportRequest {
        connection_id,
        request_id,
        query,
        default_name,
        format,
        max_rows,
        tab_id,
        query_params,
    } = request;

    let (label, extension) = format.file_type();
    // The check comes before the dialog, so the user does not pick a file
    // for an export that cannot run.
    let dialect = state.connection(&connection_id).await?.dialect;
    refuse_export_of_writes(&query, dialect)?;
    let Some(path) = ask_save_path(&app, &default_name, label, extension, None).await else {
        return Ok(None);
    };

    let token = state.start_request(&request_id, &connection_id).await;
    let prepared = async {
        let (open, session, key) =
            session_for(&app, &state, &connection_id, tab_id.as_deref(), &token).await?;
        let options = ExecOptions {
            max_rows,
            timeout_secs: open.descriptor.exec_options().timeout_secs,
            one_statement: true,
        };
        let (query, bound) = prepare_parameters(&query, open.dialect, query_params.as_ref())?;
        // The sink writes to a temporary path. An error, a stop or a time
        // limit leaves the run before `finish`. The drop of the sink then
        // closes the queue of the writer thread, and the writer removes the
        // part that was written.
        let sink = FileSink::create(&path, format).await?;
        Ok::<_, Error>((open, session, key, options, query, bound, sink))
    }
    .await;
    let (open, session, key, options, ran, bound, mut sink) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            state.end_request(&request_id).await;
            return Err(error);
        }
    };
    let outcome = match driver_for_request(&state, &request_id, &session, &token).await {
        Ok(mut guard) => {
            run_bounded(
                guard.execute_stream(&ran, bound.as_ref(), &options, &mut sink),
                &token,
                options.timeout_secs,
                stop_grace(&session),
                session.cancel_handle.clone(),
            )
            .await
        }
        Err(error) => Bounded::Answered(Err(error)),
    };
    let outcome = in_sent_text(outcome, &query, &ran);
    state.end_request(&request_id).await;
    finish_run(&state, &connection_id, &open, &key, &session, outcome).await?;
    finish_export(sink).await.map(Some)
}

/// Closes the file of an export whose read ended well, and gives what the
/// export wrote. A read that gave no result set writes no file.
async fn finish_export(sink: FileSink) -> Result<ExportSummary> {
    if !sink.saw_set {
        return Err(Error::Unsupported(
            "The statement returned no result set.".to_string(),
        ));
    }
    let summary = sink.finish().await?;
    log::info!(
        "Wrote {} rows to the file '{}'.",
        summary.rows,
        summary.path
    );
    Ok(summary)
}

/// What an export of a kept result needs to know.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeptExportRequest {
    /// The identifier of the kept set, from the frame at the end of its run.
    pub kept_id: String,
    /// The identifier that the Stop button of the export names.
    pub request_id: String,
    /// The file name that the save dialog suggests.
    pub default_name: String,
    pub format: ExportFormat,
    /// The row limit of the export, which is higher than the one of the view.
    pub max_rows: usize,
}

/// Writes every row of a kept result set to a file, and does not run the
/// statement again.
///
/// The read uses the kept result alone and takes no driver of a session, so
/// the tab can run other statements while the export goes on. The Stop
/// button and the time limit of the connection end the read, as they end a
/// run.
#[tauri::command]
pub async fn export_kept<R: Runtime>(
    app: AppHandle<R>,
    request: KeptExportRequest,
    state: tauri::State<'_, AppState>,
) -> Result<Option<ExportSummary>> {
    // The checks come before the dialog, so the user does not pick a file
    // for an export that cannot run.
    let kept = kept_result(&state, &request.kept_id)?;
    let open = state.connection(&kept.connection_id).await?;
    let (label, extension) = request.format.file_type();
    let Some(path) = ask_save_path(&app, &request.default_name, label, extension, None).await
    else {
        return Ok(None);
    };
    let options = ExecOptions {
        max_rows: request.max_rows,
        timeout_secs: open.descriptor.exec_options().timeout_secs,
        one_statement: true,
    };
    write_kept(
        &state,
        &request.request_id,
        &kept,
        &path,
        request.format,
        &options,
    )
    .await
    .map(Some)
}

/// The kept result with the identifier, or the error that tells the user to
/// run the statement again.
fn kept_result(state: &AppState, kept_id: &str) -> Result<Arc<crate::kept::KeptResult>> {
    state.kept.get(kept_id).ok_or_else(|| {
        Error::Invalid(
            "The saved result of this query is gone. Run the query again to export all rows."
                .to_string(),
        )
    })
}

/// Reads a kept result into a file at the path, under the Stop button of the
/// request and the time limit of the options.
async fn write_kept(
    state: &AppState,
    request_id: &str,
    kept: &crate::kept::KeptResult,
    path: &std::path::Path,
    format: ExportFormat,
    options: &ExecOptions,
) -> Result<ExportSummary> {
    let token = state.start_request(request_id, &kept.connection_id).await;
    let written = async {
        // An error, a stop or the time limit drops the sink before
        // `finish`, and the writer then removes the part that was written.
        let mut sink = FileSink::create(path, format).await?;
        // No server statement runs, so the stop needs no grace and no
        // handle. The drop of the read ends its requests.
        let read = kept.source.read(options, &mut sink);
        match run_bounded(
            read,
            &token,
            options.timeout_secs,
            std::time::Duration::ZERO,
            None,
        )
        .await
        {
            Bounded::Answered(result) => result?,
            Bounded::Stopped(error) => return Err(error),
        }
        Ok(sink)
    }
    .await;
    state.end_request(request_id).await;
    finish_export(written?).await
}

/// Removes one kept result from the registry. The window calls this when the
/// result leaves the interface: a new run of the tab, a close of the result
/// or a close of the tab. An identifier that the registry does not contain
/// is not an error, because the bounds of the registry can remove an entry
/// first.
#[tauri::command]
pub async fn release_kept(kept_id: String, state: tauri::State<'_, AppState>) -> Result<()> {
    state.kept.release(&kept_id);
    Ok(())
}

/// Refuses an export of a statement that changes data. The export runs the
/// statement again, so a change would happen twice.
fn refuse_export_of_writes(query: &str, dialect: crate::sql::Dialect) -> Result<()> {
    if crate::sql::only_reads(query, dialect) {
        return Ok(());
    }
    Err(Error::Unsupported(
        "Exporting to a file runs the statement again, so only read-only statements can be exported."
            .to_string(),
    ))
}

/// The number of pieces that wait for the writer of an export. A driver that
/// reads faster than the disk writes waits when the queue is full, so the
/// memory of an export stays bounded.
const EXPORT_QUEUE: usize = 1024;

/// One piece of an export on its way to the writer thread.
enum Piece {
    Begin(Vec<crate::db::ColumnInfo>),
    Row(Vec<serde_json::Value>),
    /// The run ended well, so the writer closes the file and renames it.
    Finish,
}

/// The error of the writer thread, kept for the sink. The writer stops at
/// its first error and drops the end of the queue, so the next send of the
/// sink fails and gives this error to the run.
type WriterFault = Arc<std::sync::Mutex<Option<Error>>>;

/// A sink that sends the rows of the first result set to a writer thread.
///
/// The driver calls the sink on an async thread of the runtime. The writes
/// to the file, the deflate of an xlsx file and the flushes to the disk run
/// on a thread of their own, so a slow disk does not stop the async thread
/// that serves the other commands. The sink keeps the counts and the limits,
/// so it answers `Stop` without a wait for the writer. It answers `Stop` for
/// a row of a second set, because the export writes one file.
struct FileSink {
    pieces: std::sync::mpsc::SyncSender<Piece>,
    fault: WriterFault,
    /// The result of `ExportWriter::finish`, which the writer thread sends
    /// after a `Finish` piece.
    done: Option<tokio::sync::oneshot::Receiver<Result<u64>>>,
    final_path: std::path::PathBuf,
    /// The number of rows that still fit in the sheet of an xlsx file. Other
    /// formats have no such bound.
    sheet_room: Option<usize>,
    rows: usize,
    truncated: bool,
    /// True when the sheet of an xlsx file was full and rows were left out.
    sheet_full: bool,
    /// True once the first set began.
    saw_set: bool,
    /// True once the first set ended.
    set_done: bool,
}

impl FileSink {
    /// Creates the temporary file on a blocking thread and starts the writer
    /// thread.
    async fn create(path: &std::path::Path, format: ExportFormat) -> Result<Self> {
        let target = path.to_path_buf();
        let writer = off_thread(move || ExportWriter::create(&target, format)).await?;
        let (pieces, queue) = std::sync::mpsc::sync_channel(EXPORT_QUEUE);
        let (sender, done) = tokio::sync::oneshot::channel();
        let fault = WriterFault::default();
        let kept = Arc::clone(&fault);
        std::thread::Builder::new()
            .name("export-writer".to_string())
            .spawn(move || write_pieces(writer, queue, kept, sender))?;
        let sheet_room =
            matches!(format, ExportFormat::Xlsx).then_some(crate::xlsx::MAX_SHEET_ROWS - 1);
        Ok(Self::new(pieces, fault, done, path, sheet_room))
    }

    fn new(
        pieces: std::sync::mpsc::SyncSender<Piece>,
        fault: WriterFault,
        done: tokio::sync::oneshot::Receiver<Result<u64>>,
        path: &std::path::Path,
        sheet_room: Option<usize>,
    ) -> Self {
        Self {
            pieces,
            fault,
            done: Some(done),
            final_path: path.to_path_buf(),
            sheet_room,
            rows: 0,
            truncated: false,
            sheet_full: false,
            saw_set: false,
            set_done: false,
        }
    }

    /// The error that stopped the writer thread. A thread that stopped
    /// without an error, such as one that panicked, gives a general error.
    fn writer_fault(&self) -> Error {
        self.fault
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .unwrap_or_else(|| {
                Error::Anyhow(anyhow::anyhow!(
                    "The export stopped because the file writer quit unexpectedly."
                ))
            })
    }

    /// Puts one piece in the queue of the writer. A full queue makes the
    /// driver wait for the writer, which slows the read to the speed of the
    /// disk.
    fn send(&self, piece: Piece) -> Result<()> {
        use std::sync::mpsc::TrySendError;
        let piece = match self.pieces.try_send(piece) {
            Ok(()) => return Ok(()),
            Err(TrySendError::Full(piece)) => piece,
            Err(TrySendError::Disconnected(_)) => return Err(self.writer_fault()),
        };
        wait_in_place(|| self.pieces.send(piece)).map_err(|_| self.writer_fault())
    }

    /// Tells the writer to close the file and rename it onto the path the
    /// user chose, and waits for the writer to finish.
    async fn finish(mut self) -> Result<ExportSummary> {
        self.send(Piece::Finish)?;
        let done = self.done.take().expect("the sink finishes once");
        let cut_cells = match done.await {
            Ok(result) => result?,
            Err(_) => return Err(self.writer_fault()),
        };
        let warning = cut_cells_warning(cut_cells);
        if let Some(text) = &warning {
            log::warn!("{text}");
        }
        Ok(ExportSummary {
            rows: self.rows,
            truncated: self.truncated,
            path: self.final_path.to_string_lossy().to_string(),
            sheet_full: self.sheet_full,
            cut_cells,
            warning,
        })
    }
}

/// Runs blocking work on the current thread. On a worker of a runtime with
/// many threads, the runtime first moves the other tasks of this worker to
/// another thread, so they do not wait for the work. A runtime with one
/// thread cannot move its tasks, and the work then runs as it is.
pub(crate) fn wait_in_place<T>(work: impl FnOnce() -> T) -> T {
    use tokio::runtime::{Handle, RuntimeFlavor};
    match Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(work)
        }
        _ => work(),
    }
}

/// The loop of the writer thread. It writes each piece in the order of the
/// queue. A queue that closes before a `Finish` piece means that the run
/// failed or stopped, and the drop of the writer then removes the part that
/// was written.
fn write_pieces(
    mut writer: ExportWriter,
    queue: std::sync::mpsc::Receiver<Piece>,
    fault: WriterFault,
    done: tokio::sync::oneshot::Sender<Result<u64>>,
) {
    for piece in queue {
        let step = match piece {
            Piece::Begin(columns) => writer.begin(columns),
            Piece::Row(row) => writer.row(&row),
            Piece::Finish => {
                let _ = done.send(writer.finish());
                return;
            }
        };
        if let Err(error) = step {
            *fault
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(error);
            return;
        }
    }
}

/// The writer of an export file, which runs on the writer thread. It writes
/// to a temporary path beside the file and renames it at a successful end,
/// so a run that fails or stops leaves no file.
struct ExportWriter {
    format: ExportFormat,
    final_path: std::path::PathBuf,
    temp_path: std::path::PathBuf,
    out: Option<std::io::BufWriter<std::fs::File>>,
    /// The writer of the sheet. In the xlsx form it owns the file from the
    /// start of the set until `finish`.
    sheet: Option<crate::xlsx::SheetWriter<std::io::BufWriter<std::fs::File>>>,
    /// The name the one sheet of an xlsx file carries.
    sheet_title: String,
    /// The unique column names of the set, for the JSON objects.
    names: Vec<String>,
    rows: usize,
    /// True once the set began.
    began: bool,
    /// True once the file reached its final path.
    finished: bool,
}

impl ExportWriter {
    fn create(path: &std::path::Path, format: ExportFormat) -> Result<Self> {
        // The writer removes the temporary file itself when the export does
        // not finish, so the file leaves the cleanup of `tempfile`.
        let (file, temp_path) = files::temp_file_beside(path)?
            .keep()
            .map_err(|error| error.error)?;
        // The sheet takes the name of the file that the user chose.
        let sheet_title = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_default();
        Ok(Self {
            format,
            final_path: path.to_path_buf(),
            temp_path,
            out: Some(std::io::BufWriter::new(file)),
            sheet: None,
            sheet_title,
            names: Vec::new(),
            rows: 0,
            began: false,
            finished: false,
        })
    }

    fn writer(&mut self) -> Result<&mut std::io::BufWriter<std::fs::File>> {
        self.out
            .as_mut()
            .ok_or_else(|| Error::Anyhow(anyhow::anyhow!("The export file is closed.")))
    }

    fn begin(&mut self, columns: Vec<crate::db::ColumnInfo>) -> Result<()> {
        use std::io::Write;
        self.began = true;
        match self.format {
            ExportFormat::Csv => {
                let names: Vec<serde_json::Value> = columns
                    .into_iter()
                    .map(|column| serde_json::Value::String(column.name))
                    .collect();
                // The mark of the byte order stands at the head of the file,
                // because Excel reads a file without it in the code page of
                // the system and damages every value outside ASCII.
                let out = self.writer()?;
                out.write_all(CSV_BOM.as_bytes())?;
                write_csv_line(out, &names)?;
            }
            ExportFormat::Json => {
                self.names = crate::db::unique_column_names(&columns);
                writeln!(self.writer()?, "[")?;
            }
            ExportFormat::Xlsx => {
                let names = crate::db::unique_column_names(&columns);
                let numeric = columns
                    .iter()
                    .map(|column| crate::xlsx::is_numeric_type(&column.type_name))
                    .collect();
                let file = self
                    .out
                    .take()
                    .ok_or_else(|| Error::Anyhow(anyhow::anyhow!("The export file is closed.")))?;
                self.sheet = Some(crate::xlsx::SheetWriter::create_typed(
                    file,
                    &self.sheet_title,
                    &names,
                    numeric,
                )?);
            }
        }
        Ok(())
    }

    /// Writes one row. The sink keeps the bound of a sheet, so a row of an
    /// xlsx file always fits.
    fn row(&mut self, row: &[serde_json::Value]) -> Result<()> {
        use std::io::Write;
        match self.format {
            ExportFormat::Csv => write_csv_line(self.writer()?, row)?,
            ExportFormat::Json => {
                let rows = self.rows;
                let out = self
                    .out
                    .as_mut()
                    .ok_or_else(|| Error::Anyhow(anyhow::anyhow!("The export file is closed.")))?;
                if rows > 0 {
                    writeln!(out, ",")?;
                }
                write!(out, "  ")?;
                write_json_object(out, &self.names, row)?;
            }
            ExportFormat::Xlsx => {
                let sheet = self
                    .sheet
                    .as_mut()
                    .ok_or_else(|| Error::Anyhow(anyhow::anyhow!("The sheet isn't open.")))?;
                sheet.row(row)?;
            }
        }
        self.rows += 1;
        Ok(())
    }

    /// Closes the file and renames it onto the path the user chose. Gives
    /// the number of text cells of an xlsx file that were cut.
    fn finish(mut self) -> Result<u64> {
        use std::io::Write;
        let mut cut_cells = 0;
        if self.began {
            match self.format {
                ExportFormat::Json => {
                    let rows = self.rows;
                    let out = self.writer()?;
                    if rows == 0 {
                        writeln!(out, "]")?;
                    } else {
                        writeln!(out, "\n]")?;
                    }
                }
                // The sheet owns the file while it is open, so the close of
                // the container gives the file back.
                ExportFormat::Xlsx => {
                    if let Some(sheet) = self.sheet.take() {
                        cut_cells = sheet.cut_cells();
                        self.out = Some(sheet.finish()?);
                    }
                }
                ExportFormat::Csv => {}
            }
        }
        let out = self.out.take().expect("the file is open until here");
        // The rows go to the disk before the rename, so a power loss cannot
        // leave an empty or a partial file at the path the user chose.
        let file = out.into_inner().map_err(|error| error.into_error())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&self.temp_path, &self.final_path)?;
        files::sync_folder_of(&self.final_path);
        self.finished = true;
        Ok(cut_cells)
    }
}

impl Drop for ExportWriter {
    fn drop(&mut self) {
        if !self.finished {
            self.sheet.take();
            self.out.take();
            let _ = std::fs::remove_file(&self.temp_path);
        }
    }
}

impl crate::db::sink::RowSink for FileSink {
    fn begin_set(&mut self, columns: Vec<crate::db::ColumnInfo>) -> Result<()> {
        if self.saw_set {
            return Ok(());
        }
        self.saw_set = true;
        self.send(Piece::Begin(columns))
    }

    fn row(&mut self, row: Vec<serde_json::Value>) -> Result<crate::db::sink::SinkControl> {
        if self.set_done {
            return Ok(crate::db::sink::SinkControl::Stop);
        }
        // A sheet has a limit of MAX_SHEET_ROWS rows. The rows past the
        // limit stay out of the file, and the summary reports the result as
        // truncated.
        if let Some(room) = self.sheet_room.as_mut() {
            if *room == 0 {
                self.truncated = true;
                self.sheet_full = true;
                return Ok(crate::db::sink::SinkControl::Stop);
            }
            *room -= 1;
        }
        self.send(Piece::Row(row))?;
        self.rows += 1;
        Ok(crate::db::sink::SinkControl::Continue)
    }

    fn end_set(&mut self, truncated: bool) -> Result<()> {
        if self.set_done {
            return Ok(());
        }
        self.truncated = self.truncated || truncated;
        self.set_done = true;
        Ok(())
    }

    fn message(&mut self, _message: crate::db::Message) {}
}

/// The mark of the byte order that a comma separated file carries, so that
/// Excel reads the file in UTF-8.
const CSV_BOM: &str = "\u{feff}";

/// The line end of a comma separated file, which Excel expects.
const CSV_LINE_END: &str = "\r\n";

/// Writes one line of a comma separated file straight to the writer, so a
/// row of the export makes no joined copy of its fields.
fn write_csv_line(out: &mut impl std::io::Write, values: &[serde_json::Value]) -> Result<()> {
    for (position, value) in values.iter().enumerate() {
        if position > 0 {
            out.write_all(b",")?;
        }
        out.write_all(csv_field(value).as_bytes())?;
    }
    out.write_all(CSV_LINE_END.as_bytes())?;
    Ok(())
}

/// Writes one row as a JSON object straight to the writer. The keys keep
/// the order of the columns. A `serde_json::Map` sorts its keys, so an
/// object built through it puts the columns in the order of the alphabet.
/// A column with no value in the row gets `null`.
fn write_json_object(
    out: &mut impl std::io::Write,
    names: &[String],
    row: &[serde_json::Value],
) -> Result<()> {
    out.write_all(b"{")?;
    for (position, name) in names.iter().enumerate() {
        if position > 0 {
            out.write_all(b",")?;
        }
        serde_json::to_writer(&mut *out, name)?;
        out.write_all(b":")?;
        let value = row.get(position).unwrap_or(&serde_json::Value::Null);
        serde_json::to_writer(&mut *out, value)?;
    }
    out.write_all(b"}")?;
    Ok(())
}

/// Writes one field of a comma separated file.
///
/// A text value that starts with a formula mark gets an apostrophe in front,
/// because a spreadsheet would otherwise run the value as a formula. The
/// apostrophe changes the exported text, and the safety of the reader weighs
/// more than the exact form of such a value.
fn csv_field(value: &serde_json::Value) -> String {
    let text = match value {
        serde_json::Value::Null => return String::new(),
        serde_json::Value::String(text) if starts_a_formula(text) => format!("'{text}"),
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    if text.contains([',', '"', '\n', '\r']) || text.trim() != text {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text
    }
}

/// True when a spreadsheet would read the text as a formula. A number that
/// arrives as text, such as a DECIMAL value or a PostgreSQL value of the
/// simple protocol, keeps its sign, because a spreadsheet reads `-5` as a
/// number, and an apostrophe would stay in the value that a loader reads.
fn starts_a_formula(text: &str) -> bool {
    matches!(
        text.chars().next(),
        Some('=') | Some('+') | Some('-') | Some('@') | Some('\t') | Some('\r')
    ) && !crate::xlsx::is_plain_number(text)
}

/// Reports the engines this build supports, so the connection form can
/// show them.
#[tauri::command]
pub fn supported_engines() -> Vec<db::EngineInfo> {
    db::supported_engines()
}

/// Reports whether a saved password stays after the application closes. It
/// does not when the keychain of the system was not reachable at the start,
/// and the connection form then says so.
///
/// The answer can wait for the probe of the keychain that selects the
/// store, so it runs on a blocking thread and not on the main thread.
#[tauri::command]
pub async fn passwords_persist(state: tauri::State<'_, AppState>) -> Result<bool> {
    with_store(&state, |store| Ok(store.persists())).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{ColumnInfo, ResultSet};
    use crate::secrets::MemoryStore;
    use crate::sql::Dialect;
    use crate::storage::ConnectionOptions;

    #[test]
    fn a_schema_scope_reads_a_full_request() {
        let scope: SchemaScope =
            serde_json::from_str(r#"{"connectionId":"c1","database":"db","schemaName":"dbo"}"#)
                .unwrap();
        assert_eq!(scope.connection_id, "c1");
        assert_eq!(scope.database, "db");
        assert_eq!(scope.schema_name.as_deref(), Some("dbo"));
    }

    #[test]
    fn a_schema_scope_takes_a_schema_that_is_null_or_absent() {
        let with_null: SchemaScope =
            serde_json::from_str(r#"{"connectionId":"c1","database":"db","schemaName":null}"#)
                .unwrap();
        assert_eq!(with_null.schema_name, None);

        let absent: SchemaScope =
            serde_json::from_str(r#"{"connectionId":"c1","database":"db"}"#).unwrap();
        assert_eq!(absent.schema_name, None);
    }

    #[test]
    fn a_table_scope_reads_a_full_request() {
        let scope: TableScope = serde_json::from_str(
            r#"{"connectionId":"c1","database":"db","schemaName":"dbo","tableName":"t"}"#,
        )
        .unwrap();
        assert_eq!(scope.connection_id, "c1");
        assert_eq!(scope.database, "db");
        assert_eq!(scope.schema_name.as_deref(), Some("dbo"));
        assert_eq!(scope.table_name, "t");
    }

    #[test]
    fn a_table_scope_takes_a_schema_that_is_null_or_absent() {
        let with_null: TableScope = serde_json::from_str(
            r#"{"connectionId":"c1","database":"db","schemaName":null,"tableName":"t"}"#,
        )
        .unwrap();
        assert_eq!(with_null.schema_name, None);

        let absent: TableScope =
            serde_json::from_str(r#"{"connectionId":"c1","database":"db","tableName":"t"}"#)
                .unwrap();
        assert_eq!(absent.schema_name, None);
        assert_eq!(absent.table_name, "t");
    }

    #[test]
    fn an_execute_request_reads_a_full_request() {
        let request: ExecuteRequest = serde_json::from_str(
            r#"{"connectionId":"c1","requestId":"r1","query":"SELECT :id",
                "queryParams":{"id":7},"options":{"maxRows":10,"timeoutSecs":5}}"#,
        )
        .unwrap();
        assert_eq!(request.connection_id, "c1");
        assert_eq!(request.request_id, "r1");
        assert_eq!(request.query, "SELECT :id");
        assert_eq!(
            request.query_params.unwrap().get("id"),
            Some(&serde_json::json!(7))
        );
        let options = request.options.unwrap();
        assert_eq!(options.max_rows, 10);
        assert_eq!(options.timeout_secs, 5);
    }

    #[test]
    fn an_execute_request_takes_limits_that_are_null_or_absent() {
        let with_null: ExecuteRequest = serde_json::from_str(
            r#"{"connectionId":"c1","requestId":"r1","query":"SELECT 1",
                "queryParams":null,"options":null}"#,
        )
        .unwrap();
        assert!(with_null.query_params.is_none());
        assert!(with_null.options.is_none());

        let absent: ExecuteRequest =
            serde_json::from_str(r#"{"connectionId":"c1","requestId":"r1","query":"SELECT 1"}"#)
                .unwrap();
        assert!(absent.query_params.is_none());
        assert!(absent.options.is_none());
    }

    #[test]
    fn a_snapshot_request_reads_a_full_request() {
        let request: SnapshotRequest = serde_json::from_str(
            r#"{"connectionId":"c1","database":"db","maxColumns":100,"ownConnection":false}"#,
        )
        .unwrap();
        assert_eq!(request.connection_id, "c1");
        assert_eq!(request.database, "db");
        assert_eq!(request.max_columns, Some(100));
        assert_eq!(request.own_connection, Some(false));
    }

    #[test]
    fn a_snapshot_request_takes_bounds_that_are_null_or_absent() {
        let with_null: SnapshotRequest = serde_json::from_str(
            r#"{"connectionId":"c1","database":"db","maxColumns":null,"ownConnection":null}"#,
        )
        .unwrap();
        assert_eq!(with_null.max_columns, None);
        assert_eq!(with_null.own_connection, None);

        let absent: SnapshotRequest =
            serde_json::from_str(r#"{"connectionId":"c1","database":"db"}"#).unwrap();
        assert_eq!(absent.max_columns, None);
        assert_eq!(absent.own_connection, None);
    }

    #[test]
    fn a_preview_request_reads_a_full_request() {
        let request: PreviewRequest = serde_json::from_str(
            r#"{"connectionId":"c1","database":"db","schemaName":"dbo",
                "tableName":"t","limit":50}"#,
        )
        .unwrap();
        assert_eq!(request.connection_id, "c1");
        assert_eq!(request.database.as_deref(), Some("db"));
        assert_eq!(request.schema_name.as_deref(), Some("dbo"));
        assert_eq!(request.table_name, "t");
        assert_eq!(request.limit, Some(50));
    }

    #[test]
    fn a_preview_request_takes_names_that_are_null_or_absent() {
        let with_null: PreviewRequest = serde_json::from_str(
            r#"{"connectionId":"c1","database":null,"schemaName":null,
                "tableName":"t","limit":null}"#,
        )
        .unwrap();
        assert_eq!(with_null.database, None);
        assert_eq!(with_null.schema_name, None);
        assert_eq!(with_null.limit, None);

        let absent: PreviewRequest =
            serde_json::from_str(r#"{"connectionId":"c1","tableName":"t"}"#).unwrap();
        assert_eq!(absent.database, None);
        assert_eq!(absent.schema_name, None);
        assert_eq!(absent.limit, None);
    }

    #[test]
    fn a_statement_without_a_name_keeps_its_text_and_carries_no_parameter() {
        let (text, bound) = prepare_parameters("SELECT 1; SELECT 2", Dialect::MsSql, None).unwrap();
        assert_eq!(text, "SELECT 1; SELECT 2");
        assert!(bound.is_none());
    }

    #[test]
    fn the_values_travel_bound_and_in_the_order_of_the_placeholders() {
        let mut values = ParamValues::new();
        values.insert("second".to_string(), serde_json::json!(2));
        values.insert("first".to_string(), serde_json::json!("a"));

        let (text, bound) =
            prepare_parameters("SELECT :first, :second", Dialect::Postgres, Some(&values)).unwrap();
        assert_eq!(text, "SELECT $1, $2");
        let bound = bound.unwrap();
        assert_eq!(bound.len(), 2);
        assert_eq!(bound[0].value, serde_json::json!("a"));
        assert_eq!(bound[1].value, serde_json::json!(2));
    }

    #[test]
    fn a_parameter_without_a_value_stops_the_run() {
        let error = prepare_parameters("SELECT :id", Dialect::MsSql, None).unwrap_err();
        assert_eq!(error.category(), crate::error::ErrorCategory::Invalid);
        assert!(error.to_string().contains("':id'"));

        let athena = prepare_parameters("SELECT :id", Dialect::Athena, None).unwrap_err();
        assert_eq!(athena.category(), crate::error::ErrorCategory::Invalid);
    }

    #[test]
    fn athena_takes_its_values_in_the_text_and_binds_none() {
        let mut values = ParamValues::new();
        values.insert("name".to_string(), serde_json::json!("a"));
        let (text, bound) =
            prepare_parameters("SELECT :name", Dialect::Athena, Some(&values)).unwrap();
        assert_eq!(text, "SELECT 'a'");
        assert!(bound.is_none());

        // A statement of Athena that names nothing keeps its text.
        let (plain, none) = prepare_parameters("SELECT 1", Dialect::Athena, Some(&values)).unwrap();
        assert_eq!(plain, "SELECT 1");
        assert!(none.is_none());
    }

    #[tokio::test]
    async fn the_names_of_a_statement_reach_the_interface() {
        assert_eq!(
            query_parameters("SELECT :a, :b".to_string(), Dialect::MsSql)
                .await
                .unwrap(),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[tokio::test]
    async fn a_bounded_run_gives_the_answer_of_the_work() {
        let token = CancellationToken::new();
        let outcome = run_bounded(async { Ok(7_u8) }, &token, 30, STOP_GRACE, None).await;
        assert!(matches!(outcome, Bounded::Answered(Ok(7))));

        // An error of the driver is an answer, so the connection stays open.
        let failed: Bounded<u8> = run_bounded(
            async { Err(Error::Cancelled) },
            &token,
            30,
            STOP_GRACE,
            None,
        )
        .await;
        assert!(matches!(failed, Bounded::Answered(Err(Error::Cancelled))));
    }

    #[tokio::test]
    async fn a_stop_takes_the_answer_of_the_driver_when_one_arrives_in_time() {
        tokio::time::pause();
        let token = CancellationToken::new();
        token.cancel();

        // The server ended the statement and the driver reports that failure
        // through the connection. The failure is an answer, so the caller
        // keeps the connection.
        let outcome: Bounded<u8> = run_bounded(
            async {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                Err(Error::Cancelled)
            },
            &token,
            30,
            STOP_GRACE,
            None,
        )
        .await;

        assert!(matches!(outcome, Bounded::Answered(Err(Error::Cancelled))));
    }

    #[tokio::test]
    async fn a_server_error_after_a_stop_is_a_cancel() {
        let interrupted = || {
            Error::MySql(mysql_async::Error::Server(mysql_async::ServerError {
                code: 1317,
                state: "70100".to_string(),
                message: "Query execution was interrupted".to_string(),
            }))
        };
        let token = CancellationToken::new();
        let failed: Bounded<u8> =
            run_bounded(async { Err(interrupted()) }, &token, 30, STOP_GRACE, None).await;
        // Without a stop the error of the server stays as it is.
        assert!(matches!(failed, Bounded::Answered(Err(Error::MySql(_)))));

        token.cancel();
        let stopped: Bounded<u8> =
            run_bounded(async { Err(interrupted()) }, &token, 30, STOP_GRACE, None).await;
        assert!(matches!(stopped, Bounded::Answered(Err(Error::Cancelled))));
        let answered: Bounded<u8> =
            run_bounded(async { Ok(1) }, &token, 30, STOP_GRACE, None).await;
        assert!(matches!(answered, Bounded::Answered(Ok(1))));
    }

    #[tokio::test]
    async fn another_failure_after_a_stop_keeps_its_reason() {
        let token = CancellationToken::new();
        token.cancel();
        let deadlock: Bounded<u8> = run_bounded(
            async {
                Err(Error::MySql(mysql_async::Error::Server(
                    mysql_async::ServerError {
                        code: 1213,
                        state: "40001".to_string(),
                        message: "Deadlock found".to_string(),
                    },
                )))
            },
            &token,
            30,
            STOP_GRACE,
            None,
        )
        .await;
        assert!(matches!(deadlock, Bounded::Answered(Err(Error::MySql(_)))));
        let syntax: Bounded<u8> = run_bounded(
            async { Err(Error::Athena("syntax error".into())) },
            &token,
            30,
            STOP_GRACE,
            None,
        )
        .await;
        assert!(matches!(syntax, Bounded::Answered(Err(Error::Athena(_)))));
    }

    #[tokio::test]
    async fn a_stop_gives_up_on_a_driver_that_says_nothing() {
        tokio::time::pause();
        let token = CancellationToken::new();
        token.cancel();
        let outcome: Bounded<u8> =
            run_bounded(std::future::pending(), &token, 0, STOP_GRACE, None).await;
        assert!(matches!(outcome, Bounded::Stopped(Error::Cancelled)));
    }

    #[tokio::test]
    async fn a_stop_waits_for_nothing_when_the_server_cannot_be_asked() {
        let token = CancellationToken::new();
        token.cancel();
        let outcome: Bounded<u8> = run_bounded(
            std::future::pending(),
            &token,
            30,
            std::time::Duration::ZERO,
            None,
        )
        .await;
        assert!(matches!(outcome, Bounded::Stopped(Error::Cancelled)));
    }

    #[tokio::test]
    async fn a_bounded_run_stops_at_the_time_limit() {
        tokio::time::pause();
        let token = CancellationToken::new();
        let outcome: Bounded<u8> =
            run_bounded(std::future::pending(), &token, 5, STOP_GRACE, None).await;
        assert!(matches!(outcome, Bounded::Stopped(Error::Timeout(5))));
    }

    /// A handle that counts its calls, for the tests of the time limit.
    fn counting_cancel(
        fails: bool,
    ) -> (Arc<dyn CancelHandle>, Arc<std::sync::atomic::AtomicUsize>) {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let handle = Arc::new(CountingCancel {
            calls: calls.clone(),
            fails,
        });
        (handle, calls)
    }

    #[tokio::test(start_paused = true)]
    async fn the_time_limit_asks_the_server_to_stop_and_reads_its_reply() {
        let token = CancellationToken::new();
        let (handle, calls) = counting_cancel(false);
        // The driver reads the reply of the server to the stop.
        let outcome: Bounded<u8> = run_bounded(
            async {
                tokio::time::sleep(std::time::Duration::from_secs(6)).await;
                Err(Error::Cancelled)
            },
            &token,
            5,
            STOP_GRACE,
            Some(handle),
        )
        .await;
        assert!(matches!(outcome, Bounded::Answered(Err(Error::Timeout(5)))));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);

        // A statement that ends in the grace gives its own answer.
        let (handle, _) = counting_cancel(true);
        let outcome: Bounded<u8> = run_bounded(
            async {
                tokio::time::sleep(std::time::Duration::from_secs(6)).await;
                Ok(3)
            },
            &token,
            5,
            STOP_GRACE,
            Some(handle),
        )
        .await;
        assert!(matches!(outcome, Bounded::Answered(Ok(3))));
    }

    #[tokio::test(start_paused = true)]
    async fn the_time_limit_drops_a_driver_that_says_nothing_after_the_stop() {
        let token = CancellationToken::new();
        let (handle, calls) = counting_cancel(false);
        let outcome: Bounded<u8> = run_bounded(
            std::future::pending(),
            &token,
            5,
            STOP_GRACE,
            Some(handle.clone()),
        )
        .await;
        assert!(matches!(outcome, Bounded::Stopped(Error::Timeout(5))));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);

        // A connection with no grace does not wait.
        let outcome: Bounded<u8> = run_bounded(
            std::future::pending(),
            &token,
            5,
            std::time::Duration::ZERO,
            Some(handle),
        )
        .await;
        assert!(matches!(outcome, Bounded::Stopped(Error::Timeout(5))));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_limit_of_zero_seconds_is_no_limit() {
        tokio::time::pause();
        let waiting = tokio::spawn(until_the_limit(0));
        tokio::time::advance(std::time::Duration::from_secs(60 * 60)).await;
        assert!(!waiting.is_finished());
        waiting.abort();
    }

    fn response_with(rows: Vec<Vec<serde_json::Value>>) -> QueryResponse {
        let mut set = ResultSet::new(vec![ColumnInfo::new("text", "text")]);
        set.rows = rows;
        QueryResponse {
            results: vec![set],
            ..QueryResponse::default()
        }
    }

    fn columns() -> Vec<AppColumn> {
        vec![AppColumn {
            name: "id".into(),
            data_type: "int".into(),
            nullable: false,
            is_primary_key: true,
            is_generated: false,
        }]
    }

    #[test]
    fn the_text_of_a_column_joins_every_row() {
        let response = response_with(vec![
            vec![serde_json::json!("CREATE TABLE t (")],
            vec![serde_json::json!(")")],
        ]);
        assert_eq!(text_of_column(&response, 0).unwrap(), "CREATE TABLE t (\n)");
    }

    #[test]
    fn an_answer_without_text_gives_nothing() {
        assert_eq!(text_of_column(&response_with(Vec::new()), 0), None);
        let blank = response_with(vec![vec![serde_json::json!("  ")]]);
        assert_eq!(text_of_column(&blank, 0), None);
        let other_type = response_with(vec![vec![serde_json::json!(7)]]);
        assert_eq!(text_of_column(&other_type, 0), None);
    }

    #[test]
    fn a_query_with_a_terminator_keeps_a_body_of_statements_whole() {
        let body = "CREATE EVENT e ON SCHEDULE EVERY 1 DAY DO BEGIN SELECT 1; SELECT 2; END";
        let response = response_with(vec![vec![serde_json::json!(body)]]);
        let bare = CreateQuery::new("SHOW CREATE EVENT e", 0);
        assert_eq!(create_text_of(&response, &bare).unwrap(), body);
        let wrapped = bare.with_delimiter();
        assert_eq!(
            create_text_of(&response, &wrapped).unwrap(),
            format!("DELIMITER $$\n{body}$$\nDELIMITER ;")
        );
        assert_eq!(create_text_of(&response_with(Vec::new()), &wrapped), None);
    }

    #[test]
    fn the_text_of_the_engine_wins_for_the_create_form() {
        let text = script_text(
            Dialect::Sqlite,
            "\"t\"",
            ScriptStatement::Create,
            &columns(),
            Some("CREATE TABLE t (id integer)".to_string()),
        )
        .unwrap();
        assert_eq!(text, "CREATE TABLE t (id integer)");
    }

    #[test]
    fn an_engine_without_text_gives_a_draft() {
        let text = script_text(
            Dialect::Sqlite,
            "\"t\"",
            ScriptStatement::Create,
            &columns(),
            None,
        )
        .unwrap();
        assert!(text.contains("CREATE TABLE \"t\" ("));
    }

    #[test]
    fn each_statement_builds_its_own_text() {
        let select =
            script_text(Dialect::Sqlite, "\"t\"", ScriptStatement::Select, &[], None).unwrap();
        assert_eq!(select, "SELECT *\nFROM \"t\";");
        let insert = script_text(
            Dialect::Sqlite,
            "\"t\"",
            ScriptStatement::Insert,
            &columns(),
            None,
        )
        .unwrap();
        assert!(insert.starts_with("INSERT INTO \"t\" ("));
        let update = script_text(
            Dialect::Sqlite,
            "\"t\"",
            ScriptStatement::Update,
            &columns(),
            None,
        )
        .unwrap();
        assert!(update.starts_with("UPDATE \"t\""));
    }

    #[test]
    fn an_object_without_columns_gives_no_statement() {
        let error = script_text(Dialect::Sqlite, "\"t\"", ScriptStatement::Insert, &[], None)
            .expect_err("a statement cannot be built");
        assert!(error.to_string().contains("has no columns"));
    }

    #[test]
    fn a_script_request_names_a_relation_or_another_object() {
        let read = |word: &str| {
            serde_json::from_value::<ScriptRequest>(serde_json::json!({
                "connectionId": "c1",
                "database": null,
                "schemaName": null,
                "tableName": "x",
                "target": word,
                "statement": "create",
            }))
        };
        let view = read("view").unwrap();
        assert_eq!(view.target, ScriptTarget::Relation(RelationType::View));
        assert_eq!(view.parent_name, None);
        assert_eq!(
            read("trigger").unwrap().target,
            ScriptTarget::Object(ObjectType::Trigger)
        );
        assert_eq!(
            read("event").unwrap().target,
            ScriptTarget::Object(ObjectType::Event)
        );
        assert!(read("cursor").is_err());
    }

    #[tokio::test]
    async fn a_trigger_gives_its_create_text_and_no_other_statement() {
        let mut driver = open_driver(&sqlite_connection(":memory:")).await.unwrap();
        driver
            .execute_query(
                "CREATE TABLE t (a); \
                 CREATE TRIGGER audit AFTER INSERT ON t BEGIN SELECT 1; END;",
                None,
                &ExecOptions::default(),
            )
            .await
            .unwrap();
        let place = |name| ObjectPlace {
            database: None,
            schema: None,
            parent: Some("t"),
            name,
        };
        let text = object_script(
            driver.as_mut(),
            place("audit"),
            ObjectType::Trigger,
            ScriptStatement::Create,
        )
        .await
        .unwrap();
        assert_eq!(
            text,
            "CREATE TRIGGER audit AFTER INSERT ON t BEGIN SELECT 1; END"
        );

        let select = object_script(
            driver.as_mut(),
            place("audit"),
            ObjectType::Trigger,
            ScriptStatement::Select,
        )
        .await
        .unwrap_err();
        assert!(select.to_string().contains("only be scripted as CREATE"));
        assert_eq!(select.category(), crate::error::ErrorCategory::Invalid);

        // A trigger that is gone gives no text, and SQLite has no event.
        for (name, object_type) in [
            ("gone", ObjectType::Trigger),
            ("nightly", ObjectType::Event),
        ] {
            let error = object_script(
                driver.as_mut(),
                place(name),
                object_type,
                ScriptStatement::Create,
            )
            .await
            .unwrap_err();
            assert!(error
                .to_string()
                .contains(&format!("no CREATE script for '{name}'")));
            assert_eq!(error.category(), crate::error::ErrorCategory::Invalid);
        }
    }

    fn state() -> AppState {
        AppState::new(Arc::new(MemoryStore::default()))
    }

    fn sqlite_connection(path: &str) -> SavedConnection {
        SavedConnection {
            id: "s1".into(),
            name: "Local".into(),
            db_type: DbType::Sqlite,
            host: None,
            port: None,
            user: None,
            database: None,
            password: None,
            aws_secret_access_key: None,
            aws_session_token: None,
            options: ConnectionOptions {
                file_path: Some(path.to_string()),
                ..ConnectionOptions::default()
            },
            color: None,
            group: None,
        }
    }

    #[tokio::test]
    async fn a_record_that_is_not_complete_is_refused_before_a_socket_opens() {
        let mut connection = sqlite_connection("");
        connection.options.file_path = None;
        assert_eq!(
            open_driver(&connection).await.err().unwrap().category(),
            crate::error::ErrorCategory::Configuration
        );
    }

    #[tokio::test]
    async fn a_sqlite_record_opens_a_driver() {
        let driver = open_driver(&sqlite_connection("file:cmd_test?mode=memory&cache=shared"))
            .await
            .unwrap();
        assert_eq!(driver.dialect(), crate::sql::Dialect::Sqlite);
    }

    #[test]
    fn a_connection_string_with_a_password_is_refused() {
        let mut connection = sqlite_connection("/tmp/a.db");
        assert!(refuse_password_in_string(&connection).is_ok());
        // SQLite and Athena do not read a connection string.
        connection.options.connection_url = Some("password=x".into());
        assert!(refuse_password_in_string(&connection).is_ok());

        let cases = [
            (
                DbType::Mssql,
                "server=tcp:a,1433;pwd=x",
                "server=tcp:a,1433",
            ),
            (
                DbType::Postgres,
                "postgresql://u:x@h/d",
                "postgresql://u@h/d",
            ),
            (DbType::Mysql, "mysql://u:x@h/d", "mysql://u@h/d"),
        ];
        for (db_type, with_password, without) in cases {
            connection.db_type = db_type;
            connection.options.connection_url = Some(with_password.into());
            let error = refuse_password_in_string(&connection).unwrap_err();
            assert_eq!(error.category(), crate::error::ErrorCategory::Configuration);
            assert!(error.to_string().contains("Password field"));
            connection.options.connection_url = Some(without.into());
            assert!(refuse_password_in_string(&connection).is_ok());
        }

        // A string that cannot be read is refused.
        connection.options.connection_url = Some("not-a-url".into());
        assert!(refuse_password_in_string(&connection).is_err());
    }

    #[tokio::test]
    async fn the_password_comes_from_the_secret_store() {
        let state = state();
        state.secrets.set("s1", "from-the-store").unwrap();

        let filled = with_secrets(&state, sqlite_connection("/tmp/a.db"))
            .await
            .unwrap();
        assert_eq!(filled.password.as_deref(), Some("from-the-store"));

        let mut given = sqlite_connection("/tmp/a.db");
        given.password = Some("typed".into());
        let kept = with_secrets(&state, given).await.unwrap();
        assert_eq!(kept.password.as_deref(), Some("typed"));
    }

    #[tokio::test]
    async fn a_password_that_is_absent_stays_absent() {
        let state = state();
        let filled = with_secrets(&state, sqlite_connection("/tmp/a.db"))
            .await
            .unwrap();
        assert_eq!(filled.password, None);
    }

    #[tokio::test]
    async fn the_keys_of_aws_come_from_the_secret_store() {
        let state = state();
        state
            .secrets
            .set(&secrets::aws_secret_key("s1"), "the-secret")
            .unwrap();
        state
            .secrets
            .set(&secrets::aws_token_key("s1"), "the-token")
            .unwrap();

        let mut athena = sqlite_connection("/tmp/a.db");
        athena.db_type = DbType::Athena;
        let filled = with_secrets(&state, athena).await.unwrap();
        assert_eq!(filled.aws_secret_access_key.as_deref(), Some("the-secret"));
        assert_eq!(filled.aws_session_token.as_deref(), Some("the-token"));

        // A key that the caller gave stays as it is.
        let mut given = sqlite_connection("/tmp/a.db");
        given.db_type = DbType::Athena;
        given.aws_secret_access_key = Some("typed".into());
        given.aws_session_token = Some("typed-token".into());
        let kept = with_secrets(&state, given).await.unwrap();
        assert_eq!(kept.aws_secret_access_key.as_deref(), Some("typed"));
        assert_eq!(kept.aws_session_token.as_deref(), Some("typed-token"));
    }

    #[tokio::test]
    async fn a_secret_is_written_kept_or_taken_away() {
        let state = state();

        // A text writes the secret, and the store then holds it.
        assert!(store_secret(state.secrets.as_ref(), "k1", Some("first")).unwrap());
        assert_eq!(state.secrets.get("k1").unwrap().as_deref(), Some("first"));

        // An absent field leaves the store as it stands.
        assert!(store_secret(state.secrets.as_ref(), "k1", None).unwrap());
        assert_eq!(state.secrets.get("k1").unwrap().as_deref(), Some("first"));

        // An empty text takes the secret away.
        assert!(!store_secret(state.secrets.as_ref(), "k1", Some("")).unwrap());
        assert_eq!(state.secrets.get("k1").unwrap(), None);

        // An absent field over an empty store reports no secret.
        assert!(!store_secret(state.secrets.as_ref(), "k1", None).unwrap());
    }
    /// True when no temporary file of a write is left in a folder.
    fn no_temporary_file(folder: &std::path::Path) -> bool {
        std::fs::read_dir(folder).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".part")
        })
    }

    /// Waits until a folder is empty, for at most five seconds. The writer
    /// thread of an export removes its part after the sink is gone.
    fn wait_for_empty(folder: &std::path::Path) -> bool {
        for _ in 0..500 {
            if std::fs::read_dir(folder).unwrap().next().is_none() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        false
    }

    /// A sink whose queue a test reads itself, with room for one piece.
    fn bare_sink(
        path: &std::path::Path,
    ) -> (
        FileSink,
        std::sync::mpsc::Receiver<Piece>,
        tokio::sync::oneshot::Sender<Result<u64>>,
    ) {
        let (pieces, queue) = std::sync::mpsc::sync_channel(1);
        let (sender, done) = tokio::sync::oneshot::channel();
        let sink = FileSink::new(pieces, WriterFault::default(), done, path, None);
        (sink, queue, sender)
    }

    #[test]
    fn a_full_queue_waits_for_the_writer() {
        let (sink, queue, _done) = bare_sink(std::path::Path::new("/tmp/x.csv"));
        sink.send(Piece::Finish).unwrap();
        // The queue is full, so the next send waits until a reader takes
        // the first piece.
        let reader = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            queue.iter().count()
        });
        sink.send(Piece::Finish).unwrap();
        drop(sink);
        assert_eq!(reader.join().unwrap(), 2);
    }

    #[test]
    fn a_writer_that_quits_while_the_queue_is_full_stops_the_export() {
        let (sink, queue, _done) = bare_sink(std::path::Path::new("/tmp/x.csv"));
        sink.send(Piece::Finish).unwrap();
        let reader = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            drop(queue);
        });
        let error = sink.send(Piece::Finish).unwrap_err();
        assert!(error.to_string().contains("quit unexpectedly"), "{error}");
        reader.join().unwrap();
    }

    #[tokio::test]
    async fn a_writer_that_ends_without_an_answer_stops_the_export() {
        let (sink, _queue, done) = bare_sink(std::path::Path::new("/tmp/x.csv"));
        drop(done);
        let error = sink.finish().await.unwrap_err();
        assert!(error.to_string().contains("quit unexpectedly"), "{error}");
    }

    #[tokio::test]
    async fn an_error_of_the_writer_reaches_the_run() {
        use crate::db::sink::RowSink;
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("wide.xlsx");
        let mut sink = FileSink::create(&path, ExportFormat::Xlsx).await.unwrap();
        // A sheet cannot take this many columns, so the writer stops at the
        // start of the set.
        let columns = (0..=crate::xlsx::MAX_SHEET_COLUMNS)
            .map(|index| ColumnInfo::new(format!("c{index}"), "int"))
            .collect();
        sink.begin_set(columns).unwrap();
        let mut error = None;
        for _ in 0..500 {
            if let Err(found) = sink.row(vec![serde_json::json!(1)]) {
                error = Some(found);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(matches!(error, Some(Error::Unsupported(_))), "{error:?}");
        // The error went to the run once. A later finish still fails.
        assert!(sink.finish().await.is_err());
        assert!(wait_for_empty(folder.path()));
    }

    #[tokio::test]
    async fn an_error_at_the_finish_of_the_writer_reaches_the_run() {
        use crate::db::sink::RowSink;
        let folder = tempfile::tempdir().unwrap();
        // A folder with a file in it stands at the path, so the rename fails.
        let path = folder.path().join("taken.csv");
        std::fs::create_dir_all(path.join("inside")).unwrap();
        let mut sink = FileSink::create(&path, ExportFormat::Csv).await.unwrap();
        sink.begin_set(vec![ColumnInfo::new("id", "int")]).unwrap();
        sink.end_set(false).unwrap();
        assert!(matches!(sink.finish().await, Err(Error::Io(_))));
        assert!(no_temporary_file(folder.path()));
    }

    #[test]
    fn a_wait_in_place_runs_the_work_with_or_without_a_runtime() {
        assert_eq!(wait_in_place(|| 1), 1);
        let one_thread = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        assert_eq!(one_thread.block_on(async { wait_in_place(|| 2) }), 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_wait_in_place_on_a_runtime_with_many_threads_runs_the_work() {
        assert_eq!(wait_in_place(|| 3), 3);
    }

    /// Builds an application of the tests with a context, so the files of
    /// the settings answer.
    fn app_with_store() -> tauri::App<tauri::test::MockRuntime> {
        tauri::test::mock_builder()
            .build(tauri::generate_context!())
            .unwrap()
    }

    #[test]
    fn the_paths_of_the_roots_go_out_as_text() {
        let roots = vec![
            std::path::PathBuf::from("/data"),
            std::path::PathBuf::from("/data/other"),
        ];
        assert_eq!(path_names(&roots), vec!["/data", "/data/other"]);
        assert!(path_names(&[]).is_empty());
    }

    #[tokio::test]
    async fn a_folder_that_the_user_accepted_survives_a_restart() {
        let app = app_with_store();
        let state = state();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();

        accept_folder(app.handle(), &state, root.clone()).await;
        assert_eq!(state.file_roots().await, vec![root.clone()]);

        // A new session reads the record of the backend and holds the same
        // folder against every path.
        let next = AppState::new(Arc::new(MemoryStore::default()));
        let names = file_roots_for(app.handle(), &next).await.unwrap();
        assert_eq!(names, path_names(std::slice::from_ref(&root)));
        assert_eq!(next.file_roots().await, vec![root.clone()]);
    }

    #[tokio::test]
    async fn a_folder_that_is_gone_drops_out_of_the_record() {
        let app = app_with_store();
        let state = state();
        let dir = tempfile::tempdir().unwrap();
        let kept = dir.path().to_path_buf();
        let gone = kept.join("nowhere");

        accept_folder(app.handle(), &state, kept.clone()).await;
        accept_folder(app.handle(), &state, gone).await;

        let names = file_roots_for(app.handle(), &state).await.unwrap();
        assert_eq!(names, path_names(std::slice::from_ref(&kept)));
        // The record holds the folder that is left alone.
        assert_eq!(
            store::read_file_roots(app.handle()).unwrap(),
            path_names(std::slice::from_ref(&kept))
        );
    }

    #[tokio::test]
    async fn a_folder_that_closes_leaves_the_state_and_the_record() {
        let app = app_with_store();
        let state = state();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        accept_folder(app.handle(), &state, root.clone()).await;

        close_folder_for(app.handle(), &root.to_string_lossy(), &state)
            .await
            .unwrap();

        assert!(state.file_roots().await.is_empty());
        assert!(store::read_file_roots(app.handle()).unwrap().is_empty());
        // No path under the folder passes the guard any more.
        let file = root.join("a.sql");
        std::fs::write(&file, "SELECT 1").unwrap();
        assert!(files::path_inside_roots(&file, &state.file_roots().await).is_err());
    }

    #[tokio::test]
    async fn a_file_that_the_user_accepted_survives_a_restart_without_its_folder() {
        let app = app_with_store();
        let state = state();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.sql");
        std::fs::write(&file, "SELECT 1").unwrap();
        let beside = dir.path().join("b.sql");
        std::fs::write(&beside, "SELECT 2").unwrap();

        accept_file(app.handle(), &state, &file).await;
        // A path that no file holds gives no grant.
        accept_file(app.handle(), &state, &dir.path().join("gone.sql")).await;
        let resolved = std::fs::canonicalize(&file).unwrap();
        assert_eq!(state.file_grants().await, vec![resolved.clone()]);
        assert!(state.file_roots().await.is_empty());

        // A new session reads the grant back, and the file beside it stays
        // out of reach.
        let next = AppState::new(Arc::new(MemoryStore::default()));
        file_roots_for(app.handle(), &next).await.unwrap();
        assert_eq!(next.file_grants().await, vec![resolved]);
        assert!(accepted_path(&file.to_string_lossy(), &next).await.is_ok());
        assert!(accepted_path(&beside.to_string_lossy(), &next)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn a_folder_lists_inside_a_root_and_not_outside_it() {
        use tauri::Manager;
        let app = app_with_store();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.sql"), "SELECT 1").unwrap();
        let state = state();
        state.add_file_root(root.clone()).await;
        app.manage(state);

        let entries = list_folder(root.to_string_lossy().to_string(), app.state())
            .await
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "a.sql");
        let outside = list_folder(dir.path().to_string_lossy().to_string(), app.state()).await;
        assert!(matches!(outside, Err(Error::Invalid(_))));
    }

    #[tokio::test]
    async fn a_granted_file_that_is_gone_drops_out_of_the_record() {
        let app = app_with_store();
        let state = state();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.sql");
        std::fs::write(&file, "SELECT 1").unwrap();
        accept_file(app.handle(), &state, &file).await;

        std::fs::remove_file(&file).unwrap();
        file_roots_for(app.handle(), &state).await.unwrap();

        assert!(state.file_grants().await.is_empty());
        assert!(store::read_file_grants(app.handle()).unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_test_of_the_saved_record_takes_the_stored_secret() {
        let app = app_with_store();
        let state = state();
        let saved = sqlite_connection("/tmp/a.db");
        store::write_connection(app.handle(), &saved).unwrap();
        state.secrets.set(&saved.id, "from-the-store").unwrap();

        let filled = with_secrets_for_test(app.handle(), &state, saved)
            .await
            .unwrap();
        assert_eq!(filled.password.as_deref(), Some("from-the-store"));
    }

    #[tokio::test]
    async fn a_connect_that_fails_reports_its_error_once() {
        use tauri::{Listener, Manager};
        let app = app_with_store();
        app.manage(state());
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("no-folder").join("a.db");
        let saved = sqlite_connection(missing.to_str().unwrap());
        store::write_connection(app.handle(), &saved).unwrap();
        let events = Arc::new(std::sync::Mutex::new(Vec::<serde_json::Value>::new()));
        let kept = events.clone();
        app.listen(CONNECTION_STATUS_EVENT, move |event| {
            kept.lock()
                .unwrap()
                .push(serde_json::from_str(event.payload()).unwrap());
        });

        let error = connect(app.handle().clone(), saved.id.clone(), app.state())
            .await
            .err()
            .unwrap();
        assert_eq!(error.category(), crate::error::ErrorCategory::Io);
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["health"], "disconnected");
        assert!(events[0]["message"].is_null());
    }

    #[tokio::test]
    async fn a_test_of_a_changed_record_asks_for_the_secret() {
        let app = app_with_store();
        let state = state();
        let saved = sqlite_connection("/tmp/a.db");
        store::write_connection(app.handle(), &saved).unwrap();
        state.secrets.set(&saved.id, "from-the-store").unwrap();

        // The caller names another file, so the stored password does not
        // belong to the record any more.
        let mut changed = saved.clone();
        changed.options.file_path = Some("/tmp/elsewhere.db".into());
        let error = with_secrets_for_test(app.handle(), &state, changed.clone())
            .await
            .err()
            .unwrap();
        assert_eq!(error.category(), crate::error::ErrorCategory::Configuration);
        assert!(error.to_string().contains("Enter the password"));

        // The same record with the password of the caller goes through, and
        // the store gives nothing to it.
        let mut typed = changed.clone();
        typed.password = Some("typed".into());
        let kept = with_secrets_for_test(app.handle(), &state, typed)
            .await
            .unwrap();
        assert_eq!(kept.password.as_deref(), Some("typed"));
    }

    #[tokio::test]
    async fn a_test_of_a_changed_record_names_the_secret_of_aws_that_it_needs() {
        let app = app_with_store();
        let state = state();
        let saved = sqlite_connection("/tmp/a.db");
        store::write_connection(app.handle(), &saved).unwrap();
        state
            .secrets
            .set(&secrets::aws_secret_key(&saved.id), "the-secret")
            .unwrap();
        state
            .secrets
            .set(&secrets::aws_token_key(&saved.id), "the-token")
            .unwrap();

        let mut changed = saved.clone();
        changed.host = Some("elsewhere".into());
        let error = with_secrets_for_test(app.handle(), &state, changed.clone())
            .await
            .err()
            .unwrap();
        assert!(error.to_string().contains("secret access key"));

        changed.aws_secret_access_key = Some("typed".into());
        let error = with_secrets_for_test(app.handle(), &state, changed.clone())
            .await
            .err()
            .unwrap();
        assert!(error.to_string().contains("session token"));

        changed.aws_session_token = Some("typed-token".into());
        let kept = with_secrets_for_test(app.handle(), &state, changed)
            .await
            .unwrap();
        assert_eq!(kept.aws_secret_access_key.as_deref(), Some("typed"));
    }

    #[tokio::test]
    async fn a_test_of_a_record_that_is_not_saved_holds_its_own_fields() {
        let app = app_with_store();
        let state = state();
        let mut given = sqlite_connection("/tmp/a.db");
        given.password = Some("typed".into());

        let kept = with_secrets_for_test(app.handle(), &state, given)
            .await
            .unwrap();
        assert_eq!(kept.password.as_deref(), Some("typed"));
    }

    #[tokio::test]
    async fn a_saved_connection_reads_back_without_its_secret_and_deletes() {
        use tauri::Manager;
        let app = app_with_store();
        app.manage(state());
        let mut record = sqlite_connection("/tmp/saved.db");
        record.password = Some("secret".into());

        save_connection(app.handle().clone(), record.clone(), app.state())
            .await
            .unwrap();
        let listed = get_connections(app.handle().clone()).await.unwrap();
        let found = listed.iter().find(|saved| saved.id == record.id).unwrap();
        assert_eq!(found.password, None);

        delete_connection(app.handle().clone(), record.id.clone(), app.state())
            .await
            .unwrap();
        let listed = get_connections(app.handle().clone()).await.unwrap();
        assert!(listed.iter().all(|saved| saved.id != record.id));
    }

    #[tokio::test]
    async fn a_store_in_memory_reports_that_passwords_do_not_persist() {
        use tauri::Manager;
        let app = app_with_store();
        app.manage(state());
        assert!(!passwords_persist(app.state()).await.unwrap());
    }

    #[tokio::test]
    async fn the_menu_commands_change_in_one_task() {
        // A menu item of muda can only be made on the main thread of macOS,
        // and a test runs on another thread, so the application of the test
        // has no menu. The command then takes the states and changes nothing.
        let app = app_with_store();
        let states = vec![
            MenuCommandState {
                id: "query.save".into(),
                enabled: false,
            },
            MenuCommandState {
                id: "close_window".into(),
                enabled: false,
            },
        ];
        set_menu_commands(app.handle().clone(), states)
            .await
            .unwrap();
        let changed =
            crate::menu::set_commands_enabled(app.handle(), &[("tab.new".to_string(), false)])
                .unwrap();
        assert_eq!(changed, 0);
    }

    #[tokio::test]
    async fn an_identifier_that_no_record_carries_cannot_open() {
        let app = app_with_store();
        assert!(saved_record(app.handle(), "nowhere")
            .await
            .unwrap()
            .is_none());
    }

    #[test]
    fn base64_gives_bytes_and_damaged_content_is_refused() {
        // "PK" is the mark that a ZIP container starts with.
        assert_eq!(decode_base64("UEs=").unwrap(), b"PK");
        let error = decode_base64("not base64!").err().unwrap();
        assert_eq!(error.category(), crate::error::ErrorCategory::Invalid);
    }
    #[test]
    fn a_field_of_a_comma_separated_file_is_quoted_when_it_needs_it() {
        use serde_json::json;
        assert_eq!(csv_field(&json!(null)), "");
        assert_eq!(csv_field(&json!(7)), "7");
        assert_eq!(csv_field(&json!("plain")), "plain");
        assert_eq!(csv_field(&json!("a,b")), "\"a,b\"");
        assert_eq!(csv_field(&json!("say \"no\"")), "\"say \"\"no\"\"\"");
        assert_eq!(csv_field(&json!(" pad ")), "\" pad \"");
        assert_eq!(csv_field(&json!("two\nlines")), "\"two\nlines\"");
    }

    #[tokio::test]
    async fn a_result_reaches_a_file_in_both_forms() {
        use crate::db::sink::{RowSink, SinkControl};
        use crate::db::ColumnInfo;
        let columns = vec![
            ColumnInfo::new("id", "int"),
            ColumnInfo::new("name", "text"),
        ];
        let rows = [
            vec![serde_json::json!(1), serde_json::json!("Ada")],
            vec![serde_json::json!(2), serde_json::json!(null)],
        ];

        let folder = tempfile::tempdir().unwrap();
        let csv = folder.path().join("out.csv");
        let mut sink = FileSink::create(&csv, ExportFormat::Csv).await.unwrap();
        sink.begin_set(columns.clone()).unwrap();
        for row in &rows {
            assert_eq!(sink.row(row.clone()).unwrap(), SinkControl::Continue);
        }
        sink.end_set(false).unwrap();
        let summary = sink.finish().await.unwrap();
        assert_eq!(summary.rows, 2);
        assert!(!summary.truncated);
        assert_eq!(
            std::fs::read_to_string(&csv).unwrap(),
            "\u{feff}id,name\r\n1,Ada\r\n2,\r\n"
        );
        // The temporary file is gone after the rename.
        assert!(no_temporary_file(folder.path()));

        let json = folder.path().join("out.json");
        let mut sink = FileSink::create(&json, ExportFormat::Json).await.unwrap();
        sink.begin_set(columns).unwrap();
        for row in &rows {
            sink.row(row.clone()).unwrap();
        }
        sink.end_set(true).unwrap();
        let summary = sink.finish().await.unwrap();
        assert!(summary.truncated);
        assert_eq!(
            std::fs::read_to_string(&json).unwrap(),
            "[\n  {\"id\":1,\"name\":\"Ada\"},\n  {\"id\":2,\"name\":null}\n]\n"
        );
    }

    #[test]
    fn a_json_object_keeps_the_order_of_the_columns() {
        let names = ["zeta".to_string(), "alpha".to_string(), "mid".to_string()];
        let mut out = Vec::new();
        write_json_object(
            &mut out,
            &names,
            &[serde_json::json!(1), serde_json::json!("a\"b")],
        )
        .unwrap();
        // The row has no value for the last column, so it gets null.
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "{\"zeta\":1,\"alpha\":\"a\\\"b\",\"mid\":null}"
        );
    }

    #[tokio::test]
    async fn a_result_reaches_an_excel_file_as_one_sheet() {
        use crate::db::sink::{RowSink, SinkControl};
        use crate::db::ColumnInfo;
        use std::io::Read;

        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("Daily count.xlsx");
        let mut sink = FileSink::create(&path, ExportFormat::Xlsx).await.unwrap();
        sink.begin_set(vec![
            ColumnInfo::new("id", "int"),
            ColumnInfo::new("name", "text"),
        ])
        .unwrap();
        assert_eq!(
            sink.row(vec![serde_json::json!(1), serde_json::json!("Ada")])
                .unwrap(),
            SinkControl::Continue
        );
        sink.row(vec![serde_json::json!(2), serde_json::json!(null)])
            .unwrap();
        sink.end_set(false).unwrap();
        let summary = sink.finish().await.unwrap();

        assert_eq!(summary.rows, 2);
        assert!(!summary.truncated);
        assert!(no_temporary_file(folder.path()));

        let file = std::fs::File::open(&path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut sheet = String::new();
        archive
            .by_name("xl/worksheets/sheet1.xml")
            .unwrap()
            .read_to_string(&mut sheet)
            .unwrap();
        assert!(sheet.contains("<t xml:space=\"preserve\">id</t>"));
        assert!(sheet.contains("<row r=\"2\"><c r=\"A2\"><v>1</v></c>"));
        // The empty value of the second row leaves out its cell.
        assert!(sheet.contains("<row r=\"3\"><c r=\"A3\"><v>2</v></c></row>"));

        // The sheet takes the name of the file that the user chose.
        let mut workbook = String::new();
        archive
            .by_name("xl/workbook.xml")
            .unwrap()
            .read_to_string(&mut workbook)
            .unwrap();
        assert!(workbook.contains(r#"<sheet name="Daily count""#));
    }

    #[tokio::test]
    async fn a_stopped_export_of_an_excel_file_leaves_no_file() {
        use crate::db::sink::RowSink;
        use crate::db::ColumnInfo;
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("part.xlsx");
        let mut sink = FileSink::create(&path, ExportFormat::Xlsx).await.unwrap();
        sink.begin_set(vec![ColumnInfo::new("id", "int")]).unwrap();
        sink.row(vec![serde_json::json!(1)]).unwrap();
        drop(sink);
        // The writer thread removes the part after the queue closes.
        assert!(wait_for_empty(folder.path()));
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn an_excel_export_stops_at_the_bound_of_a_sheet() {
        use crate::db::sink::{RowSink, SinkControl};
        use crate::db::ColumnInfo;
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("full.xlsx");
        let mut sink = FileSink::create(&path, ExportFormat::Xlsx).await.unwrap();
        sink.begin_set(vec![ColumnInfo::new("id", "int")]).unwrap();

        // The sheet has room for one more row, so the next row is the last
        // one that fits.
        assert_eq!(sink.sheet_room, Some(crate::xlsx::MAX_SHEET_ROWS - 1));
        sink.sheet_room = Some(1);
        assert_eq!(
            sink.row(vec![serde_json::json!(1)]).unwrap(),
            SinkControl::Continue
        );
        assert_eq!(
            sink.row(vec![serde_json::json!(2)]).unwrap(),
            SinkControl::Stop
        );
        sink.end_set(false).unwrap();

        let summary = sink.finish().await.unwrap();
        assert!(summary.truncated);
        assert!(summary.sheet_full);
        assert_eq!(summary.cut_cells, 0);
        assert_eq!(summary.warning, None);
        assert!(path.exists());
    }

    #[tokio::test]
    async fn an_excel_export_counts_the_cells_it_cut() {
        use crate::db::sink::RowSink;
        use crate::db::ColumnInfo;
        use std::io::Read;
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("long.xlsx");
        let mut sink = FileSink::create(&path, ExportFormat::Xlsx).await.unwrap();
        sink.begin_set(vec![
            ColumnInfo::new("note", "text"),
            ColumnInfo::new("code", "varchar"),
        ])
        .unwrap();
        let long = "x".repeat(40_000);
        sink.row(vec![serde_json::json!(long), serde_json::json!("007")])
            .unwrap();
        sink.end_set(false).unwrap();
        let summary = sink.finish().await.unwrap();
        assert!(!summary.sheet_full);
        assert_eq!(summary.cut_cells, 1);
        assert!(summary.warning.unwrap().starts_with("1 cell had more"));

        // A text column keeps a text that looks like a number as text.
        let file = std::fs::File::open(&path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut sheet = String::new();
        archive
            .by_name("xl/worksheets/sheet1.xml")
            .unwrap()
            .read_to_string(&mut sheet)
            .unwrap();
        assert!(sheet.contains("007</t>"), "{sheet}");
    }

    #[test]
    fn the_warning_for_cut_cells_counts_them() {
        assert_eq!(cut_cells_warning(0), None);
        assert!(cut_cells_warning(3).unwrap().starts_with("3 cells had"));
        let value = serde_json::to_value(ExportSummary {
            rows: 1,
            truncated: false,
            path: "p".into(),
            sheet_full: true,
            cut_cells: 2,
            warning: None,
        })
        .unwrap();
        assert_eq!(value["sheetFull"], true);
        assert_eq!(value["cutCells"], 2);
    }

    #[tokio::test]
    async fn a_stopped_export_leaves_no_file() {
        use crate::db::sink::RowSink;
        use crate::db::ColumnInfo;
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("part.csv");
        let mut sink = FileSink::create(&path, ExportFormat::Csv).await.unwrap();
        sink.begin_set(vec![ColumnInfo::new("id", "int")]).unwrap();
        sink.row(vec![serde_json::json!(1)]).unwrap();
        drop(sink);
        // The writer thread removes the part after the queue closes.
        assert!(wait_for_empty(folder.path()));
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn an_export_writes_the_first_set_alone() {
        use crate::db::sink::{RowSink, SinkControl};
        use crate::db::ColumnInfo;
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("first.csv");
        let mut sink = FileSink::create(&path, ExportFormat::Csv).await.unwrap();
        sink.begin_set(vec![ColumnInfo::new("id", "int")]).unwrap();
        sink.row(vec![serde_json::json!(1)]).unwrap();
        sink.end_set(false).unwrap();

        // The second set is not written, and its rows stop the run.
        sink.begin_set(vec![ColumnInfo::new("other", "int")])
            .unwrap();
        assert_eq!(
            sink.row(vec![serde_json::json!(9)]).unwrap(),
            SinkControl::Stop
        );
        sink.end_set(false).unwrap();

        let summary = sink.finish().await.unwrap();
        assert_eq!(summary.rows, 1);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "\u{feff}id\r\n1\r\n"
        );
    }

    #[tokio::test]
    async fn an_empty_json_export_is_a_valid_list() {
        use crate::db::sink::RowSink;
        use crate::db::ColumnInfo;
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("empty.json");
        let mut sink = FileSink::create(&path, ExportFormat::Json).await.unwrap();
        sink.begin_set(vec![ColumnInfo::new("id", "int")]).unwrap();
        sink.end_set(false).unwrap();
        let summary = sink.finish().await.unwrap();
        assert_eq!(summary.rows, 0);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[\n]\n");
    }

    #[tokio::test]
    async fn an_export_refuses_a_statement_that_changes_data() {
        assert!(!crate::sql::only_reads("DELETE FROM t", Dialect::Postgres));
        assert!(crate::sql::only_reads(
            "  /* note */ SELECT 1",
            Dialect::Postgres
        ));
    }

    #[test]
    fn a_field_that_starts_a_formula_gets_an_apostrophe() {
        use serde_json::json;
        assert_eq!(csv_field(&json!("=SUM(A1:A9)")), "'=SUM(A1:A9)");
        assert_eq!(csv_field(&json!("+cmd")), "'+cmd");
        assert_eq!(csv_field(&json!("-cmd")), "'-cmd");
        assert_eq!(csv_field(&json!("@name")), "'@name");
        assert_eq!(csv_field(&json!("\rcmd")), "\"'\rcmd\"");
        // A number that arrives as text keeps its sign too.
        for number in ["-5", "+1", "-10.00", "-.5", "-5.", "-1e10", "+2.5E-3"] {
            assert_eq!(csv_field(&json!(number)), number);
        }
        for text in [
            "-", "-.", "-1e", "-1e+", "-1.2.3", "-1x", "-e5", "-1e5x", "-1-2",
        ] {
            assert_eq!(csv_field(&json!(text)), format!("'{text}"));
        }
        // A number keeps its sign, because a spreadsheet reads it as a
        // number and not as a formula.
        assert_eq!(csv_field(&json!(-5)), "-5");
        assert_eq!(csv_field(&json!("a=b")), "a=b");
    }

    /// Builds a state with one open SQLite connection, for the tests of the
    /// sessions of the tabs.
    pub(super) async fn state_with_sqlite(
        descriptor: SavedConnection,
    ) -> (tauri::App<tauri::test::MockRuntime>, AppState) {
        let app = tauri::test::mock_app();
        let driver = open_driver(&descriptor).await.unwrap();
        let state = AppState::new(Arc::new(MemoryStore::default()));
        let id = descriptor.id.clone();
        state
            .insert(&id, OpenConnection::new(descriptor, driver))
            .await;
        (app, state)
    }

    pub(super) fn temp_sqlite() -> (tempfile::TempDir, SavedConnection) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tabs.db");
        (dir, sqlite_connection(path.to_str().unwrap()))
    }

    /// A channel that keeps the first byte of every message, which names
    /// the type of the first frame of that message.
    fn frame_type_channel() -> (
        Channel<InvokeResponseBody>,
        std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    ) {
        let frame_types = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let kept = frame_types.clone();
        let channel = Channel::new(move |body| {
            if let InvokeResponseBody::Raw(bytes) = body {
                kept.lock().unwrap().push(bytes[0]);
            }
            Ok(())
        });
        (channel, frame_types)
    }

    fn run_request(connection_id: &str, query: &str) -> ExecuteRequest {
        ExecuteRequest {
            connection_id: connection_id.into(),
            request_id: "r1".into(),
            query: query.into(),
            tab_id: Some("t1".into()),
            query_params: None,
            options: None,
        }
    }

    #[tokio::test]
    async fn a_run_ends_its_channel_with_the_end_frame() {
        use crate::db::columnar::FRAME_END;
        use tauri::Manager;
        let (_dir, descriptor) = temp_sqlite();
        let (app, state) = state_with_sqlite(descriptor).await;
        app.manage(state);

        let (channel, frame_types) = frame_type_channel();
        execute_query(
            app.handle().clone(),
            run_request("s1", "SELECT 1"),
            app.state::<AppState>(),
            channel,
        )
        .await
        .unwrap();
        assert_eq!(frame_types.lock().unwrap().last(), Some(&FRAME_END));

        // A run that fails before it reaches the server sends the end frame
        // too, because the window waits for it.
        let (channel, frame_types) = frame_type_channel();
        let error = execute_query(
            app.handle().clone(),
            run_request("missing", "SELECT 1"),
            app.state::<AppState>(),
            channel,
        )
        .await
        .err()
        .unwrap();
        assert!(matches!(&error, Error::NotConnected(id) if id == "missing"));
        assert_eq!(error.category(), crate::error::ErrorCategory::NotConnected);
        assert_eq!(*frame_types.lock().unwrap(), vec![FRAME_END]);
    }

    #[tokio::test]
    async fn each_tab_takes_a_session_of_its_own() {
        let (_dir, descriptor) = temp_sqlite();
        let (app, state) = state_with_sqlite(descriptor).await;

        let (open, first, key_one) = session_for(
            app.handle(),
            &state,
            "s1",
            Some("t1"),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        let (_, second, key_two) = session_for(
            app.handle(),
            &state,
            "s1",
            Some("t2"),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(key_one, "t1");
        assert_eq!(key_two, "t2");
        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(open.sessions.tab_count().await, 2);

        // The tab keeps its session from one run to the next.
        let (_, again, _) = session_for(
            app.handle(),
            &state,
            "s1",
            Some("t1"),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(Arc::ptr_eq(&first, &again));
    }

    #[tokio::test]
    async fn a_request_without_a_tab_takes_the_default_session() {
        let (_dir, descriptor) = temp_sqlite();
        let (app, state) = state_with_sqlite(descriptor).await;

        let (open, session, key) =
            session_for(app.handle(), &state, "s1", None, &CancellationToken::new())
                .await
                .unwrap();
        assert_eq!(key, DEFAULT_SESSION);
        let default = open.default_session().await.unwrap();
        assert!(Arc::ptr_eq(&session, &default));
        assert_eq!(open.sessions.tab_count().await, 0);
    }

    #[tokio::test]
    async fn the_cap_stops_a_new_tab_with_a_clear_message() {
        let (_dir, mut descriptor) = temp_sqlite();
        descriptor.options.max_sessions = 1;
        let (app, state) = state_with_sqlite(descriptor).await;

        let (_, first, _) = session_for(
            app.handle(),
            &state,
            "s1",
            Some("t1"),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        // A session that runs a statement cannot close to make room.
        let busy = first.driver.lock().await;
        let error = session_for(
            app.handle(),
            &state,
            "s1",
            Some("t2"),
            &CancellationToken::new(),
        )
        .await
        .err()
        .unwrap();
        drop(busy);
        assert_eq!(error.category(), crate::error::ErrorCategory::Invalid);
        assert!(error.to_string().contains("Max sessions"));

        // The tab that holds a session keeps it, and the default session
        // stays outside the cap.
        assert!(session_for(
            app.handle(),
            &state,
            "s1",
            Some("t1"),
            &CancellationToken::new()
        )
        .await
        .is_ok());
        assert!(
            session_for(app.handle(), &state, "s1", None, &CancellationToken::new())
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn the_cap_closes_an_idle_session_to_make_room() {
        let (_dir, mut descriptor) = temp_sqlite();
        descriptor.options.max_sessions = 1;
        let (app, state) = state_with_sqlite(descriptor).await;

        let (open, first, _) = session_for(
            app.handle(),
            &state,
            "s1",
            Some("t1"),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        // The request that took the session has ended.
        first.age(crate::session::EVICT_IDLE_AFTER).await;
        drop(first);
        session_for(
            app.handle(),
            &state,
            "s1",
            Some("t2"),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(open.sessions.get("t1").await.is_none());
    }

    #[tokio::test]
    async fn the_cap_keeps_an_idle_session_inside_a_transaction() {
        let (_dir, mut descriptor) = temp_sqlite();
        descriptor.options.max_sessions = 1;
        let (app, state) = state_with_sqlite(descriptor).await;

        let (open, first, _) = session_for(
            app.handle(),
            &state,
            "s1",
            Some("t1"),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        first
            .driver
            .lock()
            .await
            .execute_query("BEGIN", None, &ExecOptions::default())
            .await
            .unwrap();
        first.age(crate::session::SESSION_IDLE_REAP).await;
        assert!(session_for(
            app.handle(),
            &state,
            "s1",
            Some("t2"),
            &CancellationToken::new()
        )
        .await
        .is_err());
        let kept = open.sessions.get("t1").await.unwrap();
        assert!(Arc::ptr_eq(&kept, &first));
    }

    #[tokio::test]
    async fn a_database_in_memory_gives_every_tab_the_default_session() {
        let descriptor = sqlite_connection(":memory:");
        let (app, state) = state_with_sqlite(descriptor).await;

        let (open, session, key) = session_for(
            app.handle(),
            &state,
            "s1",
            Some("t1"),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(open.single_session);
        assert_eq!(key, DEFAULT_SESSION);
        let default = open.default_session().await.unwrap();
        assert!(Arc::ptr_eq(&session, &default));
    }

    #[tokio::test]
    async fn a_database_in_memory_reads_its_catalog_on_the_default_session() {
        let descriptor = sqlite_connection(":memory:");
        let (app, state) = state_with_sqlite(descriptor).await;
        let open = state.connection("s1").await.unwrap();
        let default = open.default_session().await.unwrap();
        default
            .driver
            .lock()
            .await
            .execute_query("CREATE TABLE kept (a)", None, &ExecOptions::default())
            .await
            .unwrap();

        // The read opens no second connection, so it sees the table that
        // the session of the user made.
        let read = metadata_read(app.handle(), &state, "s1").await.unwrap();
        assert!(Arc::ptr_eq(&read.session, &default));
        let mut guard = read.lock().await.unwrap();
        let tables = read.run(guard.list_tables("main", None)).await.unwrap();
        assert_eq!(tables[0].name, "kept");
        assert!(state
            .background_session("s1", BackgroundRole::Catalog)
            .await
            .is_none());
    }

    #[tokio::test]
    async fn a_released_tab_session_leaves_the_pool() {
        use tauri::Manager;
        let (_dir, descriptor) = temp_sqlite();
        let (app, state) = state_with_sqlite(descriptor).await;
        session_for(
            app.handle(),
            &state,
            "s1",
            Some("t1"),
            &CancellationToken::new(),
        )
        .await
        .unwrap();

        app.manage(state);
        let managed: tauri::State<'_, AppState> = app.state();
        release_session("s1".to_string(), "t1".to_string(), managed.clone())
            .await
            .unwrap();
        let open = managed.connection("s1").await.unwrap();
        assert_eq!(open.sessions.tab_count().await, 0);

        // A tab or a connection that is unknown changes nothing.
        release_session("s1".to_string(), "t9".to_string(), managed.clone())
            .await
            .unwrap();
        release_session("nope".to_string(), "t1".to_string(), managed)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_stop_closes_only_the_session_that_ran_the_statement() {
        struct FrailDriver;

        #[async_trait::async_trait]
        impl DatabaseDriver for FrailDriver {
            fn capabilities(&self) -> crate::db::DriverCapabilities {
                crate::db::DriverCapabilities::default()
            }
            fn dialect(&self) -> Dialect {
                Dialect::Sqlite
            }
            async fn ping(&mut self) -> Result<()> {
                Ok(())
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

        let (_dir, descriptor) = temp_sqlite();
        let (app, state) = state_with_sqlite(descriptor).await;
        let open = state.connection("s1").await.unwrap();

        // A driver that does not keep its connection after a stop sits in
        // the slot of the tab.
        let frail = open
            .sessions
            .insert("t1", crate::session::Session::new(Box::new(FrailDriver)))
            .await;
        let default_before = open.default_session().await.unwrap();

        let outcome: Bounded<()> = Bounded::Stopped(Error::Cancelled);
        let result = finish_run(&state, "s1", &open, "t1", &frail, outcome).await;
        assert!(result.is_err());

        // The session leaves the slot of the tab at once, and the default
        // session stays as it was. The next request of the tab opens a new
        // session.
        assert!(frail.is_broken());
        assert!(open.sessions.get("t1").await.is_none());
        let (_, reopened, _) = session_for(
            app.handle(),
            &state,
            "s1",
            Some("t1"),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(!Arc::ptr_eq(&frail, &reopened));
        let default_after = open.default_session().await.unwrap();
        assert!(Arc::ptr_eq(&default_before, &default_after));
    }

    #[tokio::test]
    async fn a_statement_goes_to_an_accepted_file_without_a_dialog() {
        let app = app_with_store();
        let state = state();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.sql");
        std::fs::write(&file, "SELECT 1").unwrap();
        let name = file.to_string_lossy().to_string();

        // A file that the user did not accept stays out of reach, and the
        // caller asks for a path.
        assert_eq!(
            write_accepted(&name, "SELECT 2", None, &state)
                .await
                .unwrap(),
            None
        );
        assert_eq!(std::fs::read(&file).unwrap(), b"SELECT 1");

        accept_file(app.handle(), &state, &file).await;
        let saved = write_accepted(&name, "SELECT 2", None, &state)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.path, name);
        assert_eq!(saved.encoding, files::TextEncoding::Utf8);
        assert_eq!(std::fs::read(&file).unwrap(), b"SELECT 2");
    }

    #[test]
    fn the_save_dialog_starts_at_the_file_of_the_tab() {
        let request = |value: serde_json::Value| -> SaveStatementRequest {
            serde_json::from_value(value).unwrap()
        };
        let known = request(serde_json::json!({
            "path": "/data/reports/daily.sql",
            "defaultName": "Query 1.sql",
            "defaultFolder": "/work",
            "contents": "",
        }));
        assert_eq!(
            dialog_start(&known),
            (
                "daily.sql".to_owned(),
                Some(std::path::PathBuf::from("/data/reports"))
            )
        );

        // A tab without a file uses the name and the folder of the request.
        let fresh = request(serde_json::json!({
            "defaultName": "Query 1.sql",
            "defaultFolder": "/work",
            "contents": "",
        }));
        assert_eq!(
            dialog_start(&fresh),
            (
                "Query 1.sql".to_owned(),
                Some(std::path::PathBuf::from("/work"))
            )
        );

        // A bare file name gives no folder of its own.
        let bare = request(serde_json::json!({
            "path": "daily.sql",
            "defaultName": "x.sql",
            "defaultFolder": null,
            "contents": "",
        }));
        assert_eq!(dialog_start(&bare), ("daily.sql".to_owned(), None));
    }

    #[test]
    fn a_saved_statement_gives_its_path_and_its_encoding() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new.sql");
        let saved = write_statement(&path, "SELECT 1", None).unwrap();
        assert_eq!(saved.encoding, files::TextEncoding::Utf8);
        assert_eq!(saved.path, path.to_string_lossy());
        assert_eq!(std::fs::read(&path).unwrap(), b"SELECT 1");
        assert_eq!(
            serde_json::to_value(&saved).unwrap()["encoding"],
            serde_json::json!("utf8")
        );

        // Text that Windows-1252 cannot store goes out as UTF-8 with a mark.
        let kept =
            write_statement(&path, "SELECT 'é'", Some(files::TextEncoding::Windows1252)).unwrap();
        assert_eq!(kept.encoding, files::TextEncoding::Windows1252);
        let moved =
            write_statement(&path, "SELECT '😀'", Some(files::TextEncoding::Windows1252)).unwrap();
        assert_eq!(moved.encoding, files::TextEncoding::Utf8Bom);

        let request: SaveStatementRequest = serde_json::from_value(serde_json::json!({
            "defaultName": "a.sql",
            "defaultFolder": null,
            "contents": "",
        }))
        .unwrap();
        assert_eq!(request.encoding, None);
    }

    #[test]
    fn the_place_of_an_error_moves_into_the_sent_text() {
        let sent = "SELECT 1\nWHERE a = :alpha AND b = c\n  AND d = e";
        let ran = "SELECT 1\nWHERE a = $1 AND b = c\n  AND d = e";
        let place = |outcome: Bounded<()>| match outcome {
            Bounded::Answered(Err(Error::Located { line, column, .. })) => (line, column),
            _ => panic!("the error has no place"),
        };
        let failed =
            |line, column| Bounded::Answered(Err(Error::Invalid("bad".into()).at(line, column)));

        // A column after the parameter on its line moves to 1.
        assert_eq!(place(in_sent_text(failed(2, 22), sent, ran)), (2, 1));
        // A column before the parameter, or on a line without one, stays.
        assert_eq!(place(in_sent_text(failed(2, 7), sent, ran)), (2, 7));
        assert_eq!(place(in_sent_text(failed(3, 5), sent, ran)), (3, 5));
        // A text that the rewrite did not change keeps every place.
        assert_eq!(place(in_sent_text(failed(2, 22), sent, sent)), (2, 22));
        // A place outside the text stays as it is.
        assert_eq!(place(in_sent_text(failed(9, 4), sent, ran)), (9, 4));
        assert_eq!(place(in_sent_text(failed(0, 4), sent, ran)), (0, 4));

        // An answer without a place passes through.
        assert!(matches!(
            in_sent_text(Bounded::Answered(Ok(())), sent, ran),
            Bounded::Answered(Ok(()))
        ));
        assert!(matches!(
            in_sent_text::<()>(Bounded::Stopped(Error::Cancelled), sent, ran),
            Bounded::Stopped(Error::Cancelled)
        ));
    }

    #[tokio::test]
    async fn a_stop_lets_the_ping_of_a_session_end() {
        use tokio::sync::Notify;

        /// A driver whose ping waits for the test.
        struct GatedDriver {
            started: Arc<Notify>,
            go: Arc<Notify>,
        }

        #[async_trait::async_trait]
        impl DatabaseDriver for GatedDriver {
            fn capabilities(&self) -> crate::db::DriverCapabilities {
                crate::db::DriverCapabilities::default()
            }
            fn dialect(&self) -> Dialect {
                Dialect::Sqlite
            }
            async fn ping(&mut self) -> Result<()> {
                self.started.notify_one();
                self.go.notified().await;
                Ok(())
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

        let (_dir, descriptor) = temp_sqlite();
        let (app, state) = state_with_sqlite(descriptor).await;
        let open = state.connection("s1").await.unwrap();
        let started = Arc::new(Notify::new());
        let go = Arc::new(Notify::new());
        let gated = open
            .sessions
            .insert(
                "t1",
                Session::new(Box::new(GatedDriver {
                    started: started.clone(),
                    go: go.clone(),
                })),
            )
            .await;
        gated.age(crate::state::HEALTH_CHECK_AFTER).await;

        let state = Arc::new(state);
        let token = CancellationToken::new();
        let request = tokio::spawn({
            let state = state.clone();
            let token = token.clone();
            let handle = app.handle().clone();
            async move {
                session_for(&handle, &state, "s1", Some("t1"), &token)
                    .await
                    .map(|(_, session, _)| session)
            }
        });
        started.notified().await;
        token.cancel();
        tokio::task::yield_now().await;
        assert!(!request.is_finished());
        go.notify_one();

        // The ping ends, so the session stays in its slot and answered.
        let session = request.await.unwrap().unwrap();
        assert!(Arc::ptr_eq(&session, &gated));
        assert!(!gated.needs_check().await);
        let kept = open.sessions.get("t1").await.unwrap();
        assert!(Arc::ptr_eq(&kept, &gated));
    }

    #[tokio::test]
    async fn a_stop_ends_the_wait_for_a_session() {
        let token = CancellationToken::new();
        assert!(matches!(
            unless_stopped(async { Ok(1) }, &token).await,
            Ok(1)
        ));
        token.cancel();
        let waited: Result<u8> = unless_stopped(std::future::pending(), &token).await;
        assert!(matches!(waited, Err(Error::Cancelled)));
    }

    #[test]
    fn an_export_of_a_change_is_refused_before_the_dialog() {
        assert!(refuse_export_of_writes("SELECT 1", Dialect::Postgres).is_ok());
        let error = refuse_export_of_writes("DELETE FROM t", Dialect::Postgres).unwrap_err();
        assert!(error.to_string().contains("read-only"));
    }

    #[test]
    fn a_secret_goes_when_the_record_names_another_server() {
        let saved = sqlite_connection("/a.db");
        assert!(!target_changed(&saved, &saved.clone()));
        let mut other = saved.clone();
        other.name = "Renamed".into();
        assert!(!target_changed(&saved, &other));
        for change in [
            |c: &mut SavedConnection| c.host = Some("evil".into()),
            |c: &mut SavedConnection| c.port = Some(1),
            |c: &mut SavedConnection| c.user = Some("x".into()),
            |c: &mut SavedConnection| c.db_type = DbType::Postgres,
            |c: &mut SavedConnection| c.options.file_path = Some("/b.db".into()),
            |c: &mut SavedConnection| c.options.aws_region = Some("eu".into()),
            |c: &mut SavedConnection| c.options.instance_name = Some("SQL2".into()),
            |c: &mut SavedConnection| c.options.connection_url = Some("pg://h".into()),
            |c: &mut SavedConnection| c.options.aws_access_key_id = Some("AK".into()),
        ] {
            let mut moved = saved.clone();
            change(&mut moved);
            assert!(target_changed(&saved, &moved));
        }

        // A field that is missing, empty, or only spaces names no server,
        // and the spaces at the ends of a value do not count.
        for blank in [None, Some(String::new()), Some("  ".to_string())] {
            let mut cleared = saved.clone();
            cleared.host = blank.clone();
            cleared.user = blank.clone();
            cleared.options.instance_name = blank;
            assert!(!target_changed(&saved, &cleared));
            assert!(!target_changed(&cleared, &saved));
        }
        let mut spaced = saved.clone();
        spaced.options.file_path = Some(" /a.db ".into());
        assert!(!target_changed(&saved, &spaced));
        assert_eq!(kept(None, true), Some(""));
        assert_eq!(kept(None, false), None);
        assert_eq!(kept(Some("new"), true), Some("new"));
    }

    #[tokio::test]
    async fn only_athena_reads_the_keys_of_aws() {
        let state = state();
        state.secrets.set("c1", "pw").unwrap();
        state
            .secrets
            .set(&secrets::aws_secret_key("c1"), "key")
            .unwrap();
        let mut record = sqlite_connection("/a.db");
        record.id = "c1".into();
        let full = with_secrets(&state, record.clone()).await.unwrap();
        assert_eq!(full.password.as_deref(), Some("pw"));
        assert_eq!(full.aws_secret_access_key, None);
        record.db_type = DbType::Athena;
        let full = with_secrets(&state, record).await.unwrap();
        assert_eq!(full.aws_secret_access_key.as_deref(), Some("key"));
    }

    /// A store whose every call fails, as a keychain that refuses access.
    struct RefusingStore;

    impl SecretStore for RefusingStore {
        fn set(&self, _id: &str, _password: &str) -> Result<()> {
            Err(keyring::Error::NoStorageAccess("refused".into()).into())
        }
        fn get(&self, _id: &str) -> Result<Option<String>> {
            Err(keyring::Error::NoStorageAccess("refused".into()).into())
        }
        fn delete(&self, _id: &str) -> Result<()> {
            Err(keyring::Error::NoStorageAccess("refused".into()).into())
        }
    }

    #[tokio::test]
    async fn an_error_of_the_keychain_comes_back_unchanged() {
        let state = AppState::new(Arc::new(RefusingStore));
        let error = with_secrets(&state, sqlite_connection("/tmp/a.db"))
            .await
            .err()
            .unwrap();
        assert!(matches!(
            error,
            Error::Keyring(keyring::Error::NoStorageAccess(_))
        ));
        let error = with_store(&state, |store| store_secret(store, "k1", Some("new")))
            .await
            .err()
            .unwrap();
        assert!(matches!(error, Error::Keyring(_)));
        assert!(RefusingStore.set("k1", "new").is_err());

        // A delete that the keychain refuses goes to the log, and the
        // delete of the record still goes ahead.
        let app = app_with_store();
        close_deleted_connection(app.handle(), &state, "s1").await;
    }

    #[tokio::test]
    async fn a_deleted_connection_closes_and_leaves_the_keychain() {
        let (_dir, descriptor) = temp_sqlite();
        let (app, state) = state_with_sqlite(descriptor).await;
        state.secrets.set("s1", "pw").unwrap();
        state
            .secrets
            .set(&secrets::aws_token_key("s1"), "t")
            .unwrap();
        let token = state.start_request("r1", "s1").await;

        close_deleted_connection(app.handle(), &state, "s1").await;

        assert!(state.connection("s1").await.is_err());
        assert!(token.is_cancelled());
        assert_eq!(state.secrets.get("s1").unwrap(), None);
        assert_eq!(
            state.secrets.get(&secrets::aws_token_key("s1")).unwrap(),
            None
        );
        // A second delete finds nothing open and still works.
        close_deleted_connection(app.handle(), &state, "s1").await;
    }

    #[tokio::test]
    async fn a_metadata_read_does_not_wait_behind_the_default_session() {
        let (_dir, descriptor) = temp_sqlite();
        let (app, state) = state_with_sqlite(descriptor).await;
        let open = state.connection("s1").await.unwrap();
        let default = open.default_session().await.unwrap();
        default.age(crate::state::HEALTH_CHECK_AFTER).await;
        let _busy = default.driver.lock().await;
        let found = tokio::time::timeout(SHORT, ensure_healthy(app.handle(), &state, "s1")).await;
        assert!(found.unwrap().is_ok());
        assert!(ensure_healthy(app.handle(), &state, "none").await.is_err());
    }

    #[tokio::test]
    async fn one_change_of_the_files_record_runs_at_a_time() {
        let app = app_with_store();
        let state = state();
        let folder = tempfile::tempdir().unwrap();
        let held = state.files_record.lock().await;
        let root = folder.path().to_path_buf();
        let waiting =
            tokio::time::timeout(SHORT, accept_folder(app.handle(), &state, root.clone()));
        assert!(waiting.await.is_err());
        let file = folder.path().join("a.sql");
        std::fs::write(&file, "SELECT 1").unwrap();
        assert!(
            tokio::time::timeout(SHORT, accept_file(app.handle(), &state, &file))
                .await
                .is_err()
        );
        assert!(
            tokio::time::timeout(SHORT, file_roots_for(app.handle(), &state))
                .await
                .is_err()
        );
        assert!(
            tokio::time::timeout(SHORT, close_folder_for(app.handle(), "/x", &state))
                .await
                .is_err()
        );
        drop(held);
        accept_folder(app.handle(), &state, root).await;
        assert_eq!(state.file_roots().await.len(), 1);
    }

    #[tokio::test]
    async fn a_session_that_stopped_answering_and_cannot_reopen_goes() {
        let broken = sqlite_connection("/no/such/folder/x.db");
        let driver = Box::new(PingDriver {
            pings: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            answers: true,
            hangs: false,
        });
        let state = state();
        state
            .insert("s1", OpenConnection::new(broken, driver))
            .await;
        let app = tauri::test::mock_app();
        let open = state.connection("s1").await.unwrap();
        let pings = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let frail = open
            .sessions
            .insert(
                "t1",
                Session::new(Box::new(PingDriver {
                    pings: pings.clone(),
                    answers: false,
                    hangs: false,
                })),
            )
            .await;
        frail.age(crate::state::HEALTH_CHECK_AFTER).await;

        let failed = session_for(
            app.handle(),
            &state,
            "s1",
            Some("t1"),
            &CancellationToken::new(),
        )
        .await;
        assert!(failed.is_err());
        assert_eq!(pings.load(std::sync::atomic::Ordering::SeqCst), 1);
        // The tab session goes, and the connection stays for its default
        // session.
        assert!(open.sessions.get("t1").await.is_none());
        assert!(state.connection("s1").await.is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn a_tab_session_whose_ping_passes_its_limit_opens_again() {
        let (_dir, descriptor) = temp_sqlite();
        let (app, state) = state_with_sqlite(descriptor).await;
        let open = state.connection("s1").await.unwrap();
        let pings = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let silent = open
            .sessions
            .insert(
                "t1",
                Session::new(Box::new(PingDriver {
                    pings: pings.clone(),
                    answers: true,
                    hangs: true,
                })),
            )
            .await;
        silent.age(crate::state::HEALTH_CHECK_AFTER).await;

        let (_, session, _) = session_for(
            app.handle(),
            &state,
            "s1",
            Some("t1"),
            &CancellationToken::new(),
        )
        .await
        .unwrap();

        assert_eq!(pings.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(!Arc::ptr_eq(&session, &silent));
    }

    #[tokio::test]
    async fn a_failed_reopen_keeps_a_connection_with_other_sessions() {
        let (_dir, descriptor) = temp_sqlite();
        let (app, state) = state_with_sqlite(descriptor).await;
        let (open, _, _) = session_for(
            app.handle(),
            &state,
            "s1",
            Some("t1"),
            &CancellationToken::new(),
        )
        .await
        .unwrap();

        reopen_failed(app.handle(), &state, "s1", &open, "t1", Some("gone".into())).await;
        assert!(open.sessions.get("t1").await.is_none());
        assert!(state.connection("s1").await.is_ok());

        // The last session goes, and the connection with it.
        reopen_failed(
            app.handle(),
            &state,
            "s1",
            &open,
            DEFAULT_SESSION,
            Some("gone".into()),
        )
        .await;
        assert!(state.connection("s1").await.is_err());
    }

    #[tokio::test]
    async fn a_failed_reopen_leaves_a_connection_that_a_connect_replaced() {
        let (_dir, descriptor) = temp_sqlite();
        let (app, state) = state_with_sqlite(descriptor.clone()).await;
        let stale = state.connection("s1").await.unwrap();
        let driver = open_driver(&descriptor).await.unwrap();
        state
            .insert("s1", OpenConnection::new(descriptor, driver))
            .await;

        reopen_failed(
            app.handle(),
            &state,
            "s1",
            &stale,
            DEFAULT_SESSION,
            Some("gone".into()),
        )
        .await;
        let current = state.connection("s1").await.unwrap();
        assert!(!Arc::ptr_eq(&current.sessions, &stale.sessions));
        assert!(!state.remove_if_same("s1", &stale.sessions).await);
        assert!(state.remove_if_same("s1", &current.sessions).await);

        // A connection that is gone is left alone too.
        reopen_failed(
            app.handle(),
            &state,
            "s1",
            &stale,
            DEFAULT_SESSION,
            Some("gone".into()),
        )
        .await;
        assert!(stale.sessions.get(DEFAULT_SESSION).await.is_some());
    }

    /// A driver for the tests of the background connection. It counts the
    /// pings it answered and fails every ping when it is told to.
    struct PingDriver {
        pings: Arc<std::sync::atomic::AtomicUsize>,
        answers: bool,
        /// True for a driver whose ping never ends, as on a connection that
        /// a firewall dropped without a word.
        hangs: bool,
    }

    #[async_trait::async_trait]
    impl DatabaseDriver for PingDriver {
        fn capabilities(&self) -> crate::db::DriverCapabilities {
            crate::db::DriverCapabilities::default()
        }
        fn dialect(&self) -> Dialect {
            Dialect::Sqlite
        }
        async fn ping(&mut self) -> Result<()> {
            self.pings.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if self.hangs {
                std::future::pending::<()>().await;
            }
            match self.answers {
                true => Ok(()),
                false => Err(Error::NotConnected("the second connection".into())),
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

    /// Puts a background driver into the state and gives back the count of
    /// its pings, together with the session that holds it.
    async fn background_stub(
        state: &AppState,
        answers: bool,
    ) -> (Arc<Session>, Arc<std::sync::atomic::AtomicUsize>) {
        ping_stub(state, answers, false).await
    }

    /// Puts a background driver into the state, with the choice of a ping
    /// that never ends.
    async fn ping_stub(
        state: &AppState,
        answers: bool,
        hangs: bool,
    ) -> (Arc<Session>, Arc<std::sync::atomic::AtomicUsize>) {
        let pings = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let session = state
            .set_background_driver(
                "s1",
                BackgroundRole::Catalog,
                Box::new(PingDriver {
                    pings: pings.clone(),
                    answers,
                    hangs,
                }),
            )
            .await;
        (session, pings)
    }

    #[tokio::test]
    async fn a_background_driver_that_stood_idle_answers_a_ping_first() {
        let (_dir, descriptor) = temp_sqlite();
        let (_app, state) = state_with_sqlite(descriptor).await;
        let open = state.connection("s1").await.unwrap();
        let (session, pings) = background_stub(&state, true).await;

        // A driver that answered a moment ago goes out without a ping.
        let fresh = background_session(&state, "s1", &open, BackgroundRole::Catalog, later())
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&fresh, &session));
        assert_eq!(pings.load(std::sync::atomic::Ordering::SeqCst), 0);

        session.age(crate::state::HEALTH_CHECK_AFTER).await;
        let checked = background_session(&state, "s1", &open, BackgroundRole::Catalog, later())
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&checked, &session));
        assert_eq!(pings.load(std::sync::atomic::Ordering::SeqCst), 1);

        // The ping moved the moment of the last answer, so the next read
        // sends no second ping.
        background_session(&state, "s1", &open, BackgroundRole::Catalog, later())
            .await
            .unwrap();
        assert_eq!(pings.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_background_driver_that_stopped_answering_opens_again() {
        let (_dir, descriptor) = temp_sqlite();
        let (_app, state) = state_with_sqlite(descriptor).await;
        let open = state.connection("s1").await.unwrap();
        let (session, pings) = background_stub(&state, false).await;
        session.age(crate::state::HEALTH_CHECK_AFTER).await;

        let opened = background_session(&state, "s1", &open, BackgroundRole::Catalog, later())
            .await
            .unwrap();

        assert_eq!(pings.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(!Arc::ptr_eq(&opened, &session));
        // The new driver reaches the database.
        opened.driver.lock().await.ping().await.unwrap();
    }

    /// A deadline that a test never reaches.
    fn later() -> tokio::time::Instant {
        tokio::time::Instant::now() + CATALOG_LIMIT
    }

    #[tokio::test(start_paused = true)]
    async fn a_ping_past_its_limit_opens_a_new_background_driver() {
        let (_dir, descriptor) = temp_sqlite();
        let (_app, state) = state_with_sqlite(descriptor).await;
        let open = state.connection("s1").await.unwrap();
        let (session, pings) = ping_stub(&state, true, true).await;
        session.age(crate::state::HEALTH_CHECK_AFTER).await;

        let opened = background_session(&state, "s1", &open, BackgroundRole::Catalog, later())
            .await
            .unwrap();

        assert_eq!(pings.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(!Arc::ptr_eq(&opened, &session));
        let kept = state
            .background_session("s1", BackgroundRole::Catalog)
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&kept, &opened));
    }

    #[tokio::test(start_paused = true)]
    async fn a_health_check_waits_for_a_busy_driver_until_the_deadline() {
        let (session, pings) = {
            let (_dir, descriptor) = temp_sqlite();
            let (_app, state) = state_with_sqlite(descriptor).await;
            background_stub(&state, true).await
        };
        session.age(crate::state::HEALTH_CHECK_AFTER).await;
        // Another read keeps the driver.
        let _reading = session.driver.lock().await;

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
        let outcome = background_answers(&session, deadline).await;

        assert!(matches!(outcome, Err(Error::Timeout(60))));
        assert_eq!(pings.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn work_past_the_deadline_gives_the_timeout_of_the_catalog() {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
        let outcome = before_deadline(deadline, std::future::pending::<()>()).await;
        assert!(matches!(outcome, Err(Error::Timeout(60))));
        assert_eq!(before_deadline(later(), async { 7 }).await.unwrap(), 7);
    }

    #[tokio::test]
    async fn a_read_of_the_tree_does_not_wait_behind_a_read_of_the_schema() {
        let (_dir, descriptor) = temp_sqlite();
        let (app, state) = state_with_sqlite(descriptor).await;
        let open = state.connection("s1").await.unwrap();
        let snapshot = background_session(&state, "s1", &open, BackgroundRole::Snapshot, later())
            .await
            .unwrap();
        // The read of the schema keeps its driver for the whole test.
        let _reading = snapshot.driver.lock().await;

        let read = metadata_read(app.handle(), &state, "s1").await.unwrap();
        assert!(!Arc::ptr_eq(&read.session, &snapshot));
        let mut guard = tokio::time::timeout(std::time::Duration::from_secs(5), read.lock())
            .await
            .expect("the tree waited behind the read of the schema")
            .unwrap();
        assert!(read.run(guard.list_tables("main", None)).await.is_ok());
    }

    #[tokio::test]
    async fn a_stop_keeps_the_session_of_a_driver_that_survives_it() {
        let (_dir, descriptor) = temp_sqlite();
        let (app, state) = state_with_sqlite(descriptor).await;
        let (open, session, key) = session_for(
            app.handle(),
            &state,
            "s1",
            Some("t1"),
            &CancellationToken::new(),
        )
        .await
        .unwrap();

        // SQLite aborts a statement cleanly, so the session stays.
        let outcome: Bounded<()> = Bounded::Stopped(Error::Cancelled);
        let result = finish_run(&state, "s1", &open, &key, &session, outcome).await;
        assert!(result.is_err());
        let kept = open.sessions.get("t1").await.unwrap();
        assert!(Arc::ptr_eq(&session, &kept));
    }

    /// A handle of a stop that counts its calls, and fails each one when it
    /// is told to.
    struct CountingCancel {
        calls: Arc<std::sync::atomic::AtomicUsize>,
        fails: bool,
    }

    #[async_trait::async_trait]
    impl crate::db::drivers::CancelHandle for CountingCancel {
        async fn cancel(&self) -> Result<()> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            match self.fails {
                true => Err(Error::Connection("the stop failed".into())),
                false => Ok(()),
            }
        }
    }

    /// A driver for the tests of the limit of the catalog reads. It gives
    /// the handle of a stop when it has one.
    struct CatalogDriver {
        cancel: Option<Arc<dyn crate::db::drivers::CancelHandle>>,
    }

    #[async_trait::async_trait]
    impl DatabaseDriver for CatalogDriver {
        fn capabilities(&self) -> crate::db::DriverCapabilities {
            crate::db::DriverCapabilities::default()
        }
        fn dialect(&self) -> Dialect {
            Dialect::Sqlite
        }
        fn cancel_handle(&self) -> Option<Arc<dyn crate::db::drivers::CancelHandle>> {
            self.cancel.clone()
        }
        async fn ping(&mut self) -> Result<()> {
            Ok(())
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

    /// A driver that keeps the lock limit it was given, and refuses the
    /// limit when it is told to.
    struct LockLimitDriver {
        asked: Arc<std::sync::Mutex<Option<std::time::Duration>>>,
        refuses: bool,
    }

    #[async_trait::async_trait]
    impl DatabaseDriver for LockLimitDriver {
        fn capabilities(&self) -> crate::db::DriverCapabilities {
            crate::db::DriverCapabilities::default()
        }
        fn dialect(&self) -> Dialect {
            Dialect::MsSql
        }
        async fn ping(&mut self) -> Result<()> {
            Ok(())
        }
        async fn limit_lock_waits(&mut self, limit: std::time::Duration) -> Result<()> {
            *self.asked.lock().unwrap() = Some(limit);
            match self.refuses {
                true => Err(Error::Unsupported("no lock limit".into())),
                false => Ok(()),
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

    #[tokio::test]
    async fn a_new_background_driver_gets_the_lock_limit_and_a_refusal_is_no_failure() {
        for refuses in [false, true] {
            let asked = Arc::new(std::sync::Mutex::new(None));
            let mut driver = LockLimitDriver {
                asked: asked.clone(),
                refuses,
            };
            limit_lock_waits(&mut driver, "s1").await;
            assert_eq!(*asked.lock().unwrap(), Some(LOCK_WAIT_LIMIT));
        }
    }

    #[tokio::test]
    async fn a_driver_without_locks_of_a_session_accepts_the_lock_limit() {
        let (mut driver, _calls) = catalog_driver(None);
        assert!(driver.limit_lock_waits(LOCK_WAIT_LIMIT).await.is_ok());
    }

    #[tokio::test]
    async fn a_lock_wait_in_a_catalog_read_names_the_lock() {
        let (_dir, descriptor) = temp_sqlite();
        let (_app, state) = state_with_sqlite(descriptor).await;
        let (driver, _calls) = catalog_driver(None);
        let read = CatalogRead::new(&state, "s1", Arc::new(Session::new(driver)), CATALOG_LIMIT);

        let outcome: Result<()> = read
            .run(async {
                Err(Error::MySql(mysql_async::Error::Server(
                    mysql_async::ServerError {
                        code: 1205,
                        state: "HY000".to_string(),
                        message: "Lock wait timeout exceeded".to_string(),
                    },
                )))
            })
            .await;
        assert!(matches!(outcome, Err(Error::LockWait(_))));

        // Another error of the read stays as it is.
        let outcome: Result<()> = read.run(async { Err(Error::Cancelled) }).await;
        assert!(matches!(outcome, Err(Error::Cancelled)));
    }

    fn catalog_driver(
        fails: Option<bool>,
    ) -> (Box<dyn DatabaseDriver>, Arc<std::sync::atomic::AtomicUsize>) {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let cancel = fails.map(|fails| {
            Arc::new(CountingCancel {
                calls: calls.clone(),
                fails,
            }) as Arc<dyn crate::db::drivers::CancelHandle>
        });
        (Box::new(CatalogDriver { cancel }), calls)
    }

    fn stops(calls: &std::sync::atomic::AtomicUsize) -> usize {
        calls.load(std::sync::atomic::Ordering::SeqCst)
    }

    const SHORT: std::time::Duration = std::time::Duration::from_millis(20);

    #[tokio::test]
    async fn a_request_with_the_driver_gets_the_handle_of_its_session() {
        let state = state();
        let (driver, calls) = catalog_driver(Some(false));
        let session = Session::new(driver);
        let token = state.start_request("r1", "s1").await;

        let guard = driver_for_request(&state, "r1", &session, &token).await;

        assert!(guard.is_ok());
        let request = state.take_request("r1").await.unwrap();
        request.cancel_handle.unwrap().cancel().await.unwrap();
        assert_eq!(stops(&calls), 1);
    }

    #[tokio::test]
    async fn a_stop_during_the_wait_for_the_driver_leaves_the_other_statement() {
        let state = state();
        let (driver, calls) = catalog_driver(Some(false));
        let session = Session::new(driver);
        let _other = session.driver.lock().await;
        let token = state.start_request("r1", "s1").await;

        let waiting = driver_for_request(&state, "r1", &session, &token);
        let stop = async {
            tokio::time::sleep(SHORT).await;
            let request = state.take_request("r1").await.unwrap();
            assert!(request.cancel_handle.is_none());
            request.token.cancel();
        };
        let (outcome, ()) = tokio::join!(waiting, stop);

        assert!(matches!(outcome, Err(Error::Cancelled)));
        assert_eq!(stops(&calls), 0);
    }

    #[tokio::test]
    async fn a_request_that_a_stop_took_before_the_driver_came_does_not_run() {
        let state = state();
        let (driver, _calls) = catalog_driver(None);
        let session = Session::new(driver);
        let token = state.start_request("r1", "s1").await;
        state.take_request("r1").await.unwrap();

        let outcome = driver_for_request(&state, "r1", &session, &token).await;
        assert!(matches!(outcome, Err(Error::Cancelled)));

        let token = state.start_request("r2", "s1").await;
        token.cancel();
        let outcome = driver_for_request(&state, "r2", &session, &token).await;
        assert!(matches!(outcome, Err(Error::Cancelled)));
    }

    #[tokio::test]
    async fn a_stop_opens_no_session_for_a_tab_that_closed() {
        let (_dir, descriptor) = temp_sqlite();
        let (_app, state) = state_with_sqlite(descriptor).await;
        let open = state.connection("s1").await.unwrap();
        let (driver, _calls) = catalog_driver(None);
        let session = open.sessions.insert("t1", Session::new(driver)).await;
        open.sessions.release("t1").await;

        let outcome: Bounded<()> = Bounded::Stopped(Error::Cancelled);
        let result = finish_run(&state, "s1", &open, "t1", &session, outcome).await;

        assert!(result.is_err());
        assert!(open.sessions.get("t1").await.is_none());
    }

    #[tokio::test]
    async fn a_stop_opens_no_session_for_a_connection_that_closed() {
        let (_dir, descriptor) = temp_sqlite();
        let (_app, state) = state_with_sqlite(descriptor).await;
        let open = state.connection("s1").await.unwrap();
        let (driver, _calls) = catalog_driver(None);
        let session = open.sessions.insert("t1", Session::new(driver)).await;
        state.remove("s1").await;

        let outcome: Bounded<()> = Bounded::Stopped(Error::Cancelled);
        let result = finish_run(&state, "s1", &open, "t1", &session, outcome).await;

        assert!(result.is_err());
        assert!(state.connection("s1").await.is_err());
        let held = open.sessions.get("t1").await.unwrap();
        assert!(Arc::ptr_eq(&held, &session));
    }

    #[tokio::test]
    async fn a_stop_opens_no_session_in_a_connection_that_opened_again() {
        let (_dir, descriptor) = temp_sqlite();
        let (_app, state) = state_with_sqlite(descriptor.clone()).await;
        let open = state.connection("s1").await.unwrap();
        let (driver, _calls) = catalog_driver(None);
        let session = open.sessions.insert("t1", Session::new(driver)).await;
        let driver = open_driver(&descriptor).await.unwrap();
        state
            .insert("s1", OpenConnection::new(descriptor, driver))
            .await;

        let outcome: Bounded<()> = Bounded::Stopped(Error::Cancelled);
        let result = finish_run(&state, "s1", &open, "t1", &session, outcome).await;

        assert!(result.is_err());
        let current = state.connection("s1").await.unwrap();
        assert!(current.sessions.get("t1").await.is_none());
    }

    /// A handle whose stop never ends, as on a server that does not answer.
    struct SilentCancel;

    #[async_trait::async_trait]
    impl crate::db::drivers::CancelHandle for SilentCancel {
        async fn cancel(&self) -> Result<()> {
            std::future::pending().await
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_stops_of_a_connection_run_together_under_one_limit() {
        let state = state();
        let mut tokens = Vec::new();
        for id in ["r1", "r2", "r3"] {
            tokens.push(state.start_request(id, "c1").await);
            state.arm_request(id, Some(Arc::new(SilentCancel))).await;
        }

        let started = tokio::time::Instant::now();
        stop_requests(state.take_requests_of("c1").await).await;

        assert_eq!(started.elapsed(), STOP_GRACE);
        assert!(tokens.iter().all(CancellationToken::is_cancelled));
    }

    #[tokio::test]
    async fn a_request_does_not_send_on_a_session_that_a_stop_closed() {
        let state = state();
        let (driver, _calls) = catalog_driver(None);
        let session = Session::new(driver);
        session.mark_broken();
        let token = state.start_request("r1", "s1").await;
        let outcome = driver_for_request(&state, "r1", &session, &token).await;
        assert!(matches!(outcome, Err(Error::Connection(_))));
    }

    #[tokio::test]
    async fn a_catalog_read_does_not_send_on_a_session_that_a_limit_closed() {
        let (_dir, descriptor) = temp_sqlite();
        let (_app, state) = state_with_sqlite(descriptor).await;
        let (driver, _calls) = catalog_driver(None);
        let session = Arc::new(Session::new(driver));
        session.mark_broken();
        let read = CatalogRead::new(&state, "s1", session, SHORT);
        assert!(matches!(read.lock().await, Err(Error::Connection(_))));
    }

    #[tokio::test]
    async fn reads_that_start_together_open_one_background_driver() {
        let (_dir, descriptor) = temp_sqlite();
        let (_app, state) = state_with_sqlite(descriptor).await;
        let open = state.connection("s1").await.unwrap();

        // The first read keeps the lock of the open, as during a slow
        // connect, so the second read waits for it.
        let opening = state.background_open_lock("s1", BackgroundRole::Catalog);
        let held = opening.lock().await;
        let waiting = background_session(&state, "s1", &open, BackgroundRole::Catalog, later());
        tokio::pin!(waiting);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), &mut waiting)
                .await
                .is_err()
        );
        let (first, _) = background_stub(&state, true).await;
        drop(held);

        let second = waiting.await.unwrap();
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[tokio::test]
    async fn a_background_driver_of_a_closed_connection_stays_out_of_the_slot() {
        let (_dir, descriptor) = temp_sqlite();
        let (_app, state) = state_with_sqlite(descriptor).await;
        let open = state.connection("s1").await.unwrap();
        state.remove("s1").await;

        let session = background_session(&state, "s1", &open, BackgroundRole::Catalog, later())
            .await
            .unwrap();

        session.driver.lock().await.ping().await.unwrap();
        assert!(state
            .background_session("s1", BackgroundRole::Catalog)
            .await
            .is_none());
    }

    #[tokio::test]
    async fn the_stop_of_a_connection_stops_each_of_its_statements() {
        let state = state();
        let (driver, calls) = catalog_driver(Some(true));
        let session = Session::new(driver);
        let armed = state.start_request("r1", "c1").await;
        state.arm_request("r1", session.cancel_handle.clone()).await;
        let waiting = state.start_request("r2", "c1").await;
        let other = state.start_request("r3", "c2").await;

        stop_requests(state.take_requests_of("c1").await).await;

        assert!(armed.is_cancelled());
        assert!(waiting.is_cancelled());
        assert!(!other.is_cancelled());
        assert_eq!(stops(&calls), 1);
    }

    #[tokio::test]
    async fn a_catalog_read_that_answers_in_time_gives_its_answer() {
        let (_dir, descriptor) = temp_sqlite();
        let (_app, state) = state_with_sqlite(descriptor).await;
        let (driver, _calls) = catalog_driver(None);
        let read = CatalogRead::new(&state, "s1", Arc::new(Session::new(driver)), SHORT);
        let mut guard = read.lock().await.unwrap();
        assert!(read.run(guard.list_databases()).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_catalog_read_past_its_limit_stops_and_closes_the_background_session() {
        let (_dir, descriptor) = temp_sqlite();
        let (_app, state) = state_with_sqlite(descriptor).await;
        let (driver, calls) = catalog_driver(Some(false));
        let background = state
            .set_background_driver("s1", BackgroundRole::Catalog, driver)
            .await;

        let read = CatalogRead::new(&state, "s1", background, SHORT);
        let _guard = read.lock().await.unwrap();
        let outcome: Result<()> = read.run(std::future::pending()).await;

        assert!(matches!(outcome, Err(Error::Timeout(0))));
        assert_eq!(stops(&calls), 1);
        assert!(state
            .background_session("s1", BackgroundRole::Catalog)
            .await
            .is_none());
    }

    #[tokio::test]
    async fn a_read_of_the_schema_past_its_limit_leaves_the_driver_of_the_tree() {
        let (_dir, descriptor) = temp_sqlite();
        let (_app, state) = state_with_sqlite(descriptor).await;
        let (_catalog, _pings) = background_stub(&state, true).await;
        let (driver, calls) = catalog_driver(Some(false));
        let snapshot = state
            .set_background_driver("s1", BackgroundRole::Snapshot, driver)
            .await;

        let read = CatalogRead::new(&state, "s1", snapshot, SHORT);
        let outcome: Result<()> = read.run(std::future::pending()).await;

        assert!(matches!(outcome, Err(Error::Timeout(0))));
        assert_eq!(stops(&calls), 1);
        assert!(state
            .background_session("s1", BackgroundRole::Snapshot)
            .await
            .is_none());
        assert!(state
            .background_session("s1", BackgroundRole::Catalog)
            .await
            .is_some());
    }

    #[tokio::test]
    async fn a_catalog_read_past_its_limit_closes_the_default_session_it_ran_on() {
        let (_dir, descriptor) = temp_sqlite();
        let (_app, state) = state_with_sqlite(descriptor).await;
        let open = state.connection("s1").await.unwrap();
        // A stop that fails still lets the session go.
        let (driver, calls) = catalog_driver(Some(true));
        let default = open
            .sessions
            .insert(DEFAULT_SESSION, Session::new(driver))
            .await;

        let read = CatalogRead::new(&state, "s1", default, SHORT);
        let outcome: Result<()> = read.run(std::future::pending()).await;

        assert!(outcome.is_err());
        assert_eq!(stops(&calls), 1);
        assert!(open.default_session().await.is_err());
    }

    #[tokio::test]
    async fn a_catalog_read_past_its_limit_keeps_the_only_session_of_a_database_in_memory() {
        let descriptor = sqlite_connection(":memory:");
        let (_app, state) = state_with_sqlite(descriptor).await;
        let open = state.connection("s1").await.unwrap();
        // A driver that does not keep its connection after a stop sits in
        // the default slot.
        let (driver, calls) = catalog_driver(Some(false));
        let default = open
            .sessions
            .insert(DEFAULT_SESSION, Session::new(driver))
            .await;

        let read = CatalogRead::new(&state, "s1", default.clone(), SHORT);
        let outcome: Result<()> = read.run(std::future::pending()).await;

        assert!(matches!(outcome, Err(Error::Timeout(0))));
        assert_eq!(stops(&calls), 1);
        assert!(Arc::ptr_eq(
            &open.default_session().await.unwrap(),
            &default
        ));
    }

    #[tokio::test]
    async fn a_catalog_read_past_its_limit_leaves_the_sessions_it_did_not_run_on() {
        let (_dir, descriptor) = temp_sqlite();
        let (_app, state) = state_with_sqlite(descriptor).await;
        let open = state.connection("s1").await.unwrap();
        let (background, _pings) = background_stub(&state, true).await;
        let default = open.default_session().await.unwrap();

        // The session of the read stands in neither slot, for example because
        // a health check put a new session in its place.
        let (driver, _calls) = catalog_driver(None);
        let read = CatalogRead::new(&state, "s1", Arc::new(Session::new(driver)), SHORT);
        let outcome: Result<()> = read.run(std::future::pending()).await;

        assert!(outcome.is_err());
        let kept = state
            .background_session("s1", BackgroundRole::Catalog)
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&kept, &background));
        assert!(Arc::ptr_eq(
            &open.default_session().await.unwrap(),
            &default
        ));

        // A connection that closed during the read leaves nothing to drop.
        state.remove("s1").await;
        let (driver, _calls) = catalog_driver(None);
        let read = CatalogRead::new(&state, "s1", Arc::new(Session::new(driver)), SHORT);
        assert!(read
            .run(std::future::pending::<Result<()>>())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn a_catalog_read_past_its_limit_keeps_a_session_that_survives_a_stop() {
        let (_dir, descriptor) = temp_sqlite();
        let (_app, state) = state_with_sqlite(descriptor).await;
        let open = state.connection("s1").await.unwrap();
        // SQLite aborts a statement cleanly, so the session stays.
        let default = open.default_session().await.unwrap();
        let read = CatalogRead::new(&state, "s1", default.clone(), SHORT);
        let outcome: Result<()> = read.run(std::future::pending()).await;

        assert!(outcome.is_err());
        assert!(Arc::ptr_eq(
            &open.default_session().await.unwrap(),
            &default
        ));
    }

    #[tokio::test]
    async fn a_catalog_read_that_waits_for_the_driver_past_its_limit_keeps_the_session() {
        let (_dir, descriptor) = temp_sqlite();
        let (_app, state) = state_with_sqlite(descriptor).await;
        let (driver, calls) = catalog_driver(Some(false));
        let background = state
            .set_background_driver("s1", BackgroundRole::Catalog, driver)
            .await;
        let _other = background.driver.lock().await;

        let read = CatalogRead::new(&state, "s1", background.clone(), SHORT);
        assert!(matches!(read.lock().await, Err(Error::Timeout(0))));

        assert_eq!(stops(&calls), 0);
        assert!(state
            .background_session("s1", BackgroundRole::Catalog)
            .await
            .is_some());
    }

    /// A kept result of the connection `s1` with a fixed list of rows.
    fn kept_rows(rows: usize) -> crate::kept::KeptResult {
        crate::kept::KeptResult::new("s1", crate::kept::tests::fixed(rows))
    }

    fn export_options(max_rows: usize, timeout_secs: u64) -> ExecOptions {
        ExecOptions {
            max_rows,
            timeout_secs,
            one_statement: true,
        }
    }

    #[tokio::test]
    async fn a_kept_result_writes_its_rows_to_the_file() {
        let state = AppState::new(Arc::new(MemoryStore::default()));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("all.csv");
        let summary = write_kept(
            &state,
            "e1",
            &kept_rows(3),
            &path,
            ExportFormat::Csv,
            &export_options(2, 30),
        )
        .await
        .unwrap();
        assert_eq!(summary.rows, 2);
        assert!(summary.truncated);
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, format!("{CSV_BOM}n\r\n0\r\n1\r\n"));
        // The request of the export ended with the export.
        assert!(state.take_request("e1").await.is_none());
    }

    #[tokio::test]
    async fn the_stop_button_ends_the_read_of_a_kept_result() {
        let state = Arc::new(AppState::new(Arc::new(MemoryStore::default())));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("all.json");
        let kept = crate::kept::KeptResult::new("s1", crate::kept::KeptSource::Pending);
        let stopper = state.clone();
        let options = export_options(10, 0);
        let (outcome, ()) = tokio::join!(
            write_kept(&state, "e1", &kept, &path, ExportFormat::Json, &options),
            async move {
                loop {
                    if let Some(request) = stopper.take_request("e1").await {
                        stop_requests(vec![request]).await;
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            }
        );
        assert!(matches!(outcome, Err(Error::Cancelled)));
        // The file never takes its name, and the writer thread removes the
        // temporary part.
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn the_time_limit_ends_the_read_of_a_kept_result() {
        let state = AppState::new(Arc::new(MemoryStore::default()));
        let dir = tempfile::tempdir().unwrap();
        let kept = crate::kept::KeptResult::new("s1", crate::kept::KeptSource::Pending);
        let outcome = write_kept(
            &state,
            "e1",
            &kept,
            &dir.path().join("all.xlsx"),
            ExportFormat::Xlsx,
            &export_options(10, 1),
        )
        .await;
        assert!(matches!(outcome, Err(Error::Timeout(1))));
        assert!(state.take_request("e1").await.is_none());
    }

    #[tokio::test]
    async fn a_kept_result_without_a_set_writes_no_file() {
        let state = AppState::new(Arc::new(MemoryStore::default()));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("all.csv");
        let kept = crate::kept::KeptResult::new("s1", crate::kept::KeptSource::Empty);
        let outcome = write_kept(
            &state,
            "e1",
            &kept,
            &path,
            ExportFormat::Csv,
            &export_options(10, 30),
        )
        .await;
        assert!(matches!(outcome, Err(Error::Unsupported(_))));
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn an_export_of_a_result_that_is_gone_asks_for_a_new_run() {
        use tauri::Manager;
        let (_dir, descriptor) = temp_sqlite();
        let (app, state) = state_with_sqlite(descriptor).await;
        app.manage(state);
        let request = |kept_id: &str| KeptExportRequest {
            kept_id: kept_id.into(),
            request_id: "e1".into(),
            default_name: "all.csv".into(),
            format: ExportFormat::Csv,
            max_rows: 10,
        };

        let error = export_kept(app.handle().clone(), request("r1:0"), app.state())
            .await
            .unwrap_err();
        assert!(matches!(&error, Error::Invalid(text) if text.contains("Run the query again")));

        // A result of a connection that closed cannot be read.
        let kept = app.state::<AppState>().kept.keep(
            "r1",
            "closed",
            vec![(0, crate::kept::tests::fixed(1))],
        );
        let error = export_kept(app.handle().clone(), request(&kept[0].id), app.state())
            .await
            .unwrap_err();
        assert!(matches!(error, Error::NotConnected(_)));

        release_kept(kept[0].id.clone(), app.state()).await.unwrap();
        assert_eq!(app.state::<AppState>().kept.len(), 0);
        // A second release of the same result is not an error.
        release_kept(kept[0].id.clone(), app.state()).await.unwrap();
    }

    #[test]
    fn each_export_format_names_its_file_type() {
        assert_eq!(ExportFormat::Csv.file_type(), ("CSV", "csv"));
        assert_eq!(ExportFormat::Json.file_type(), ("JSON", "json"));
        assert_eq!(ExportFormat::Xlsx.file_type(), ("Excel", "xlsx"));
    }
}
