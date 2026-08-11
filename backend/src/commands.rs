//! The commands the user interface calls.

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
use crate::history::{HistoryEntry, SavedQuery};
use crate::script::{self, ScriptKind};
use crate::sql::ParamValues;
use crate::state::{
    AppState, ConnectionHealth, ConnectionInfo, ConnectionStatusEvent, OpenConnection,
    CONNECTION_STATUS_EVENT,
};
use crate::storage::{DbType, SavedConnection};
use crate::store;
use std::sync::Arc;
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

/// Fills the password of a record from the secret store, unless the caller
/// already gave one.
fn with_password(state: &AppState, mut connection: SavedConnection) -> Result<SavedConnection> {
    if connection.password.is_none() {
        connection.password = state.secrets.get(&connection.id)?;
    }
    Ok(connection)
}

#[tauri::command]
pub async fn connect<R: Runtime>(
    app: AppHandle<R>,
    connection: SavedConnection,
    state: tauri::State<'_, AppState>,
) -> Result<ConnectionInfo> {
    let id = connection.id.clone();
    let full = with_password(&state, connection)?;

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
    let full = with_password(&state, connection)?;
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

/// Confirms that a connection that stood idle still answers, and opens it
/// again when it does not.
async fn ensure_healthy<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    connection_id: &str,
) -> Result<OpenConnection> {
    let open = state.connection(connection_id).await?;
    if !open.needs_ping || !open.needs_check().await {
        return Ok(open);
    }

    // One check at a time for each connection. A command that waited here
    // takes the connection that the first check left behind, so two
    // commands never open the same connection twice.
    let lock = state.health_check_lock(connection_id).await;
    let _guard = lock.lock().await;
    let open = state.connection(connection_id).await?;
    if !open.needs_check().await {
        return Ok(open);
    }

    let healthy = {
        let mut driver = open.driver.lock().await;
        driver.ping().await.is_ok()
    };
    if healthy {
        open.mark_ok().await;
        return Ok(open);
    }

    announce(app, connection_id, ConnectionHealth::Reconnecting, None);
    log::warn!("The connection '{connection_id}' stopped answering. Opening it again.");

    let full = with_password(state, open.descriptor.clone())?;
    match open_driver(&full).await {
        Ok(driver) => {
            let replacement = OpenConnection::new(full, driver);
            state.insert(connection_id, replacement.clone()).await;
            // The background driver shares the fate of the session that
            // stopped answering, so the next metadata read opens a new one.
            state.clear_background(connection_id).await;
            announce(app, connection_id, ConnectionHealth::Connected, None);
            Ok(replacement)
        }
        Err(error) => {
            state.remove(connection_id).await;
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

/// Returns the driver that a metadata read runs on. The read goes to a
/// second connection when one can open, so that the tree of the explorer
/// does not wait behind a statement of the user.
async fn metadata_driver<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    connection_id: &str,
) -> Result<crate::state::BackgroundDriver> {
    let open = ensure_healthy(app, state, connection_id).await?;
    Ok(background_driver(state, connection_id, &open).await)
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

/// The time the Stop button gives one connection to report the failure that
/// the server sends it. A connection with no way to ask the server to stop
/// gets none, because no such failure is coming.
fn stop_grace(open: &OpenConnection) -> std::time::Duration {
    if open.cancel_handle.is_some() {
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

#[tauri::command]
pub async fn execute_query<R: Runtime>(
    app: AppHandle<R>,
    connection_id: String,
    request_id: String,
    query: String,
    query_params: Option<ParamValues>,
    options: Option<ExecOptions>,
    state: tauri::State<'_, AppState>,
) -> Result<QueryResponse> {
    let open = ensure_healthy(&app, &state, &connection_id).await?;
    let options = options.unwrap_or_else(|| open.descriptor.exec_options());
    let (query, bound) = prepare_parameters(&query, open.dialect, query_params.as_ref())?;
    let token = state.start_request(&request_id).await;

    let outcome = {
        let driver = open.driver.clone();
        let mut guard = driver.lock().await;
        run_bounded(
            guard.execute_query(&query, bound.as_ref(), &options),
            &token,
            options.timeout_secs,
            stop_grace(&open),
        )
        .await
    };

    state.end_request(&request_id).await;
    finish_run(&app, &state, &connection_id, &open, outcome).await
}

/// What one request for a plan carries.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanRequest {
    pub connection_id: String,
    pub request_id: String,
    pub query: String,
    pub kind: PlanKind,
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
        query_params,
        options,
    } = request;
    let open = ensure_healthy(&app, &state, &connection_id).await?;
    let options = options.unwrap_or_else(|| open.descriptor.exec_options());
    // A plan needs the values of the parameters, because the plan of a
    // statement depends on the values it holds.
    let (query, bound) = prepare_parameters(&query, open.dialect, query_params.as_ref())?;
    let token = state.start_request(&request_id).await;

    let outcome = {
        let driver = open.driver.clone();
        let mut guard = driver.lock().await;
        run_bounded(
            guard.explain(&query, bound.as_ref(), kind, &options),
            &token,
            options.timeout_secs,
            stop_grace(&open),
        )
        .await
    };

    state.end_request(&request_id).await;
    finish_run(&app, &state, &connection_id, &open, outcome).await
}

/// Closes the accounts of one exchange. A limit that ended the exchange asks
/// the server to stop the statement, and the connection then goes unless the
/// driver reports that it stays fit for use.
async fn finish_run<R: Runtime, T>(
    app: &AppHandle<R>,
    state: &AppState,
    connection_id: &str,
    open: &OpenConnection,
    outcome: Bounded<T>,
) -> Result<T> {
    match outcome {
        Bounded::Answered(Ok(value)) => {
            open.mark_ok().await;
            Ok(value)
        }
        Bounded::Answered(Err(error)) => Err(error),
        Bounded::Stopped(error) => {
            // The wait ended, but the server may still run the statement.
            // The handle asks the server to stop it. A second request for a
            // statement that already stopped does no harm.
            if let Some(handle) = open.cancel_handle.clone() {
                if let Err(stop_error) = handle.cancel().await {
                    log::warn!("The server did not stop the statement: {stop_error}");
                }
            }
            if open.keeps_connection_after_stop {
                return Err(error);
            }
            // The exchange was dropped in the middle of a message, so nothing
            // can be sent on this connection again. A new one goes in its
            // place at once, so the user is not left with a tab that cannot
            // run anything.
            reopen_after_stop(app, state, connection_id, open, &error).await;
            Err(error)
        }
    }
}

/// Puts a new connection in the place of one that a limit left unusable.
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
    error: &Error,
) {
    announce(app, connection_id, ConnectionHealth::Reconnecting, None);

    let full = match with_password(state, open.descriptor.clone()) {
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
            state
                .insert(connection_id, OpenConnection::new(full, driver))
                .await;
            // The background driver shared the session that has gone, so the
            // next metadata read opens one of its own.
            state.clear_background(connection_id).await;
            announce(app, connection_id, ConnectionHealth::Connected, None);
            log::info!("The connection '{connection_id}' was opened again after a stop.");
        }
        Err(open_error) => {
            state.remove(connection_id).await;
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
#[tauri::command]
pub async fn cancel_query(
    connection_id: String,
    request_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<()> {
    // The handle does not need the lock of the driver, so it works while
    // the statement runs.
    if let Ok(open) = state.connection(&connection_id).await {
        if let Some(handle) = open.cancel_handle.clone() {
            if let Err(error) = handle.cancel().await {
                log::warn!("The server did not stop the statement: {error}");
            }
        }
    }
    state.cancel_request(&request_id).await;
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
#[tauri::command]
pub async fn schema_snapshot<R: Runtime>(
    app: AppHandle<R>,
    connection_id: String,
    database: String,
    max_columns: Option<usize>,
    own_connection: Option<bool>,
    state: tauri::State<'_, AppState>,
) -> Result<SchemaSnapshot> {
    let open = ensure_healthy(&app, &state, &connection_id).await?;
    let limit = max_columns.unwrap_or(DEFAULT_SNAPSHOT_COLUMNS).max(1);

    let driver = match own_connection.unwrap_or(true) {
        true => background_driver(&state, &connection_id, &open).await,
        false => open.driver.clone(),
    };

    let mut guard = driver.lock().await;
    guard.schema_snapshot(&database, limit).await
}

/// Returns the background driver of a connection, and opens one when the
/// connection has none. A driver that cannot open gives the driver of the
/// user, because a snapshot that waits is better than no completions.
async fn background_driver(
    state: &AppState,
    connection_id: &str,
    open: &OpenConnection,
) -> Arc<tokio::sync::Mutex<Box<dyn DatabaseDriver>>> {
    if let Some(driver) = state.background_driver(connection_id).await {
        return driver;
    }
    let full = match with_password(state, open.descriptor.clone()) {
        Ok(full) => full,
        Err(error) => {
            log::warn!("The password of '{connection_id}' could not be read: {error}");
            return open.driver.clone();
        }
    };
    match open_driver(&full).await {
        Ok(driver) => state.set_background_driver(connection_id, driver).await,
        Err(error) => {
            log::warn!(
                "A second connection for '{connection_id}' could not open, so the schema is \
                 read on the connection of the user: {error}"
            );
            open.driver.clone()
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

    let mut guard = open.driver.lock().await;
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
    let text = script_text(dialect, &name, script_kind, &columns, from_engine)?;
    open.mark_ok().await;
    Ok(text)
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
    connection_id: String,
    database: Option<String>,
    schema_name: Option<String>,
    table_name: String,
    limit: Option<usize>,
    state: tauri::State<'_, AppState>,
) -> Result<String> {
    let open = state.connection(&connection_id).await?;
    Ok(open.dialect.preview_query(
        database.as_deref(),
        schema_name.as_deref(),
        &table_name,
        limit.unwrap_or(1000),
    ))
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

    match connection.password.as_deref() {
        Some(password) if !password.is_empty() => {
            state.secrets.set(&connection.id, password)?;
        }
        Some(_) => state.secrets.delete(&connection.id)?,
        None => {}
    }

    store::write_connection(&app, &connection.without_password())
}

#[tauri::command]
pub async fn delete_connection<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    state: tauri::State<'_, AppState>,
) -> Result<()> {
    state.remove(&id).await;
    let _ = state.secrets.delete(&id);
    store::delete_connection(&app, &id)
}

// --- The query history and the saved queries ---

#[tauri::command]
pub async fn get_history<R: Runtime>(app: AppHandle<R>) -> Result<Vec<HistoryEntry>> {
    store::read_history(&app)
}

#[tauri::command]
pub async fn add_history_entry<R: Runtime>(
    app: AppHandle<R>,
    entry: HistoryEntry,
) -> Result<Vec<HistoryEntry>> {
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
) -> Option<std::path::PathBuf> {
    use tauri_plugin_dialog::DialogExt;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(default_name)
        .add_filter(filter_label, &[extension])
        .save_file(move |path| {
            let _ = sender.send(path);
        });
    receiver
        .await
        .ok()
        .flatten()
        .and_then(|path| path.into_path().ok())
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
/// straight to a file. A large result therefore never passes through the
/// user interface.
///
/// The statement must only read, because an export runs it a second time.
/// The rows still gather in the memory of the backend, because a driver
/// gives the whole result set at once.
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
        query_params,
    } = request;

    let (label, extension) = match format {
        ExportFormat::Csv => ("CSV", "csv"),
        ExportFormat::Json => ("JSON", "json"),
    };
    let Some(path) = ask_save_path(&app, &default_name, label, extension).await else {
        return Ok(None);
    };

    let open = ensure_healthy(&app, &state, &connection_id).await?;
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
    let token = state.start_request(&request_id).await;

    let outcome = {
        let driver = open.driver.clone();
        let mut guard = driver.lock().await;
        run_bounded(
            guard.execute_query(&query, bound.as_ref(), &options),
            &token,
            options.timeout_secs,
            stop_grace(&open),
        )
        .await
    };
    state.end_request(&request_id).await;
    let response = finish_run(&app, &state, &connection_id, &open, outcome).await?;

    let result =
        response.results.into_iter().next().ok_or_else(|| {
            Error::Unsupported("The statement returned no result set.".to_string())
        })?;

    let rows = result.rows.len();
    let truncated = result.truncated;
    write_result_file(&path, &result, format)?;
    let path = path.to_string_lossy().to_string();
    log::info!("Wrote {rows} rows to the file '{path}'.");
    Ok(Some(ExportSummary {
        rows,
        truncated,
        path,
    }))
}

/// Writes one result set to a file, one row at a time.
fn write_result_file(
    path: &std::path::Path,
    result: &crate::db::ResultSet,
    format: ExportFormat,
) -> Result<()> {
    use std::io::Write;
    let file = std::fs::File::create(path)?;
    let mut out = std::io::BufWriter::new(file);

    match format {
        ExportFormat::Csv => {
            let names: Vec<String> = result
                .columns
                .iter()
                .map(|column| csv_field(&serde_json::Value::String(column.name.clone())))
                .collect();
            writeln!(out, "{}", names.join(","))?;
            for row in &result.rows {
                let fields: Vec<String> = row.iter().map(csv_field).collect();
                writeln!(out, "{}", fields.join(","))?;
            }
        }
        ExportFormat::Json => {
            let names = crate::db::unique_column_names(&result.columns);
            writeln!(out, "[")?;
            for (index, row) in result.rows.iter().enumerate() {
                let mut object = serde_json::Map::new();
                for (position, name) in names.iter().enumerate() {
                    let value = row
                        .get(position)
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    object.insert(name.clone(), value);
                }
                let comma = if index + 1 == result.rows.len() {
                    ""
                } else {
                    ","
                };
                writeln!(out, "  {}{comma}", serde_json::Value::Object(object))?;
            }
            writeln!(out, "]")?;
        }
    }
    out.flush()?;
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

        let filled = with_password(&state, sqlite_connection("/tmp/a.db")).unwrap();
        assert_eq!(filled.password.as_deref(), Some("from-the-store"));

        let mut given = sqlite_connection("/tmp/a.db");
        given.password = Some("typed".into());
        let kept = with_password(&state, given).unwrap();
        assert_eq!(kept.password.as_deref(), Some("typed"));
    }

    #[tokio::test]
    async fn a_password_that_is_absent_stays_absent() {
        let state = state();
        let filled = with_password(&state, sqlite_connection("/tmp/a.db")).unwrap();
        assert_eq!(filled.password, None);
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
        use crate::db::{ColumnInfo, ResultSet};
        let mut result = ResultSet::new(vec![
            ColumnInfo::new("id", "int"),
            ColumnInfo::new("name", "text"),
        ]);
        result.rows = vec![
            vec![serde_json::json!(1), serde_json::json!("Ada")],
            vec![serde_json::json!(2), serde_json::json!(null)],
        ];

        let folder = tempfile::tempdir().unwrap();
        let csv = folder.path().join("out.csv");
        write_result_file(&csv, &result, ExportFormat::Csv).unwrap();
        assert_eq!(
            std::fs::read_to_string(&csv).unwrap(),
            "id,name\n1,Ada\n2,\n"
        );

        let json = folder.path().join("out.json");
        write_result_file(&json, &result, ExportFormat::Json).unwrap();
        let text = std::fs::read_to_string(&json).unwrap();
        assert!(text.starts_with("[\n"));
        assert!(text.contains("{\"id\":1,\"name\":\"Ada\"},"));
        assert!(text.trim_end().ends_with("]"));
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
}
