//! The run that writes its rows to a file and shows the first rows in the
//! grid.
//!
//! An export of all rows runs the statement a second time. This run sends
//! the statement to the server one time. Each row of the first result set
//! goes to the file writer, up to the export row limit. The rows up to the
//! row limit of the grid also go to the window, as in a normal run.
//!
//! The user chooses the file before the run starts, in a separate command.
//! The interface clears the results of the tab when a run starts, so a user
//! who closes the dialog keeps the old results. The backend keeps the chosen
//! path and gives the interface a ticket for it. The run accepts only a
//! ticket, so it never writes to a path that the user did not accept.

use super::{
    driver_for_request, end_message_log, finish_run, in_sent_text, prepare_parameters, run_bounded,
    session_for, stop_grace, Bounded, ExportFormat, ExportSummary, FileSink,
};
use crate::db::columnar::ChunkSink;
use crate::db::sink::{RowSink, SinkControl};
use crate::db::{ColumnInfo, ExecOptions, Message};
use crate::error::{Error, Result};
use crate::message_log::{MessageLogs, MessageTee};
use crate::sql::ParamValues;
use crate::state::AppState;
use std::path::{Path, PathBuf};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Runtime};

/// The number of chosen files that the backend keeps. A choice that the
/// interface never uses stays in the list until newer choices push it out.
const MAX_CHOSEN_FILES: usize = 16;

/// One file that the user chose for a run, with the format of the file.
struct ChosenFile {
    ticket: String,
    path: PathBuf,
    format: ExportFormat,
}

/// The files that the user chose in the save dialog and that no run used
/// yet. Each ticket works one time.
#[derive(Default)]
pub struct ChosenFiles {
    files: std::sync::Mutex<Vec<ChosenFile>>,
}

impl ChosenFiles {
    /// Records one file and gives back its ticket. The oldest choice goes
    /// when the list is full.
    fn remember(&self, path: PathBuf, format: ExportFormat) -> String {
        let ticket = uuid::Uuid::new_v4().to_string();
        let mut files = self
            .files
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if files.len() >= MAX_CHOSEN_FILES {
            files.remove(0);
        }
        files.push(ChosenFile {
            ticket: ticket.clone(),
            path,
            format,
        });
        ticket
    }

    /// Removes the file of one ticket from the list and gives it back.
    fn take(&self, ticket: &str) -> Option<ChosenFile> {
        let mut files = self
            .files
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let position = files.iter().position(|file| file.ticket == ticket)?;
        Some(files.remove(position))
    }
}

/// The format of a file, from the extension that the user gave it. An
/// extension that is not JSON or Excel gives a CSV file.
fn format_for_path(path: &Path) -> ExportFormat {
    let extension = path
        .extension()
        .map(|extension| extension.to_string_lossy().to_lowercase());
    match extension.as_deref() {
        Some("json") => ExportFormat::Json,
        Some("xlsx") => ExportFormat::Xlsx,
        _ => ExportFormat::Csv,
    }
}

/// What the interface sends to choose the file of a run.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChooseRunFileRequest {
    /// The file name that the save dialog suggests.
    pub default_name: String,
}

/// The file that the user chose, and the ticket that the run sends back.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChosenRunFile {
    pub ticket: String,
    pub path: String,
}

/// Asks the user for the file of a run. The dialog offers CSV, JSON and
/// Excel files, and the extension of the chosen name sets the format.
/// Returns `None` when the user closed the dialog.
#[tauri::command]
pub async fn choose_run_file<R: Runtime>(
    app: AppHandle<R>,
    request: ChooseRunFileRequest,
    chosen: tauri::State<'_, ChosenFiles>,
) -> Result<Option<ChosenRunFile>> {
    use tauri_plugin_dialog::DialogExt;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(&request.default_name)
        .add_filter("CSV", &["csv"])
        .add_filter("JSON", &["json"])
        .add_filter("Excel", &["xlsx"])
        .save_file(move |path| {
            let _ = sender.send(path);
        });
    let path = receiver
        .await
        .ok()
        .flatten()
        .and_then(|path| path.into_path().ok());
    Ok(path.map(|path| remember_choice(&chosen, path)))
}

/// Records the chosen file and builds the answer for the interface.
fn remember_choice(chosen: &ChosenFiles, path: PathBuf) -> ChosenRunFile {
    let shown = path.to_string_lossy().to_string();
    let format = format_for_path(&path);
    ChosenRunFile {
        ticket: chosen.remember(path, format),
        path: shown,
    }
}

