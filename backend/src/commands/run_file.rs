//! The run that writes its rows to a file and shows the first rows in the
//! grid.
//!
//! An export of all rows runs the statement a second time. This run sends
//! the statement to the server one time. Each row of the first result set
//! goes to the file writer, up to the export row limit. The rows up to the
//! row limit of the grid also go to the window, as in a normal run.
//!
//! The user can also send each result set to the file. Each set of a CSV or
//! a JSON run then goes to a file of its own beside the chosen file, and
//! each set of an Excel run goes to a sheet of its own in the chosen file.
//!
//! The user chooses the file before the run starts, in a separate command.
//! The interface clears the results of the tab when a run starts, so a user
//! who closes the dialog keeps the old results. The backend keeps the chosen
//! path and gives the interface a ticket for it. The run accepts only a
//! ticket, so it never writes to a path that the user did not accept.

use super::{
    cut_cells_warning, driver_for_request, end_message_log, finish_run, in_sent_text, off_thread,
    prepare_parameters, run_bounded, session_after_run, session_for, stop_grace, Bounded,
    ExportFormat, ExportSummary, FileSink, RunExit,
};
use crate::db::columnar::ChunkSink;
use crate::db::sink::{RowSink, SinkControl};
use crate::db::{ColumnInfo, ExecOptions, Message};
use crate::error::{Error, Result};
use crate::kept::UnsavedReason;
use crate::message_log::{MessageLogs, MessageTee};
use crate::sql::ParamValues;
use crate::state::AppState;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Runtime};

/// The number of chosen files that the backend keeps. A choice that the
/// interface never uses stays in the list until newer choices push it out,
/// or until it is older than [`MAX_CHOSEN_AGE`].
const MAX_CHOSEN_FILES: usize = 16;

/// The time a chosen file stays valid. A run that did not start keeps its
/// ticket, so the user can try again with the same file. After this time,
/// the user must choose the file again.
const MAX_CHOSEN_AGE: Duration = Duration::from_secs(60 * 60);

/// One file that the user chose for a run, with the format of the file.
struct ChosenFile {
    ticket: String,
    path: PathBuf,
    format: ExportFormat,
    /// The time of the choice.
    chosen_at: Instant,
}

/// The files that the user chose in the save dialog and that no run used
/// yet. Each ticket works for one run that starts.
#[derive(Default)]
pub struct ChosenFiles {
    files: std::sync::Mutex<Vec<ChosenFile>>,
}

impl ChosenFiles {
    /// Locks the list and removes the choices that are too old at `now`.
    fn current(&self, now: Instant) -> std::sync::MutexGuard<'_, Vec<ChosenFile>> {
        let mut files = self
            .files
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        files.retain(|file| now.saturating_duration_since(file.chosen_at) < MAX_CHOSEN_AGE);
        files
    }

    /// Records one file and gives back its ticket. The oldest choice goes
    /// when the list is full.
    fn remember(&self, path: PathBuf, format: ExportFormat) -> String {
        self.remember_at(path, format, Instant::now())
    }

    fn remember_at(&self, path: PathBuf, format: ExportFormat, now: Instant) -> String {
        let ticket = uuid::Uuid::new_v4().to_string();
        let mut files = self.current(now);
        if files.len() >= MAX_CHOSEN_FILES {
            files.remove(0);
        }
        files.push(ChosenFile {
            ticket: ticket.clone(),
            path,
            format,
            chosen_at: now,
        });
        ticket
    }

    /// Gives the path and the format of one ticket, and keeps the ticket.
    fn peek(&self, ticket: &str) -> Option<(PathBuf, ExportFormat)> {
        self.peek_at(ticket, Instant::now())
    }

    fn peek_at(&self, ticket: &str, now: Instant) -> Option<(PathBuf, ExportFormat)> {
        self.current(now)
            .iter()
            .find(|file| file.ticket == ticket)
            .map(|file| (file.path.clone(), file.format))
    }

    /// Removes one ticket from the list. Gives false when the list does not
    /// have the ticket.
    fn take(&self, ticket: &str) -> bool {
        self.take_at(ticket, Instant::now())
    }

    fn take_at(&self, ticket: &str, now: Instant) -> bool {
        let mut files = self.current(now);
        let before = files.len();
        files.retain(|file| file.ticket != ticket);
        files.len() < before
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
    /// The format that the extension of the path sets.
    pub format: ExportFormat,
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
        format,
    }
}

