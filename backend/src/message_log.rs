//! The copy of the messages of a run in a text file that the user chose.
//!
//! The Messages tab of the window keeps the last few thousand messages of a
//! run. A loop on the server can print millions of lines, so the user can
//! send every message of a run to a file as well.
//!
//! The user chooses the file in a save dialog that the backend opens. The
//! backend keeps the path and gives the window an identifier for it, so the
//! window never names a path that the backend writes to. A run that names
//! the identifier appends its messages to the file from its first message.
//!
//! Each run also keeps its last [`KEPT_RUN_MESSAGES`] messages in memory. A
//! user who chooses a file while the run goes on gets those messages first,
//! then each new message. The writes go to a thread of their own, so a slow
//! disk does not stop the read of the rows.

use crate::db::columnar::ChunkSink;
use crate::db::sink::{PausePoint, RowSink, SinkControl};
use crate::db::{ColumnInfo, Message, MessageLevel};
use crate::error::{Error, Result};
use crate::kept::KeptSource;
use serde_json::Value as JsonValue;
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

/// The count of the last messages of a run that the backend keeps for a
/// file that the user chooses during the run.
pub const KEPT_RUN_MESSAGES: usize = 4000;

/// The count of chosen files that the backend keeps. A choice that no tab
/// uses stays until newer choices push it out.
const MAX_MESSAGE_FILES: usize = 64;

/// The sink that gives the rows and the messages of a run to the window,
/// with a copy of each message for the log of the run.
pub type GridSink = MessageTee<ChunkSink>;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Writes one message as lines of the file. A warning and an error start
/// with their level, and each line of the detail follows with an indent of
/// four spaces.
pub fn message_lines(message: &Message) -> String {
    let prefix = match message.level {
        MessageLevel::Info => "",
        MessageLevel::Warning => "Warning: ",
        MessageLevel::Error => "Error: ",
    };
    let mut text = format!("{prefix}{}\n", message.text);
    if let Some(detail) = &message.detail {
        for line in detail.lines() {
            text.push_str("    ");
            text.push_str(line);
            text.push('\n');
        }
    }
    text
}

/// The line that starts the messages of one run in the file.
fn run_header(started: &str) -> String {
    format!("-- Run started {started}\n")
}

/// The line for the first messages of a run that the backend no longer
/// kept when the user chose the file.
pub fn dropped_note(count: u64) -> String {
    if count == 1 {
        "-- 1 earlier message of this run wasn't kept\n".to_string()
    } else {
        format!("-- {count} earlier messages of this run weren't kept\n")
    }
}

/// The text of a list of messages, after the note for the messages that
/// the list dropped.
pub fn messages_text(messages: &[Message], dropped: u64) -> String {
    let mut text = if dropped > 0 {
        dropped_note(dropped)
    } else {
        String::new()
    };
    for message in messages {
        text.push_str(&message_lines(message));
    }
    text
}

/// The local time as the header of a run gives it.
fn now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// The warning for a file that the messages of a run did not reach.
fn write_failed(path: &Path, reason: &str) -> String {
    format!("Couldn't save the messages to {}: {reason}", path.display())
}

/// A thread that appends text to one file. The text arrives over a
/// channel, so a write never waits for the disk.
pub struct LogWriter {
    path: PathBuf,
    lines: Option<mpsc::Sender<String>>,
    thread: Option<JoinHandle<std::io::Result<()>>>,
}

