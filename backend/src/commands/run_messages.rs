//! The commands that save the messages of a run to a text file. See
//! [`crate::message_log`] for the log of each run.

use super::{ask_save_path, off_thread};
use crate::db::Message;
use crate::error::Result;
use crate::message_log::{messages_text, LogWriter, MessageLogs};
use std::path::PathBuf;
use tauri::{AppHandle, Runtime};

/// The file that the user chose for messages, and the identifier that a
/// run sends back.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChosenMessagesFile {
    pub id: String,
    pub path: String,
}

/// Creates the chosen file, or empties it, so a path that cannot take a
/// write fails at the choice and not during a run. Records the file and
/// gives back its identifier.
async fn accept_messages_file(logs: &MessageLogs, path: PathBuf) -> Result<ChosenMessagesFile> {
    let target = path.clone();
    off_thread(move || {
        std::fs::File::create(&target).map_err(crate::files::on_path("create", &target))?;
        Ok(())
    })
    .await?;
    Ok(ChosenMessagesFile {
        path: path.to_string_lossy().to_string(),
        id: logs.remember(path),
    })
}

/// Asks the user for the file that gets every message of the runs of a
/// tab. Returns `None` when the user closed the dialog.
#[tauri::command]
pub async fn choose_messages_file<R: Runtime>(
    app: AppHandle<R>,
    default_name: String,
    logs: tauri::State<'_, MessageLogs>,
) -> Result<Option<ChosenMessagesFile>> {
    match ask_save_path(&app, &default_name, "Text", "txt", None).await {
        Some(path) => accept_messages_file(&logs, path).await.map(Some),
        None => Ok(None),
    }
}

/// Closes writers on a blocking thread. A write that failed goes to the
/// log of the backend, because the run that used the file no longer has
/// the file.
async fn close_writers(writers: Vec<LogWriter>) {
    let closed = off_thread(move || {
        Ok(writers
            .into_iter()
            .filter_map(LogWriter::close)
            .collect::<Vec<_>>())
    })
    .await;
    for warning in closed.unwrap_or_default() {
        log::warn!("{warning}");
    }
}

/// Forgets a chosen file. A run that writes to it stops writing.
#[tauri::command]
pub async fn forget_messages_file(id: String, logs: tauri::State<'_, MessageLogs>) -> Result<()> {
    close_writers(logs.forget(&id)).await;
    Ok(())
}

/// Sends the messages of a run that goes on to a chosen file: the last
/// messages that the backend kept, then each new message. Returns false
/// when the run already ended.
#[tauri::command]
pub async fn save_run_messages(
    request_id: String,
    file_id: String,
    logs: tauri::State<'_, MessageLogs>,
) -> Result<bool> {
    let (attached, replaced) = logs.attach(&request_id, &file_id)?;
    close_writers(replaced.into_iter().collect()).await;
    Ok(attached)
}

/// What the window sends to save the messages that a tab shows.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveShownMessagesRequest {
    /// The file name that the save dialog suggests.
    pub default_name: String,
    pub messages: Vec<Message>,
    /// The count of the first messages of the run that the tab dropped.
    #[serde(default)]
    pub dropped: u64,
}

/// Writes messages to a file in the format of the message log.
async fn write_messages(path: PathBuf, messages: Vec<Message>, dropped: u64) -> Result<String> {
    let target = path.clone();
    off_thread(move || {
        crate::files::write_bytes(&target, messages_text(&messages, dropped).as_bytes())
    })
    .await?;
    Ok(path.to_string_lossy().to_string())
}

/// Asks the user for a path and writes the messages that a tab shows to
/// it. Returns the path, or `None` when the user closed the dialog.
#[tauri::command]
pub async fn save_shown_messages<R: Runtime>(
    app: AppHandle<R>,
    request: SaveShownMessagesRequest,
) -> Result<Option<String>> {
    let SaveShownMessagesRequest {
        default_name,
        messages,
        dropped,
    } = request;
    match ask_save_path(&app, &default_name, "Text", "txt", None).await {
        Some(path) => write_messages(path, messages, dropped).await.map(Some),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests;