/// Tells the interface whether a statement can give more than one result
/// set, so the interface can ask where the sets after the first go. The
/// check reads the text alone. A procedure can give sets that the text does
/// not show, so the run itself also counts the sets.
#[tauri::command]
pub async fn several_result_sets(query: String, dialect: crate::sql::Dialect) -> Result<bool> {
    off_thread(move || Ok(crate::sql::may_give_several_sets(&query, dialect))).await
}

/// Tells the interface whether the file of a ticket can still take a run.
/// A run that did not start keeps its ticket, so the interface can offer to
/// try again with the same file.
#[tauri::command]
pub fn run_file_ready(ticket: String, chosen: tauri::State<'_, ChosenFiles>) -> bool {
    chosen.peek(&ticket).is_some()
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
    /// The row limit of the file. Each set has this limit.
    pub max_rows: usize,
    /// True when each result set goes to the file: a CSV or a JSON run
    /// writes a file for each set, and an Excel run writes a sheet for each
    /// set. False sends the first set alone to the file.
    #[serde(default)]
    pub each_set: bool,
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

/// Runs a script one time. The rows of the first result set, or of each
/// set, go to the file that the user chose, and the first rows of each set
/// go to the window as binary chunks, as `execute_query` sends them.
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
) -> Result<RunFileSummary> {
    let RunToFileRequest {
        connection_id,
        request_id,
        query,
        ticket,
        max_rows,
        each_set,
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
        // The ticket stays valid until the statement starts, so a run that
        // fails before then can try again with the same file.
        let (path, format) = chosen.peek(&ticket).ok_or_else(unknown_ticket)?;
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
        let layout = SetLayout::new(each_set, format);
        let first = FileSink::create(&path, format).await?;
        let first = if layout == SetLayout::Sheets {
            first.sheet_per_set()
        } else {
            first
        };
        let sink = SetFiles::new(first, format, layout);
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
    let may_stop = !each_set && crate::sql::one_read(&ran, open.dialect);
    let mut sink = TeeSink::new(file, grid, grid_rows).stopping_when_full(may_stop);
    let outcome = match driver_for_request(&state, &request_id, &session, &token).await {
        // A second run with the same ticket can take it first. That run
        // then owns the file, and this run stops before its statement.
        Ok(_) if !chosen.take(&ticket) => Bounded::Answered(Err(unknown_ticket())),
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
    grid.report_session(
        session_after_run(
            &state,
            &connection_id,
            &open,
            &key,
            &session,
            &ran,
            RunExit::of(&finished),
        )
        .await,
    );
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
    let written = file.finish().await;
    // The run itself ended well, so the window gets its numbers also when
    // the file failed. The error of the file then travels on the answer.
    grid.finish(summary)?;
    let written = written?;
    log::info!(
        "Wrote {} rows of {} result sets to the file '{}' and showed the first rows.",
        written.file.rows,
        written.sets.len(),
        written.file.path
    );
    Ok(written)
}

/// Where the result sets of a run go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetLayout {
    /// The first set goes to the chosen file. The other sets go to the grid
    /// alone.
    First,
    /// Each set goes to a file of its own. The first set goes to the chosen
    /// file.
    Files,
    /// Each set goes to a sheet of its own in the chosen xlsx file.
    Sheets,
}

impl SetLayout {
    fn new(each_set: bool, format: ExportFormat) -> Self {
        match (each_set, format) {
            (false, _) => SetLayout::First,
            (true, ExportFormat::Xlsx) => SetLayout::Sheets,
            (true, _) => SetLayout::Files,
        }
    }
}

/// What one result set put in the file of a run.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedSet {
    /// The file of the set.
    pub path: String,
    /// The sheet of the set, in an xlsx file with a sheet for each set.
    pub sheet: Option<String>,
    pub rows: usize,
    /// True when the export row limit or the room of a sheet stopped the
    /// set.
    pub truncated: bool,
    /// True when the sheet of the set was full and rows were left out.
    pub sheet_full: bool,
}

