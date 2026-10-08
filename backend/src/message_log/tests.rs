use super::*;
use serde_json::json;
use std::time::Duration;

fn info(text: &str) -> Message {
    Message::info(text)
}

/// The text of a file after the writers of the test closed it.
fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

/// The lines of a file after its header line.
fn body(path: &Path) -> Vec<String> {
    let text = read(path);
    let mut lines = text.lines();
    assert!(lines.next().unwrap().starts_with("-- Run started "));
    lines.map(str::to_string).collect()
}

#[test]
fn a_message_gives_its_level_and_an_indented_detail() {
    assert_eq!(message_lines(&info("3 rows affected")), "3 rows affected\n");
    let warning = Message {
        level: MessageLevel::Warning,
        text: "Null value is eliminated".into(),
        detail: Some("Severity 10\nLine 4".into()),
    };
    assert_eq!(
        message_lines(&warning),
        "Warning: Null value is eliminated\n    Severity 10\n    Line 4\n"
    );
    let error = Message {
        level: MessageLevel::Error,
        text: "Divide by zero".into(),
        detail: None,
    };
    assert_eq!(message_lines(&error), "Error: Divide by zero\n");
}

#[test]
fn the_note_counts_the_messages_that_were_not_kept() {
    assert_eq!(
        dropped_note(1),
        "-- 1 earlier message of this run wasn't kept\n"
    );
    assert_eq!(
        dropped_note(12),
        "-- 12 earlier messages of this run weren't kept\n"
    );
    assert_eq!(messages_text(&[info("a"), info("b")], 0), "a\nb\n");
    assert_eq!(
        messages_text(&[info("c")], 2),
        "-- 2 earlier messages of this run weren't kept\nc\n"
    );
    assert!(run_header("2026-10-08 10:00:00").starts_with("-- Run started 2026-10-08"));
    assert_eq!(now().len(), "2026-10-08 10:00:00".len());
}

#[test]
fn a_writer_appends_to_the_file_and_closes_without_a_warning() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("messages.txt");
    std::fs::write(&path, "before\n").unwrap();
    let writer = LogWriter::open(path.clone());
    writer.write("one\n".into());
    // The thread empties the queue and writes the buffer to the disk
    // before the close, so a reader sees the line during the run.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while read(&path) != "before\none\n" {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    writer.write("two\n".into());
    assert_eq!(writer.close(), None);
    assert_eq!(read(&path), "before\none\ntwo\n");
}

#[test]
fn a_writer_that_cannot_open_its_file_gives_a_warning() {
    let dir = tempfile::tempdir().unwrap();
    let writer = LogWriter::open(dir.path().to_path_buf());
    writer.write("lost\n".into());
    let warning = writer.close().unwrap();
    assert!(warning.starts_with("Couldn't save the messages to "));
    assert!(warning.contains(&dir.path().display().to_string()));
}

#[test]
fn a_writer_that_stopped_gives_a_warning() {
    let writer = LogWriter {
        path: PathBuf::from("gone.txt"),
        lines: None,
        thread: Some(std::thread::spawn(|| panic!("the writer panics"))),
    };
    assert_eq!(
        writer.close().as_deref(),
        Some("Couldn't save the messages to gone.txt: the writer stopped")
    );
    let closed = LogWriter {
        path: PathBuf::from("closed.txt"),
        lines: None,
        thread: None,
    };
    closed.write("nowhere\n".into());
    assert_eq!(closed.close(), None);
}

#[test]
fn a_run_with_a_file_writes_each_message_and_the_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.txt");
    let logs = MessageLogs::default();
    let id = logs.remember(path.clone());
    assert_eq!(logs.path(&id), Some(path.clone()));

    let log = logs.start("r1", Some(&id));
    lock(&log).record(&info("first"));
    lock(&log).record(&Message::warning("second"));
    let ending = logs.end("r1", Some(&Error::Invalid("The run failed.".into())));
    assert_eq!(ending.close(), None);
    assert_eq!(
        body(&path),
        ["first", "Warning: second", "Error: The run failed."]
    );

    // The next run appends its own header and messages.
    let log = logs.start("r2", Some(&id));
    lock(&log).record(&info("third"));
    assert_eq!(logs.end("r2", None).close(), None);
    let text = read(&path);
    assert_eq!(text.matches("-- Run started ").count(), 2);
    let second = text.rsplit("-- Run started ").next().unwrap();
    assert_eq!(second.lines().skip(1).collect::<Vec<_>>(), ["third"]);
}