/// What one run to a file sends.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunToFileRequest {
    pub connection_id: String,
    pub request_id: String,
    pub query: String,
    /// The ticket of the file that `choose_run_file` gave.
    pub ticket: String,
    /// The row limit of the file.
    pub max_rows: usize,
    /// The tab that runs the statement. A request without a tab runs on the
    /// default session.
    #[serde(default)]
    pub tab_id: Option<String>,
    #[serde(default)]
    pub query_params: Option<ParamValues>,
    /// The limits of the grid. The row limit applies to the rows that go to
    /// the window, and the time limit applies to the whole run.
    #[serde(default)]
    pub options: Option<ExecOptions>,
    /// The identifier of the file that gets every message of the run, from
    /// `choose_messages_file`.
    #[serde(default)]
    pub messages_file: Option<String>,
}

/// The message for a ticket that the backend does not know.
fn unknown_ticket() -> Error {
    Error::Invalid("The file choice for this run has expired. Run to file again.".to_string())
}

/// Runs a script one time. The rows of the first result set go to the file
/// that the user chose, and the first rows of each set go to the window as
/// binary chunks, as `execute_query` sends them.
///
/// The statement runs one time, so a statement that changes data is
/// accepted, as in a normal run. A stop, an error or the time limit removes
/// the part of the file that was written. The command gives back what the
/// file received. The window gets the messages and the numbers of the run
/// in the last frame of the channel, also when the file fails.
#[tauri::command]
pub async fn run_to_file<R: Runtime>(
    app: AppHandle<R>,
    request: RunToFileRequest,
    state: tauri::State<'_, AppState>,
    chosen: tauri::State<'_, ChosenFiles>,
    on_chunk: Channel<InvokeResponseBody>,
) -> Result<ExportSummary> {
    let RunToFileRequest {
        connection_id,
        request_id,
        query,
        ticket,
        max_rows,
        tab_id,
        query_params,
        options,
        messages_file,
    } = request;
    let started = std::time::Instant::now();
    let elapsed = || started.elapsed().as_millis() as u64;
    let token = state.start_request(&request_id, &connection_id).await;
    let logs = tauri::Manager::try_state::<MessageLogs>(&app);
    let log = logs
        .as_ref()
        .map(|logs| logs.start(&request_id, messages_file.as_deref()));
    let prepared = async {
        let file = chosen.take(&ticket).ok_or_else(unknown_ticket)?;
        let (open, session, key) =
            session_for(&app, &state, &connection_id, tab_id.as_deref(), &token).await?;
        let grid = options.unwrap_or_else(|| open.descriptor.exec_options());
        // The driver stops at the limit of the file. The sink gives the
        // window the rows up to the limit of the grid.
        let options = ExecOptions {
            max_rows,
            timeout_secs: grid.timeout_secs,
            one_statement: false,
        };
        let (query, bound) = prepare_parameters(&query, open.dialect, query_params.as_ref())?;
        // The drop of the sink before `finish` removes the part of the file
        // that was written.
        let sink = FileSink::create(&file.path, file.format).await?;
        Ok::<_, Error>((
            open,
            session,
            key,
            grid.max_rows,
            options,
            query,
            bound,
            sink,
        ))
    }
    .await;
    let (open, session, key, grid_rows, options, ran, bound, file) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            state.end_request(&request_id).await;
            // The window waits for the end frame of every run.
            let mut sink = ChunkSink::new(on_chunk, 0);
            end_message_log(logs.as_deref(), &request_id, Some(&error), &mut sink).await;
            let _ = sink.fail(elapsed());
            return Err(error);
        }
    };

    let grid = MessageTee::new(ChunkSink::new(on_chunk, grid_rows), log);
    let mut sink = TeeSink::new(file, grid, grid_rows);
    let outcome = match driver_for_request(&state, &request_id, &session, &token).await {
        Ok(mut guard) => {
            run_bounded(
                guard.execute_stream(&ran, bound.as_ref(), &options, &mut sink),
                &token,
                options.timeout_secs,
                stop_grace(&session),
                session.cancel_handle.clone(),
            )
            .await
        }
        Err(error) => Bounded::Answered(Err(error)),
    };
    let outcome = in_sent_text(outcome, &query, &ran);
    state.end_request(&request_id).await;

    let TeeSink { file, grid, .. } = sink;
    let mut grid = grid.into_inner();
    let finished = finish_run(&state, &connection_id, &open, &key, &session, outcome).await;
    end_message_log(
        logs.as_deref(),
        &request_id,
        finished.as_ref().err(),
        &mut grid,
    )
    .await;
    let summary = match finished {
        Ok(summary) => summary,
        Err(error) => {
            drop(file);
            let _ = grid.fail(elapsed());
            return Err(error);
        }
    };
    let written = if file.saw_set {
        file.finish().await
    } else {
        Err(Error::Unsupported(
            "The statement returned no result set, so no file was written.".to_string(),
        ))
    };
    // The run itself ended well, so the window gets its numbers also when
    // the file failed. The error of the file then travels on the answer.
    grid.finish(summary)?;
    let written = written?;
    log::info!(
        "Wrote {} rows to the file '{}' and showed the first rows.",
        written.rows,
        written.path
    );
    Ok(written)
}

