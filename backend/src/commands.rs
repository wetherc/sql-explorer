// backend/src/commands.rs
use crate::{db, error::Error, state::AppState, storage};
use serde_json::Value as JsonValue;

type CommandResult<T> = Result<T, Error>;

#[tauri::command]
pub async fn connect(
    connection_string: String,
    state: tauri::State<'_, AppState>,
) -> CommandResult<()> {
    let client = db::db_connect(&connection_string).await?;
    *state.db.lock().await = Some(client);
    Ok(())
}

#[tauri::command]
pub async fn execute_query(
    query: String,
    state: tauri::State<'_, AppState>,
) -> CommandResult<JsonValue> {
    let mut client_guard = state.db.lock().await;
    let client = client_guard.as_mut().ok_or(Error::NotConnected)?;
    db::db_execute_query(client, &query).await
}

#[tauri::command]
pub async fn list_databases(state: tauri::State<'_, AppState>) -> CommandResult<Vec<db::Database>> {
    let mut client_guard = state.db.lock().await;
    let client = client_guard.as_mut().ok_or(Error::NotConnected)?;
    db::db_list_databases(client).await
}

#[tauri::command]
pub async fn list_schemas(state: tauri::State<'_, AppState>) -> CommandResult<Vec<db::Schema>> {
    let mut client_guard = state.db.lock().await;
    let client = client_guard.as_mut().ok_or(Error::NotConnected)?;
    db::db_list_schemas(client).await
}

#[tauri::command]
pub async fn list_tables(
    schema_name: String,
    state: tauri::State<'_, AppState>,
) -> CommandResult<Vec<db::Table>> {
    let mut client_guard = state.db.lock().await;
    let client = client_guard.as_mut().ok_or(Error::NotConnected)?;
    db::db_list_tables(client, &schema_name).await
}

#[tauri::command]
pub async fn list_columns(
    schema_name: String,
    table_name: String,
    state: tauri::State<'_, AppState>,
) -> CommandResult<Vec<db::Column>> {
    let mut client_guard = state.db.lock().await;
    let client = client_guard.as_mut().ok_or(Error::NotConnected)?;
    db::db_list_columns(client, &schema_name, &table_name).await
}

#[tauri::command]
pub fn list_connections() -> CommandResult<Vec<storage::SavedConnection>> {
    storage::get_all_connections().map_err(Error::from)
}

#[tauri::command]
pub fn save_connection(
    name: String,
    server: String,
    database: String,
    auth_type: storage::AuthType,
    user: Option<String>,
    password: Option<String>,
) -> CommandResult<()> {
    let connection = storage::SavedConnection {
        name: name.clone(),
        server,
        database,
        auth_type,
        user,
    };
    storage::save_connection_details(&connection)?;
    if let Some(password) = password {
        storage::save_password(&name, &password)?;
    } else {
        // If there's no password provided for a SQL Auth connection,
        // we should delete any existing password for this connection.
        if connection.auth_type == storage::AuthType::Sql {
            let _ = storage::delete_password(&name);
        }
    }
    Ok(())
}

#[tauri::command]
pub fn delete_connection(name: String) -> CommandResult<()> {
    storage::delete_connection_details(&name)?;
    storage::delete_password(&name)?;
    Ok(())
}

#[tauri::command]
pub fn get_connection_password(name: String) -> CommandResult<String> {
    storage::get_password(&name).map_err(Error::from)
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use tauri::Manager;
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn test_connect_invalid_string() {
        let app = tauri::test::mock_app();
        app.manage(AppState { db: Mutex::new(None) });
        let state = app.state::<AppState>();
        let connection_string = "server=localhost;user=sa;password=Password123;database=master;TrustServerCertificate=true".to_string();
        let result = connect(connection_string, state).await;
        assert!(result.is_err(), "Connection should fail with an invalid connection string");
    }

    #[tokio::test]
    async fn test_execute_query_without_connection() {
        let app = tauri::test::mock_app();
        app.manage(AppState { db: Mutex::new(None) });
        let state = app.state::<AppState>();
        let query = "SELECT 1".to_string();
        let result = execute_query(query, state).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::NotConnected => (), // Correct error
            _ => panic!("Wrong error type returned"),
        }
    }

    #[tokio::test]
    async fn test_list_databases_without_connection() {
        let app = tauri::test::mock_app();
        app.manage(AppState { db: Mutex::new(None) });
        let state = app.state::<AppState>();
        let result = list_databases(state).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::NotConnected => (),
            _ => panic!("Wrong error type returned"),
        }
    }

    #[tokio::test]
    async fn test_list_schemas_without_connection() {
        let app = tauri::test::mock_app();
        app.manage(AppState { db: Mutex::new(None) });
        let state = app.state::<AppState>();
        let result = list_schemas(state).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::NotConnected => (),
            _ => panic!("Wrong error type returned"),
        }
    }

    #[tokio::test]
    async fn test_list_tables_without_connection() {
        let app = tauri::test::mock_app();
        app.manage(AppState { db: Mutex::new(None) });
        let state = app.state::<AppState>();
        let result = list_tables("dbo".to_string(), state).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::NotConnected => (),
            _ => panic!("Wrong error type returned"),
        }
    }

    #[tokio::test]
    async fn test_list_columns_without_connection() {
        let app = tauri::test::mock_app();
        app.manage(AppState { db: Mutex::new(None) });
        let state = app.state::<AppState>();
        let result = list_columns("dbo".to_string(), "mytable".to_string(), state).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::NotConnected => (),
            _ => panic!("Wrong error type returned"),
        }
    }
}
