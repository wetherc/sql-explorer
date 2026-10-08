//! The pause of a read at the row limit of the grid.
//!
//! A read that reaches the row limit normally ends there: the driver closes
//! its cursor or asks the server to stop the statement. When the user turns
//! the pause on, the read stops at the limit and the statement stays open on
//! the session of the tab. The grid gets the first rows, and the run ends for
//! the user. An export of all rows then continues the same read into a file,
//! so the statement does not run a second time.
//!
//! The driver runs in a task of its own, and the task owns the lock of the
//! driver of the session. [`PausingSink`] gives the rows to the grid. At the
//! first row past the limit it gives the grid back to the command of the run
//! and waits for a [`Resume`] command. The command of the run puts a
//! [`PausedRead`] in the registry of kept results, and the window gets its
//! identifier.
//!
//! While the read is paused, the task keeps the lock of the driver. A command
//! that needs the driver of that session therefore releases the paused read
//! first, and then waits for the lock. A release makes the sink answer `Stop`,
//! so the driver ends the statement in its normal way. A release that does not
//! end within [`RELEASE_LIMIT`] drops the exchange, and the session closes.

use crate::db::columnar::value_weight;
use crate::db::drivers::DatabaseDriver;
use crate::db::sink::{PausePoint, RowSink, RunSummary, SinkControl};
use crate::db::{ColumnInfo, ExecOptions, Message, QueryParams};
use crate::error::{Error, Result};
use crate::kept::KeptSource;
use crate::session::{Session, SessionPool};
use serde_json::Value as JsonValue;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::sync::{oneshot, Notify, OwnedMutexGuard};
use tokio::task::JoinHandle;

/// The longest time a read stays paused, whatever the window asks for.
pub const MAX_PAUSE: Duration = Duration::from_secs(60 * 60);

/// The longest time a released read takes to end its statement. A driver
/// that ends a statement reads or drops the rows in flight, and a server
/// that stopped answering would keep the session busy without this limit.
pub const RELEASE_LIMIT: Duration = Duration::from_secs(30);

/// The most bytes of grid rows that a [`PausingSink`] copies, as
/// `value_weight` measures them. The copy stays in memory for the whole
/// pause, and the grid limit can be one million rows. A set whose first rows
/// weigh more than this ends at the row limit as a read without a pause, and
/// the grid gets a note that tells why.
pub const PAUSE_COPY_BYTES: usize = 128 * 1024 * 1024;

/// The note for a read that did not pause, because its rows passed
/// [`PAUSE_COPY_BYTES`].
pub const TOO_LARGE_TO_PAUSE: &str = "The rows shown are too large to keep in memory during a \
     pause, so the read ended at the row limit. Export all rows runs the statement again.";

/// What the window asks of a paused read.
pub enum Resume {
    /// Gives the rest of the rows to the sink, after the rows that the grid
    /// shows, up to `max_rows` rows in total.
    Continue {
        sink: Box<dyn RowSink>,
        max_rows: usize,
    },
    /// Ends the statement and frees the session.
    Release,
}

/// What the sink gives the command of the run when the read pauses.
pub struct Handoff<G> {
    /// The sink of the grid, after the end of the set that paused.
    pub grid: G,
    /// The number of the set that paused, from zero, as the frames name it.
    pub set: u32,
}

/// The part of the read that the sink serves now, with the sink that takes
/// the rows of that part.
enum Phase<G> {
    /// The rows go to the grid.
    Visible(G),
    /// The read waits for a command.
    Paused,
    /// The rows go to the sink of an export.
    Export(Box<dyn RowSink>),
    /// The read ends, and the sink takes no more rows.
    Released,
}

/// A sink that gives the rows to the grid and pauses the read at the row
/// limit. It keeps a copy of the rows of the grid, because an export of all
/// rows writes them first and the grid sink does not keep them.
///
/// The copy has a limit of [`PAUSE_COPY_BYTES`]. When the rows of the open
/// set pass it, the sink frees the copy and stops the copy for that set. The
/// first row past the row limit then goes to the grid, which answers `Stop`,
/// so the driver ends the read as it does without a pause.
pub struct PausingSink<G> {
    phase: Phase<G>,
    point: PausePoint,
    /// The count of the sets that began.
    sets: u32,
    columns: Vec<ColumnInfo>,
    /// The copy of the rows of the open set that the grid received.
    rows: Vec<Vec<JsonValue>>,
    /// The count of the rows of the open set that the grid received.
    given: usize,
    /// The measure of the bytes of `rows`.
    weight: usize,
    /// The most bytes that `rows` can have, normally [`PAUSE_COPY_BYTES`].
    budget: usize,
    /// False after the copy of the open set passed `budget`. The set then
    /// cannot pause.
    copying: bool,
    /// The first row past the limit, which made the read pause.
    held: Option<Vec<JsonValue>>,
    handoff: Option<oneshot::Sender<Handoff<G>>>,
    commands: Option<oneshot::Receiver<Resume>>,
    /// Wakes the task when the read is released, so the limit of the
    /// release starts.
    released: Arc<Notify>,
    /// The rows that the export can still take.
    export_room: usize,
    /// True when the row limit of the export cut the set.
    export_cut: bool,
}

