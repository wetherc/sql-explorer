//! The commands the user interface calls.

use crate::db::columnar::ChunkSink;
use crate::db::drivers::{
    athena::AthenaDriver, mssql::MssqlDriver, mysql::MysqlDriver, postgres::PostgresDriver,
    sqlite::SqliteDriver,
};
use crate::db::{
    self, drivers::DatabaseDriver, AppColumn, Constraint, Database, ExecOptions, IndexInfo,
    Partition, PlanKind, QueryParams, QueryResponse, Routine, Schema, SchemaSnapshot, Table,
    TableDetails, TableKind,
};
use crate::error::{Error, Result};
use crate::files;
use crate::history::{HistoryEntry, SavedQuery};
use crate::script::{self, ScriptKind};
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

#[tauri::command]
pub async fn connect<R: Runtime>(
    app: AppHandle<R>,
    connection: SavedConnection,
    state: tauri::State<'_, AppState>,
) -> Result<ConnectionInfo> {
    let id = connection.id.clone();
    let full = with_secrets(&state, connection)?;

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

/// Opens a connection, confirms that it answers, and closes it again.
#[tauri::command]
pub async fn test_connection(
    connection: SavedConnection,
    state: tauri::State<'_, AppState>,
) -> Result<String> {
    let full = with_secrets(&state, connection)?;
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
/// tab without a session gets a new one, up to the cap of the pool. The
/// sessions that other tabs left idle go on the way in.
async fn session_for<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    connection_id: &str,
    tab_id: Option<&str>,
) -> Result<(OpenConnection, Arc<Session>, String)> {
    let open = state.connection(connection_id).await?;
    let key = open.session_key(tab_id);
    open.sessions.reap_idle().await;

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

/// Returns the driver that a metadata read runs on. The read goes to a
/// second connection when one can open, so that the tree of the explorer
/// does not wait behind a statement of the user.
async fn metadata_driver<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    connection_id: &str,
) -> Result<crate::state::BackgroundDriver> {
    let open = ensure_healthy(app, state, connection_id).await?;
    background_driver(state, connection_id, &open).await
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
    let (open, session, key) = session_for(&app, &state, &connection_id, tab_id.as_deref()).await?;
    let options = options.unwrap_or_else(|| open.descriptor.exec_options());
    let (query, bound) = prepare_parameters(&query, open.dialect, query_params.as_ref())?;
    let token = state
        .start_request(&request_id, session.cancel_handle.clone())
        .await;

    let mut sink = ChunkSink::new(on_chunk, options.max_rows);
    let started = std::time::Instant::now();
    let outcome = {
        let mut guard = session.driver.lock().await;
        run_bounded(
            guard.execute_stream(&query, bound.as_ref(), &options, &mut sink),
            &token,
            options.timeout_secs,
            stop_grace(&session),
        )
        .await
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
    pub kind: PlanKind,
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
        kind,
        tab_id,
        query_params,
        options,
    } = request;
    let (open, session, key) = session_for(&app, &state, &connection_id, tab_id.as_deref()).await?;
    let options = options.unwrap_or_else(|| open.descriptor.exec_options());
    // A plan needs the values of the parameters, because the plan of a
    // statement depends on the values it holds.
    let (query, bound) = prepare_parameters(&query, open.dialect, query_params.as_ref())?;
    let token = state
        .start_request(&request_id, session.cancel_handle.clone())
        .await;

    let outcome = {
        let mut guard = session.driver.lock().await;
        run_bounded(
            guard.explain(&query, bound.as_ref(), kind, &options),
            &token,
            options.timeout_secs,
            stop_grace(&session),
        )
        .await
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
            reopen_after_stop(app, state, connection_id, open, session_key, &error).await;
            Err(error)
        }
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
#[tauri::command]
pub async fn cancel_query(
    #[allow(unused_variables)] connection_id: String,
    request_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<()> {
    if let Some(request) = state.take_request(&request_id).await {
        // The handle does not need the lock of the driver, so it works
        // while the statement runs.
        if let Some(handle) = request.cancel_handle {
            if let Err(error) = handle.cancel().await {
                log::warn!("The server did not stop the statement: {error}");
            }
        }
        request.token.cancel();
    }
    Ok(())
}

#[tauri::command]
pub async fn list_databases<R: Runtime>(
    app: AppHandle<R>,
    connection_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Database>> {
    let driver = metadata_driver(&app, &state, &connection_id).await?;
    let mut guard = driver.lock().await;
    guard.list_databases().await
}

#[tauri::command]
pub async fn list_schemas<R: Runtime>(
    app: AppHandle<R>,
    connection_id: String,
    database: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Schema>> {
    let driver = metadata_driver(&app, &state, &connection_id).await?;
    let mut guard = driver.lock().await;
    guard.list_schemas(&database).await
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
    let driver = metadata_driver(&app, &state, &request.connection_id).await?;
    let mut guard = driver.lock().await;
    guard
        .list_tables(&request.database, request.schema_name.as_deref())
        .await
}

#[tauri::command]
pub async fn list_columns<R: Runtime>(
    app: AppHandle<R>,
    request: TableScope,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<AppColumn>> {
    let driver = metadata_driver(&app, &state, &request.connection_id).await?;
    let mut guard = driver.lock().await;
    guard
        .list_columns(
            &request.database,
            request.schema_name.as_deref(),
            &request.table_name,
        )
        .await
}

#[tauri::command]
pub async fn list_routines<R: Runtime>(
    app: AppHandle<R>,
    request: SchemaScope,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Routine>> {
    let driver = metadata_driver(&app, &state, &request.connection_id).await?;
    let mut guard = driver.lock().await;
    guard
        .list_routines(&request.database, request.schema_name.as_deref())
        .await
}

#[tauri::command]
pub async fn list_indexes<R: Runtime>(
    app: AppHandle<R>,
    request: TableScope,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<IndexInfo>> {
    let driver = metadata_driver(&app, &state, &request.connection_id).await?;
    let mut guard = driver.lock().await;
    guard
        .list_indexes(
            &request.database,
            request.schema_name.as_deref(),
            &request.table_name,
        )
        .await
}

#[tauri::command]
pub async fn list_constraints<R: Runtime>(
    app: AppHandle<R>,
    request: TableScope,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Constraint>> {
    let driver = metadata_driver(&app, &state, &request.connection_id).await?;
    let mut guard = driver.lock().await;
    guard
        .list_constraints(
            &request.database,
            request.schema_name.as_deref(),
            &request.table_name,
        )
        .await
}

#[tauri::command]
pub async fn list_partitions<R: Runtime>(
    app: AppHandle<R>,
    request: TableScope,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Partition>> {
    let driver = metadata_driver(&app, &state, &request.connection_id).await?;
    let mut guard = driver.lock().await;
    guard
        .list_partitions(
            &request.database,
            request.schema_name.as_deref(),
            &request.table_name,
        )
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
    let driver = metadata_driver(&app, &state, &request.connection_id).await?;
    let database = &request.database;
    let schema = request.schema_name.as_deref();
    let table = &request.table_name;
    let mut guard = driver.lock().await;

    let details = TableDetails {
        facts: guard.table_facts(database, schema, table).await?,
        columns: guard.list_columns(database, schema, table).await?,
        indexes: guard.list_indexes(database, schema, table).await?,
        constraints: guard.list_constraints(database, schema, table).await?,
    };

    Ok(details)
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

    let driver = match request.own_connection.unwrap_or(true) {
        true => background_driver(&state, &request.connection_id, &open).await?,
        false => open.default_session().await?.driver.clone(),
    };

    let mut guard = driver.lock().await;
    guard.schema_snapshot(&request.database, limit).await
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

/// Returns the background driver of a connection, and opens one when the
/// connection has none or when the one it has stopped answering. A driver
/// that cannot open gives the driver of the default session, because a
/// snapshot that waits is better than no completions.
async fn background_driver(
    state: &AppState,
    connection_id: &str,
    open: &OpenConnection,
) -> Result<Arc<tokio::sync::Mutex<Box<dyn DatabaseDriver>>>> {
    if let Some(session) = state.background_session(connection_id).await {
        if background_answers(&session).await {
            return Ok(session.driver.clone());
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
            return Ok(open.default_session().await?.driver.clone());
        }
    };
    match open_driver(&full).await {
        Ok(driver) => Ok(state
            .set_background_driver(connection_id, driver)
            .await
            .driver
            .clone()),
        Err(error) => {
            log::warn!(
                "A second connection for '{connection_id}' could not open, so the schema is \
                 read on the session of the user: {error}"
            );
            Ok(open.default_session().await?.driver.clone())
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
        kind,
        script_kind,
    } = request;

    let open = ensure_healthy(&app, &state, &connection_id).await?;
    let dialect = open.dialect;
    let name = dialect.qualified_name(database.as_deref(), schema_name.as_deref(), &table_name);

    // The work only reads the catalog, so it runs on the driver of the
    // metadata reads and leaves the sessions of the tabs free.
    let driver = background_driver(&state, &connection_id, &open).await?;
    let mut guard = driver.lock().await;
    let columns = guard
        .list_columns(
            database.as_deref().unwrap_or_default(),
            schema_name.as_deref(),
            &table_name,
        )
        .await?;

    let from_engine = match script_kind {
        ScriptKind::Create => match guard.create_query(
            database.as_deref(),
            schema_name.as_deref(),
            &table_name,
            kind,
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

    drop(guard);
    script_text(dialect, &name, script_kind, &columns, from_engine)
}

/// What the user interface asks for when it wants the text of one object.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptRequest {
    pub connection_id: String,
    pub database: Option<String>,
    pub schema_name: Option<String>,
    pub table_name: String,
    /// The kind of the object, which decides where the CREATE text lives.
    pub kind: TableKind,
    /// The statement the user asked for.
    pub script_kind: ScriptKind,
}

/// Selects the statement for the kind that the user asked for. The text of
/// the engine wins for the CREATE form, and a draft serves when there is no
/// such text.
fn script_text(
    dialect: crate::sql::Dialect,
    name: &str,
    kind: ScriptKind,
    columns: &[AppColumn],
    from_engine: Option<String>,
) -> Result<String> {
    if kind == ScriptKind::Select {
        return Ok(script::select_statement(dialect, name, columns));
    }
    if let (ScriptKind::Create, Some(text)) = (kind, from_engine) {
        return Ok(text);
    }
    if columns.is_empty() {
        // The other forms are built from the columns, and a relation that
        // reports none gives no statement at all.
        return Err(Error::Configuration(
            "The object reports no column, so the statement cannot be built.".to_string(),
        ));
    }
    Ok(match kind {
        ScriptKind::Insert => script::insert_statement(dialect, name, columns),
        ScriptKind::Update => script::update_statement(dialect, name, columns),
        // The select form left this function above.
        ScriptKind::Create | ScriptKind::Select => script::create_draft(dialect, name, columns),
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

    store_secret(&state, &connection.id, connection.password.as_deref())?;
    store_secret(
        &state,
        &secrets::aws_secret_key(&connection.id),
        connection.aws_secret_access_key.as_deref(),
    )?;
    let token_held = store_secret(
        &state,
        &secrets::aws_token_key(&connection.id),
        connection.aws_session_token.as_deref(),
    )?;

    // The flag of the record follows the store, so the form always sees
    // what the keychain holds.
    let mut record = connection.without_secrets();
    record.options.aws_session_token_set = token_held;
    store::write_connection(&app, &record)
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
    state.add_file_root(path.clone()).await;
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
/// The folder of the file becomes a root, so a later save of the same tab
/// reaches the file and the panel can list the folder beside it. Returns
/// `None` when the user closed the dialog.
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

    if let Some(folder) = files::folder_of(&path) {
        state.add_file_root(folder).await;
    }
    let contents = files::read_text(&path)?;
    let opened = path.to_string_lossy().to_string();
    log::info!("Opened the file '{opened}'.");
    Ok(Some(OpenedFile {
        path: opened,
        contents,
    }))
}

/// Records a folder that the workspace file held, so the folders of the last
/// session are reachable again. The folder must still be a folder on the
/// disk, so a record that names something else brings nothing back.
#[tauri::command]
pub async fn restore_folder(path: String, state: tauri::State<'_, AppState>) -> Result<bool> {
    let Some(root) = files::root_from_record(&path) else {
        return Ok(false);
    };
    state.add_file_root(root).await;
    Ok(true)
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

/// Reads the text of one file inside the roots.
#[tauri::command]
pub async fn read_text_file(path: String, state: tauri::State<'_, AppState>) -> Result<String> {
    let roots = state.file_roots().await;
    let target = files::path_inside_roots(std::path::Path::new(&path), &roots)?;
    files::read_text(&target)
}

/// Writes the text of one file inside the roots.
#[tauri::command]
pub async fn write_text_file(
    path: String,
    contents: String,
    state: tauri::State<'_, AppState>,
) -> Result<()> {
    let roots = state.file_roots().await;
    let target = files::path_inside_roots(std::path::Path::new(&path), &roots)?;
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
/// The folder of the file becomes a root, so the next save of the same tab
/// reaches the file through `write_text_file`. Returns the path, or `None`
/// when the user closed the dialog.
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
    if let Some(folder) = files::folder_of(&path) {
        state.add_file_root(folder).await;
    }
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
    std::fs::write(&path, request.contents)?;
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
    std::fs::write(&path, bytes)?;
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

    let (open, session, key) = session_for(&app, &state, &connection_id, tab_id.as_deref()).await?;
    if !crate::sql::only_reads(&query, open.dialect) {
        return Err(Error::Unsupported(
            "An export to a file runs the statement again, so it accepts a statement that only reads."
                .to_string(),
        ));
    }

    let options = ExecOptions {
        max_rows,
        timeout_secs: open.descriptor.exec_options().timeout_secs,
    };
    let (query, bound) = prepare_parameters(&query, open.dialect, query_params.as_ref())?;
    let token = state
        .start_request(&request_id, session.cancel_handle.clone())
        .await;

    // The sink writes to a temporary path. An error, a stop or a time limit
    // leaves the run before `finish`, and the drop of the sink then removes
    // the part that was written.
    let mut sink = FileSink::create(&path, format)?;
    let outcome = {
        let mut guard = session.driver.lock().await;
        run_bounded(
            guard.execute_stream(&query, bound.as_ref(), &options, &mut sink),
            &token,
            options.timeout_secs,
            stop_grace(&session),
        )
        .await
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
        let mut name = path.as_os_str().to_owned();
        name.push(".part");
        let temp_path = std::path::PathBuf::from(name);
        let file = std::fs::File::create(&temp_path)?;
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
        let mut out = self.out.take().expect("the file is open until here");
        out.flush()?;
        drop(out);
        std::fs::rename(&self.temp_path, &self.final_path)?;
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
                let names: Vec<String> = columns
                    .iter()
                    .map(|column| csv_field(&serde_json::Value::String(column.name.clone())))
                    .collect();
                writeln!(self.writer()?, "{}", names.join(","))?;
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
                let fields: Vec<String> = row.iter().map(csv_field).collect();
                writeln!(self.writer()?, "{}", fields.join(","))?;
            }
            ExportFormat::Json => {
                let mut object = serde_json::Map::new();
                for (position, name) in self.names.iter().enumerate() {
                    let value = row
                        .get(position)
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    object.insert(name.clone(), value);
                }
                let text = serde_json::Value::Object(object).to_string();
                let rows = self.rows;
                let out = self.writer()?;
                if rows > 0 {
                    writeln!(out, ",")?;
                }
                write!(out, "  {text}")?;
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

/// True when a spreadsheet would read the text as a formula.
fn starts_a_formula(text: &str) -> bool {
    matches!(
        text.chars().next(),
        Some('=') | Some('+') | Some('-') | Some('@') | Some('\t')
    )
}

/// Reports the engines this build supports, so the connection form can
/// show them.
#[tauri::command]
pub fn supported_engines() -> Vec<db::EngineInfo> {
    db::supported_engines()
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
        assert_eq!(error.kind(), crate::error::ErrorKind::Configuration);
        assert!(error.to_string().contains("':id'"));

        let athena = prepare_parameters("SELECT :id", Dialect::Athena, None).unwrap_err();
        assert_eq!(athena.kind(), crate::error::ErrorKind::Configuration);
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
            ScriptKind::Create,
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
            ScriptKind::Create,
            &columns(),
            None,
        )
        .unwrap();
        assert!(text.contains("CREATE TABLE \"t\" ("));
    }

    #[test]
    fn each_kind_builds_its_own_statement() {
        let select = script_text(Dialect::Sqlite, "\"t\"", ScriptKind::Select, &[], None).unwrap();
        assert_eq!(select, "SELECT *\nFROM \"t\";");
        let insert = script_text(
            Dialect::Sqlite,
            "\"t\"",
            ScriptKind::Insert,
            &columns(),
            None,
        )
        .unwrap();
        assert!(insert.starts_with("INSERT INTO \"t\" ("));
        let update = script_text(
            Dialect::Sqlite,
            "\"t\"",
            ScriptKind::Update,
            &columns(),
            None,
        )
        .unwrap();
        assert!(update.starts_with("UPDATE \"t\""));
    }

    #[test]
    fn an_object_without_columns_gives_no_statement() {
        let error = script_text(Dialect::Sqlite, "\"t\"", ScriptKind::Insert, &[], None)
            .expect_err("a statement cannot be built");
        assert!(error.to_string().contains("reports no column"));
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
            open_driver(&connection).await.err().unwrap().kind(),
            crate::error::ErrorKind::Configuration
        );
    }

    #[tokio::test]
    async fn a_sqlite_record_opens_a_driver() {
        let driver = open_driver(&sqlite_connection("file:cmd_test?mode=memory&cache=shared"))
            .await
            .unwrap();
        assert_eq!(driver.dialect(), crate::sql::Dialect::Sqlite);
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
    #[test]
    fn base64_gives_bytes_and_damaged_content_is_refused() {
        // "PK" is the mark that a ZIP container starts with.
        assert_eq!(decode_base64("UEs=").unwrap(), b"PK");
        let error = decode_base64("not base64!").err().unwrap();
        assert_eq!(error.kind(), crate::error::ErrorKind::Configuration);
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
            "id,name\n1,Ada\n2,\n"
        );
        // The temporary file is gone after the rename.
        assert!(!folder.path().join("out.csv.part").exists());

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
        assert!(!folder.path().join("Daily count.xlsx.part").exists());

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
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "id\n1\n");
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
        assert_eq!(csv_field(&json!("+1")), "'+1");
        assert_eq!(csv_field(&json!("-cmd")), "'-cmd");
        assert_eq!(csv_field(&json!("@name")), "'@name");
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
        assert_eq!(error.kind(), crate::error::ErrorKind::Configuration);
        assert!(error.to_string().contains("session limit"));

        // The tab that holds a session keeps it, and the default session
        // stays outside the cap.
        assert!(session_for(app.handle(), &state, "s1", Some("t1"))
            .await
            .is_ok());
        assert!(session_for(app.handle(), &state, "s1", None).await.is_ok());
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
        let fresh = background_driver(&state, "s1", &open).await.unwrap();
        assert!(Arc::ptr_eq(&fresh, &session.driver));
        assert_eq!(pings.load(std::sync::atomic::Ordering::SeqCst), 0);

        session.age(crate::state::HEALTH_CHECK_AFTER).await;
        let checked = background_driver(&state, "s1", &open).await.unwrap();
        assert!(Arc::ptr_eq(&checked, &session.driver));
        assert_eq!(pings.load(std::sync::atomic::Ordering::SeqCst), 1);

        // The ping moved the moment of the last answer, so the next read
        // sends no second ping.
        background_driver(&state, "s1", &open).await.unwrap();
        assert_eq!(pings.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_background_driver_that_stopped_answering_opens_again() {
        let (_dir, descriptor) = temp_sqlite();
        let (_app, state) = state_with_sqlite(descriptor).await;
        let open = state.connection("s1").await.unwrap();
        let (session, pings) = background_stub(&state, false).await;
        session.age(crate::state::HEALTH_CHECK_AFTER).await;

        let opened = background_driver(&state, "s1", &open).await.unwrap();

        assert_eq!(pings.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(!Arc::ptr_eq(&opened, &session.driver));
        // The new driver reaches the database.
        opened.lock().await.ping().await.unwrap();
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
}