/// A sink that gives each row of the first result set to the file, and the
/// first rows of each set to the grid.
///
/// The sink never answers `Stop`. A `Stop` ends the whole run, and SQLite
/// then skips the statements after the one that stopped, so a script would
/// run fewer statements than a normal run. The driver stops each set at the
/// limit of the file, and the sink drops the rows past the limit of the
/// grid. The file sink itself drops the rows past the room of an Excel
/// sheet and the rows of the sets after the first.
struct TeeSink<F: RowSink, G: RowSink> {
    file: F,
    grid: G,
    /// The row limit of the grid.
    grid_rows: usize,
    /// The number of sets that began.
    sets: usize,
    /// The rows of the open set that went to the grid.
    shown: usize,
    /// True when the open set had more rows than the grid takes.
    cut: bool,
}

impl<F: RowSink, G: RowSink> TeeSink<F, G> {
    fn new(file: F, grid: G, grid_rows: usize) -> Self {
        Self {
            file,
            grid,
            grid_rows,
            sets: 0,
            shown: 0,
            cut: false,
        }
    }

    /// True while the open set goes to the file.
    fn in_file(&self) -> bool {
        self.sets <= 1
    }
}

impl<F: RowSink, G: RowSink> RowSink for TeeSink<F, G> {
    fn begin_set(&mut self, columns: Vec<ColumnInfo>) -> Result<()> {
        self.sets += 1;
        self.shown = 0;
        self.cut = false;
        if self.in_file() {
            self.file.begin_set(columns.clone())?;
        }
        self.grid.begin_set(columns)
    }

    fn row(&mut self, row: Vec<serde_json::Value>) -> Result<SinkControl> {
        let to_grid = self.shown < self.grid_rows;
        if to_grid {
            self.shown += 1;
        } else {
            self.cut = true;
        }
        match (self.in_file(), to_grid) {
            (true, true) => {
                self.grid.row(row.clone())?;
                self.file.row(row)?;
            }
            (true, false) => {
                self.file.row(row)?;
            }
            (false, true) => {
                self.grid.row(row)?;
            }
            (false, false) => {}
        }
        Ok(SinkControl::Continue)
    }

    fn end_set(&mut self, truncated: bool) -> Result<()> {
        if self.in_file() {
            self.file.end_set(truncated)?;
        }
        self.grid.end_set(truncated || self.cut)
    }

