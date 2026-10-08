use super::*;
use crate::db::sink::RowSink;
use crate::message_log::MessageTee;
use tauri::Manager;

/// An application of the tests that keeps a registry of message logs.
fn app() -> tauri::App<tauri::test::MockRuntime> {
    let app = tauri::test::mock_app();
    app.manage(MessageLogs::default());
    app
}

#[tokio::test]
async fn a_chosen_file_starts_empty_and_gets_an_identifier() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("messages.txt");
    std::fs::write(&path, "old text").unwrap();
    let logs = MessageLogs::default();
    let chosen = accept_messages_file(&logs, path.clone()).await.unwrap();
    assert_eq!(chosen.path, path.to_string_lossy());
    assert_eq!(logs.path(&chosen.id), Some(path.clone()));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "");

    // A folder that is not there fails at the choice.
    let error = accept_messages_file(&logs, dir.path().join("none").join("x.txt"))
        .await
        .err()
        .unwrap();
    assert!(error.to_string().starts_with("Couldn't create "));
}

#[tokio::test]
async fn a_forgotten_file_gets_no_more_messages() {
    let app = app();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("forget.txt");
    let logs = app.state::<MessageLogs>();
    let id = logs.remember(path.clone());
    let mut tee = MessageTee::new(
        crate::db::sink::BufferSink::new(10),
        Some(logs.start("r1", Some(&id))),
    );
    tee.message(Message::info("kept"));

    forget_messages_file(id.clone(), app.state()).await.unwrap();
    tee.message(Message::info("not kept"));
    assert_eq!(logs.end("r1", None).close(), None);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.ends_with("\nkept\n"));
    assert_eq!(logs.path(&id), None);
}

#[tokio::test]
async fn a_file_chosen_during_a_run_gets_its_messages() {
    let app = app();
    let dir = tempfile::tempdir().unwrap();
    let logs = app.state::<MessageLogs>();
    let mut tee = MessageTee::new(
        crate::db::sink::BufferSink::new(10),
        Some(logs.start("r1", None)),
    );
    tee.message(Message::info("early"));
    let first = logs.remember(dir.path().join("first.txt"));
    let second = logs.remember(dir.path().join("second.txt"));
    assert!(save_run_messages("r1".into(), first, app.state())
        .await
        .unwrap());
    // A second choice replaces the first, which the command closes.
    assert!(save_run_messages("r1".into(), second.clone(), app.state())
        .await
        .unwrap());
    tee.message(Message::info("late"));
    assert_eq!(logs.end("r1", None).close(), None);

    let first = std::fs::read_to_string(dir.path().join("first.txt")).unwrap();
    assert!(first.ends_with("\nearly\n"));
    let second_text = std::fs::read_to_string(dir.path().join("second.txt")).unwrap();
    assert!(second_text.ends_with("\nearly\nlate\n"));

    // A run that ended has nothing to save, and a file that the backend
    // forgot is an error.
    assert!(!save_run_messages("r1".into(), second, app.state())
        .await
        .unwrap());
    assert!(
        save_run_messages("r1".into(), "unknown".into(), app.state())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_writer_that_failed_goes_to_the_log_of_the_backend() {
    let dir = tempfile::tempdir().unwrap();
    // The writer cannot open a folder as a file, and the close reports it.
    close_writers(vec![LogWriter::open(dir.path().to_path_buf())]).await;
}

#[tokio::test]
async fn the_shown_messages_go_to_the_file_with_the_dropped_count() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shown.txt");
    let written = write_messages(path.clone(), vec![Message::warning("careful")], 3)
        .await
        .unwrap();
    assert_eq!(written, path.to_string_lossy());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "-- 3 earlier messages of this run weren't kept\nWarning: careful\n"
    );
    let request: SaveShownMessagesRequest = serde_json::from_value(serde_json::json!({
        "defaultName": "messages.txt",
        "messages": [{ "level": "info", "text": "a", "detail": null }],
    }))
    .unwrap();
    assert_eq!(request.dropped, 0);
    assert_eq!(request.messages, [Message::info("a")]);
    assert_eq!(request.default_name, "messages.txt");
}
