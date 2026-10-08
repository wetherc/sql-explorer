use super::*;
use crate::db::sink::{feed, BufferSink};
use crate::db::{AppColumn, Database, DriverCapabilities, ResultSet, Schema, Table};
use crate::sql::Dialect;
use serde_json::json;

/// A driver that reads `rows` rows of one column `n` through `feed`, as a
/// driver that pauses does.
pub(crate) struct RowsDriver {
    pub rows: usize,
    /// The driver fails before it gives this row.
    pub fail_at: Option<usize>,
    /// The driver waits forever before it gives this row, as a slow server.
    pub hang_at: Option<usize>,
    /// The driver panics before it gives this row.
    pub panic_at: Option<usize>,
    /// The driver reports a session that stays fit for use after a stop.
    pub keeps_after_stop: bool,
    /// After a stop of the sink, the driver waits forever, as a server that
    /// does not answer.
    pub hang_on_stop: bool,
    /// The row limit of each run, so a test can see what the driver got.
    pub limits: Arc<Mutex<Vec<usize>>>,
}

impl RowsDriver {
    pub(crate) fn new(rows: usize) -> Self {
        Self {
            rows,
            fail_at: None,
            hang_at: None,
            panic_at: None,
            keeps_after_stop: false,
            hang_on_stop: false,
            limits: Arc::default(),
        }
    }
}

#[async_trait::async_trait]
impl DatabaseDriver for RowsDriver {
    fn capabilities(&self) -> DriverCapabilities {
        DriverCapabilities::default()
    }
    fn dialect(&self) -> Dialect {
        Dialect::Postgres
    }
    fn pauses_reads(&self) -> bool {
        true
    }
    fn keeps_connection_after_stop(&self) -> bool {
        self.keeps_after_stop
    }
    async fn ping(&mut self) -> Result<()> {
        Ok(())
    }
    async fn execute_stream(
        &mut self,
        _query: &str,
        _params: Option<&QueryParams>,
        options: &ExecOptions,
        sink: &mut dyn RowSink,
    ) -> Result<RunSummary> {
        self.limits.lock().unwrap().push(options.max_rows);
        sink.message(Message::info("start"));
        sink.begin_set(vec![ColumnInfo::new("n", "int")])?;
        let mut truncated = false;
        for row in 0..self.rows {
            if self.fail_at == Some(row) {
                return Err(Error::Invalid("The read failed.".to_string()));
            }
            if self.hang_at == Some(row) {
                std::future::pending::<()>().await;
            }
            assert_ne!(self.panic_at, Some(row), "the driver panics");
            if feed(sink, vec![json!(row)]).await? == SinkControl::Stop {
                truncated = true;
                if self.hang_on_stop {
                    std::future::pending::<()>().await;
                }
                break;
            }
        }
        sink.end_set(truncated)?;
        sink.message(Message::info("end"));
        Ok(RunSummary {
            rows_affected: None,
            elapsed_ms: 1,
            stats: None,
        })
    }
    async fn list_databases(&mut self) -> Result<Vec<Database>> {
        Ok(Vec::new())
    }
    async fn list_schemas(&mut self, _database: &str) -> Result<Vec<Schema>> {
        Ok(Vec::new())
    }
    async fn list_tables(&mut self, _database: &str, _schema: Option<&str>) -> Result<Vec<Table>> {
        Ok(Vec::new())
    }
    async fn list_columns(
        &mut self,
        _database: &str,
        _schema: Option<&str>,
        _table: &str,
    ) -> Result<Vec<AppColumn>> {
        Ok(Vec::new())
    }
}

fn point(rows: usize) -> PausePoint {
    PausePoint {
        rows,
        limit: Duration::from_secs(60),
    }
}

fn columns() -> Vec<ColumnInfo> {
    vec![ColumnInfo::new("n", "int")]
}

/// The one set of a buffer.
fn only_set(buffer: BufferSink) -> (ResultSet, Vec<Message>) {
    let mut response = buffer.into_response(RunSummary::default());
    (response.results.remove(0), response.messages)
}

/// A shared buffer for the rows of an export.
fn export_buffer() -> (Box<dyn RowSink>, Arc<Mutex<BufferSink>>) {
    let shared = Arc::new(Mutex::new(BufferSink::new(usize::MAX)));
    (Box::new(SharedSink(shared.clone())), shared)
}

