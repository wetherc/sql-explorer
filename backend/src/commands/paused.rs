//! The runs that pause at the row limit, and the export that continues
//! them. See [`crate::pause`] for the rules of the pause.

use super::{
    arm, armed_driver, finish_export, run_bounded, stop_grace, Bounded, ExportFormat,
    ExportSummary, FileSink,
};
use crate::db::columnar::ChunkSink;
use crate::db::sink::{PausePoint, RunSummary};
use crate::db::{ExecOptions, QueryParams};
use crate::error::{Error, Result};
use crate::kept::KeptResult;
use crate::pause::{
    pause_limit, spawn_read, task_fault, AbortOnDrop, Handoff, PauseControl, PausedRead,
    PausingSink, SessionSlot, SharedSink,
};
use crate::session::Session;
use crate::sql::{only_reads, split_batches, split_statements, Dialect};
use crate::state::AppState;
use std::sync::{Arc, Mutex, PoisonError};
use tokio_util::sync::CancellationToken;

/// The place where a run pauses its read, or `None` for a run that ends its
/// read at the row limit.
///
/// A run pauses only when the window asks for it, the run belongs to a tab,
/// the driver can pause, and the text is one statement that only reads. A
/// statement after the one that pauses would wait for the end of the pause.
/// A statement that writes would keep its changes open for that time, and a
/// release that stops it would roll them back.
pub(super) fn pause_point(
    seconds: u64,
    tab: bool,
    session: &Session,
    query: &str,
    dialect: Dialect,
    rows: usize,
) -> Option<PausePoint> {
    let limit = pause_limit(seconds)?;
    (tab && session.pauses_reads && one_read(query, dialect)).then_some(PausePoint { rows, limit })
}

/// The seconds a run may pause. A run that spills its full result to the
/// local disk gets its export from the spill file, so it never pauses.
pub(super) fn pause_seconds(seconds: u64, spills: bool) -> u64 {
    if spills {
        0
    } else {
        seconds
    }
}

/// True when the text runs one statement once, and that statement only
/// reads.
fn one_read(query: &str, dialect: Dialect) -> bool {
    let runs: usize = split_batches(query, dialect)
        .iter()
        .map(|batch| split_statements(&batch.text, dialect).len() * batch.runs as usize)
        .sum();
    runs == 1 && only_reads(query, dialect)
}

/// How the part of a run that the window waits for ended.
enum Progress {
    /// The read paused at the row limit.
    Paused(Handoff<ChunkSink>),
    /// The read ended before the row limit, or the driver failed.
    Ended(RunSummary),
}

/// What a run that can pause gives back to the command of the run.
pub(super) struct PausableRun {
    pub outcome: Bounded<RunSummary>,
    /// The sink of the grid, or `None` when a limit stopped the task and the
    /// sink went with it.
    pub grid: Option<ChunkSink>,
    /// The number of the set that paused, and the read, when it paused.
    pub paused: Option<(u32, PausedRead)>,
}

/// What a run that can pause needs.
pub(super) struct PausableRequest<'a> {
    pub state: &'a AppState,
    pub request_id: &'a str,
    pub slot: SessionSlot,
    pub token: &'a CancellationToken,
    pub query: String,
    pub bound: Option<QueryParams>,
    pub options: ExecOptions,
    pub point: PausePoint,
    pub started: std::time::Instant,
}