/// What a run to a file wrote.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunFileSummary {
    /// The totals of every file. The path is the path of the chosen file.
    #[serde(flatten)]
    pub file: ExportSummary,
    /// Each set that went to a file, in the order of the run.
    pub sets: Vec<SavedSet>,
    /// The number of sets that went to the grid alone.
    pub skipped_sets: usize,
}

/// The number of names that the file of a set can try before the run
/// fails.
const MAX_NAME_TRIES: usize = 20;

/// The path of the file of a set after the first: `orders-2.csv` beside
/// `orders.csv`. The save dialog asked the user before it let the run
/// replace the chosen file, but no dialog asked about this path, so a name
/// that is taken gets a number: `orders-2 (2).csv`.
fn set_path(chosen: &Path, number: usize) -> Result<PathBuf> {
    let stem = chosen
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_default();
    let extension = chosen
        .extension()
        .map(|extension| format!(".{}", extension.to_string_lossy()))
        .unwrap_or_default();
    (1..=MAX_NAME_TRIES)
        .map(|attempt| {
            let name = if attempt == 1 {
                format!("{stem}-{number}{extension}")
            } else {
                format!("{stem}-{number} ({attempt}){extension}")
            };
            chosen.with_file_name(name)
        })
        // A link that points nowhere also takes the name.
        .find(|path| std::fs::symlink_metadata(path).is_err())
        .ok_or_else(|| {
            Error::Invalid(format!(
                "Couldn't find a free file name for result {number} beside '{}'.",
                chosen.display()
            ))
        })
}

/// A sink that sends the result sets of a run to their files, in the
/// layout that the user chose.
struct SetFiles {
    /// The files of the run, in the order of the sets. The first file is
    /// the file that the user chose.
    files: Vec<FileSink>,
    format: ExportFormat,
    layout: SetLayout,
    /// The number of sets that began.
    sets: usize,
    /// The number of sets that went to no file.
    skipped: usize,
    /// True while the open set goes to a file.
    open: bool,
}

impl SetFiles {
    fn new(first: FileSink, format: ExportFormat, layout: SetLayout) -> Self {
        Self {
            files: vec![first],
            format,
            layout,
            sets: 0,
            skipped: 0,
            open: false,
        }
    }

    /// The file of the open set.
    fn current(&mut self) -> &mut FileSink {
        let last = self.files.len() - 1;
        &mut self.files[last]
    }

    /// Closes each file and gives what the run wrote. A run that gave no
    /// result set writes no file. A failure of one file removes the files
    /// that did not finish.
    async fn finish(self) -> Result<RunFileSummary> {
        if self.sets == 0 {
            return Err(Error::Unsupported(
                "The statement returned no result set, so no file was written.".to_string(),
            ));
        }
        let mut sets = Vec::new();
        let mut total: Option<ExportSummary> = None;
        for file in self.files {
            let counts = file.sets.clone();
            let written = file.finish().await?;
            sets.extend(counts.into_iter().map(|count| SavedSet {
                path: written.path.clone(),
                sheet: count.sheet,
                rows: count.rows,
                truncated: count.truncated,
                sheet_full: count.sheet_full,
            }));
            total = Some(match total {
                None => written,
                Some(total) => ExportSummary {
                    rows: total.rows + written.rows,
                    truncated: total.truncated || written.truncated,
                    sheet_full: total.sheet_full || written.sheet_full,
                    cut_cells: total.cut_cells + written.cut_cells,
                    ..total
                },
            });
        }
        let mut file = total.expect("a run with a set has a file");
        file.warning = cut_cells_warning(file.cut_cells);
        Ok(RunFileSummary {
            file,
            sets,
            skipped_sets: self.skipped,
        })
    }
}

impl RowSink for SetFiles {
    fn begin_set(&mut self, columns: Vec<ColumnInfo>) -> Result<()> {
        self.sets += 1;
        self.open = self.layout != SetLayout::First || self.sets == 1;
        if !self.open {
            self.skipped += 1;
            return Ok(());
        }
        if self.layout == SetLayout::Files && self.sets > 1 {
            let path = set_path(&self.files[0].final_path, self.sets)?;
            self.files.push(FileSink::create_now(&path, self.format)?);
        }
        self.current().begin_set(columns)
    }

