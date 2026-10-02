//! The commands the user interface calls.

use crate::db::columnar::ChunkSink;
use crate::db::drivers::{
    athena::AthenaDriver, mssql, mssql::MssqlDriver, mysql, mysql::MysqlDriver, postgres,
    postgres::PostgresDriver, sqlite::SqliteDriver,
};
use crate::db::{
    self, drivers::DatabaseDriver, AppColumn, Constraint, Database, ExecOptions, IndexInfo,
    ObjectType, PartitionList, PlanMode, QueryParams, QueryResponse, RelationType, Routine,
    ScheduledEvent, Schema, SchemaSnapshot, Table, TableDetails, Trigger,
};
use crate::error::{Error, Result};
use crate::files;
use crate::history::{HistoryEntry, SavedQuery};
use crate::script::{self, ScriptStatement};
use crate::secrets;
use crate::session::{Session, DEFAULT_SESSION};
use crate::sql::ParamValues;
use crate::state::{
    AppState, ConnectionHealth, ConnectionInfo, ConnectionStatusEvent, OpenConnection,
    CONNECTION_STATUS_EVENT,
};
use crate::storage::{DbType, SavedConnection};
use crate::store;
use std::sync::Arc;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Emitter, Runtime};
use tokio_util::sync::CancellationToken;

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

/// Fills the secrets of a record from the secret store, unless the caller
/// already gave them. A connection holds a password and, for Athena, a
/// secret access key and a session token.
fn with_secrets(state: &AppState, mut connection: SavedConnection) -> Result<SavedConnection> {
    if connection.password.is_none() {
        connection.password = state.secrets.get(&connection.id)?;
    }
    if connection.aws_secret_access_key.is_none() {
        connection.aws_secret_access_key = state
            .secrets
            .get(&secrets::aws_secret_key(&connection.id))?;
    }
    if connection.aws_session_token.is_none() {
        connection.aws_session_token =
            state.secrets.get(&secrets::aws_token_key(&connection.id))?;
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
fn store_secret(state: &AppState, key: &str, value: Option<&str>) -> Result<bool> {
    match value {
        Some(text) if !text.is_empty() => {
            state.secrets.set(key, text)?;
            Ok(true)
        }
        Some(_) => {
            state.secrets.delete(key)?;
            Ok(false)
        }
        None => Ok(state.secrets.get(key)?.is_some()),
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
            "Remove the password from the connection string and type it in the Password box. \
             The settings file keeps the connection string as plain text, and the keychain \
             keeps the password."
                .to_string(),
        ));
    }
    Ok(())
}