impl LogWriter {
    /// Starts the thread. The thread creates the file when it is not there
    /// and appends to it. An error of the open or of a write ends the
    /// thread, and [`LogWriter::close`] gives it.
    pub fn open(path: PathBuf) -> Self {
        let (sender, receiver) = mpsc::channel::<String>();
        let target = path.clone();
        let thread = std::thread::spawn(move || {
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&target)?;
            let mut out = std::io::BufWriter::new(file);
            loop {
                // The buffer goes to the disk each time the queue is empty,
                // so a user who reads the file during the run sees the
                // last messages.
                let text = match receiver.try_recv() {
                    Ok(text) => text,
                    Err(TryRecvError::Empty) => {
                        out.flush()?;
                        match receiver.recv() {
                            Ok(text) => text,
                            Err(_) => break,
                        }
                    }
                    Err(TryRecvError::Disconnected) => break,
                };
                out.write_all(text.as_bytes())?;
            }
            out.flush()
        });
        Self {
            path,
            lines: Some(sender),
            thread: Some(thread),
        }
    }

    /// Queues text for the file. Text after an error goes nowhere, and the
    /// close gives the error.
    pub fn write(&self, text: String) {
        if let Some(lines) = &self.lines {
            let _ = lines.send(text);
        }
    }

    /// Waits until the thread wrote every queued text, and gives a warning
    /// for the user when a write failed. The wait can take as long as the
    /// disk, so the caller runs this on a blocking thread.
    pub fn close(mut self) -> Option<String> {
        self.lines = None;
        let thread = self.thread.take()?;
        match thread.join() {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(write_failed(&self.path, &error.to_string())),
            Err(_) => Some(write_failed(&self.path, "the writer stopped")),
        }
    }
}

/// The messages of one run, as the backend keeps them.
pub struct RunLog {
    /// The local time of the start of the run.
    started: String,
    /// The count of the messages of the run.
    seen: u64,
    /// The last messages of the run, up to [`KEPT_RUN_MESSAGES`].
    recent: VecDeque<Message>,
    /// The file that gets each message, when the user chose one.
    writer: Option<LogWriter>,
    /// The identifier of the file of `writer`.
    file_id: Option<String>,
    /// A warning for the end of the run, such as a file choice that the
    /// backend no longer knows.
    problem: Option<String>,
}

impl RunLog {
    fn new(started: String) -> Self {
        Self {
            started,
            seen: 0,
            recent: VecDeque::new(),
            writer: None,
            file_id: None,
            problem: None,
        }
    }

    /// Keeps one message, and gives it to the file when the run has one.
    fn record(&mut self, message: &Message) {
        self.seen += 1;
        if self.recent.len() == KEPT_RUN_MESSAGES {
            self.recent.pop_front();
        }
        self.recent.push_back(message.clone());
        if let Some(writer) = &self.writer {
            writer.write(message_lines(message));
        }
    }

    /// Sends the messages of the run to a new file: the header of the run,
    /// a note for the messages that the run no longer keeps, then the kept
    /// messages. Each later message follows. Gives back the writer that the
    /// new one replaced.
    fn attach(&mut self, file_id: &str, path: PathBuf) -> Option<LogWriter> {
        let writer = LogWriter::open(path);
        writer.write(run_header(&self.started));
        let lost = self.seen - self.recent.len() as u64;
        if lost > 0 {
            writer.write(dropped_note(lost));
        }
        for message in &self.recent {
            writer.write(message_lines(message));
        }
        self.file_id = Some(file_id.to_string());
        self.problem = None;
        self.writer.replace(writer)
    }
}

/// A log of a run, as the sink and the registry share it.
pub type SharedLog = Arc<Mutex<RunLog>>;

/// What is left to do after the end of a run: the close of its file.
#[derive(Default)]
pub struct Ending {
    writer: Option<LogWriter>,
    problem: Option<String>,
}

impl Ending {
    /// Closes the file of the run, and gives a warning for the user when
    /// the messages did not reach the file. The close waits for the disk,
    /// so the caller runs this on a blocking thread.
    pub fn close(self) -> Option<String> {
        let failed = self.writer.and_then(LogWriter::close);
        failed.or(self.problem)
    }
}

/// The files that the user chose for messages, and the logs of the runs
/// that go on.
#[derive(Default)]
pub struct MessageLogs {
    files: Mutex<VecDeque<(String, PathBuf)>>,
    runs: Mutex<HashMap<String, SharedLog>>,
}

