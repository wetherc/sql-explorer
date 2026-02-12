// backend/src/commands.rs
use crate::{
    db::{
        self,
        drivers::{
            mssql::MssqlDriver, mysql::MysqlDriver, postgres::PostgresDriver,
        }, QueryParams,
    },
    error::Error,
    state::AppState,
    storage,
};

type CommandResult<T> = Result<T, Error>;

#[tauri::command]
pub async fn connect(
    connection_string: String,
    db_type: storage::DbType,
    state: tauri::State<'_, AppState>,
) -> CommandResult<()> {
    // Only log the database type, not the full connection string or server
    // to avoid logging sensitive information like credentials.
    let logged_db_type = format!("{:?}", db_type);

    let client_result = match db_type {
        storage::DbType::Mssql => MssqlDriver::connect(&connection_string).await,
        storage::DbType::Mysql => MysqlDriver::connect(&connection_string).await,
        storage::DbType::Postgres => PostgresDriver::connect(&connection_string).await,
    };

    match client_result {
        Ok(client) => {
            *state.db.lock().await = Some(client);
            log::info!("Successfully connected to {} database.", logged_db_type);
            Ok(())
        },
        Err(e) => {
            log::error!("Failed to connect to {} database: {:?}", logged_db_type, e);
            Err(e)
        }
    }
}

#[tauri::command]
pub async fn execute_query(
    query: String,
    query_params: Option<QueryParams>,
    state: tauri::State<'_, AppState>,
) -> CommandResult<db::QueryResponse> {
    let mut client_guard = state.db.lock().await;
    let client = client_guard.as_mut().ok_or(Error::NotConnected)?;
    client.execute_query(&query, query_params.as_ref()).await
}

#[tauri::command]
pub async fn list_databases(state: tauri::State<'_, AppState>) -> CommandResult<Vec<db::Database>> {
    let mut client_guard = state.db.lock().await;
    let client = client_guard.as_mut().ok_or(Error::NotConnected)?;
    client.list_databases().await
}

#[tauri::command]
pub async fn list_schemas(database: String, state: tauri::State<'_, AppState>) -> CommandResult<Vec<db::Schema>> {
    let mut client_guard = state.db.lock().await;
    let client = client_guard.as_mut().ok_or(Error::NotConnected)?;
    // The database parameter is not directly used for all drivers, but is kept for API consistency
    client.list_schemas().await
}

#[tauri::command]
pub async fn list_tables(
    database: String,
    schema_name: Option<String>,
    state: tauri::State<'_, AppState>,
) -> CommandResult<Vec<db::Table>> {
    let mut client_guard = state.db.lock().await;
    let client = client_guard.as_mut().ok_or(Error::NotConnected)?;
    
    // For MySQL, the schema_name is not used, and the database is the primary parameter.
    // For other drivers like Postgres/MSSQL, schema_name is the key.
    // The driver implementation will handle the logic.
    let schema_param = schema_name.as_deref().unwrap_or(&database);
    client.list_tables(schema_param).await
}

#[tauri::command]
pub async fn list_columns(
    schema_name: String,
    table_name: String,
    state: tauri::State<'_, AppState>,
) -> CommandResult<Vec<db::AppColumn>> {
    let mut client_guard = state.db.lock().await;
    let client = client_guard.as_mut().ok_or(Error::NotConnected)?;
    client.list_columns(&schema_name, &table_name).await
}

#[tauri::command]
pub fn list_connections() -> CommandResult<Vec<storage::SavedConnection>> {
    storage::get_all_connections().map_err(Error::from)
}

#[tauri::command]
pub fn save_connection(
    name: String,
    db_type: storage::DbType,
    server: String,
    database: String,
    auth_type: storage::AuthType,
    user: Option<String>,
    password: Option<String>,
) -> CommandResult<()> {
    let connection = storage::SavedConnection {
        name: name.clone(),
        db_type,
        server,
        database,
        auth_type,
        user,
    };
    let result = storage::save_connection_details(&connection);
    if let Err(e) = result {
        log::error!("Failed to save connection details for '{}': {:?}", connection.name, e);
        return Err(e.into());
    }
    log::info!("Connection details for '{}' saved successfully.", connection.name);

    if let Some(password) = password {
        let result = storage::save_password(&name, &password);
        if let Err(e) = result {
            log::error!("Failed to save password for connection '{}': {:?}", name, e);
            return Err(e.into());
        }
        log::info!("Password for connection '{}' saved successfully (password not logged).", name);
    } else {
        if connection.auth_type == storage::AuthType::Sql {
            let _ = storage::delete_password(&name);
            log::info!("Existing password for SQL Auth connection '{}' deleted (no new password provided).", name);
        }
    }
    Ok(())
}