    fn message(&mut self, message: Message) {
        self.grid.message(message);
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{state_with_sqlite, temp_sqlite};
    use super::*;
    use crate::db::columnar::{FRAME_BEGIN_SET, FRAME_CHUNK, FRAME_END, FRAME_END_SET};
    use crate::db::sink::BufferSink;
    use crate::db::sink::RunSummary;
    use crate::db::QueryResponse;
    use std::sync::{Arc, Mutex};
    use tauri::Manager;

    fn row(value: i64) -> Vec<serde_json::Value> {
        vec![serde_json::json!(value)]
    }

    fn columns() -> Vec<ColumnInfo> {
        vec![ColumnInfo::new("id", "int")]
    }

    fn response(sink: BufferSink) -> QueryResponse {
        sink.into_response(RunSummary::default())
    }

    #[test]
    fn the_first_set_goes_whole_to_the_file_and_in_part_to_the_grid() {
        let mut tee = TeeSink::new(BufferSink::new(100), BufferSink::new(100), 2);
        tee.begin_set(columns()).unwrap();
        for value in 0..5 {
            assert_eq!(tee.row(row(value)).unwrap(), SinkControl::Continue);
        }
        tee.end_set(false).unwrap();
        tee.message(Message::info("5 rows"));

        let file = response(tee.file);
        assert_eq!(file.results[0].rows.len(), 5);
        assert!(!file.results[0].truncated);
        assert!(file.messages.is_empty());
        let grid = response(tee.grid);
        assert_eq!(grid.results[0].rows, vec![row(0), row(1)]);
        assert!(grid.results[0].truncated);
        assert_eq!(grid.messages.len(), 1);
    }

    #[test]
    fn a_set_that_fits_the_grid_is_not_cut() {
        let mut tee = TeeSink::new(BufferSink::new(100), BufferSink::new(100), 2);
        tee.begin_set(columns()).unwrap();
        tee.row(row(1)).unwrap();
        tee.end_set(false).unwrap();
        assert!(!response(tee.grid).results[0].truncated);
    }

    #[test]
    fn the_limit_of_the_file_also_cuts_the_grid() {
        let mut tee = TeeSink::new(BufferSink::new(100), BufferSink::new(100), 5);
        tee.begin_set(columns()).unwrap();
        tee.row(row(1)).unwrap();
        tee.end_set(true).unwrap();
        assert!(response(tee.file).results[0].truncated);
        assert!(response(tee.grid).results[0].truncated);
    }

    #[test]
    fn the_sets_after_the_first_go_to_the_grid_alone() {
        let mut tee = TeeSink::new(BufferSink::new(100), BufferSink::new(100), 2);
        tee.begin_set(columns()).unwrap();
        tee.row(row(1)).unwrap();
        tee.end_set(false).unwrap();
        tee.begin_set(vec![ColumnInfo::new("name", "text")])
            .unwrap();
        for value in 0..4 {
            // The sink goes on past the grid, so the driver reads the
            // statements after this one.
            assert_eq!(tee.row(row(value)).unwrap(), SinkControl::Continue);
        }
        tee.end_set(false).unwrap();

        let file = response(tee.file);
        assert_eq!(file.results.len(), 1);
        let grid = response(tee.grid);
        assert_eq!(grid.results.len(), 2);
        assert_eq!(grid.results[1].rows, vec![row(0), row(1)]);
        assert!(grid.results[1].truncated);
        assert!(!grid.results[0].truncated);
    }

    #[test]
    fn a_failure_of_a_sink_stops_the_run() {
        // A row before a set is an error of the buffer sink.
        let mut tee = TeeSink::new(BufferSink::new(100), BufferSink::new(100), 1);
        assert!(tee.row(row(1)).is_err());
        let mut tee = TeeSink::new(BufferSink::new(100), BufferSink::new(100), 0);
        assert!(tee.row(row(1)).is_err());
        assert!(tee.end_set(false).is_err());
        // A set after the first reaches the file no more, so only the grid
        // can fail there.
        let mut tee = TeeSink::new(BufferSink::new(100), BufferSink::new(100), 1);
        tee.sets = 2;
        assert!(tee.row(row(1)).is_err());
    }

    #[test]
    fn the_extension_sets_the_format() {
        assert!(matches!(
            format_for_path(Path::new("/a/b.json")),
            ExportFormat::Json
        ));
        assert!(matches!(
            format_for_path(Path::new("/a/b.XLSX")),
            ExportFormat::Xlsx
        ));
        assert!(matches!(
            format_for_path(Path::new("/a/b.csv")),
            ExportFormat::Csv
        ));
        assert!(matches!(
            format_for_path(Path::new("/a/b.txt")),
            ExportFormat::Csv
        ));
        assert!(matches!(
            format_for_path(Path::new("/a/b")),
            ExportFormat::Csv
        ));
    }

    #[test]
    fn a_ticket_works_one_time() {
        let chosen = ChosenFiles::default();
        let file = remember_choice(&chosen, PathBuf::from("/a/out.xlsx"));
        assert_eq!(file.path, "/a/out.xlsx");
        let taken = chosen.take(&file.ticket).unwrap();
        assert_eq!(taken.path, PathBuf::from("/a/out.xlsx"));
        assert!(matches!(taken.format, ExportFormat::Xlsx));
        assert!(chosen.take(&file.ticket).is_none());
    }

    #[test]
    fn a_full_list_forgets_the_oldest_choice() {
        let chosen = ChosenFiles::default();
        let first = chosen.remember(PathBuf::from("/a/0.csv"), ExportFormat::Csv);
        let mut last = String::new();
        for number in 1..=MAX_CHOSEN_FILES {
            last = chosen.remember(PathBuf::from(format!("/a/{number}.csv")), ExportFormat::Csv);
        }
        assert!(chosen.take(&first).is_none());
        assert!(chosen.take(&last).is_some());
    }

    /// The messages that a test channel received.
    type Frames = Arc<Mutex<Vec<Vec<u8>>>>;

    /// A channel that keeps every message whole.
    fn frame_channel() -> (Channel<InvokeResponseBody>, Frames) {
        let frames = Arc::new(Mutex::new(Vec::new()));
        let kept = frames.clone();
        let channel = Channel::new(move |body| {
            if let InvokeResponseBody::Raw(bytes) = body {
                kept.lock().unwrap().push(bytes);
            }
            Ok(())
        });
        (channel, frames)
    }

    /// The first bytes of each message, which name the frame.
    fn frame_types(frames: &Frames) -> Vec<u8> {
        frames
            .lock()
            .unwrap()
            .iter()
            .map(|bytes| bytes[0])
            .collect()
    }

    /// The truncated flag of each end-set frame.
    fn cut_flags(frames: &Frames) -> Vec<bool> {
        frames
            .lock()
            .unwrap()
            .iter()
            .filter(|bytes| bytes[0] == FRAME_END_SET)
            .map(|bytes| bytes[5] == 1)
            .collect()
    }

    /// True when the folder still has a temporary file of an export after
    /// a wait of at most five seconds. The writer thread removes its part
    /// after the sink is gone.
    fn has_part_file(folder: &Path) -> bool {
        let found = || {
            std::fs::read_dir(folder).unwrap().any(|entry| {
                entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".part")
            })
        };
        for _ in 0..500 {
            if !found() {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        true
    }

    fn request(ticket: &str, query: &str, max_rows: usize) -> RunToFileRequest {
        RunToFileRequest {
            connection_id: "s1".into(),
            request_id: "r1".into(),
            query: query.into(),
            ticket: ticket.into(),
            max_rows,
            tab_id: Some("t1".into()),
            query_params: None,
            options: Some(ExecOptions {
                max_rows: 3,
                timeout_secs: 30,
                one_statement: false,
            }),
            messages_file: None,
        }
    }

    /// A statement that gives the numbers from 1 to `count`.
    fn numbers(count: usize) -> String {
        format!(
            "WITH RECURSIVE n(v) AS (SELECT 1 UNION ALL SELECT v + 1 FROM n WHERE v < {count}) \
             SELECT v FROM n"
        )
    }

    async fn app() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
        let (dir, descriptor) = temp_sqlite();
        let (app, state) = state_with_sqlite(descriptor).await;
        app.manage(state);
        app.manage(ChosenFiles::default());
        (dir, app)
    }

    fn choose(app: &tauri::App<tauri::test::MockRuntime>, path: &Path) -> String {
        remember_choice(&app.state::<ChosenFiles>(), path.to_path_buf()).ticket
    }

    async fn run(
        app: &tauri::App<tauri::test::MockRuntime>,
        request: RunToFileRequest,
        channel: Channel<InvokeResponseBody>,
    ) -> Result<ExportSummary> {
        run_to_file(
            app.handle().clone(),
            request,
            app.state::<AppState>(),
            app.state::<ChosenFiles>(),
            channel,
        )
        .await
    }

    #[tokio::test]
    async fn one_run_fills_the_file_and_the_grid() {
        let (dir, app) = app().await;
        let path = dir.path().join("out.csv");
        let ticket = choose(&app, &path);
        let (channel, frames) = frame_channel();

        let summary = run(&app, request(&ticket, &numbers(5), 100), channel)
            .await
            .unwrap();
        assert_eq!(summary.rows, 5);
        assert!(!summary.truncated);
        assert_eq!(summary.path, path.to_string_lossy());
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 6);
        // The grid takes three rows, so its set is cut.
        let types = frame_types(&frames);
        assert_eq!(types.first(), Some(&FRAME_BEGIN_SET));
        assert!(types.contains(&FRAME_CHUNK));
        assert_eq!(types.last(), Some(&FRAME_END));
        assert_eq!(cut_flags(&frames), vec![true]);
    }

