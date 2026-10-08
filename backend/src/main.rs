#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod commands;
mod db;
mod error;
mod files;
mod history;
mod jsonfile;
mod kept;
mod menu;
mod message_log;
mod pause;
mod script;
mod secrets;
mod session;
mod spill;
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
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new(secrets::build_store()))
        .manage(commands::run_file::ChosenFiles::default())
        .manage(message_log::MessageLogs::default())
        .setup(|app| {
            spawn_session_reaper(app.handle().clone(), SESSION_REAP_INTERVAL);

            // The spill files of an earlier process go before a run can
            // write a new one. A folder with many files takes time to
            // empty, so the work runs on a thread of its own.
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                use tauri::Manager;
                let kept = &handle.state::<AppState>().kept;
                spill::start_folder(handle.path().app_cache_dir(), kept);
            });

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
            commands::run_file::choose_run_file,
            commands::run_file::run_to_file,
            commands::run_messages::choose_messages_file,
            commands::run_messages::forget_messages_file,
            commands::run_messages::save_run_messages,
            commands::run_messages::save_shown_messages,
            commands::run_file::several_result_sets,
            commands::run_file::run_file_ready,
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
            commands::list_triggers,
            commands::list_events,
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
            commands::passwords_persist,
            commands::clear_history,
            commands::get_workspace,
            commands::save_workspace,
            commands::pick_folder,
            commands::open_statement_file,
            commands::set_menu_commands,
            commands::file_roots,
            commands::close_folder,
            commands::list_folder,
            commands::read_text_file,
            commands::save_statement_file,
            commands::save_text_file,
            commands::save_binary_file,
            commands::export_query,
            commands::export_kept,
            commands::release_kept,
            commands::supported_engines,
            commands::storage_problems,
        ])
        .run(tauri::generate_context!())
        .expect("The application could not start.");
}

#[cfg(test)]
mod tests {
    /// The backend trusts the host of a saved connection and the list of
    /// folder roots, so the webview gets no grant beyond this list. A grant
    /// to write files or to set the title would let script in the webview
    /// change what the user sees or saves.
    #[test]
    fn the_webview_has_only_the_listed_grants() {
        let text = include_str!("../capabilities/default.json");
        let json: serde_json::Value = serde_json::from_str(text).unwrap();
        let grants: Vec<&str> = json["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|grant| grant.as_str().unwrap())
            .collect();
        assert_eq!(
            grants,
            [
                "core:default",
                "core:window:allow-destroy",
                "dialog:allow-open"
            ]
        );
    }
}