#[tauri::command]
pub fn delete_connection(name: String) -> CommandResult<()> {
    let result_details = storage::delete_connection_details(&name);
    let result_password = storage::delete_password(&name);

    if let Err(e) = result_details {
        log::error!("Failed to delete connection details for '{}': {:?}", name, e);
        return Err(e.into());
    }
    if let Err(e) = result_password {
        log::error!("Failed to delete password for connection '{}': {:?}", name, e);
        return Err(e.into());
    }

    log::info!("Connection '{}' and its associated password successfully deleted.", name);
    Ok(())
}

#[tauri::command]
pub fn get_connection_password(name: String) -> CommandResult<String> {
    let password_result = storage::get_password(&name);
    match password_result {
        Ok(password) => {
            log::info!("Password successfully retrieved for connection '{}' (password not logged).", name);
            Ok(password)
        },
        Err(e) => {
            log::error!("Failed to retrieve password for connection '{}': {:?}", name, e);
            Err(e.into())
        }
    }
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
        let connection_string = "server=localhost;user=sa;password=Password123;database=master;TrustServerCertificate=true".to_string();
        let result = MssqlDriver::connect(&connection_string).await;
        assert!(result.is_err(), "Connection should fail with an invalid connection string");
    }

    #[tokio::test]
    async fn test_execute_query_without_connection() {
        let app = tauri::test::mock_app();
        app.manage(AppState { db: Mutex::new(None) });
        let state = app.state::<AppState>();
        let query = "SELECT 1".to_string();
        let result = execute_query(query, None, state).await;
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
        let result = list_schemas("test_db".to_string(), state).await;
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
        let result = list_tables("test_db".to_string(), Some("dbo".to_string()), state).await;
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

    #[tokio::test]
    async fn test_save_connection_with_db_type() {
        // cleanup before test
        let _ = storage::delete_connection_details("test_mysql");
        // This test mainly checks that the command can be called without panicking.
        // A more thorough test would involve checking the filesystem, which is out of scope here.
        let result = save_connection(
            "test_mysql".to_string(),
            storage::DbType::Mysql,
            "localhost".to_string(),
            "test_db".to_string(),
            storage::AuthType::Sql,
            Some("user".to_string()),
            Some("password".to_string()),
        );
        assert!(result.is_ok());
        // cleanup after test
        let _ = storage::delete_connection_details("test_mysql");
    }

    // --- M5.5 Connection Tests ---

    // Helper to get connection string from env
    fn get_env_conn_string(env_var: &str, db_type: storage::DbType) -> Option<String> {
        match std::env::var(env_var) {
            Ok(s) => Some(s),
            Err(_) => {
                eprintln!(
                    "Skipping connect test for {:?}: {} not set.",
                    db_type, env_var
                );
                None
            }
        }
    }

    #[tokio::test]
    async fn test_connect_mssql_successful() {
        let connection_string = match get_env_conn_string("MSSQL_TEST_DB_URL", storage::DbType::Mssql) {
            Some(s) => s,
            None => return, // Skip test
        };

        let app = tauri::test::mock_app();
        app.manage(AppState { db: Mutex::new(None) });
        
        let result = connect(
            connection_string,
            storage::DbType::Mssql,
            app.state::<AppState>(), // Get a new state instance for the command
        ).await;

        let state_for_assertion = app.state::<AppState>(); // Get a fresh state for assertion
        assert!(result.is_ok(), "Failed to connect to MS SQL: {:?}", result.unwrap_err());
        assert!(state_for_assertion.db.lock().await.is_some());
    }

    #[tokio::test]
    async fn test_connect_mysql_successful() {
        let connection_string = match get_env_conn_string("MYSQL_TEST_DB_URL", storage::DbType::Mysql) {
            Some(s) => s,
            None => return, // Skip test
        };

        let app = tauri::test::mock_app();
        app.manage(AppState { db: Mutex::new(None) });
        
        let result = connect(
            connection_string,
            storage::DbType::Mysql,
            app.state::<AppState>(), // Get a new state instance for the command
        ).await;

        let state_for_assertion = app.state::<AppState>(); // Get a fresh state for assertion
        assert!(result.is_ok(), "Failed to connect to MySQL: {:?}", result.unwrap_err());
        assert!(state_for_assertion.db.lock().await.is_some());
    }

    #[tokio::test]
    async fn test_connect_postgres_successful() {
        let connection_string = match get_env_conn_string("POSTGRES_TEST_DB_URL", storage::DbType::Postgres) {
            Some(s) => s,
            None => return, // Skip test
        };

        let app = tauri::test::mock_app();
        app.manage(AppState { db: Mutex::new(None) });
        
        let result = connect(
            connection_string,
            storage::DbType::Postgres,
            app.state::<AppState>(), // Get a new state instance for the command
        ).await;

        let state_for_assertion = app.state::<AppState>(); // Get a fresh state for assertion
        assert!(result.is_ok(), "Failed to connect to PostgreSQL: {:?}", result.unwrap_err());
        assert!(state_for_assertion.db.lock().await.is_some());
    }
}