/// The two ends of a [`PausingSink`] that the command of the run keeps.
pub struct PauseControl<G> {
    /// Gives the grid when the read pauses.
    pub handoff: oneshot::Receiver<Handoff<G>>,
    /// Takes the command for the paused read.
    pub commands: oneshot::Sender<Resume>,
}

impl<G> PausingSink<G> {
    /// Builds the sink, and the ends that the command of the run keeps. The
    /// grid sink must stop at the same count of rows as `point`.
    pub fn new(grid: G, point: PausePoint) -> (Self, PauseControl<G>) {
        let (handoff_sender, handoff) = oneshot::channel();
        let (commands, command_receiver) = oneshot::channel();
        let sink = Self {
            phase: Phase::Visible(grid),
            point,
            sets: 0,
            columns: Vec::new(),
            rows: Vec::new(),
            given: 0,
            weight: 0,
            budget: PAUSE_COPY_BYTES,
            copying: true,
            held: None,
            handoff: Some(handoff_sender),
            commands: Some(command_receiver),
            released: Arc::new(Notify::new()),
            export_room: 0,
            export_cut: false,
        };
        (sink, PauseControl { handoff, commands })
    }

    /// The grid sink, when the read never paused.
    pub fn into_grid(self) -> Option<G> {
        match self.phase {
            Phase::Visible(grid) => Some(grid),
            _ => None,
        }
    }

    /// Starts the export: the columns, the rows of the grid, and the row
    /// that made the read pause.
    fn start_export(&mut self, mut sink: Box<dyn RowSink>, max_rows: usize) -> Result<SinkControl> {
        self.export_room = max_rows;
        sink.begin_set(std::mem::take(&mut self.columns))?;
        let rows = std::mem::take(&mut self.rows);
        let mut control = SinkControl::Continue;
        for row in rows.into_iter().chain(self.held.take()) {
            control = export_row(
                sink.as_mut(),
                &mut self.export_room,
                &mut self.export_cut,
                row,
            )?;
            if control == SinkControl::Stop {
                break;
            }
        }
        self.phase = Phase::Export(sink);
        Ok(control)
    }

    /// Ends the read, and frees the copy of the rows.
    fn release(&mut self) -> SinkControl {
        self.phase = Phase::Released;
        self.rows = Vec::new();
        self.held = None;
        self.released.notify_one();
        SinkControl::Stop
    }
}

/// Waits for the command of the window, up to the limit of the pause. A
/// command that came at the same moment as the limit still counts, so an
/// export never loses its sink without an error.
async fn wait_for_command(mut commands: oneshot::Receiver<Resume>, limit: Duration) -> Resume {
    tokio::select! {
        biased;
        () = tokio::time::sleep(limit) => commands.try_recv().unwrap_or(Resume::Release),
        command = &mut commands => command.unwrap_or(Resume::Release),
    }
}

/// Gives one row to the sink of an export, inside the row limit of the
/// export.
fn export_row(
    export: &mut dyn RowSink,
    room: &mut usize,
    cut: &mut bool,
    row: Vec<JsonValue>,
) -> Result<SinkControl> {
    if *room == 0 {
        *cut = true;
        return Ok(SinkControl::Stop);
    }
    *room -= 1;
    export.row(row)
}