    /// Gives the row to the file of the open set. The answer is `Stop` when
    /// that file takes no more rows of the set, or when the set goes to no
    /// file.
    fn row(&mut self, row: Vec<serde_json::Value>) -> Result<SinkControl> {
        if !self.open {
            return Ok(SinkControl::Stop);
        }
        self.current().row(row)
    }

    fn end_set(&mut self, truncated: bool) -> Result<()> {
        if !self.open {
            return Ok(());
        }
        self.current().end_set(truncated)
    }

    fn message(&mut self, _message: Message) {}
}

/// A sink that gives each row to the file sink, and the first rows of each
/// set to the grid. The file sink decides which sets go to a file.
///
/// A `Stop` ends the whole run, and SQLite then skips the statements after
/// the one that stopped. So for a script, the sink never answers `Stop`, and
/// the script runs the same statements as a normal run. The driver stops
/// each set at the limit of the file, and the sink drops the rows past the
/// limit of the grid. The file sink itself drops the rows past the room of
/// an Excel sheet and the rows of the sets that go to no file.
///
/// A run of one statement that only reads can stop early. When the grid has
/// its rows and the file takes no more, for example because the Excel sheet
/// is full, the sink answers `Stop` and the driver reads no more rows.
struct TeeSink<F: RowSink, G: RowSink> {
    file: F,
    grid: G,
    /// The row limit of the grid.
    grid_rows: usize,
    /// True when the sink can stop the run, because the run is one
    /// statement that only reads and sends one set to the file.
    may_stop: bool,
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
            may_stop: false,
            shown: 0,
            cut: false,
        }
    }

    /// Lets the sink stop the run when the grid and the file take no more
    /// rows.
    fn stopping_when_full(mut self, may_stop: bool) -> Self {
        self.may_stop = may_stop;
        self
    }
}

impl<F: RowSink, G: RowSink> RowSink for TeeSink<F, G> {
    fn begin_set(&mut self, columns: Vec<ColumnInfo>) -> Result<()> {
        self.shown = 0;
        self.cut = false;
        self.file.begin_set(columns.clone())?;
        self.grid.begin_set(columns)
    }

    fn row(&mut self, row: Vec<serde_json::Value>) -> Result<SinkControl> {
        if self.shown < self.grid_rows {
            self.shown += 1;
            self.grid.row(row.clone())?;
        } else {
            self.cut = true;
        }
        let file = self.file.row(row)?;
        if self.may_stop && self.cut && file == SinkControl::Stop {
            return Ok(SinkControl::Stop);
        }
        Ok(SinkControl::Continue)
    }

    fn end_set(&mut self, truncated: bool) -> Result<()> {
        self.file.end_set(truncated)?;
        self.grid.end_set(truncated || self.cut)
    }

    fn message(&mut self, message: Message) {
        self.grid.message(message);
    }

    fn not_kept(&mut self, reason: UnsavedReason) {
        self.grid.not_kept(reason);
    }