#[test]
fn a_run_without_a_file_writes_nothing_and_its_end_is_quiet() {
    let logs = MessageLogs::default();
    let log = logs.start("r1", None);
    lock(&log).record(&info("kept"));
    assert_eq!(logs.end("r1", Some(&Error::Cancelled)).close(), None);
    // A second end of the same run finds nothing.
    assert_eq!(logs.end("r1", None).close(), None);
}

#[test]
fn a_run_with_a_file_that_the_backend_forgot_warns_at_its_end() {
    let logs = MessageLogs::default();
    logs.start("r1", Some("unknown"));
    let warning = logs.end("r1", None).close().unwrap();
    assert!(warning.contains("no longer available"));
}

#[test]
fn a_file_chosen_during_the_run_gets_the_kept_messages_first() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("late.txt");
    let logs = MessageLogs::default();
    let log = logs.start("r1", None);
    for number in 0..KEPT_RUN_MESSAGES + 2 {
        lock(&log).record(&info(&number.to_string()));
    }
    let id = logs.remember(path.clone());
    let (attached, replaced) = logs.attach("r1", &id).unwrap();
    assert!(attached);
    assert!(replaced.is_none());
    lock(&log).record(&info("after"));
    assert_eq!(logs.end("r1", None).close(), None);

    let lines = body(&path);
    assert_eq!(lines[0], "-- 2 earlier messages of this run weren't kept");
    assert_eq!(lines[1], "2");
    assert_eq!(lines.len(), KEPT_RUN_MESSAGES + 2);
    assert_eq!(lines.last().unwrap(), "after");
}

#[test]
fn a_second_file_replaces_the_first_and_clears_the_warning() {
    let dir = tempfile::tempdir().unwrap();
    let logs = MessageLogs::default();
    let log = logs.start("r1", Some("unknown"));
    lock(&log).record(&info("one"));
    let first = logs.remember(dir.path().join("first.txt"));
    let second = logs.remember(dir.path().join("second.txt"));
    let (_, replaced) = logs.attach("r1", &first).unwrap();
    assert!(replaced.is_none());
    let (_, replaced) = logs.attach("r1", &second).unwrap();
    assert_eq!(replaced.unwrap().close(), None);
    assert_eq!(logs.end("r1", None).close(), None);
    assert_eq!(body(&dir.path().join("first.txt")), ["one"]);
    assert_eq!(body(&dir.path().join("second.txt")), ["one"]);
}

#[test]
fn an_attach_to_an_ended_run_or_with_an_unknown_file_does_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let logs = MessageLogs::default();
    let id = logs.remember(dir.path().join("x.txt"));
    let (attached, replaced) = logs.attach("gone", &id).unwrap();
    assert!(!attached);
    assert!(replaced.is_none());
    let error = logs.attach("gone", "unknown").err().unwrap();
    assert!(matches!(error, Error::Invalid(text) if text.contains("expired")));
}

#[test]
fn a_forgotten_file_stops_the_runs_that_write_to_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("stop.txt");
    let logs = MessageLogs::default();
    let id = logs.remember(path.clone());
    let other = logs.remember(dir.path().join("other.txt"));
    let log = logs.start("r1", Some(&id));
    logs.start("r2", Some(&other));
    logs.start("r3", None);
    lock(&log).record(&info("before"));

    let writers = logs.forget(&id);
    assert_eq!(writers.len(), 1);
    for writer in writers {
        assert_eq!(writer.close(), None);
    }
    assert_eq!(logs.path(&id), None);
    lock(&log).record(&info("after"));
    assert_eq!(logs.end("r1", None).close(), None);
    assert_eq!(body(&path), ["before"]);
    assert_eq!(logs.end("r2", None).close(), None);
}

