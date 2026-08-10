//! The commands the user interface calls.

use crate::db::drivers::{
    athena::AthenaDriver, mssql::MssqlDriver, mysql::MysqlDriver, postgres::PostgresDriver,
    sqlite::SqliteDriver,
};
use crate::db::{
    self, drivers::DatabaseDriver, AppColumn, Database, ExecOptions, QueryParams, QueryResponse,
    Schema, Table,
};
use crate::error::{Error, Result};
use crate::history::{HistoryEntry, SavedQuery};
use crate::state::{
    AppState, ConnectionHealth, ConnectionInfo, ConnectionStatusEvent, OpenConnection,
    CONNECTION_STATUS_EVENT,
};
use crate::storage::{DbType, SavedConnection};
use crate::store;
use tauri::{AppHandle, Emitter, Runtime};

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

#[tauri::command]
pub async fn execute_query<R: Runtime>(
    app: AppHandle<R>,
    connection_id: String,
    request_id: String,
    query: String,
    query_params: Option<QueryParams>,
    options: Option<ExecOptions>,
    state: tauri::State<'_, AppState>,
) -> Result<QueryResponse> {
    let open = ensure_healthy(&app, &state, &connection_id).await?;
    let options = options.unwrap_or_else(|| open.descriptor.exec_options());
    let token = state.start_request(&request_id).await;

    let outcome = {
        let driver = open.driver.clone();
        let mut guard = driver.lock().await;
        tokio::select! {
            result = guard.execute_query(&query, query_params.as_ref(), &options) => result,
            () = token.cancelled() => Err(Error::Cancelled),
        }
    };

    state.end_request(&request_id).await;

    match outcome {
        Ok(response) => {
            open.mark_ok().await;
            Ok(response)
        }
        Err(Error::Cancelled) => {
            // The statement was dropped in the middle of the exchange with
            // the server, so the connection is no longer in a known state.
            state.remove(&connection_id).await;
            announce(
                &app,
                &connection_id,
                ConnectionHealth::Disconnected,
                Some("The statement was stopped, so the connection was closed.".to_string()),
            );
            Err(Error::Cancelled)
        }
        Err(error) => Err(error),
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
    let open = ensure_healthy(&app, &state, &connection_id).await?;
    let result = open.driver.lock().await.list_databases().await;
    if result.is_ok() {
        open.mark_ok().await;
    }
    result
}

#[tauri::command]
pub async fn list_schemas<R: Runtime>(
    app: AppHandle<R>,
    connection_id: String,
    database: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Schema>> {
    let open = ensure_healthy(&app, &state, &connection_id).await?;
    let result = open.driver.lock().await.list_schemas(&database).await;
    if result.is_ok() {
        open.mark_ok().await;
    }
    result
}

#[tauri::command]
pub async fn list_tables<R: Runtime>(
    app: AppHandle<R>,
    connection_id: String,
    database: String,
    schema_name: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Table>> {
    let open = ensure_healthy(&app, &state, &connection_id).await?;
    let result = open
        .driver
        .lock()
        .await
        .list_tables(&database, schema_name.as_deref())
        .await;
    if result.is_ok() {
        open.mark_ok().await;
    }
    result
}

#[tauri::command]
pub async fn list_columns<R: Runtime>(
    app: AppHandle<R>,
    connection_id: String,
    database: String,
    schema_name: Option<String>,
    table_name: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<AppColumn>> {
    let open = ensure_healthy(&app, &state, &connection_id).await?;
    let result = open
        .driver
        .lock()
        .await
        .list_columns(&database, schema_name.as_deref(), &table_name)
        .await;
    if result.is_ok() {
        open.mark_ok().await;
    }
    result
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

// --- Files ---

/// Writes text to a file the user chose. The dialog runs in the user
/// interface, so this command receives a path the user already accepted.
#[tauri::command]
pub async fn write_text_file(path: String, contents: String) -> Result<()> {
    std::fs::write(&path, contents)?;
    log::info!("Wrote the file '{path}'.");
    Ok(())
}

/// Writes bytes to a file the user chose. The bytes arrive as base64 text,
/// because the raw form of the bridge carries one body and cannot carry the
/// path beside it.
#[tauri::command]
pub async fn write_binary_file(path: String, contents_base64: String) -> Result<()> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(contents_base64.as_bytes())
        .map_err(|error| Error::Configuration(format!("The file content is damaged: {error}")))?;
    std::fs::write(&path, bytes)?;
    log::info!("Wrote the file '{path}'.");
    Ok(())
}

/// Reads a text file the user chose.
#[tauri::command]
pub async fn read_text_file(path: String) -> Result<String> {
    Ok(std::fs::read_to_string(&path)?)
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
    use crate::secrets::MemoryStore;
    use crate::storage::ConnectionOptions;

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
    #[tokio::test]
    async fn bytes_reach_a_file_and_damaged_content_is_refused() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("book.xlsx");
        let target = path.to_string_lossy().to_string();

        // "PK" is the mark that a ZIP container starts with.
        write_binary_file(target.clone(), "UEs=".to_string())
            .await
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"PK");

        let error = write_binary_file(target, "not base64!".to_string())
            .await
            .err()
            .unwrap();
        assert_eq!(error.kind(), crate::error::ErrorKind::Configuration);
    }
}
