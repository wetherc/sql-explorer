#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod commands;
mod db;
mod error;
mod state;

use state::AppState;
use tokio::sync::Mutex;

fn main() {
    tauri::Builder::default()
        .manage(AppState {
            db: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            commands::connect,
            commands::execute_query
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