fn take_buffer(shared: Arc<Mutex<BufferSink>>) -> BufferSink {
    Arc::into_inner(shared)
        .expect("the test keeps the last reference")
        .into_inner()
        .unwrap()
}

/// Gives the sink `count` rows from `start`, and the answer to the last
/// one.
fn rows(sink: &mut dyn RowSink, start: usize, count: usize) -> SinkControl {
    let mut control = SinkControl::Continue;
    for row in start..start + count {
        control = sink.row(vec![json!(row)]).unwrap();
    }
    control
}

#[test]
fn a_read_below_the_limit_never_pauses() {
    let (mut sink, _control) = PausingSink::new(BufferSink::new(3), point(3));
    assert_eq!(sink.pause_point(), Some(point(3)));
    sink.message(Message::info("note"));
    sink.begin_set(columns()).unwrap();
    assert_eq!(rows(&mut sink, 0, 3), SinkControl::Continue);
    sink.keep_source(crate::kept::tests::fixed(1));
    sink.end_set(false).unwrap();
    // A second set starts its own count.
    sink.begin_set(columns()).unwrap();
    assert_eq!(rows(&mut sink, 0, 3), SinkControl::Continue);
    sink.end_set(false).unwrap();

    let grid = sink.into_grid().expect("the read never paused");
    let response = grid.into_response(RunSummary::default());
    assert_eq!(response.results.len(), 2);
    assert_eq!(response.results[1].rows.len(), 3);
    assert_eq!(response.messages[0].text, "note");
}

#[tokio::test]
async fn a_resume_without_a_pause_stops_the_read() {
    let (mut sink, _control) = PausingSink::new(BufferSink::new(3), point(3));
    assert_eq!(sink.resume().await.unwrap(), SinkControl::Stop);
    assert!(sink.into_grid().is_some());
}

#[tokio::test]
async fn an_export_takes_the_rows_of_the_grid_and_the_rest_of_the_read() {
    let (mut sink, control) = PausingSink::new(BufferSink::new(2), point(2));
    sink.begin_set(columns()).unwrap();
    assert_eq!(rows(&mut sink, 0, 2), SinkControl::Continue);
    assert_eq!(rows(&mut sink, 2, 1), SinkControl::Pause);

    let waiting = tokio::spawn(async move {
        let control = sink.resume().await.unwrap();
        (control, sink)
    });
    let handoff = control.handoff.await.unwrap();
    assert_eq!(handoff.set, 0);
    let (grid, _) = only_set(handoff.grid);
    assert_eq!(grid.rows, vec![vec![json!(0)], vec![json!(1)]]);
    assert!(grid.truncated);

    let (export, shared) = export_buffer();
    assert!(control
        .commands
        .send(Resume::Continue {
            sink: export,
            max_rows: 10,
        })
        .is_ok());
    let (resumed, mut sink) = waiting.await.unwrap();
    assert_eq!(resumed, SinkControl::Continue);
    assert_eq!(rows(&mut sink, 3, 2), SinkControl::Continue);
    sink.message(Message::info("later"));
    sink.keep_source(crate::kept::tests::fixed(1));
    sink.begin_set(columns()).unwrap();
    sink.end_set(false).unwrap();
    assert!(sink.into_grid().is_none());

    let (set, messages) = only_set(take_buffer(shared));
    let values: Vec<_> = set.rows.iter().map(|row| row[0].clone()).collect();
    assert_eq!(values, (0..5).map(|n| json!(n)).collect::<Vec<_>>());
    assert!(!set.truncated);
    assert_eq!(messages[0].text, "later");
}

/// Pauses a sink with a grid of `grid` rows after its first set, and
/// continues it into an export of `max_rows` rows.
async fn exported(grid: usize, max_rows: usize) -> (SinkControl, PausingSink<BufferSink>) {
    let (mut sink, control) = PausingSink::new(BufferSink::new(grid), point(grid));
    sink.begin_set(columns()).unwrap();
    rows(&mut sink, 0, grid + 1);
    let (export, _shared) = export_buffer();
    assert!(control
        .commands
        .send(Resume::Continue {
            sink: export,
            max_rows,
        })
        .is_ok());
    let _handoff = control.handoff;
    // The command came first, so the wait takes it at once.
    let control = sink.resume().await.unwrap();
    (control, sink)
}