/// Runs a statement whose read can pause at the row limit.
///
/// The driver runs in a task that owns the lock of the driver, so the lock
/// stays taken while the read is paused. The time limit and the Stop button
/// cover the run until it pauses or ends, and the paused time does not count.
pub(super) async fn run(request: PausableRequest<'_>, grid: ChunkSink) -> PausableRun {
    let PausableRequest {
        state,
        request_id,
        slot,
        token,
        query,
        bound,
        options,
        point,
        started,
    } = request;
    let session = slot.session.clone();
    let lock = session.driver.clone().lock_owned();
    let guard = match armed_driver(state, request_id, &session, token, lock).await {
        Ok(guard) => guard,
        Err(error) => {
            return PausableRun {
                outcome: Bounded::Answered(Err(error)),
                grid: Some(grid),
                paused: None,
            }
        }
    };
    let (sink, control) = PausingSink::new(grid, point);
    let PauseControl {
        mut handoff,
        commands,
    } = control;
    // The sink pauses the read at the row limit, so the driver reads past
    // that limit.
    let driver_options = ExecOptions {
        max_rows: usize::MAX,
        ..options
    };
    let mut task = AbortOnDrop::new(spawn_read(
        guard,
        query,
        bound,
        driver_options,
        sink,
        slot.clone(),
    ));
    let mut grid = None;
    let progress = async {
        tokio::select! {
            Ok(given) = &mut handoff => Ok(Progress::Paused(given)),
            ended = &mut task => {
                let (result, sink) = ended.map_err(task_fault)?;
                grid = sink.into_grid();
                result.map(Progress::Ended)
            }
        }
    };
    let outcome = run_bounded(
        progress,
        token,
        options.timeout_secs,
        stop_grace(&session),
        session.cancel_handle.clone(),
    )
    .await;
    match outcome {
        Bounded::Answered(Ok(Progress::Paused(given))) => {
            let read = PausedRead::new(commands, task.disarm(), slot, point.limit);
            PausableRun {
                outcome: Bounded::Answered(Ok(RunSummary {
                    rows_affected: None,
                    elapsed_ms: started.elapsed().as_millis() as u64,
                    stats: None,
                })),
                grid: Some(given.grid),
                paused: Some((given.set, read)),
            }
        }
        Bounded::Answered(Ok(Progress::Ended(summary))) => PausableRun {
            outcome: Bounded::Answered(Ok(summary)),
            grid,
            paused: None,
        },
        Bounded::Answered(Err(error)) => PausableRun {
            outcome: Bounded::Answered(Err(error)),
            grid,
            paused: None,
        },
        // The drop of the task at the end of this call stops the work of
        // the driver.
        Bounded::Stopped(error) => PausableRun {
            outcome: Bounded::Stopped(error),
            grid: None,
            paused: None,
        },
    }
}

/// Writes every row of a paused read to a file: the rows that the grid
/// shows, and then the rest of the read.
///
/// The export takes the rest of the rows once. The Stop button and the time
/// limit of the export reach the statement on the server, as they reach a
/// run. A limit that drops the read closes the session, because the
/// exchange then ends in the middle of a message.
pub(super) async fn export(
    state: &AppState,
    request_id: &str,
    kept: &KeptResult,
    read: &PausedRead,
    path: &std::path::Path,
    format: ExportFormat,
    options: &ExecOptions,
) -> Result<ExportSummary> {
    let slot = read.slot();
    let session = &slot.session;
    let token = state.start_request(request_id, &kept.connection_id).await;
    let written = async {
        arm(state, request_id, session, &token).await?;
        // An error, a stop or the time limit drops the sink before
        // `finish`, and the writer then removes the part that was written.
        let file = Arc::new(Mutex::new(FileSink::create(path, format).await?));
        let rest = read.continue_into(Box::new(SharedSink(file.clone())), options.max_rows);
        let outcome = run_bounded(
            rest,
            &token,
            options.timeout_secs,
            stop_grace(session),
            session.cancel_handle.clone(),
        )
        .await;
        match outcome {
            Bounded::Answered(result) => result?,
            Bounded::Stopped(error) => {
                if !session.keeps_connection_after_stop {
                    slot.discard().await;
                }
                return Err(error);
            }
        };
        session.mark_ok().await;
        // The read ended and dropped its reference to the sink.
        take_shared(file)
    }
    .await;
    state.end_request(request_id).await;
    finish_export(written?).await
}

/// Takes the sink from its shared box, after the read dropped its
/// reference.
fn take_shared<S>(shared: Arc<Mutex<S>>) -> Result<S> {
    let shared = Arc::try_unwrap(shared).map_err(|_| {
        Error::Anyhow(anyhow::anyhow!(
            "The export file is still in use by the read."
        ))
    })?;
    Ok(shared.into_inner().unwrap_or_else(PoisonError::into_inner))
}

#[cfg(test)]
mod tests;
