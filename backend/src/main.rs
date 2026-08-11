#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod commands;
mod db;
mod error;
mod files;
mod history;
mod menu;
mod script;
mod secrets;
mod session;
mod sql;
mod state;
mod storage;
mod store;
mod xlsx;

use state::{spawn_session_reaper, AppState, SESSION_REAP_INTERVAL};
use tauri::Emitter;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // Two crates in the dependency tree bring their own cryptography, so
    // the one to use is named here. Without this the TLS handshake of
    // PostgreSQL fails with a message about a missing provider.
    if rustls::crypto::ring::default_provider()
        .install_default()
        .is_err()
    {
        log::debug!("A cryptography provider was already in place.");
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new(secrets::build_store()))
        .setup(|app| {
            spawn_session_reaper(app.handle().clone(), SESSION_REAP_INTERVAL);

            // The menu of the operating system holds the commands of a file
            // beside the items the platform expects. A click sends the
            // identifier of the command to the window, which runs it.
            app.set_menu(menu::build(app.handle())?)?;
            app.on_menu_event(|app, event| {
                let id = event.id().as_ref();
                if !menu::names_a_command(id) {
                    return;
                }
                if let Err(error) = app.emit(menu::MENU_COMMAND_EVENT, id) {
                    log::warn!("The menu command '{id}' did not reach the window: {error}");
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::connect,
            commands::test_connection,
            commands::disconnect,
            commands::list_active_connections,
            commands::execute_query,
            commands::explain_query,
            commands::query_parameters,
            commands::cancel_query,
            commands::release_session,
            commands::list_databases,
            commands::list_schemas,
            commands::list_tables,
            commands::list_columns,
            commands::list_routines,
            commands::list_indexes,
            commands::list_constraints,
            commands::list_partitions,
            commands::table_details,
            commands::schema_snapshot,
            commands::preview_query,
            commands::script_object,
            commands::quote_identifier,
            commands::get_connections,
            commands::save_connection,
            commands::delete_connection,
            commands::get_history,
            commands::add_history_entry,
            commands::clear_history,
            commands::get_saved_queries,
            commands::save_query,
            commands::delete_saved_query,
            commands::get_workspace,
            commands::save_workspace,
            commands::pick_folder,
            commands::open_statement_file,
            commands::set_menu_commands,
            commands::restore_folder,
            commands::list_folder,
            commands::read_text_file,
            commands::write_text_file,
            commands::save_statement_file,
            commands::save_text_file,
            commands::save_binary_file,
            commands::export_query,
            commands::supported_engines,
        ])
        .run(tauri::generate_context!())
        .expect("The application could not start.");
}