#[tokio::test]
async fn the_row_limit_of_the_export_cuts_the_rows_of_the_grid() {
    let (control, mut sink) = exported(3, 2).await;
    assert_eq!(control, SinkControl::Stop);
    assert_eq!(rows(&mut sink, 4, 1), SinkControl::Stop);
    sink.end_set(false).unwrap();
}

#[tokio::test]
async fn the_row_limit_of_the_export_cuts_the_rest_of_the_read() {
    let (control, mut sink) = exported(1, 3).await;
    assert_eq!(control, SinkControl::Continue);
    assert_eq!(rows(&mut sink, 2, 1), SinkControl::Continue);
    assert_eq!(rows(&mut sink, 3, 1), SinkControl::Stop);
}

#[tokio::test]
async fn a_release_stops_the_read() {
    let (mut sink, control) = PausingSink::new(BufferSink::new(1), point(1));
    sink.begin_set(columns()).unwrap();
    rows(&mut sink, 0, 2);
    let released = sink.released.clone();
    assert!(control.commands.send(Resume::Release).is_ok());
    let _handoff = control.handoff;
    assert_eq!(sink.resume().await.unwrap(), SinkControl::Stop);
    // The task of the read hears of the release.
    released.notified().await;
    assert_eq!(rows(&mut sink, 2, 1), SinkControl::Stop);
    sink.begin_set(columns()).unwrap();
    sink.end_set(true).unwrap();
    sink.message(Message::info("dropped"));
    assert_eq!(sink.resume().await.unwrap(), SinkControl::Stop);
}

#[tokio::test]
async fn a_command_end_that_is_gone_releases_the_read() {
    let (mut sink, control) = PausingSink::new(BufferSink::new(1), point(1));
    sink.begin_set(columns()).unwrap();
    rows(&mut sink, 0, 2);
    drop(control.commands);
    let _handoff = control.handoff;
    assert_eq!(sink.resume().await.unwrap(), SinkControl::Stop);
}

#[tokio::test]
async fn a_run_that_is_gone_releases_the_read() {
    let (mut sink, control) = PausingSink::new(BufferSink::new(1), point(1));
    sink.begin_set(columns()).unwrap();
    rows(&mut sink, 0, 2);
    drop(control.handoff);
    assert_eq!(sink.resume().await.unwrap(), SinkControl::Stop);
    assert!(control.commands.is_closed());
}

#[tokio::test(start_paused = true)]
async fn the_end_of_the_pause_releases_the_read() {
    let (mut sink, control) = PausingSink::new(BufferSink::new(1), point(1));
    sink.begin_set(columns()).unwrap();
    rows(&mut sink, 0, 2);
    let _handoff = control.handoff;
    assert_eq!(sink.resume().await.unwrap(), SinkControl::Stop);
    assert!(control.commands.is_closed());
}

#[tokio::test(start_paused = true)]
async fn a_command_at_the_end_of_the_pause_still_counts() {
    let (sink, control) = PausingSink::new(BufferSink::new(1), point(1));
    drop(sink);
    let (commands, receiver) = oneshot::channel();
    assert!(commands.send(Resume::Release).is_ok());
    drop(control);
    // The limit is zero, so the wait takes the command after the limit.
    assert!(matches!(
        wait_for_command(receiver, Duration::ZERO).await,
        Resume::Release
    ));
    let (commands, receiver) = oneshot::channel();
    let (export, _shared) = export_buffer();
    assert!(commands
        .send(Resume::Continue {
            sink: export,
            max_rows: 1,
        })
        .is_ok());
    assert!(matches!(
        wait_for_command(receiver, Duration::ZERO).await,
        Resume::Continue { max_rows: 1, .. }
    ));
}