/// The saved record of one connection, out of the file of connections.
fn saved_record<R: Runtime>(app: &AppHandle<R>, id: &str) -> Result<Option<SavedConnection>> {
    Ok(store::read_connections(app)?
        .into_iter()
        .find(|record| record.id == id))
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
    let Some(record) = saved_record(&app, &id)? else {
        return Err(Error::Configuration(format!(
            "No saved connection carries the identifier '{id}'."
        )));
    };
    let full = with_secrets(&state, record)?;

    match open_driver(&full).await {
        Ok(driver) => {
            let info = state.insert(&id, OpenConnection::new(full, driver)).await;
            announce(&app, &id, ConnectionHealth::Connected, None);
            log::info!("The connection '{id}' is open.");
            Ok(info)
        }
        Err(error) => {
            announce(
                &app,
                &id,
                ConnectionHealth::Disconnected,
                Some(error.to_string()),
            );
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
fn with_secrets_for_test<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    connection: SavedConnection,
) -> Result<SavedConnection> {
    let Some(saved) = saved_record(app, &connection.id)? else {
        return Ok(connection);
    };
    if saved.without_secrets() == connection.without_secrets() {
        return with_secrets(state, connection);
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
    for (absent, key, name) in held {
        if absent && state.secrets.get(&key)?.is_some() {
            return Err(Error::Configuration(format!(
                "This record differs from the connection that is saved, so the stored {name} does not belong to it. Type the {name} to test the change."
            )));
        }
    }
    Ok(connection)
}

/// Opens a connection, confirms that it answers, and closes it again.
#[tauri::command]
pub async fn test_connection<R: Runtime>(
    app: AppHandle<R>,
    connection: SavedConnection,
    state: tauri::State<'_, AppState>,
) -> Result<String> {
    let full = with_secrets_for_test(&app, &state, connection)?;
    let mut driver = open_driver(&full).await?;
    driver.ping().await?;
    Ok("The connection works.".to_string())
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
async fn session_for<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    connection_id: &str,
    tab_id: Option<&str>,
) -> Result<(OpenConnection, Arc<Session>, String)> {
    let open = state.connection(connection_id).await?;
    let key = open.session_key(tab_id);

    if let Some(session) = open.sessions.get(&key).await {
        let session =
            ensure_session_healthy(app, state, connection_id, &open, &key, session).await?;
        return Ok((open, session, key));
    }

    // One session opens at a time, so the count against the cap stays exact
    // and two requests of one tab open one session, not two.
    let pool = open.sessions.clone();
    let _opening = pool.begin_open().await;
    if let Some(session) = pool.get(&key).await {
        return Ok((open, session, key));
    }
    if key != DEFAULT_SESSION && pool.at_cap().await {
        pool.reap_idle().await;
    }
    if key != DEFAULT_SESSION && pool.at_cap().await {
        return Err(Error::Configuration(format!(
            "This connection already uses {} sessions. Close a tab, or raise the session \
             limit in the connection options.",
            pool.cap()
        )));
    }

    let full = with_secrets(state, open.descriptor.clone())?;
    let driver = open_driver(&full).await?;
    let session = pool.insert(&key, Session::new(driver)).await;
    Ok((open, session, key))
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

    let healthy = {
        let mut driver = session.driver.lock().await;
        driver.ping().await.is_ok()
    };
    if healthy {
        session.mark_ok().await;
        return Ok(session);
    }

    announce(app, connection_id, ConnectionHealth::Reconnecting, None);
    log::warn!("A session of '{connection_id}' stopped answering. Opening it again.");

    let full = with_secrets(state, open.descriptor.clone())?;
    match open_driver(&full).await {
        Ok(driver) => {
            let replacement = open.sessions.insert(key, Session::new(driver)).await;
            // The background driver shares the fate of the session that
            // stopped answering, so the next metadata read opens a new one.
            state.clear_background(connection_id).await;
            announce(app, connection_id, ConnectionHealth::Connected, None);
            Ok(replacement)
        }
        Err(error) => {
            open.sessions.release(key).await;
            if open.sessions.is_empty().await {
                state.remove(connection_id).await;
            }
            announce(
                app,
                connection_id,
                ConnectionHealth::Disconnected,
                Some(error.to_string()),
            );
            Err(error)
        }
    }
}

/// Confirms that the default session of a connection still answers. The
/// commands that read metadata call this before they lend a driver out.
async fn ensure_healthy<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    connection_id: &str,
) -> Result<OpenConnection> {
    let (open, _session, _key) = session_for(app, state, connection_id, None).await?;
    Ok(open)
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
    let session = background_session(state, connection_id, &open).await?;
    Ok(CatalogRead::new(
        state,
        connection_id,
        session,
        CATALOG_LIMIT,
    ))
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
        Self {
            state,
            connection_id,
            session,
            limit,
            deadline: tokio::time::Instant::now() + limit,
        }
    }

    fn timeout(&self) -> Error {
        Error::Timeout(self.limit.as_secs())
    }

    /// Takes the driver of the session. When another exchange keeps the
    /// driver until the deadline, the read fails and the session stays,
    /// because that exchange has a limit of its own.
    async fn lock(&self) -> Result<tokio::sync::MutexGuard<'_, Box<dyn DatabaseDriver>>> {
        tokio::time::timeout_at(self.deadline, self.session.driver.lock())
            .await
            .map_err(|_| self.timeout())
    }

    /// Runs the read until the deadline. A read that passes the deadline is
    /// dropped in the middle of an exchange, so the server is asked to stop
    /// the statement and the session goes. The next read then opens a new
    /// session.
    async fn run<T>(&self, read: impl std::future::Future<Output = Result<T>>) -> Result<T> {
        match tokio::time::timeout_at(self.deadline, read).await {
            Ok(result) => result,
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
        if let Some(background) = self.state.background_session(self.connection_id).await {
            if Arc::ptr_eq(&background, &self.session) {
                self.state.clear_background(self.connection_id).await;
                return;
            }
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
pub async fn run_bounded<T, F>(
    work: F,
    token: &CancellationToken,
    timeout_secs: u64,
    grace: std::time::Duration,
) -> Bounded<T>
where
    F: std::future::Future<Output = Result<T>>,
{
    tokio::select! {
        result = work => Bounded::Answered(result),
        () = stopped_by_the_user(token, grace) => Bounded::Stopped(Error::Cancelled),
        () = until_the_limit(timeout_secs) => Bounded::Stopped(Error::Timeout(timeout_secs)),
    }
}

/// Reports what the interface shows when a limit ended an exchange.
fn limit_reason(error: &Error) -> String {
    match error {
        Error::Timeout(seconds) => format!(
            "The statement passed the limit of {seconds} seconds, so the connection was closed."
        ),
        _ => "The statement was stopped, so the connection was closed.".to_string(),
    }
}

/// Puts the values of the named parameters into one statement.
///
/// The text keeps the placeholders of the dialect and the values travel bound,
/// so a value never becomes part of the statement. Athena binds no value, so
/// its parameters reach the service as literals of SQL.
///
/// A statement that holds no name is left as it stands and carries no
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

/// The message for a parameter that the statement names and the request left
/// out.
fn missing_parameter(name: &str) -> Error {
    Error::Configuration(format!("The parameter ':{name}' has no value."))
}

/// Lists the names of the parameters of a statement. The interface asks for a
/// value for each name before it runs the statement.
#[tauri::command]
pub fn query_parameters(query: String, dialect: crate::sql::Dialect) -> Vec<String> {
    crate::sql::find_parameters(&query, dialect)
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
/// The rows travel on the channel while the read runs, so neither side holds
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
            session_for(&app, &state, &connection_id, tab_id.as_deref()).await?;
        let options = options.unwrap_or_else(|| open.descriptor.exec_options());
        let (query, bound) = prepare_parameters(&query, open.dialect, query_params.as_ref())?;
        Ok::<_, Error>((open, session, key, options, query, bound))
    }
    .await;
    let (open, session, key, options, query, bound) = match prepared {
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
                guard.execute_stream(&query, bound.as_ref(), &options, &mut sink),
                &token,
                options.timeout_secs,
                stop_grace(&session),
            )
            .await
        }
        Err(error) => Bounded::Answered(Err(error)),
    };

    state.end_request(&request_id).await;
    match finish_run(&app, &state, &connection_id, &open, &key, &session, outcome).await {
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
            session_for(&app, &state, &connection_id, tab_id.as_deref()).await?;
        let options = options.unwrap_or_else(|| open.descriptor.exec_options());
        // A plan needs the values of the parameters, because the plan of a
        // statement depends on the values it holds.
        let (query, bound) = prepare_parameters(&query, open.dialect, query_params.as_ref())?;
        Ok::<_, Error>((open, session, key, options, query, bound))
    }
    .await;
    let (open, session, key, options, query, bound) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            state.end_request(&request_id).await;
            return Err(error);
        }
    };

    let outcome = match driver_for_request(&state, &request_id, &session, &token).await {
        Ok(mut guard) => {
            run_bounded(
                guard.explain(&query, bound.as_ref(), mode, &options),
                &token,
                options.timeout_secs,
                stop_grace(&session),
            )
            .await
        }
        Err(error) => Bounded::Answered(Err(error)),
    };

    state.end_request(&request_id).await;
    finish_run(&app, &state, &connection_id, &open, &key, &session, outcome).await
}

/// Closes the accounts of one exchange. A limit that ended the exchange asks
/// the server to stop the statement, and the session then goes unless the
/// driver reports that it stays fit for use.
async fn finish_run<R: Runtime, T>(
    app: &AppHandle<R>,
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
            // The wait ended, but the server may still run the statement.
            // The handle asks the server to stop it. A second request for a
            // statement that already stopped does no harm.
            if let Some(handle) = session.cancel_handle.clone() {
                if let Err(stop_error) = handle.cancel().await {
                    log::warn!("The server did not stop the statement: {stop_error}");
                }
            }
            if session.keeps_connection_after_stop {
                return Err(error);
            }
            // The exchange was dropped in the middle of a message, so nothing
            // can be sent on this session again. A new one goes in its place
            // at once, so the user is not left with a tab that cannot run
            // anything. The other sessions of the connection stay as they
            // are, because the server itself is healthy.
            if still_in_use(state, connection_id, open, session_key, session).await {
                reopen_after_stop(app, state, connection_id, open, session_key, &error).await;
            }
            Err(error)
        }
    }
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
    let Ok(current) = state.connection(connection_id).await else {
        return false;
    };
    if !Arc::ptr_eq(&current.sessions, &open.sessions) {
        return false;
    }
    match open.sessions.get(session_key).await {
        Some(held) => Arc::ptr_eq(&held, session),
        None => false,
    }
}

