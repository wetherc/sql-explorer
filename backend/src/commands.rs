// backend/src/commands.rs
use crate::{db, error::Error, state::AppState};
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
}