    fn progress(&mut self, rows: u64, bytes: u64) {
        self.grid.progress(rows, bytes);
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
    use crate::sql::Dialect;
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
    fn the_reports_of_a_spill_go_to_the_grid() {
        use crate::db::sink::testing::ReportSink;
        let mut tee = TeeSink::new(ReportSink::default(), ReportSink::default(), 2);
        tee.not_kept(UnsavedReason::DiskFailed);
        tee.progress(7, 8);
        assert!(tee.file.reasons.is_empty());
        assert!(tee.file.progress.is_empty());
        assert_eq!(tee.grid.reasons, vec![UnsavedReason::DiskFailed]);
        assert_eq!(tee.grid.progress, vec![(7, 8)]);
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
    fn one_read_stops_when_the_grid_and_the_file_are_full() {
        for may_stop in [true, false] {
            let mut tee = TeeSink::new(BufferSink::new(2), BufferSink::new(100), 1)
                .stopping_when_full(may_stop);
            tee.begin_set(columns()).unwrap();
            let answers: Vec<SinkControl> =
                (0..4).map(|value| tee.row(row(value)).unwrap()).collect();
            let full = if may_stop {
                SinkControl::Stop
            } else {
                SinkControl::Continue
            };
            assert_eq!(
                answers,
                vec![SinkControl::Continue, SinkControl::Continue, full, full]
            );
        }
        // While the grid takes rows, a full file does not stop the run.
        let mut tee =
            TeeSink::new(BufferSink::new(0), BufferSink::new(100), 2).stopping_when_full(true);
        tee.begin_set(columns()).unwrap();
        assert_eq!(tee.row(row(1)).unwrap(), SinkControl::Continue);
        assert_eq!(tee.row(row(2)).unwrap(), SinkControl::Continue);
        assert_eq!(tee.row(row(3)).unwrap(), SinkControl::Stop);
        assert_eq!(response(tee.grid).results[0].rows.len(), 2);
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
    fn each_set_reaches_the_file_sink_and_the_grid() {
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
        assert_eq!(file.results.len(), 2);
        assert_eq!(file.results[1].rows.len(), 4);
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
    }

    /// A sink of the files of a run, with its first file at the path.
    async fn set_files(path: &Path, each_set: bool) -> SetFiles {
        let format = format_for_path(path);
        let layout = SetLayout::new(each_set, format);
        let first = FileSink::create(path, format).await.unwrap();
        let first = if layout == SetLayout::Sheets {
            first.sheet_per_set()
        } else {
            first
        };
        SetFiles::new(first, format, layout)
    }

    /// Sends sets with the given numbers of rows to a sink.
    fn send_sets(sink: &mut impl RowSink, sets: &[usize]) {
        for (index, count) in sets.iter().enumerate() {
            sink.begin_set(vec![ColumnInfo::new(format!("c{index}"), "int")])
                .unwrap();
            for value in 0..*count {
                sink.row(row(value as i64)).unwrap();
            }
            sink.message(Message::info("rows"));
            sink.end_set(false).unwrap();
        }
    }

    #[test]
    fn the_choice_and_the_format_set_the_layout() {
        assert_eq!(SetLayout::new(false, ExportFormat::Xlsx), SetLayout::First);
        assert_eq!(SetLayout::new(true, ExportFormat::Xlsx), SetLayout::Sheets);
        assert_eq!(SetLayout::new(true, ExportFormat::Json), SetLayout::Files);
    }

    #[tokio::test]
    async fn the_first_set_alone_goes_to_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.csv");
        let mut sink = set_files(&path, false).await;
        sink.begin_set(columns()).unwrap();
        sink.row(row(1)).unwrap();
        sink.end_set(false).unwrap();
        sink.begin_set(columns()).unwrap();
        // The file takes no rows of the second set.
        assert_eq!(sink.row(row(2)).unwrap(), SinkControl::Stop);
        sink.end_set(false).unwrap();

        let summary = sink.finish().await.unwrap();
        assert_eq!(summary.skipped_sets, 1);
        assert_eq!(summary.sets.len(), 1);
        assert_eq!(summary.file.rows, 1);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn each_set_goes_to_a_file_beside_the_chosen_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.csv");
        // A file with the name of the second set stays as it is.
        std::fs::write(dir.path().join("out-2.csv"), "mine").unwrap();
        let mut sink = set_files(&path, true).await;
        send_sets(&mut sink, &[2, 3, 1]);

        let summary = sink.finish().await.unwrap();
        assert_eq!(summary.skipped_sets, 0);
        assert_eq!(summary.file.rows, 6);
        assert_eq!(summary.file.path, path.to_string_lossy());
        let paths: Vec<String> = summary.sets.iter().map(|set| set.path.clone()).collect();
        let named = |name: &str| dir.path().join(name).to_string_lossy().to_string();
        assert_eq!(
            paths,
            vec![named("out.csv"), named("out-2 (2).csv"), named("out-3.csv")]
        );
        assert_eq!(
            summary.sets.iter().map(|set| set.rows).collect::<Vec<_>>(),
            vec![2, 3, 1]
        );
        assert!(summary.sets.iter().all(|set| set.sheet.is_none()));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("out-2.csv")).unwrap(),
            "mine"
        );
        let third = std::fs::read_to_string(dir.path().join("out-3.csv")).unwrap();
        assert!(third.contains("c2"));
    }