#[async_trait::async_trait]
impl<G: RowSink + 'static> RowSink for PausingSink<G> {
    fn begin_set(&mut self, columns: Vec<ColumnInfo>) -> Result<()> {
        match &mut self.phase {
            Phase::Visible(grid) => {
                self.sets += 1;
                self.rows.clear();
                self.given = 0;
                self.weight = 0;
                self.copying = true;
                self.columns = columns.clone();
                grid.begin_set(columns)
            }
            Phase::Export(export) => export.begin_set(columns),
            Phase::Paused | Phase::Released => Ok(()),
        }
    }

    fn row(&mut self, row: Vec<JsonValue>) -> Result<SinkControl> {
        match &mut self.phase {
            Phase::Visible(grid) if self.given < self.point.rows => {
                self.given += 1;
                if self.copying {
                    self.weight += row.iter().map(value_weight).sum::<usize>();
                    self.copying = self.weight <= self.budget;
                }
                if self.copying {
                    self.rows.push(row.clone());
                } else {
                    self.rows = Vec::new();
                }
                grid.row(row)
            }
            Phase::Visible(grid) if !self.copying => {
                // The count goes past the limit, so a second row past the
                // limit does not repeat the note.
                if self.given == self.point.rows {
                    self.given += 1;
                    grid.message(Message::info(TOO_LARGE_TO_PAUSE));
                }
                grid.row(row)
            }
            Phase::Visible(_) => {
                self.held = Some(row);
                Ok(SinkControl::Pause)
            }
            Phase::Export(export) => export_row(
                export.as_mut(),
                &mut self.export_room,
                &mut self.export_cut,
                row,
            ),
            Phase::Paused | Phase::Released => Ok(SinkControl::Stop),
        }
    }

    fn end_set(&mut self, truncated: bool) -> Result<()> {
        match &mut self.phase {
            Phase::Visible(grid) => grid.end_set(truncated),
            Phase::Export(export) => export.end_set(truncated || self.export_cut),
            Phase::Paused | Phase::Released => Ok(()),
        }
    }

    fn message(&mut self, message: Message) {
        match &mut self.phase {
            Phase::Visible(grid) => grid.message(message),
            Phase::Export(export) => export.message(message),
            Phase::Paused | Phase::Released => {}
        }
    }

    fn keep_source(&mut self, source: KeptSource) {
        if let Phase::Visible(grid) = &mut self.phase {
            grid.keep_source(source);
        }
    }

    fn pause_point(&self) -> Option<PausePoint> {
        Some(self.point)
    }

    /// Ends the set of the grid, gives the grid to the command of the run,
    /// and waits for the command of the window. A command of the run that
    /// is gone, a window that releases the read, and the end of the pause
    /// limit all end the read.
    async fn resume(&mut self) -> Result<SinkControl> {
        let mut grid = match std::mem::replace(&mut self.phase, Phase::Paused) {
            Phase::Visible(grid) if self.held.is_some() => grid,
            other => {
                self.phase = other;
                return Ok(SinkControl::Stop);
            }
        };
        if let Err(error) = grid.end_set(true) {
            self.phase = Phase::Visible(grid);
            return Err(error);
        }
        let handoff = Handoff {
            grid,
            set: self.sets.saturating_sub(1),
        };
        let commands = self.commands.take();
        let given = self
            .handoff
            .take()
            .is_some_and(|sender| sender.send(handoff).is_ok());
        let (true, Some(commands)) = (given, commands) else {
            return Ok(self.release());
        };
        match wait_for_command(commands, self.point.limit).await {
            Resume::Continue { sink, max_rows } => self.start_export(sink, max_rows),
            Resume::Release => Ok(self.release()),
        }
    }
}

/// A sink that several owners share. The export keeps one reference, and
/// the task of a paused read gets the other, so the export gets its sink
/// back when the read ends.
pub struct SharedSink<S>(pub Arc<Mutex<S>>);

impl<S> SharedSink<S> {
    fn sink(&self) -> std::sync::MutexGuard<'_, S> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl<S: RowSink> RowSink for SharedSink<S> {
    fn begin_set(&mut self, columns: Vec<ColumnInfo>) -> Result<()> {
        self.sink().begin_set(columns)
    }

    fn row(&mut self, row: Vec<JsonValue>) -> Result<SinkControl> {
        self.sink().row(row)
    }

    fn end_set(&mut self, truncated: bool) -> Result<()> {
        self.sink().end_set(truncated)
    }

    fn message(&mut self, message: Message) {
        self.sink().message(message);
    }
}

/// The slot of the session of a paused read in the pool of its connection.
#[derive(Clone)]
pub struct SessionSlot {
    pub sessions: Arc<SessionPool>,
    pub key: String,
    pub session: Arc<Session>,
}

impl SessionSlot {
    /// Marks the session as broken and takes it out of its slot, when the
    /// slot still has it. The next request of the tab opens a new session.
    pub async fn discard(&self) {
        self.session.mark_broken();
        if let Some(current) = self.sessions.get(&self.key).await {
            if Arc::ptr_eq(&current, &self.session) {
                self.sessions.release(&self.key).await;
            }
        }
    }
}

/// Runs the work of the driver until it ends. After a release, the work
/// gets `limit` to end the statement. Work that passes that limit is
/// dropped in the middle of an exchange, so `overrun` then closes the
/// session.
pub async fn drive<W, O>(
    work: W,
    released: Arc<Notify>,
    limit: Duration,
    overrun: O,
) -> Result<RunSummary>
where
    W: Future<Output = Result<RunSummary>>,
    O: Future<Output = ()>,
{
    tokio::pin!(work);
    tokio::select! {
        result = &mut work => return result,
        () = released.notified() => {}
    }
    match tokio::time::timeout(limit, work).await {
        Ok(result) => result,
        Err(_) => {
            log::warn!("A released read did not end its statement in time, so its session closes.");
            overrun.await;
            Err(Error::Timeout(limit.as_secs()))
        }
    }
}

