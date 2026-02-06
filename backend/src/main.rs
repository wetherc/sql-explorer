#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use tiberius::{Client, Config};
use tokio::net::TcpStream;
use tokio_util::compat::TokioAsyncWriteCompatExt;

#[tauri::command]
async fn connect(connection_string: String) -> Result<(), String> {
    let config = match Config::from_ado_string(&connection_string) {
        Ok(config) => config,
        Err(e) => return Err(e.to_string()),
    };

    let tcp = match TcpStream::connect(config.get_addr()).await {
        Ok(tcp) => tcp,
        Err(e) => return Err(e.to_string()),
    };
    tcp.set_nodelay(true).unwrap();

    match Client::connect(config, tcp.compat_write()).await {
        Ok(_) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![connect])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_connect_invalid_string() {
        let connection_string = "server=localhost;user=sa;password=Password123;database=master;TrustServerCertificate=true".to_string();
        let result = connect(connection_string).await;
        assert!(result.is_err(), "Connection should fail with an invalid connection string");
    }
}