    #[tokio::test]
    async fn each_set_of_an_excel_run_goes_to_a_sheet() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.xlsx");
        let mut sink = set_files(&path, true).await;
        send_sets(&mut sink, &[2, 1]);

        let summary = sink.finish().await.unwrap();
        assert_eq!(summary.file.rows, 3);
        let sheets: Vec<Option<String>> =
            summary.sets.iter().map(|set| set.sheet.clone()).collect();
        assert_eq!(
            sheets,
            vec![Some("Result 1".to_string()), Some("Result 2".to_string())]
        );
        assert!(summary.sets.iter().all(|set| set.path == summary.file.path));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn a_run_without_a_set_finishes_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let sink = set_files(&dir.path().join("out.json"), true).await;
        let error = sink.finish().await.unwrap_err();
        assert!(error.to_string().contains("no result set"));
    }

    #[test]
    fn the_file_of_a_set_never_takes_a_name_in_use() {
        let dir = tempfile::tempdir().unwrap();
        let chosen = dir.path().join("a.b.csv");
        assert_eq!(set_path(&chosen, 4).unwrap(), dir.path().join("a.b-4.csv"));
        assert_eq!(
            set_path(Path::new("/x/plain"), 2).unwrap(),
            PathBuf::from("/x/plain-2")
        );
        std::fs::write(dir.path().join("a.b-4.csv"), "").unwrap();
        for attempt in 2..=MAX_NAME_TRIES {
            std::fs::write(dir.path().join(format!("a.b-4 ({attempt}).csv")), "").unwrap();
        }
        let error = set_path(&chosen, 4).unwrap_err();
        assert!(error.to_string().contains("free file name for result 4"));
    }

    #[tokio::test]
    async fn a_set_whose_file_cannot_start_fails_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.csv");
        let mut sink = set_files(&path, true).await;
        send_sets(&mut sink, &[1]);
        // The folder goes away, so the file of the second set cannot start.
        sink.files[0].final_path = dir.path().join("gone").join("out.csv");
        assert!(sink.begin_set(columns()).is_err());
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
    fn a_ticket_works_until_a_run_takes_it() {
        let chosen = ChosenFiles::default();
        let file = remember_choice(&chosen, PathBuf::from("/a/out.xlsx"));
        assert_eq!(file.path, "/a/out.xlsx");
        let (path, format) = chosen.peek(&file.ticket).unwrap();
        assert_eq!(path, PathBuf::from("/a/out.xlsx"));
        assert!(matches!(format, ExportFormat::Xlsx));
        // A look keeps the ticket, and a take removes it.
        assert!(chosen.take(&file.ticket));
        assert!(chosen.peek(&file.ticket).is_none());
        assert!(!chosen.take(&file.ticket));
    }

    #[test]
    fn a_full_list_forgets_the_oldest_choice() {
        let chosen = ChosenFiles::default();
        let first = chosen.remember(PathBuf::from("/a/0.csv"), ExportFormat::Csv);
        let mut last = String::new();
        for number in 1..=MAX_CHOSEN_FILES {
            last = chosen.remember(PathBuf::from(format!("/a/{number}.csv")), ExportFormat::Csv);
        }
        assert!(!chosen.take(&first));
        assert!(chosen.take(&last));
    }

    #[test]
    fn a_choice_expires_after_its_age_limit() {
        let chosen = ChosenFiles::default();
        let start = Instant::now();
        let old = chosen.remember_at(PathBuf::from("/a/old.csv"), ExportFormat::Csv, start);
        let later = start + MAX_CHOSEN_AGE - Duration::from_secs(1);
        let young = chosen.remember_at(PathBuf::from("/a/new.csv"), ExportFormat::Csv, later);
        assert!(chosen.peek_at(&old, later).is_some());

        let expired = start + MAX_CHOSEN_AGE;
        assert!(chosen.peek_at(&old, expired).is_none());
        assert!(chosen.take_at(&young, expired));
        assert!(!chosen.take_at(&old, expired));
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
            each_set: false,
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
    ) -> Result<RunFileSummary> {
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
        assert_eq!(summary.file.rows, 5);
        assert!(!summary.file.truncated);
        assert_eq!(summary.file.path, path.to_string_lossy());
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
        assert_eq!(summary.file.rows, 4);
        assert!(summary.file.truncated);
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
        assert_eq!(summary.file.rows, 5);
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
        assert_eq!(check.file.rows, 1);
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
    async fn a_run_that_fails_before_its_statement_keeps_the_ticket() {
        let (dir, app) = app().await;
        let path = dir.path().join("out.csv");
        let ticket = choose(&app, &path);
        let (channel, _) = frame_channel();
        let mut missing = request(&ticket, "SELECT 1", 100);
        missing.connection_id = "missing".to_string();

        assert!(run(&app, missing, channel).await.is_err());
        assert!(run_file_ready(ticket.clone(), app.state::<ChosenFiles>()));
        assert!(!path.exists());

        // The same ticket then runs, and the run uses it up.
        let (channel, _) = frame_channel();
        run(&app, request(&ticket, "SELECT 1", 100), channel)
            .await
            .unwrap();
        assert!(path.exists());
        assert!(!run_file_ready(ticket, app.state::<ChosenFiles>()));
    }

    #[tokio::test]
    async fn two_runs_with_one_ticket_write_the_file_once() {
        let (dir, app) = app().await;
        let path = dir.path().join("out.csv");
        let ticket = choose(&app, &path);
        let (first, _) = frame_channel();
        let (second, _) = frame_channel();
        let mut other = request(&ticket, &numbers(3), 100);
        other.request_id = "r2".to_string();

        let (one, two) = tokio::join!(
            run(&app, request(&ticket, &numbers(3), 100), first),
            run(&app, other, second)
        );
        let errors: Vec<String> = [one, two]
            .into_iter()
            .filter_map(|outcome| outcome.err().map(|error| error.to_string()))
            .collect();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("expired"));
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 4);
        // The run that lost removed its part of a file.
        assert!(!has_part_file(dir.path()));
    }

    #[tokio::test]
    async fn a_script_can_write_each_of_its_sets() {
        let (dir, app) = app().await;
        let path = dir.path().join("out.json");
        let ticket = choose(&app, &path);
        let (channel, frames) = frame_channel();
        let mut files_request =
            request(&ticket, &format!("{}; SELECT 'a' AS name", numbers(5)), 100);
        files_request.each_set = true;

        let summary = run(&app, files_request, channel).await.unwrap();
        assert_eq!(summary.file.rows, 6);
        assert_eq!(summary.sets.len(), 2);
        assert_eq!(summary.skipped_sets, 0);
        assert!(dir.path().join("out-2.json").exists());
        assert_eq!(cut_flags(&frames), vec![true, false]);

        // An Excel run puts each set on a sheet of the chosen file.
        let path = dir.path().join("out.xlsx");
        let ticket = choose(&app, &path);
        let (channel, _) = frame_channel();
        let mut sheets_request = request(&ticket, "SELECT 1 AS a; SELECT 2 AS b", 100);
        sheets_request.each_set = true;
        let summary = run(&app, sheets_request, channel).await.unwrap();
        let sheets: Vec<Option<String>> =
            summary.sets.iter().map(|set| set.sheet.clone()).collect();
        assert_eq!(
            sheets,
            vec![Some("Result 1".to_string()), Some("Result 2".to_string())]
        );
        assert!(!dir.path().join("out-2.xlsx").exists());
    }

    #[tokio::test]
    async fn the_text_tells_whether_a_run_can_give_several_sets() {
        let several = |query: &str| several_result_sets(query.to_string(), Dialect::Sqlite);
        assert!(several("SELECT 1; SELECT 2").await.unwrap());
        assert!(!several("SELECT 1").await.unwrap());
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
        assert_eq!(summary.file.rows, 5);
        assert_eq!(cut_flags(&frames), vec![false]);
    }
}