#[test]
fn the_oldest_choice_goes_when_the_list_is_full() {
    let logs = MessageLogs::default();
    let first = logs.remember(PathBuf::from("0.txt"));
    for number in 1..MAX_MESSAGE_FILES {
        logs.remember(PathBuf::from(format!("{number}.txt")));
    }
    assert!(logs.path(&first).is_some());
    let last = logs.remember(PathBuf::from("last.txt"));
    assert_eq!(logs.path(&first), None);
    assert_eq!(logs.path(&last), Some(PathBuf::from("last.txt")));
}

/// A sink that records each call, pauses after its first row, and goes on
/// after the pause.
#[derive(Default)]
struct Recorder {
    calls: Vec<String>,
    messages: Vec<Message>,
    sources: usize,
}

#[async_trait::async_trait]
impl RowSink for Recorder {
    fn begin_set(&mut self, columns: Vec<ColumnInfo>) -> Result<()> {
        self.calls.push(format!("begin {}", columns.len()));
        Ok(())
    }
    fn row(&mut self, row: Vec<JsonValue>) -> Result<SinkControl> {
        self.calls.push(format!("row {}", row[0]));
        Ok(SinkControl::Pause)
    }
    fn end_set(&mut self, truncated: bool) -> Result<()> {
        self.calls.push(format!("end {truncated}"));
        Ok(())
    }
    fn message(&mut self, message: Message) {
        self.messages.push(message);
    }
    fn keep_source(&mut self, _source: KeptSource) {
        self.sources += 1;
    }
    fn pause_point(&self) -> Option<PausePoint> {
        Some(PausePoint {
            rows: 7,
            limit: Duration::from_secs(9),
        })
    }
    async fn resume(&mut self) -> Result<SinkControl> {
        self.calls.push("resume".into());
        Ok(SinkControl::Continue)
    }
}

#[tokio::test]
async fn the_tee_gives_every_call_to_the_inner_sink() {
    let logs = MessageLogs::default();
    let log = logs.start("r1", None);
    let mut tee = MessageTee::new(Recorder::default(), Some(log.clone()));

    assert_eq!(
        tee.pause_point(),
        Some(PausePoint {
            rows: 7,
            limit: Duration::from_secs(9),
        })
    );
    tee.begin_set(vec![ColumnInfo::new("n", "int")]).unwrap();
    // The pause of the inner sink reaches the driver, and the wait for the
    // resume goes to the inner sink.
    let control = crate::db::sink::feed(&mut tee, vec![json!(1)])
        .await
        .unwrap();
    assert_eq!(control, SinkControl::Continue);
    tee.keep_source(crate::kept::tests::fixed(1));
    tee.end_set(true).unwrap();
    tee.message(info("copied"));

    let inner = tee.into_inner();
    assert_eq!(inner.calls, ["begin 1", "row 1", "resume", "end true"]);
    assert_eq!(inner.sources, 1);
    assert_eq!(inner.messages, [info("copied")]);
    let log = lock(&log);
    assert_eq!(log.seen, 1);
    assert_eq!(log.recent, [info("copied")]);
}

#[tokio::test]
async fn a_tee_without_a_log_only_passes_the_calls_on() {
    let mut tee = MessageTee::new(Recorder::default(), None);
    tee.message(info("plain"));
    assert_eq!(tee.resume().await.unwrap(), SinkControl::Continue);
    assert_eq!(tee.into_inner().messages, [info("plain")]);
}

#[test]
fn a_tee_gives_the_reports_of_a_spill_to_the_inner_sink() {
    let mut tee = MessageTee::new(crate::db::sink::testing::ReportSink::default(), None);
    tee.not_kept(UnsavedReason::DiskLimit);
    tee.inner_mut().not_kept(UnsavedReason::Script);
    tee.progress(3, 40);
    let inner = tee.into_inner();
    assert_eq!(
        inner.reasons,
        [UnsavedReason::DiskLimit, UnsavedReason::Script]
    );
    assert_eq!(inner.progress, [(3, 40)]);
}