#[test]
fn a_set_too_large_to_copy_ends_at_the_limit_without_a_pause() {
    let (mut sink, _control) = PausingSink::new(BufferSink::new(4), point(4));
    // Each number weighs 24 bytes, so the third row passes the budget.
    sink.budget = 50;
    sink.begin_set(columns()).unwrap();
    assert_eq!(rows(&mut sink, 0, 2), SinkControl::Continue);
    assert_eq!(sink.rows.len(), 2);
    assert_eq!(rows(&mut sink, 2, 2), SinkControl::Continue);
    assert!(sink.rows.is_empty());
    // The row past the limit goes to the grid, which ends the read.
    assert_eq!(rows(&mut sink, 4, 2), SinkControl::Stop);
    assert!(sink.held.is_none());
    sink.end_set(true).unwrap();

    // The next set copies its rows again.
    sink.begin_set(columns()).unwrap();
    assert_eq!(rows(&mut sink, 0, 1), SinkControl::Continue);
    assert_eq!(sink.rows.len(), 1);
    sink.end_set(false).unwrap();

    let grid = sink.into_grid().expect("the read never paused");
    let response = grid.into_response(RunSummary::default());
    assert!(response.results[0].truncated);
    let notes: Vec<_> = response
        .messages
        .iter()
        .filter(|message| message.text == TOO_LARGE_TO_PAUSE)
        .collect();
    assert_eq!(notes.len(), 1);
}

/// A sink of an export that refuses each row.
struct RefusingRows;

impl RowSink for RefusingRows {
    fn begin_set(&mut self, _columns: Vec<ColumnInfo>) -> Result<()> {
        Ok(())
    }
    fn row(&mut self, _row: Vec<JsonValue>) -> Result<SinkControl> {
        Err(Error::Invalid("The disk is full.".to_string()))
    }
    fn end_set(&mut self, _truncated: bool) -> Result<()> {
        Ok(())
    }
    fn message(&mut self, _message: Message) {}
}

#[tokio::test]
async fn an_export_that_refuses_a_row_of_the_grid_fails_the_read() {
    let (mut sink, control) = PausingSink::new(BufferSink::new(1), point(1));
    sink.begin_set(columns()).unwrap();
    rows(&mut sink, 0, 2);
    assert!(control
        .commands
        .send(Resume::Continue {
            sink: Box::new(RefusingRows),
            max_rows: 5,
        })
        .is_ok());
    let _handoff = control.handoff;
    assert!(matches!(sink.resume().await, Err(Error::Invalid(_))));
}

/// A grid whose end fails, as a window that closed.
struct BrokenGrid;

impl RowSink for BrokenGrid {
    fn begin_set(&mut self, _columns: Vec<ColumnInfo>) -> Result<()> {
        Ok(())
    }
    fn row(&mut self, _row: Vec<JsonValue>) -> Result<SinkControl> {
        Ok(SinkControl::Continue)
    }
    fn end_set(&mut self, _truncated: bool) -> Result<()> {
        Err(Error::Invalid("The window closed.".to_string()))
    }
    fn message(&mut self, _message: Message) {}
}

#[tokio::test]
async fn a_grid_that_fails_at_the_pause_fails_the_read() {
    let (mut sink, _control) = PausingSink::new(BrokenGrid, point(1));
    sink.begin_set(columns()).unwrap();
    rows(&mut sink, 0, 2);
    assert!(matches!(sink.resume().await, Err(Error::Invalid(_))));
    // The grid stays, so the run can send its end frame.
    assert!(sink.into_grid().is_some());
}

/// A sink of an export that refuses the columns.
struct RefusingExport;

impl RowSink for RefusingExport {
    fn begin_set(&mut self, _columns: Vec<ColumnInfo>) -> Result<()> {
        Err(Error::Invalid("The disk is full.".to_string()))
    }
    fn row(&mut self, _row: Vec<JsonValue>) -> Result<SinkControl> {
        Ok(SinkControl::Continue)
    }
    fn end_set(&mut self, _truncated: bool) -> Result<()> {
        Ok(())
    }
    fn message(&mut self, _message: Message) {}
}

#[tokio::test]
async fn an_export_that_fails_at_the_start_fails_the_read() {
    let (mut sink, control) = PausingSink::new(BufferSink::new(1), point(1));
    sink.begin_set(columns()).unwrap();
    rows(&mut sink, 0, 2);
    assert!(control
        .commands
        .send(Resume::Continue {
            sink: Box::new(RefusingExport),
            max_rows: 5,
        })
        .is_ok());
    let _handoff = control.handoff;
    assert!(matches!(sink.resume().await, Err(Error::Invalid(_))));
}