/// Puts a new session in the place of one that a limit left unusable.
///
/// The session is a new one. Whatever the old session held, such as a
/// temporary table, an open transaction or a `SET` of its own, is gone with
/// it. The alternative is a tab that can run nothing until the user opens the
/// connection by hand.
async fn reopen_after_stop<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    connection_id: &str,
    open: &OpenConnection,
    session_key: &str,
    error: &Error,
) {
    announce(app, connection_id, ConnectionHealth::Reconnecting, None);

    let full = match with_secrets(state, open.descriptor.clone()) {
        Ok(full) => full,
        Err(secret_error) => {
            log::warn!("The password of '{connection_id}' could not be read: {secret_error}");
            state.remove(connection_id).await;
            announce(
                app,
                connection_id,
                ConnectionHealth::Disconnected,
                Some(limit_reason(error)),
            );
            return;
        }
    };

    match open_driver(&full).await {
        Ok(driver) => {
            open.sessions
                .insert(session_key, Session::new(driver))
                .await;
            announce(app, connection_id, ConnectionHealth::Connected, None);
            log::info!("A session of '{connection_id}' was opened again after a stop.");
        }
        Err(open_error) => {
            open.sessions.release(session_key).await;
            if open.sessions.is_empty().await {
                state.remove(connection_id).await;
            }
            announce(
                app,
                connection_id,
                ConnectionHealth::Disconnected,
                Some(open_error.to_string()),
            );
        }
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
    for request in requests {
        // The handle does not need the lock of the driver, so it works
        // while the statement runs.
        if let Some(handle) = request.cancel_handle {
            if let Err(error) = handle.cancel().await {
                log::warn!("The server did not stop the statement: {error}");
            }
        }
        request.token.cancel();
    }
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

/// Reads every relation and every column of one database, for the
/// completions of the editor.
///
/// The read runs on a second driver of the same record, so that it never
/// waits behind a statement of the user and no statement of the user waits
/// behind it. A caller that asks for the one session, and a second driver
/// that cannot open, put the read on the session of the user instead.
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

    let session = match request.own_connection.unwrap_or(true) {
        true => background_session(&state, &request.connection_id, &open).await?,
        false => open.default_session().await?,
    };

    let read = CatalogRead::new(&state, &request.connection_id, session, CATALOG_LIMIT);
    let mut guard = read.lock().await?;
    read.run(guard.schema_snapshot(&request.database, limit))
        .await
}

/// Confirms that a background driver that stood idle still answers. A driver
/// that gives no answer must go, because the server closed its side.
async fn background_answers(session: &Arc<Session>) -> bool {
    if !session.needs_ping || !session.needs_check().await {
        return true;
    }
    // One check at a time for each driver, so two reads send one ping.
    let _guard = session.health.lock().await;
    if !session.needs_check().await {
        return true;
    }
    let healthy = {
        let mut driver = session.driver.lock().await;
        driver.ping().await.is_ok()
    };
    if healthy {
        session.mark_ok().await;
    }
    healthy
}

/// Returns the background session of a connection, and opens one when the
/// connection has none or when the one it has stopped answering. A driver
/// that cannot open gives the default session, because a snapshot that waits
/// is better than no completions.
///
/// A connection with one session alone gives its default session and opens
/// no second connection. A second connection to a SQLite database in memory
/// opens a separate empty database, so the tree would show no table.
async fn background_session(
    state: &AppState,
    connection_id: &str,
    open: &OpenConnection,
) -> Result<Arc<Session>> {
    if open.single_session {
        return open.default_session().await;
    }
    if let Some(session) = state.background_session(connection_id).await {
        if background_answers(&session).await {
            return Ok(session);
        }
        log::warn!(
            "The second connection of '{connection_id}' stopped answering. Opening it again."
        );
        state.clear_background(connection_id).await;
    }
    let full = match with_secrets(state, open.descriptor.clone()) {
        Ok(full) => full,
        Err(error) => {
            log::warn!("The password of '{connection_id}' could not be read: {error}");
            return open.default_session().await;
        }
    };
    match open_driver(&full).await {
        Ok(driver) => Ok(state.set_background_driver(connection_id, driver).await),
        Err(error) => {
            log::warn!(
                "A second connection for '{connection_id}' could not open, so the schema is \
                 read on the session of the user: {error}"
            );
            open.default_session().await
        }
    }
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
    let session = background_session(&state, &connection_id, &open).await?;
    let read = CatalogRead::new(&state, &connection_id, session, CATALOG_LIMIT);
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
                        text_of_column(&response, query.column)
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
        return Err(Error::Configuration(
            "A trigger or an event gives a CREATE statement alone.".to_string(),
        ));
    }
    let no_text = || {
        Error::Configuration(format!(
            "The engine gives no CREATE text for '{}'.",
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
    text_of_column(&response, query.column).ok_or_else(no_text)
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
        return Err(Error::Configuration(
            "The object reports no column, so the statement cannot be built.".to_string(),
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
    store::read_connections(&app)
}

#[tauri::command]
pub async fn save_connection<R: Runtime>(
    app: AppHandle<R>,
    connection: SavedConnection,
    state: tauri::State<'_, AppState>,
) -> Result<()> {
    connection.validate().map_err(Error::Configuration)?;
    refuse_password_in_string(&connection)?;

    store_secret(&state, &connection.id, connection.password.as_deref())?;
    store_secret(
        &state,
        &secrets::aws_secret_key(&connection.id),
        connection.aws_secret_access_key.as_deref(),
    )?;
    store_secret(
        &state,
        &secrets::aws_token_key(&connection.id),
        connection.aws_session_token.as_deref(),
    )?;

    store::write_connection(&app, &connection.without_secrets())
}

#[tauri::command]
pub async fn delete_connection<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    state: tauri::State<'_, AppState>,
) -> Result<()> {
    state.remove(&id).await;
    // Every key of the connection goes, or a removed connection leaves its
    // secrets in the keychain.
    let _ = state.secrets.delete(&id);
    let _ = state.secrets.delete(&secrets::aws_secret_key(&id));
    let _ = state.secrets.delete(&secrets::aws_token_key(&id));
    store::delete_connection(&app, &id)
}

// --- The query history and the saved queries ---

#[tauri::command]
pub async fn get_history<R: Runtime>(app: AppHandle<R>) -> Result<Vec<HistoryEntry>> {
    store::read_history(&app)
}

#[tauri::command]
pub async fn add_history_entry<R: Runtime>(app: AppHandle<R>, entry: HistoryEntry) -> Result<()> {
    store::add_history(&app, entry)
}

#[tauri::command]
pub async fn clear_history<R: Runtime>(app: AppHandle<R>) -> Result<()> {
    store::clear_history(&app)
}

#[tauri::command]
pub async fn get_saved_queries<R: Runtime>(app: AppHandle<R>) -> Result<Vec<SavedQuery>> {
    store::read_saved_queries(&app)
}

#[tauri::command]
pub async fn save_query<R: Runtime>(app: AppHandle<R>, query: SavedQuery) -> Result<()> {
    store::write_saved_query(&app, &query)
}

#[tauri::command]
pub async fn delete_saved_query<R: Runtime>(app: AppHandle<R>, id: String) -> Result<()> {
    store::delete_saved_query(&app, &id)
}

// --- The open tabs ---

#[tauri::command]
pub async fn get_workspace<R: Runtime>(app: AppHandle<R>) -> Result<serde_json::Value> {
    store::read_workspace(&app)
}

#[tauri::command]
pub async fn save_workspace<R: Runtime>(
    app: AppHandle<R>,
    workspace: serde_json::Value,
) -> Result<()> {
    store::write_workspace(&app, workspace)
}

/// The form a file export takes.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportFormat {
    Csv,
    Json,
    Xlsx,
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
    state.add_file_root(root).await;
    let names = path_names(&state.file_roots().await);
    if let Err(error) = store::write_file_roots(app, &names) {
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
    let Some(file) = files::grant_for(path) else {
        return;
    };
    state.add_file_grant(file).await;
    let names = path_names(&state.file_grants().await);
    if let Err(error) = store::write_file_grants(app, &names) {
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
#[tauri::command]
pub async fn set_menu_commands<R: Runtime>(
    app: AppHandle<R>,
    states: Vec<MenuCommandState>,
) -> Result<()> {
    for state in states {
        if !crate::menu::names_a_command(&state.id) {
            continue;
        }
        crate::menu::set_command_enabled(&app, &state.id, state.enabled)?;
    }
    Ok(())
}

/// One file that the user opened through the dialog.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedFile {
    pub path: String,
    pub contents: String,
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

    let contents = files::read_text(&path)?;
    accept_file(&app, &state, &path).await;
    let opened = path.to_string_lossy().to_string();
    log::info!("Opened the file '{opened}'.");
    Ok(Some(OpenedFile {
        path: opened,
        contents,
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
    let recorded = store::read_file_roots(app)?;
    let kept: Vec<std::path::PathBuf> = recorded
        .iter()
        .filter_map(|path| files::root_from_record(path))
        .collect();
    state.set_file_roots(kept.clone()).await;
    let names = path_names(&kept);
    if names.len() != recorded.len() {
        store::write_file_roots(app, &names)?;
    }

    let recorded = store::read_file_grants(app)?;
    let grants: Vec<std::path::PathBuf> = recorded
        .iter()
        .filter_map(|path| files::grant_for(std::path::Path::new(path)))
        .collect();
    state.set_file_grants(grants.clone()).await;
    let granted = path_names(&grants);
    if granted != recorded {
        store::write_file_grants(app, &granted)?;
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
    state.remove_file_root(std::path::Path::new(path)).await;
    let names = path_names(&state.file_roots().await);
    store::write_file_roots(app, &names)?;
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
    let target = files::path_inside_roots(std::path::Path::new(&path), &roots)?;
    files::read_folder(&target)
}

/// Resolves a path that a read or a write of a file names, and refuses it
/// when it is neither a grant nor inside a root.
async fn accepted_path(path: &str, state: &AppState) -> Result<std::path::PathBuf> {
    let roots = state.file_roots().await;
    let grants = state.file_grants().await;
    files::path_accepted(std::path::Path::new(path), &roots, &grants)
}

/// Reads the text of one file that is a grant or inside the roots.
#[tauri::command]
pub async fn read_text_file(path: String, state: tauri::State<'_, AppState>) -> Result<String> {
    files::read_text(&accepted_path(&path, &state).await?)
}

/// Writes the text of one file that is a grant or inside the roots.
#[tauri::command]
pub async fn write_text_file(
    path: String,
    contents: String,
    state: tauri::State<'_, AppState>,
) -> Result<()> {
    let target = accepted_path(&path, &state).await?;
    files::write_text(&target, &contents)?;
    log::info!("Wrote the file '{}'.", target.display());
    Ok(())
}

/// What a request to save the statement of a tab carries.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveStatementRequest {
    /// The file name that the save dialog suggests.
    pub default_name: String,
    /// The folder the dialog opens in, when the interface knows one.
    pub default_folder: Option<String>,
    pub contents: String,
}

/// Asks the user for a path and writes the statement of a tab there.
///
/// The file becomes a grant, so the next save of the same tab reaches it
/// through `write_text_file`. Returns the path, or `None` when the user
/// closed the dialog.
#[tauri::command]
pub async fn save_statement_file<R: Runtime>(
    app: AppHandle<R>,
    request: SaveStatementRequest,
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>> {
    let start_folder = request.default_folder.as_deref().map(std::path::Path::new);
    let Some(path) = ask_save_path(&app, &request.default_name, "SQL", "sql", start_folder).await
    else {
        return Ok(None);
    };
    files::write_text(&path, &request.contents)?;
    accept_file(&app, &state, &path).await;
    let written = path.to_string_lossy().to_string();
    log::info!("Wrote the file '{written}'.");
    Ok(Some(written))
}

/// What a request to save one file carries. The content is text, or base64
/// text when the file is binary.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveFileRequest {
    /// The file name that the save dialog suggests.
    pub default_name: String,
    /// The label of the file kind in the dialog.
    pub filter_label: String,
    /// The extension of the file kind, without the period.
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
    let Some(path) = ask_save_path(
        &app,
        &request.default_name,
        &request.filter_label,
        &request.extension,
        None,
    )
    .await
    else {
        return Ok(None);
    };
    files::write_bytes(&path, request.contents.as_bytes())?;
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
    let bytes = decode_base64(&request.contents)?;
    let Some(path) = ask_save_path(
        &app,
        &request.default_name,
        &request.filter_label,
        &request.extension,
        None,
    )
    .await
    else {
        return Ok(None);
    };
    files::write_bytes(&path, &bytes)?;
    let written = path.to_string_lossy().to_string();
    log::info!("Wrote the file '{written}'.");
    Ok(Some(written))
}

/// Reads the bytes out of base64 text.
fn decode_base64(text: &str) -> Result<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(text.as_bytes())
        .map_err(|error| Error::Configuration(format!("The file content is damaged: {error}")))
}

/// Runs a statement again with a higher row limit and writes the rows
/// straight to a file as they arrive. A large result therefore never
/// passes through the user interface, and the backend holds one row at a
/// time.
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

    let (label, extension) = match format {
        ExportFormat::Csv => ("CSV", "csv"),
        ExportFormat::Json => ("JSON", "json"),
        ExportFormat::Xlsx => ("Excel", "xlsx"),
    };
    let Some(path) = ask_save_path(&app, &default_name, label, extension, None).await else {
        return Ok(None);
    };

    let token = state.start_request(&request_id, &connection_id).await;
    let prepared = async {
        let (open, session, key) =
            session_for(&app, &state, &connection_id, tab_id.as_deref()).await?;
        if !crate::sql::only_reads(&query, open.dialect) {
            return Err(Error::Unsupported(
                "An export to a file runs the statement again, so it accepts a statement that only reads."
                    .to_string(),
            ));
        }
        let options = ExecOptions {
            max_rows,
            timeout_secs: open.descriptor.exec_options().timeout_secs,
            one_statement: true,
        };
        let (query, bound) = prepare_parameters(&query, open.dialect, query_params.as_ref())?;
        // The sink writes to a temporary path. An error, a stop or a time
        // limit leaves the run before `finish`, and the drop of the sink then
        // removes the part that was written.
        let sink = FileSink::create(&path, format)?;
        Ok::<_, Error>((open, session, key, options, query, bound, sink))
    }
    .await;
    let (open, session, key, options, query, bound, mut sink) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            state.end_request(&request_id).await;
            return Err(error);
        }
    };
    let outcome = match driver_for_request(&state, &request_id, &session, &token).await {
        Ok(mut guard) => {
            run_bounded(
                guard.execute_stream(&query, bound.as_ref(), &options, &mut sink),
                &token,
                options.timeout_secs,
                stop_grace(&session),
            )
            .await
        }
        Err(error) => Bounded::Answered(Err(error)),
    };
    state.end_request(&request_id).await;
    finish_run(&app, &state, &connection_id, &open, &key, &session, outcome).await?;

    if !sink.saw_set {
        return Err(Error::Unsupported(
            "The statement returned no result set.".to_string(),
        ));
    }
    let summary = sink.finish()?;
    log::info!(
        "Wrote {} rows to the file '{}'.",
        summary.rows,
        summary.path
    );
    Ok(Some(summary))
}

/// A sink that writes the rows of the first result set to a file as they
/// arrive. It writes to a temporary path beside the file and renames it at
/// a successful end, so a run that fails or stops leaves no file. It
/// answers `Stop` for a row of a second set, because the export writes one
/// file.
struct FileSink {
    format: ExportFormat,
    final_path: std::path::PathBuf,
    temp_path: std::path::PathBuf,
    out: Option<std::io::BufWriter<std::fs::File>>,
    /// The writer of the sheet, which holds the file while a set is open in
    /// the xlsx form.
    sheet: Option<crate::xlsx::SheetWriter<std::io::BufWriter<std::fs::File>>>,
    /// The name the one sheet of an xlsx file carries.
    sheet_title: String,
    /// The unique column names of the set, for the JSON objects.
    names: Vec<String>,
    rows: usize,
    truncated: bool,
    /// True once the first set began.
    saw_set: bool,
    /// True once the first set ended.
    set_done: bool,
    /// True once the file reached its final path.
    finished: bool,
}

impl FileSink {
    fn create(path: &std::path::Path, format: ExportFormat) -> Result<Self> {
        // The sink removes the temporary file itself when the export does
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
            truncated: false,
            saw_set: false,
            set_done: false,
            finished: false,
        })
    }

    fn writer(&mut self) -> Result<&mut std::io::BufWriter<std::fs::File>> {
        self.out
            .as_mut()
            .ok_or_else(|| Error::Anyhow(anyhow::anyhow!("The export file is closed.")))
    }

    /// Closes the file and renames it onto the path the user chose.
    fn finish(mut self) -> Result<ExportSummary> {
        use std::io::Write;
        if self.saw_set {
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
                // The sheet holds the file while it is open, so the close of
                // the container gives the file back.
                ExportFormat::Xlsx => {
                    if let Some(sheet) = self.sheet.take() {
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
        Ok(ExportSummary {
            rows: self.rows,
            truncated: self.truncated,
            path: self.final_path.to_string_lossy().to_string(),
        })
    }
}

impl Drop for FileSink {
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
        use std::io::Write;
        if self.saw_set {
            return Ok(());
        }
        self.saw_set = true;
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
                let file = self
                    .out
                    .take()
                    .ok_or_else(|| Error::Anyhow(anyhow::anyhow!("The export file is closed.")))?;
                self.sheet = Some(crate::xlsx::SheetWriter::create(
                    file,
                    &self.sheet_title,
                    &names,
                )?);
            }
        }
        Ok(())
    }

    fn row(&mut self, row: Vec<serde_json::Value>) -> Result<crate::db::sink::SinkControl> {
        use std::io::Write;
        if self.set_done {
            return Ok(crate::db::sink::SinkControl::Stop);
        }
        match self.format {
            ExportFormat::Csv => {
                write_csv_line(self.writer()?, &row)?;
            }
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
                write_json_object(out, &self.names, &row)?;
            }
            ExportFormat::Xlsx => {
                let sheet = self
                    .sheet
                    .as_mut()
                    .ok_or_else(|| Error::Anyhow(anyhow::anyhow!("The sheet is not open.")))?;
                // A sheet holds a bounded number of rows. The rows past the
                // bound stay out of the file, and the summary reports the
                // result as truncated.
                if !sheet.row(&row)? {
                    self.truncated = true;
                    return Ok(crate::db::sink::SinkControl::Stop);
                }
            }
        }
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
#[tauri::command]
pub fn passwords_persist(state: tauri::State<'_, AppState>) -> bool {
    state.secrets.persists()
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
        assert_eq!(error.category(), crate::error::ErrorCategory::Configuration);
        assert!(error.to_string().contains("':id'"));

        let athena = prepare_parameters("SELECT :id", Dialect::Athena, None).unwrap_err();
        assert_eq!(
            athena.category(),
            crate::error::ErrorCategory::Configuration
        );
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

    #[test]
    fn the_names_of_a_statement_reach_the_interface() {
        assert_eq!(
            query_parameters("SELECT :a, :b".to_string(), Dialect::MsSql),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[tokio::test]
    async fn a_bounded_run_gives_the_answer_of_the_work() {
        let token = CancellationToken::new();
        let outcome = run_bounded(async { Ok(7_u8) }, &token, 30, STOP_GRACE).await;
        assert!(matches!(outcome, Bounded::Answered(Ok(7))));

        // An error of the driver is an answer, so the connection stays open.
        let failed: Bounded<u8> =
            run_bounded(async { Err(Error::Cancelled) }, &token, 30, STOP_GRACE).await;
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
        )
        .await;

        assert!(matches!(outcome, Bounded::Answered(Err(Error::Cancelled))));
    }

    #[tokio::test]
    async fn a_stop_gives_up_on_a_driver_that_says_nothing() {
        tokio::time::pause();
        let token = CancellationToken::new();
        token.cancel();
        let outcome: Bounded<u8> = run_bounded(std::future::pending(), &token, 0, STOP_GRACE).await;
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
        )
        .await;
        assert!(matches!(outcome, Bounded::Stopped(Error::Cancelled)));
    }

    #[tokio::test]
    async fn a_bounded_run_stops_at_the_time_limit() {
        tokio::time::pause();
        let token = CancellationToken::new();
        let outcome: Bounded<u8> = run_bounded(std::future::pending(), &token, 5, STOP_GRACE).await;
        assert!(matches!(outcome, Bounded::Stopped(Error::Timeout(5))));
    }

    #[tokio::test]
    async fn a_limit_of_zero_seconds_is_no_limit() {
        tokio::time::pause();
        let waiting = tokio::spawn(until_the_limit(0));
        tokio::time::advance(std::time::Duration::from_secs(60 * 60)).await;
        assert!(!waiting.is_finished());
        waiting.abort();
    }

    #[test]
    fn a_limit_that_ended_a_run_names_the_limit() {
        assert!(limit_reason(&Error::Timeout(90)).contains("limit of 90 seconds"));
        assert!(limit_reason(&Error::Cancelled).contains("stopped"));
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
        assert!(error.to_string().contains("reports no column"));
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
        assert!(select.to_string().contains("CREATE statement alone"));

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
                .contains(&format!("no CREATE text for '{name}'")));
        }
    }

    fn state() -> AppState {
        AppState::new(Box::new(MemoryStore::default()))
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
            assert!(error.to_string().contains("Password box"));
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

        let filled = with_secrets(&state, sqlite_connection("/tmp/a.db")).unwrap();
        assert_eq!(filled.password.as_deref(), Some("from-the-store"));

        let mut given = sqlite_connection("/tmp/a.db");
        given.password = Some("typed".into());
        let kept = with_secrets(&state, given).unwrap();
        assert_eq!(kept.password.as_deref(), Some("typed"));
    }

    #[tokio::test]
    async fn a_password_that_is_absent_stays_absent() {
        let state = state();
        let filled = with_secrets(&state, sqlite_connection("/tmp/a.db")).unwrap();
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

        let filled = with_secrets(&state, sqlite_connection("/tmp/a.db")).unwrap();
        assert_eq!(filled.aws_secret_access_key.as_deref(), Some("the-secret"));
        assert_eq!(filled.aws_session_token.as_deref(), Some("the-token"));

        // A key that the caller gave stays as it is.
        let mut given = sqlite_connection("/tmp/a.db");
        given.aws_secret_access_key = Some("typed".into());
        given.aws_session_token = Some("typed-token".into());
        let kept = with_secrets(&state, given).unwrap();
        assert_eq!(kept.aws_secret_access_key.as_deref(), Some("typed"));
        assert_eq!(kept.aws_session_token.as_deref(), Some("typed-token"));
    }

    #[tokio::test]
    async fn a_secret_is_written_kept_or_taken_away() {
        let state = state();

        // A text writes the secret, and the store then holds it.
        assert!(store_secret(&state, "k1", Some("first")).unwrap());
        assert_eq!(state.secrets.get("k1").unwrap().as_deref(), Some("first"));

        // An absent field leaves the store as it stands.
        assert!(store_secret(&state, "k1", None).unwrap());
        assert_eq!(state.secrets.get("k1").unwrap().as_deref(), Some("first"));

        // An empty text takes the secret away.
        assert!(!store_secret(&state, "k1", Some("")).unwrap());
        assert_eq!(state.secrets.get("k1").unwrap(), None);

        // An absent field over an empty store reports no secret.
        assert!(!store_secret(&state, "k1", None).unwrap());
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

    /// Builds an application of the tests that holds the store plugin, so
    /// the files of the settings answer.
    fn app_with_store() -> tauri::App<tauri::test::MockRuntime> {
        tauri::test::mock_builder()
            .plugin(tauri_plugin_store::Builder::default().build())
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
        let next = AppState::new(Box::new(MemoryStore::default()));
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
        let next = AppState::new(Box::new(MemoryStore::default()));
        file_roots_for(app.handle(), &next).await.unwrap();
        assert_eq!(next.file_grants().await, vec![resolved]);
        assert!(accepted_path(&file.to_string_lossy(), &next).await.is_ok());
        assert!(accepted_path(&beside.to_string_lossy(), &next)
            .await
            .is_err());
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

        let filled = with_secrets_for_test(app.handle(), &state, saved).unwrap();
        assert_eq!(filled.password.as_deref(), Some("from-the-store"));
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
            .err()
            .unwrap();
        assert_eq!(error.category(), crate::error::ErrorCategory::Configuration);
        assert!(error.to_string().contains("Type the password"));

        // The same record with the password of the caller goes through, and
        // the store gives nothing to it.
        let mut typed = changed.clone();
        typed.password = Some("typed".into());
        let kept = with_secrets_for_test(app.handle(), &state, typed).unwrap();
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
            .err()
            .unwrap();
        assert!(error.to_string().contains("secret access key"));

        changed.aws_secret_access_key = Some("typed".into());
        let error = with_secrets_for_test(app.handle(), &state, changed.clone())
            .err()
            .unwrap();
        assert!(error.to_string().contains("session token"));

        changed.aws_session_token = Some("typed-token".into());
        let kept = with_secrets_for_test(app.handle(), &state, changed).unwrap();
        assert_eq!(kept.aws_secret_access_key.as_deref(), Some("typed"));
    }

    #[tokio::test]
    async fn a_test_of_a_record_that_is_not_saved_holds_its_own_fields() {
        let app = app_with_store();
        let state = state();
        let mut given = sqlite_connection("/tmp/a.db");
        given.password = Some("typed".into());

        let kept = with_secrets_for_test(app.handle(), &state, given).unwrap();
        assert_eq!(kept.password.as_deref(), Some("typed"));
    }

    #[tokio::test]
    async fn an_identifier_that_no_record_carries_cannot_open() {
        let app = app_with_store();
        assert!(saved_record(app.handle(), "nowhere").unwrap().is_none());
    }

    #[test]
    fn base64_gives_bytes_and_damaged_content_is_refused() {
        // "PK" is the mark that a ZIP container starts with.
        assert_eq!(decode_base64("UEs=").unwrap(), b"PK");
        let error = decode_base64("not base64!").err().unwrap();
        assert_eq!(error.category(), crate::error::ErrorCategory::Configuration);
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

    #[test]
    fn a_result_reaches_a_file_in_both_forms() {
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
        let mut sink = FileSink::create(&csv, ExportFormat::Csv).unwrap();
        sink.begin_set(columns.clone()).unwrap();
        for row in &rows {
            assert_eq!(sink.row(row.clone()).unwrap(), SinkControl::Continue);
        }
        sink.end_set(false).unwrap();
        let summary = sink.finish().unwrap();
        assert_eq!(summary.rows, 2);
        assert!(!summary.truncated);
        assert_eq!(
            std::fs::read_to_string(&csv).unwrap(),
            "\u{feff}id,name\r\n1,Ada\r\n2,\r\n"
        );
        // The temporary file is gone after the rename.
        assert!(no_temporary_file(folder.path()));

        let json = folder.path().join("out.json");
        let mut sink = FileSink::create(&json, ExportFormat::Json).unwrap();
        sink.begin_set(columns).unwrap();
        for row in &rows {
            sink.row(row.clone()).unwrap();
        }
        sink.end_set(true).unwrap();
        let summary = sink.finish().unwrap();
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

    #[test]
    fn a_result_reaches_an_excel_file_as_one_sheet() {
        use crate::db::sink::{RowSink, SinkControl};
        use crate::db::ColumnInfo;
        use std::io::Read;

        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("Daily count.xlsx");
        let mut sink = FileSink::create(&path, ExportFormat::Xlsx).unwrap();
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
        let summary = sink.finish().unwrap();

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

    #[test]
    fn a_stopped_export_of_an_excel_file_leaves_no_file() {
        use crate::db::sink::RowSink;
        use crate::db::ColumnInfo;
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("part.xlsx");
        let mut sink = FileSink::create(&path, ExportFormat::Xlsx).unwrap();
        sink.begin_set(vec![ColumnInfo::new("id", "int")]).unwrap();
        sink.row(vec![serde_json::json!(1)]).unwrap();
        drop(sink);
        assert!(!path.exists());
        assert!(std::fs::read_dir(folder.path()).unwrap().next().is_none());
    }

    #[test]
    fn an_excel_export_stops_at_the_bound_of_a_sheet() {
        use crate::db::sink::{RowSink, SinkControl};
        use crate::db::ColumnInfo;
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("full.xlsx");
        let mut sink = FileSink::create(&path, ExportFormat::Xlsx).unwrap();
        sink.begin_set(vec![ColumnInfo::new("id", "int")]).unwrap();

        // The sheet stands one row below its bound, so the next row is the
        // last one that fits.
        sink.sheet
            .as_mut()
            .unwrap()
            .set_rows(crate::xlsx::MAX_SHEET_ROWS - 1);
        assert_eq!(
            sink.row(vec![serde_json::json!(1)]).unwrap(),
            SinkControl::Continue
        );
        assert_eq!(
            sink.row(vec![serde_json::json!(2)]).unwrap(),
            SinkControl::Stop
        );
        sink.end_set(false).unwrap();

        let summary = sink.finish().unwrap();
        assert!(summary.truncated);
        assert!(path.exists());
    }

    #[test]
    fn a_stopped_export_leaves_no_file() {
        use crate::db::sink::RowSink;
        use crate::db::ColumnInfo;
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("part.csv");
        let mut sink = FileSink::create(&path, ExportFormat::Csv).unwrap();
        sink.begin_set(vec![ColumnInfo::new("id", "int")]).unwrap();
        sink.row(vec![serde_json::json!(1)]).unwrap();
        drop(sink);
        assert!(!path.exists());
        assert!(std::fs::read_dir(folder.path()).unwrap().next().is_none());
    }

    #[test]
    fn an_export_writes_the_first_set_alone() {
        use crate::db::sink::{RowSink, SinkControl};
        use crate::db::ColumnInfo;
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("first.csv");
        let mut sink = FileSink::create(&path, ExportFormat::Csv).unwrap();
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

        let summary = sink.finish().unwrap();
        assert_eq!(summary.rows, 1);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "\u{feff}id\r\n1\r\n"
        );
    }

    #[test]
    fn an_empty_json_export_is_a_valid_list() {
        use crate::db::sink::RowSink;
        use crate::db::ColumnInfo;
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("empty.json");
        let mut sink = FileSink::create(&path, ExportFormat::Json).unwrap();
        sink.begin_set(vec![ColumnInfo::new("id", "int")]).unwrap();
        sink.end_set(false).unwrap();
        let summary = sink.finish().unwrap();
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
    async fn state_with_sqlite(
        descriptor: SavedConnection,
    ) -> (tauri::App<tauri::test::MockRuntime>, AppState) {
        let app = tauri::test::mock_app();
        let driver = open_driver(&descriptor).await.unwrap();
        let state = AppState::new(Box::new(MemoryStore::default()));
        let id = descriptor.id.clone();
        state
            .insert(&id, OpenConnection::new(descriptor, driver))
            .await;
        (app, state)
    }

    fn temp_sqlite() -> (tempfile::TempDir, SavedConnection) {
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
        assert!(!error.to_string().is_empty());
        assert_eq!(*frame_types.lock().unwrap(), vec![FRAME_END]);
    }

    #[tokio::test]
    async fn each_tab_takes_a_session_of_its_own() {
        let (_dir, descriptor) = temp_sqlite();
        let (app, state) = state_with_sqlite(descriptor).await;

        let (open, first, key_one) = session_for(app.handle(), &state, "s1", Some("t1"))
            .await
            .unwrap();
        let (_, second, key_two) = session_for(app.handle(), &state, "s1", Some("t2"))
            .await
            .unwrap();
        assert_eq!(key_one, "t1");
        assert_eq!(key_two, "t2");
        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(open.sessions.tab_count().await, 2);

        // The tab keeps its session from one run to the next.
        let (_, again, _) = session_for(app.handle(), &state, "s1", Some("t1"))
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&first, &again));
    }

    #[tokio::test]
    async fn a_request_without_a_tab_takes_the_default_session() {
        let (_dir, descriptor) = temp_sqlite();
        let (app, state) = state_with_sqlite(descriptor).await;

        let (open, session, key) = session_for(app.handle(), &state, "s1", None).await.unwrap();
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

        session_for(app.handle(), &state, "s1", Some("t1"))
            .await
            .unwrap();
        let error = session_for(app.handle(), &state, "s1", Some("t2"))
            .await
            .err()
            .unwrap();
        assert_eq!(error.category(), crate::error::ErrorCategory::Configuration);
        assert!(error.to_string().contains("session limit"));

        // The tab that holds a session keeps it, and the default session
        // stays outside the cap.
        assert!(session_for(app.handle(), &state, "s1", Some("t1"))
            .await
            .is_ok());
        assert!(session_for(app.handle(), &state, "s1", None).await.is_ok());
    }

    #[tokio::test]
    async fn the_cap_closes_an_idle_session_to_make_room() {
        let (_dir, mut descriptor) = temp_sqlite();
        descriptor.options.max_sessions = 1;
        let (app, state) = state_with_sqlite(descriptor).await;

        let (open, first, _) = session_for(app.handle(), &state, "s1", Some("t1"))
            .await
            .unwrap();
        first.age(crate::session::SESSION_IDLE_REAP).await;
        session_for(app.handle(), &state, "s1", Some("t2"))
            .await
            .unwrap();
        assert!(open.sessions.get("t1").await.is_none());
    }

    #[tokio::test]
    async fn the_cap_keeps_an_idle_session_inside_a_transaction() {
        let (_dir, mut descriptor) = temp_sqlite();
        descriptor.options.max_sessions = 1;
        let (app, state) = state_with_sqlite(descriptor).await;

        let (open, first, _) = session_for(app.handle(), &state, "s1", Some("t1"))
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
        assert!(session_for(app.handle(), &state, "s1", Some("t2"))
            .await
            .is_err());
        let kept = open.sessions.get("t1").await.unwrap();
        assert!(Arc::ptr_eq(&kept, &first));
    }

    #[tokio::test]
    async fn a_database_in_memory_gives_every_tab_the_default_session() {
        let descriptor = sqlite_connection(":memory:");
        let (app, state) = state_with_sqlite(descriptor).await;

        let (open, session, key) = session_for(app.handle(), &state, "s1", Some("t1"))
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
        assert!(state.background_session("s1").await.is_none());
    }

    #[tokio::test]
    async fn a_released_tab_session_leaves_the_pool() {
        use tauri::Manager;
        let (_dir, descriptor) = temp_sqlite();
        let (app, state) = state_with_sqlite(descriptor).await;
        session_for(app.handle(), &state, "s1", Some("t1"))
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
    async fn a_stop_replaces_only_the_session_that_ran_the_statement() {
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
        let result = finish_run(app.handle(), &state, "s1", &open, "t1", &frail, outcome).await;
        assert!(result.is_err());

        // The slot of the tab holds a new session, and the default session
        // stays as it was.
        let replaced = open.sessions.get("t1").await.unwrap();
        assert!(!Arc::ptr_eq(&frail, &replaced));
        let default_after = open.default_session().await.unwrap();
        assert!(Arc::ptr_eq(&default_before, &default_after));
    }

    /// A driver for the tests of the background connection. It counts the
    /// pings it answered and fails every ping when it is told to.
    struct PingDriver {
        pings: Arc<std::sync::atomic::AtomicUsize>,
        answers: bool,
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
        let pings = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let session = state
            .set_background_driver(
                "s1",
                Box::new(PingDriver {
                    pings: pings.clone(),
                    answers,
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
        let fresh = background_session(&state, "s1", &open).await.unwrap();
        assert!(Arc::ptr_eq(&fresh, &session));
        assert_eq!(pings.load(std::sync::atomic::Ordering::SeqCst), 0);

        session.age(crate::state::HEALTH_CHECK_AFTER).await;
        let checked = background_session(&state, "s1", &open).await.unwrap();
        assert!(Arc::ptr_eq(&checked, &session));
        assert_eq!(pings.load(std::sync::atomic::Ordering::SeqCst), 1);

        // The ping moved the moment of the last answer, so the next read
        // sends no second ping.
        background_session(&state, "s1", &open).await.unwrap();
        assert_eq!(pings.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_background_driver_that_stopped_answering_opens_again() {
        let (_dir, descriptor) = temp_sqlite();
        let (_app, state) = state_with_sqlite(descriptor).await;
        let open = state.connection("s1").await.unwrap();
        let (session, pings) = background_stub(&state, false).await;
        session.age(crate::state::HEALTH_CHECK_AFTER).await;

        let opened = background_session(&state, "s1", &open).await.unwrap();

        assert_eq!(pings.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(!Arc::ptr_eq(&opened, &session));
        // The new driver reaches the database.
        opened.driver.lock().await.ping().await.unwrap();
    }

    #[tokio::test]
    async fn a_stop_keeps_the_session_of_a_driver_that_survives_it() {
        let (_dir, descriptor) = temp_sqlite();
        let (app, state) = state_with_sqlite(descriptor).await;
        let (open, session, key) = session_for(app.handle(), &state, "s1", Some("t1"))
            .await
            .unwrap();

        // SQLite aborts a statement cleanly, so the session stays.
        let outcome: Bounded<()> = Bounded::Stopped(Error::Cancelled);
        let result = finish_run(app.handle(), &state, "s1", &open, &key, &session, outcome).await;
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
        let (app, state) = state_with_sqlite(descriptor).await;
        let open = state.connection("s1").await.unwrap();
        let (driver, _calls) = catalog_driver(None);
        let session = open.sessions.insert("t1", Session::new(driver)).await;
        open.sessions.release("t1").await;

        let outcome: Bounded<()> = Bounded::Stopped(Error::Cancelled);
        let result = finish_run(app.handle(), &state, "s1", &open, "t1", &session, outcome).await;

        assert!(result.is_err());
        assert!(open.sessions.get("t1").await.is_none());
    }

    #[tokio::test]
    async fn a_stop_opens_no_session_for_a_connection_that_closed() {
        let (_dir, descriptor) = temp_sqlite();
        let (app, state) = state_with_sqlite(descriptor).await;
        let open = state.connection("s1").await.unwrap();
        let (driver, _calls) = catalog_driver(None);
        let session = open.sessions.insert("t1", Session::new(driver)).await;
        state.remove("s1").await;

        let outcome: Bounded<()> = Bounded::Stopped(Error::Cancelled);
        let result = finish_run(app.handle(), &state, "s1", &open, "t1", &session, outcome).await;

        assert!(result.is_err());
        assert!(state.connection("s1").await.is_err());
        let held = open.sessions.get("t1").await.unwrap();
        assert!(Arc::ptr_eq(&held, &session));
    }

    #[tokio::test]
    async fn a_stop_opens_no_session_in_a_connection_that_opened_again() {
        let (_dir, descriptor) = temp_sqlite();
        let (app, state) = state_with_sqlite(descriptor.clone()).await;
        let open = state.connection("s1").await.unwrap();
        let (driver, _calls) = catalog_driver(None);
        let session = open.sessions.insert("t1", Session::new(driver)).await;
        let driver = open_driver(&descriptor).await.unwrap();
        state
            .insert("s1", OpenConnection::new(descriptor, driver))
            .await;

        let outcome: Bounded<()> = Bounded::Stopped(Error::Cancelled);
        let result = finish_run(app.handle(), &state, "s1", &open, "t1", &session, outcome).await;

        assert!(result.is_err());
        let current = state.connection("s1").await.unwrap();
        assert!(current.sessions.get("t1").await.is_none());
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
        let background = state.set_background_driver("s1", driver).await;

        let read = CatalogRead::new(&state, "s1", background, SHORT);
        let _guard = read.lock().await.unwrap();
        let outcome: Result<()> = read.run(std::future::pending()).await;

        assert!(matches!(outcome, Err(Error::Timeout(0))));
        assert_eq!(stops(&calls), 1);
        assert!(state.background_session("s1").await.is_none());
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
        let kept = state.background_session("s1").await.unwrap();
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
        let background = state.set_background_driver("s1", driver).await;
        let _other = background.driver.lock().await;

        let read = CatalogRead::new(&state, "s1", background.clone(), SHORT);
        assert!(matches!(read.lock().await, Err(Error::Timeout(0))));

        assert_eq!(stops(&calls), 0);
        assert!(state.background_session("s1").await.is_some());
    }
}