/// The end of the task of a paused read: the result of the driver and the
/// sink.
pub type TaskEnd<G> = (Result<RunSummary>, PausingSink<G>);

/// Starts the task that runs the read. The task owns the lock of the driver
/// until the read ends.
pub fn spawn_read<G: RowSink + 'static>(
    mut driver: OwnedMutexGuard<Box<dyn DatabaseDriver>>,
    query: String,
    params: Option<QueryParams>,
    options: ExecOptions,
    mut sink: PausingSink<G>,
    slot: SessionSlot,
) -> JoinHandle<TaskEnd<G>> {
    let released = sink.released.clone();
    tokio::spawn(async move {
        let work = driver.execute_stream(&query, params.as_ref(), &options, &mut sink);
        let result = drive(work, released, RELEASE_LIMIT, slot.discard()).await;
        (result, sink)
    })
}

/// A task that the drop of this value stops. A limit that drops the wait
/// for the task then also drops the work of the driver.
pub struct AbortOnDrop<T>(Option<JoinHandle<T>>);

impl<T> AbortOnDrop<T> {
    pub fn new(task: JoinHandle<T>) -> Self {
        Self(Some(task))
    }

    /// Gives the task back, so the drop of this value no longer stops it.
    pub fn disarm(mut self) -> JoinHandle<T> {
        self.0
            .take()
            .expect("the task is present until it is disarmed")
    }
}

impl<T> Future for AbortOnDrop<T> {
    type Output = std::result::Result<T, tokio::task::JoinError>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let task = self
            .0
            .as_mut()
            .expect("the task is present until it is disarmed");
        Pin::new(task).poll(context)
    }
}

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
        }
    }
}

/// The error of a task that panicked or stopped.
pub fn task_fault(error: tokio::task::JoinError) -> Error {
    Error::Anyhow(anyhow::anyhow!("The read stopped unexpectedly: {error}"))
}

/// The task and the command end of a paused read.
struct Parked<G> {
    commands: oneshot::Sender<Resume>,
    task: JoinHandle<TaskEnd<G>>,
}

/// A read that paused at the row limit, as the registry of kept results
/// keeps it. The drop of the value releases the read, because the sink then
/// finds the command end gone.
pub struct PausedRead<G = crate::db::columnar::ChunkSink> {
    parked: Mutex<Option<Parked<G>>>,
    slot: SessionSlot,
    limit: Duration,
}

impl<G> PausedRead<G> {
    pub fn new(
        commands: oneshot::Sender<Resume>,
        task: JoinHandle<TaskEnd<G>>,
        slot: SessionSlot,
        limit: Duration,
    ) -> Self {
        Self {
            parked: Mutex::new(Some(Parked { commands, task })),
            slot,
            limit,
        }
    }

    fn parked(&self) -> std::sync::MutexGuard<'_, Option<Parked<G>>> {
        self.parked.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The slot of the session that the read uses.
    pub fn slot(&self) -> &SessionSlot {
        &self.slot
    }

    /// The longest time the read stays paused.
    pub fn limit(&self) -> Duration {
        self.limit
    }

    /// True while the read waits for a command. A read that an export took,
    /// or that the pause limit released, is not live.
    pub fn is_live(&self) -> bool {
        self.parked()
            .as_ref()
            .is_some_and(|parked| !parked.commands.is_closed())
    }

    /// True when the read runs on the session.
    pub fn uses(&self, session: &Arc<Session>) -> bool {
        Arc::ptr_eq(&self.slot.session, session)
    }

    /// Continues the read into the sink, and waits for its end. The read
    /// takes the rest of its rows once. The drop of the returned future
    /// before its end stops the task, and with it the work of the driver.
    pub async fn continue_into(
        &self,
        sink: Box<dyn RowSink>,
        max_rows: usize,
    ) -> Result<RunSummary> {
        let Some(parked) = self.parked().take() else {
            return Err(gone());
        };
        if parked
            .commands
            .send(Resume::Continue { sink, max_rows })
            .is_err()
        {
            return Err(gone());
        }
        let (result, sink) = AbortOnDrop::new(parked.task).await.map_err(task_fault)?;
        // The sink keeps a reference to the sink of the export, which the
        // caller takes back after this drop.
        drop(sink);
        result
    }
}

/// The error for a paused read that was exported or released.
fn gone() -> Error {
    Error::Invalid(
        "The paused read was released, so its rows are gone. Run the query again to export all \
         rows."
            .to_string(),
    )
}

/// The time a read may pause, from the seconds that the window asks for.
/// Zero means no pause.
pub fn pause_limit(seconds: u64) -> Option<Duration> {
    (seconds > 0).then(|| Duration::from_secs(seconds).min(MAX_PAUSE))
}

#[cfg(test)]
pub(crate) mod tests;
