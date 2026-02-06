#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use futures_util::StreamExt;
use serde_json::Value as JsonValue;
use tiberius::{Client, Config, QueryItem, Row};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

// AppState that holds the database connection
struct AppState {
    db: Mutex<Option<Client<Compat<TcpStream>>>>,
}

#[tauri::command]
async fn connect(
    connection_string: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let config = match Config::from_ado_string(&connection_string) {
        Ok(config) => config,
        Err(e) => return Err(e.to_string()),
    };

    let tcp = match TcpStream::connect(config.get_addr()).await {
        Ok(tcp) => tcp,
        Err(e) => return Err(e.to_string()),
    };
    tcp.set_nodelay(true).unwrap();

    let client = match Client::connect(config, tcp.compat_write()).await {
        Ok(client) => client,
        Err(e) => return Err(e.to_string()),
    };

    *state.db.lock().await = Some(client);
    Ok(())
}

#[tauri::command]
async fn execute_query(
    query: String,
    state: tauri::State<'_, AppState>,
) -> Result<JsonValue, String> {
    let mut client_guard = state.db.lock().await;
    let client = client_guard.as_mut().ok_or("Not connected")?;

    let mut stream = match client.simple_query(query).await {
        Ok(stream) => stream,
        Err(e) => return Err(e.to_string()),
    };

    let mut results = Vec::new();

    while let Some(result) = stream.next().await {
        match result {
            Ok(QueryItem::Row(row)) => {
                results.push(row_to_json(row));
            }
            Ok(_) => {} // Ignore non-row results for now
            Err(e) => return Err(e.to_string()),
        }
    }

    Ok(JsonValue::Array(results))
}

fn row_to_json(row: Row) -> JsonValue {
    let mut map = serde_json::Map::new();
    for (i, col) in row.columns().iter().enumerate() {
        let col_name = col.name().to_string();
        let val = match row.get::<&str, _>(i) {
            Some(s) => JsonValue::String(s.to_string()),
            None => JsonValue::Null,
        };
        map.insert(col_name, val);
    }
    JsonValue::Object(map)
}

fn main() {
    tauri::Builder::default()
        .manage(AppState {
            db: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![connect, execute_query])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::Manager;

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
        assert_eq!(result.unwrap_err(), "Not connected");
    }
}