#[test]
fn a_shared_sink_gives_each_call_to_its_sink() {
    let (mut shared, buffer) = export_buffer();
    shared.begin_set(columns()).unwrap();
    assert_eq!(rows(shared.as_mut(), 0, 1), SinkControl::Continue);
    shared.message(Message::info("note"));
    shared.end_set(true).unwrap();
    drop(shared);
    let (set, messages) = only_set(take_buffer(buffer));
    assert_eq!(set.rows.len(), 1);
    assert!(set.truncated);
    assert_eq!(messages.len(), 1);
}

/// A session of a pool of its own, under the key `t1`.
async fn slot(driver: RowsDriver) -> SessionSlot {
    let sessions = Arc::new(SessionPool::new(4));
    let session = sessions.insert("t1", Session::new(Box::new(driver))).await;
    SessionSlot {
        sessions,
        key: "t1".to_string(),
        session,
    }
}

#[tokio::test]
async fn a_discard_closes_the_session_of_the_slot_alone() {
    let slot = slot(RowsDriver::new(0)).await;
    // A new session in the slot stays.
    let newer = slot
        .sessions
        .insert("t1", Session::new(Box::new(RowsDriver::new(0))))
        .await;
    slot.discard().await;
    assert!(slot.session.is_broken());
    assert!(Arc::ptr_eq(&slot.sessions.get("t1").await.unwrap(), &newer));

    // The session of the slot goes.
    let slot = SessionSlot {
        session: newer,
        ..slot
    };
    slot.discard().await;
    assert!(slot.sessions.get("t1").await.is_none());
    // A slot that is already empty stays empty.
    slot.discard().await;
}

#[tokio::test]
async fn work_that_ends_needs_no_release() {
    let released = Arc::new(Notify::new());
    let summary = drive(
        async { Ok(RunSummary::default()) },
        released,
        Duration::from_secs(1),
        async { panic!("the work ended in time") },
    )
    .await
    .unwrap();
    assert_eq!(summary, RunSummary::default());
}

#[tokio::test(start_paused = true)]
async fn a_release_gives_the_work_a_limit() {
    let released = Arc::new(Notify::new());
    released.notify_one();
    let ended = drive(
        async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            Ok(RunSummary::default())
        },
        released.clone(),
        Duration::from_secs(5),
        async {},
    )
    .await;
    assert!(ended.is_ok());

    released.notify_one();
    let overran = Arc::new(Mutex::new(false));
    let flag = overran.clone();
    let ended = drive(
        std::future::pending(),
        released,
        Duration::from_secs(5),
        async move { *flag.lock().unwrap() = true },
    )
    .await;
    assert!(matches!(ended, Err(Error::Timeout(5))));
    assert!(*overran.lock().unwrap());
}

#[tokio::test]
async fn a_stopped_task_ends_with_its_holder() {
    let task = AbortOnDrop::new(tokio::spawn(std::future::pending::<()>()));
    let handle = task.disarm();
    assert!(!handle.is_finished());
    let task = AbortOnDrop::new(handle);
    drop(task);
    // A disarmed task runs on until its own end.
    let kept = AbortOnDrop::new(tokio::spawn(async { 7 })).disarm();
    assert_eq!(kept.await.unwrap(), 7);

    let failed = tokio::spawn(async { panic!("stop") });
    let error = AbortOnDrop::new(failed).await.unwrap_err();
    assert!(task_fault(error)
        .to_string()
        .contains("stopped unexpectedly"));
}

#[test]
fn the_pause_limit_has_a_ceiling() {
    assert_eq!(pause_limit(0), None);
    assert_eq!(pause_limit(30), Some(Duration::from_secs(30)));
    assert_eq!(pause_limit(u64::MAX), Some(MAX_PAUSE));
}

/// Starts a read of `rows` rows that pauses after `grid` rows, and waits
/// for the pause.
async fn paused_read(driver: RowsDriver, grid: usize) -> (PausedRead<BufferSink>, BufferSink) {
    paused_read_into(driver, BufferSink::new(grid), grid).await
}