    #[tokio::test]
    async fn the_file_stops_at_its_own_limit() {
        let (dir, app) = app().await;
        let path = dir.path().join("out.json");
        let ticket = choose(&app, &path);
        let (channel, _) = frame_channel();

        let summary = run(&app, request(&ticket, &numbers(10), 4), channel)
            .await
            .unwrap();
        assert_eq!(summary.rows, 4);
        assert!(summary.truncated);
        let values: Vec<serde_json::Value> =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(values.len(), 4);
    }

    #[tokio::test]
    async fn a_script_writes_its_first_set_and_runs_every_statement() {
        let (dir, app) = app().await;
        let path = dir.path().join("out.csv");
        let ticket = choose(&app, &path);
        let (channel, frames) = frame_channel();
        let script = format!(
            "{}; SELECT 'a' AS name; CREATE TABLE after_sets(id INTEGER)",
            numbers(5)
        );

        let summary = run(&app, request(&ticket, &script, 100), channel)
            .await
            .unwrap();
        assert_eq!(summary.rows, 5);
        assert_eq!(cut_flags(&frames), vec![true, false]);
        // The statement after the sets ran, so the table is there.
        let (channel, _) = frame_channel();
        let ticket = choose(&app, &dir.path().join("check.csv"));
        let check = run(
            &app,
            request(
                &ticket,
                "SELECT name FROM sqlite_master WHERE name = 'after_sets'",
                100,
            ),
            channel,
        )
        .await
        .unwrap();
        assert_eq!(check.rows, 1);
    }