impl MessageLogs {
    /// Records a file that the user chose, and gives back its identifier.
    /// The oldest choice goes when the list is full.
    pub fn remember(&self, path: PathBuf) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let mut files = lock(&self.files);
        if files.len() >= MAX_MESSAGE_FILES {
            files.pop_front();
        }
        files.push_back((id.clone(), path));
        id
    }

    /// The path of a chosen file.
    pub fn path(&self, id: &str) -> Option<PathBuf> {
        lock(&self.files)
            .iter()
            .find(|(known, _)| known == id)
            .map(|(_, path)| path.clone())
    }

    /// Forgets a chosen file. The runs that write to it stop, and the
    /// caller closes the writers that this gives back.
    pub fn forget(&self, id: &str) -> Vec<LogWriter> {
        lock(&self.files).retain(|(known, _)| known != id);
        let runs: Vec<SharedLog> = lock(&self.runs).values().cloned().collect();
        runs.iter()
            .filter_map(|log| {
                let mut log = lock(log);
                if log.file_id.as_deref() != Some(id) {
                    return None;
                }
                log.file_id = None;
                log.writer.take()
            })
            .collect()
    }

    /// Starts the log of a run. With a file identifier, each message of the
    /// run goes to that file, after the header of the run.
    pub fn start(&self, request_id: &str, file_id: Option<&str>) -> SharedLog {
        let mut log = RunLog::new(now());
        if let Some(id) = file_id {
            match self.path(id) {
                Some(path) => {
                    log.attach(id, path);
                }
                None => {
                    log.problem = Some(
                        "The file for the messages is no longer available, so this run's \
                         messages weren't saved. Choose the file again."
                            .to_string(),
                    );
                }
            }
        }
        let log = Arc::new(Mutex::new(log));
        lock(&self.runs).insert(request_id.to_string(), log.clone());
        log
    }

    /// Sends the messages of a run that goes on to a chosen file. Gives
    /// back false when the run already ended, and the writer that the new
    /// file replaced, which the caller closes.
    pub fn attach(&self, request_id: &str, file_id: &str) -> Result<(bool, Option<LogWriter>)> {
        let path = self.path(file_id).ok_or_else(|| {
            Error::Invalid("The file for the messages has expired. Choose it again.".to_string())
        })?;
        let Some(log) = lock(&self.runs).get(request_id).cloned() else {
            return Ok((false, None));
        };
        let replaced = lock(&log).attach(file_id, path);
        Ok((true, replaced))
    }

    /// Ends the log of a run. The error of a failed run goes to the file
    /// after the messages. The caller closes the file through the answer.
    pub fn end(&self, request_id: &str, error: Option<&Error>) -> Ending {
        let Some(log) = lock(&self.runs).remove(request_id) else {
            return Ending::default();
        };
        let mut log = lock(&log);
        if let (Some(writer), Some(error)) = (&log.writer, error) {
            let payload = error.to_payload();
            writer.write(message_lines(&Message {
                level: MessageLevel::Error,
                text: payload.message,
                detail: payload.detail,
            }));
        }
        log.file_id = None;
        Ending {
            writer: log.writer.take(),
            problem: log.problem.take(),
        }
    }
}

/// A sink that copies each message into the log of the run and gives every
/// call to the sink inside it. A message reaches the log before the inner
/// sink, so the log has it also when the inner sink fails to send it.
pub struct MessageTee<S> {
    inner: S,
    log: Option<SharedLog>,
}

impl<S> MessageTee<S> {
    /// Wraps a sink. Without a log, the sink only passes the calls on.
    pub fn new(inner: S, log: Option<SharedLog>) -> Self {
        Self { inner, log }
    }

    /// Gives back the inner sink.
    pub fn into_inner(self) -> S {
        self.inner
    }
}

#[async_trait::async_trait]
impl<S: RowSink> RowSink for MessageTee<S> {
    fn begin_set(&mut self, columns: Vec<ColumnInfo>) -> Result<()> {
        self.inner.begin_set(columns)
    }

    fn row(&mut self, row: Vec<JsonValue>) -> Result<SinkControl> {
        self.inner.row(row)
    }

    fn end_set(&mut self, truncated: bool) -> Result<()> {
        self.inner.end_set(truncated)
    }

    fn message(&mut self, message: Message) {
        if let Some(log) = &self.log {
            lock(log).record(&message);
        }
        self.inner.message(message);
    }

    fn keep_source(&mut self, source: KeptSource) {
        self.inner.keep_source(source);
    }

    fn pause_point(&self) -> Option<PausePoint> {
        self.inner.pause_point()
    }

    async fn resume(&mut self) -> Result<SinkControl> {
        self.inner.resume().await
    }
}

#[cfg(test)]
mod tests;
