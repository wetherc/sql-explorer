#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod commands;
mod db;
mod error;
mod state;
mod storage;

use state::AppState;
use tokio::sync::Mutex;

fn main() {
    env_logger::init();
    tauri::Builder::default()
        .plugin(storage::init())
        .manage(AppState {
            db: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            commands::connect,
            commands::execute_query,
            commands::list_databases,
            commands::list_schemas,
            commands::list_tables,
            commands::list_columns,
            commands::get_connections,
            commands::save_connection,
            commands::delete_connection,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