    #[tokio::test]
    async fn a_run_without_a_set_writes_no_file() {
        let (dir, app) = app().await;
        let path = dir.path().join("out.csv");
        let ticket = choose(&app, &path);
        let (channel, frames) = frame_channel();

        let error = run(
            &app,
            request(&ticket, "CREATE TABLE t(id INTEGER)", 100),
            channel,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("no result set"));
        assert!(!path.exists());
        // The run itself ended well, so the window gets its end frame.
        assert_eq!(frame_types(&frames).last(), Some(&FRAME_END));
        assert!(!has_part_file(dir.path()));
    }

    #[tokio::test]
    async fn a_failed_run_removes_the_file() {
        let (dir, app) = app().await;
        let path = dir.path().join("out.csv");
        let ticket = choose(&app, &path);
        let (channel, frames) = frame_channel();

        let error = run(
            &app,
            request(&ticket, "SELECT * FROM missing_table", 100),
            channel,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("missing_table"));
        assert!(!path.exists());
        assert_eq!(frame_types(&frames), vec![FRAME_END]);
        assert!(!has_part_file(dir.path()));
    }

    #[tokio::test]
    async fn a_run_to_file_sends_its_messages_to_the_chosen_file() {
        use crate::message_log::MessageLogs;
        let (dir, app) = app().await;
        app.manage(MessageLogs::default());
        let log_path = dir.path().join("messages.txt");
        let id = app.state::<MessageLogs>().remember(log_path.clone());
        let ticket = choose(&app, &dir.path().join("out.csv"));
        let logged = |ticket: &str, query: &str| RunToFileRequest {
            messages_file: Some(id.clone()),
            ..request(ticket, query, 100)
        };

        let (channel, _) = frame_channel();
        run(&app, logged(&ticket, &numbers(5)), channel)
            .await
            .unwrap();
        let ticket = choose(&app, &dir.path().join("failed.csv"));
        let (channel, _) = frame_channel();
        run(
            &app,
            logged(&ticket, "SELECT * FROM missing_table"),
            channel,
        )
        .await
        .unwrap_err();
        let (channel, _) = frame_channel();
        run(&app, logged("nothing", "SELECT 1"), channel)
            .await
            .unwrap_err();

        let text = std::fs::read_to_string(&log_path).unwrap();
        let lines: Vec<&str> = text
            .lines()
            .filter(|line| !line.starts_with("-- Run started "))
            .collect();
        assert_eq!(lines[0], "5 rows returned.");
        assert!(lines[1].starts_with("Error: ") && lines[1].contains("missing_table"));
        let last = lines.last().unwrap();
        assert!(last.starts_with("Error: ") && last.contains("expired"));
    }

    #[tokio::test]
    async fn an_unknown_ticket_ends_the_run_before_it_starts() {
        let (_dir, app) = app().await;
        let (channel, frames) = frame_channel();

        let error = run(&app, request("nothing", "SELECT 1", 100), channel)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("expired"));
        assert_eq!(frame_types(&frames), vec![FRAME_END]);
    }

    #[tokio::test]
    async fn a_run_without_options_uses_the_limits_of_the_connection() {
        let (dir, app) = app().await;
        let path = dir.path().join("out.csv");
        let ticket = choose(&app, &path);
        let (channel, frames) = frame_channel();
        let mut request = request(&ticket, &numbers(5), 100);
        request.options = None;

        let summary = run(&app, request, channel).await.unwrap();
        assert_eq!(summary.rows, 5);
        assert_eq!(cut_flags(&frames), vec![false]);
    }
}