/// Starts a read that pauses after `rows` rows into the grid, and waits for
/// the pause.
async fn paused_read_into<G: RowSink + 'static>(
    driver: RowsDriver,
    grid: G,
    rows: usize,
) -> (PausedRead<G>, G) {
    let slot = slot(driver).await;
    let guard = slot.session.driver.clone().lock_owned().await;
    let (sink, control) = PausingSink::new(grid, point(rows));
    let task = spawn_read(
        guard,
        "SELECT n".to_string(),
        None,
        ExecOptions::default(),
        sink,
        slot.clone(),
    );
    let handoff = control.handoff.await.unwrap();
    let read = PausedRead::new(control.commands, task, slot, Duration::from_secs(60));
    (read, handoff.grid)
}

/// A read that paused after `grid` rows, as the registry keeps it. The grid
/// sends its frames nowhere.
pub(crate) async fn paused_chunk_read(driver: RowsDriver, grid: usize) -> PausedRead {
    let channel = tauri::ipc::Channel::new(|_| Ok(()));
    let chunks = crate::db::columnar::ChunkSink::new(channel, grid);
    paused_read_into(driver, chunks, grid).await.0
}

#[tokio::test]
async fn a_paused_read_continues_into_an_export_once() {
    let (read, grid) = paused_read(RowsDriver::new(5), 2).await;
    assert!(read.is_live());
    assert!(read.uses(&read.slot().session.clone()));
    assert_eq!(read.limit(), Duration::from_secs(60));
    assert_eq!(only_set(grid).0.rows.len(), 2);
    // The task keeps the driver while the read is paused.
    assert!(read.slot().session.driver.try_lock().is_err());

    let (export, shared) = export_buffer();
    let summary = read.continue_into(export, 100).await.unwrap();
    assert_eq!(summary.elapsed_ms, 1);
    let (set, messages) = only_set(take_buffer(shared));
    assert_eq!(set.rows.len(), 5);
    assert_eq!(messages.last().unwrap().text, "end");
    assert!(!read.is_live());
    assert!(read.slot().session.driver.try_lock().is_ok());

    // The rest of the rows went to the first export.
    let (export, _shared) = export_buffer();
    let error = read.continue_into(export, 100).await.unwrap_err();
    assert!(matches!(&error, Error::Invalid(text) if text.contains("released")));
}

#[tokio::test]
async fn a_read_that_panics_after_the_pause_fails_the_export() {
    let driver = RowsDriver {
        panic_at: Some(3),
        ..RowsDriver::new(5)
    };
    let (read, _grid) = paused_read(driver, 2).await;
    let (export, _shared) = export_buffer();
    let error = read.continue_into(export, 100).await.unwrap_err();
    assert!(error.to_string().contains("stopped unexpectedly"));
}

#[tokio::test]
async fn a_dropped_paused_read_frees_the_driver() {
    let (read, _grid) = paused_read(RowsDriver::new(5), 2).await;
    let session = read.slot().session.clone();
    drop(read);
    // The release makes the driver end the statement and free the lock.
    let guard = session.driver.lock().await;
    drop(guard);
    assert!(!session.is_broken());
}

#[tokio::test]
async fn a_paused_read_that_its_limit_released_is_gone() {
    let slot = slot(RowsDriver::new(5)).await;
    let guard = slot.session.driver.clone().lock_owned().await;
    let limit = PausePoint {
        rows: 1,
        limit: Duration::from_millis(1),
    };
    let (sink, control) = PausingSink::new(BufferSink::new(1), limit);
    let task = spawn_read(
        guard,
        "SELECT n".to_string(),
        None,
        ExecOptions::default(),
        sink,
        slot.clone(),
    );
    let _grid = control.handoff.await.unwrap();
    let session = slot.session.clone();
    let read = PausedRead::new(control.commands, task, slot, limit.limit);
    // The task ends the read after its limit and frees the driver.
    drop(session.driver.lock().await);
    assert!(!read.is_live());
    let (export, _shared) = export_buffer();
    assert!(matches!(
        read.continue_into(export, 10).await,
        Err(Error::Invalid(_))
    ));
}

#[tokio::test(start_paused = true)]
async fn a_release_that_passes_its_limit_closes_the_session() {
    let driver = RowsDriver {
        hang_on_stop: true,
        ..RowsDriver::new(5)
    };
    let (read, _grid) = paused_read(driver, 2).await;
    let session = read.slot().session.clone();
    let sessions = read.slot().sessions.clone();
    drop(read);
    drop(session.driver.lock().await);
    assert!(session.is_broken());
    assert!(sessions.get("t1").await.is_none());
}
