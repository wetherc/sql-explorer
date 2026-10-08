use super::super::tests::{state_with_sqlite, temp_sqlite};
use super::super::{
    execute_query, release_kept, release_session, stop_requests, write_kept, ExecuteRequest,
    SpillRequest, CSV_BOM,
};
use super::*;
use crate::db::columnar::FRAME_END;
use crate::kept::KeptSource;
use crate::pause::tests::{paused_chunk_read, RowsDriver};
use crate::secrets::MemoryStore;
use std::time::Duration;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::Manager;

fn session(driver: RowsDriver) -> Session {
    Session::new(Box::new(driver))
}

#[test]
fn a_run_pauses_only_one_read_of_a_tab_on_a_driver_that_can_pause() {
    let pausing = session(RowsDriver::new(0));
    let point = |seconds, tab, session: &Session, query| {
        pause_point(seconds, tab, session, query, Dialect::MsSql, 100)
    };
    assert_eq!(
        point(60, true, &pausing, "SELECT 1"),
        Some(PausePoint {
            rows: 100,
            limit: Duration::from_secs(60),
        })
    );
    assert_eq!(point(0, true, &pausing, "SELECT 1"), None);
    assert_eq!(point(60, false, &pausing, "SELECT 1"), None);
    assert_eq!(point(60, true, &pausing, "SELECT 1; SELECT 2"), None);
    assert_eq!(point(60, true, &pausing, "SELECT 1\nGO 2"), None);
    assert_eq!(point(60, true, &pausing, "UPDATE t SET a = 1"), None);
    assert_eq!(point(60, true, &pausing, ""), None);

    let mut plain = session(RowsDriver::new(0));
    plain.pauses_reads = false;
    assert_eq!(point(60, true, &plain, "SELECT 1"), None);
}

fn grid(rows: usize) -> GridSink {
    crate::message_log::MessageTee::new(
        crate::db::columnar::ChunkSink::new(Channel::new(|_| Ok(())), rows),
        None,
    )
}

/// A slot of its own for the driver, under the key `t1`.
async fn slot(driver: RowsDriver) -> SessionSlot {
    let sessions = Arc::new(crate::session::SessionPool::new(4));
    let session = sessions.insert("t1", session(driver)).await;
    SessionSlot {
        sessions,
        key: "t1".to_string(),
        session,
    }
}

/// Runs a read that can pause after `rows` rows, under a request `r1`.
async fn pausable(
    state: &AppState,
    driver: RowsDriver,
    rows: usize,
    timeout_secs: u64,
    token: &CancellationToken,
) -> PausableRun {
    let request = PausableRequest {
        state,
        request_id: "r1",
        slot: slot(driver).await,
        token,
        query: "SELECT n".to_string(),
        bound: None,
        options: ExecOptions {
            max_rows: rows,
            timeout_secs,
            one_statement: false,
        },
        point: PausePoint {
            rows,
            limit: Duration::from_secs(60),
        },
        started: std::time::Instant::now(),
    };
    run(request, grid(rows)).await
}

fn state() -> AppState {
    AppState::new(Arc::new(MemoryStore::default()))
}

#[tokio::test]
async fn a_read_past_the_limit_pauses_and_the_run_ends() {
    let state = state();
    let token = state.start_request("r1", "c1").await;
    let driver = RowsDriver::new(5);
    let limits = driver.limits.clone();
    let run = pausable(&state, driver, 2, 30, &token).await;
    assert!(matches!(run.outcome, Bounded::Answered(Ok(_))));
    assert!(run.grid.is_some());
    let (set, read) = run.paused.expect("the read paused");
    assert_eq!(set, 0);
    assert!(read.is_live());
    // The sink ends the read, so the driver reads past the row limit.
    assert_eq!(*limits.lock().unwrap(), vec![usize::MAX]);
}

#[tokio::test]
async fn a_read_below_the_limit_ends_as_a_run() {
    let state = state();
    let token = state.start_request("r1", "c1").await;
    let run = pausable(&state, RowsDriver::new(2), 2, 30, &token).await;
    assert!(matches!(run.outcome, Bounded::Answered(Ok(ref summary)) if summary.elapsed_ms == 1));
    assert!(run.grid.is_some());
    assert!(run.paused.is_none());
}

#[tokio::test]
async fn a_read_that_fails_keeps_its_grid() {
    let state = state();
    let token = state.start_request("r1", "c1").await;
    let driver = RowsDriver {
        fail_at: Some(1),
        ..RowsDriver::new(5)
    };
    let run = pausable(&state, driver, 2, 30, &token).await;
    assert!(matches!(
        run.outcome,
        Bounded::Answered(Err(Error::Invalid(_)))
    ));
    assert!(run.grid.is_some());

    let token = state.start_request("r1", "c1").await;
    let driver = RowsDriver {
        panic_at: Some(1),
        ..RowsDriver::new(5)
    };
    let run = pausable(&state, driver, 2, 30, &token).await;
    assert!(matches!(
        run.outcome,
        Bounded::Answered(Err(Error::Anyhow(_)))
    ));
    assert!(run.grid.is_none());
}

#[tokio::test]
async fn a_stopped_request_never_takes_the_driver() {
    let state = state();
    let token = state.start_request("r1", "c1").await;
    token.cancel();
    let run = pausable(&state, RowsDriver::new(5), 2, 30, &token).await;
    assert!(matches!(
        run.outcome,
        Bounded::Answered(Err(Error::Cancelled))
    ));
    assert!(run.grid.is_some());
}

#[tokio::test(start_paused = true)]
async fn the_time_limit_covers_the_read_up_to_the_pause() {
    let state = state();
    let token = state.start_request("r1", "c1").await;
    let driver = RowsDriver {
        hang_at: Some(1),
        ..RowsDriver::new(5)
    };
    let run = pausable(&state, driver, 2, 1, &token).await;
    assert!(matches!(run.outcome, Bounded::Stopped(Error::Timeout(1))));
    assert!(run.grid.is_none());
}

fn export_options(max_rows: usize, timeout_secs: u64) -> ExecOptions {
    ExecOptions {
        max_rows,
        timeout_secs,
        one_statement: true,
    }
}

#[tokio::test]
async fn an_export_writes_every_row_of_a_paused_read_once() {
    let state = state();
    let read = paused_chunk_read(RowsDriver::new(5), 2).await;
    let session = read.slot().session.clone();
    let kept = KeptResult::new("c1", KeptSource::PausedRead(read));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("all.csv");
    let options = export_options(4, 30);
    let summary = write_kept(&state, "e1", &kept, &path, ExportFormat::Csv, &options)
        .await
        .unwrap();
    assert_eq!(summary.rows, 4);
    assert!(summary.truncated);
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text, format!("{CSV_BOM}n\r\n0\r\n1\r\n2\r\n3\r\n"));
    assert!(state.take_request("e1").await.is_none());
    assert!(session.driver.try_lock().is_ok());

    // The rest of the rows went to the first export.
    let again = dir.path().join("again.csv");
    let error = write_kept(&state, "e2", &kept, &again, ExportFormat::Csv, &options)
        .await
        .unwrap_err();
    assert!(matches!(error, Error::Invalid(_)));
    assert!(!again.exists());
}

#[tokio::test]
async fn the_stop_button_ends_the_export_of_a_paused_read() {
    let state = Arc::new(state());
    let driver = RowsDriver {
        hang_at: Some(3),
        ..RowsDriver::new(5)
    };
    let read = paused_chunk_read(driver, 2).await;
    let slot = read.slot().clone();
    let kept = KeptResult::new("c1", KeptSource::PausedRead(read));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("all.json");
    let options = export_options(10, 0);
    let stopper = state.clone();
    let (outcome, ()) = tokio::join!(
        write_kept(&state, "e1", &kept, &path, ExportFormat::Json, &options),
        async move {
            loop {
                if let Some(request) = stopper.take_request("e1").await {
                    stop_requests(vec![request]).await;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }
    );
    assert!(matches!(outcome, Err(Error::Cancelled)));
    assert!(!path.exists());
    // The stop dropped the read in the middle of an exchange, so the
    // session closes.
    assert!(slot.session.is_broken());
    assert!(slot.sessions.get("t1").await.is_none());
}

#[tokio::test]
async fn a_stopped_export_keeps_a_session_that_stays_fit_after_a_stop() {
    let state = state();
    let driver = RowsDriver {
        hang_at: Some(3),
        keeps_after_stop: true,
        ..RowsDriver::new(5)
    };
    let read = paused_chunk_read(driver, 2).await;
    let slot = read.slot().clone();
    let kept = KeptResult::new("c1", KeptSource::PausedRead(read));
    let dir = tempfile::tempdir().unwrap();
    let error = write_kept(
        &state,
        "e1",
        &kept,
        &dir.path().join("all.csv"),
        ExportFormat::Csv,
        &export_options(10, 1),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, Error::Timeout(1)));
    assert!(!slot.session.is_broken());
    assert!(slot.sessions.get("t1").await.is_some());
}

#[tokio::test]
async fn a_run_in_a_tab_pauses_and_a_new_run_of_the_tab_releases_it() {
    let (_dir, descriptor) = temp_sqlite();
    let (app, state) = state_with_sqlite(descriptor).await;
    let open = state.connection("s1").await.unwrap();
    let paused = open
        .sessions
        .insert("t1", session(RowsDriver::new(5)))
        .await;
    app.manage(state);

    let request = |request_id: &str| ExecuteRequest {
        connection_id: "s1".into(),
        request_id: request_id.into(),
        query: "SELECT n".into(),
        tab_id: Some("t1".into()),
        query_params: None,
        options: Some(ExecOptions {
            max_rows: 2,
            timeout_secs: 30,
            one_statement: false,
        }),
        spill: None,
        pause_secs: 60,
        messages_file: None,
    };
    let ends = Arc::new(std::sync::Mutex::new(Vec::new()));
    let channel = || {
        let ends = ends.clone();
        Channel::new(move |body| {
            if let InvokeResponseBody::Raw(bytes) = body {
                if bytes[0] == FRAME_END {
                    ends.lock()
                        .unwrap()
                        .push(String::from_utf8_lossy(&bytes).to_string());
                }
            }
            Ok(())
        })
    };
    execute_query(app.handle().clone(), request("r1"), app.state(), channel())
        .await
        .unwrap();
    let end = ends.lock().unwrap().pop().unwrap();
    assert!(end.contains(r#""kept":[{"set":0,"id":"r1:0","pausedSecs":60}]"#));
    let registry = &app.state::<AppState>().kept;
    assert!(registry.get("r1:0").is_some());
    assert!(paused.driver.try_lock().is_err());

    // A new run of the tab releases the paused read, so it gets the driver.
    execute_query(app.handle().clone(), request("r2"), app.state(), channel())
        .await
        .unwrap();
    assert!(registry.get("r1:0").is_none());
    assert!(registry.get("r2:0").is_some());

    // The close of the tab releases the read of the second run.
    release_session("s1".into(), "t1".into(), app.state())
        .await
        .unwrap();
    assert!(registry.get("r2:0").is_none());
    drop(paused.driver.lock().await);
    release_kept("r2:0".into(), app.state()).await.unwrap();
}

#[tokio::test]
async fn a_run_stopped_by_its_time_limit_still_ends_its_channel() {
    let (_dir, descriptor) = temp_sqlite();
    let (app, state) = state_with_sqlite(descriptor).await;
    let open = state.connection("s1").await.unwrap();
    let driver = RowsDriver {
        hang_at: Some(1),
        ..RowsDriver::new(5)
    };
    open.sessions.insert("t1", session(driver)).await;
    app.manage(state);
    let frames = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = frames.clone();
    let channel = Channel::new(move |body| {
        if let InvokeResponseBody::Raw(bytes) = body {
            seen.lock().unwrap().push(bytes[0]);
        }
        Ok(())
    });
    let request = ExecuteRequest {
        connection_id: "s1".into(),
        request_id: "r1".into(),
        query: "SELECT n".into(),
        tab_id: Some("t1".into()),
        query_params: None,
        options: Some(ExecOptions {
            max_rows: 2,
            timeout_secs: 1,
            one_statement: false,
        }),
        spill: None,
        pause_secs: 60,
        messages_file: None,
    };
    let error = execute_query(app.handle().clone(), request, app.state(), channel)
        .await
        .unwrap_err();
    assert!(matches!(error, Error::Timeout(1)));
    assert_eq!(frames.lock().unwrap().last(), Some(&FRAME_END));
    // The session closed, because the limit dropped its exchange.
    assert!(open.sessions.get("t1").await.is_none());
}

#[test]
fn a_sink_that_the_read_still_uses_stays_shared() {
    let shared = Arc::new(Mutex::new(7));
    let other = shared.clone();
    assert!(matches!(take_shared(shared), Err(Error::Anyhow(_))));
    assert_eq!(take_shared(other).unwrap(), 7);
}

#[tokio::test]
async fn a_stop_while_a_run_waits_for_the_driver_ends_the_run() {
    let (_dir, descriptor) = temp_sqlite();
    let (app, state) = state_with_sqlite(descriptor).await;
    let open = state.connection("s1").await.unwrap();
    let busy = open
        .sessions
        .insert("t1", session(RowsDriver::new(5)))
        .await;
    app.manage(state);
    let guard = busy.driver.lock().await;
    let request = ExecuteRequest {
        connection_id: "s1".into(),
        request_id: "r1".into(),
        query: "SELECT n".into(),
        tab_id: Some("t1".into()),
        query_params: None,
        options: None,
        spill: None,
        pause_secs: 0,
        messages_file: None,
    };
    let stopper = app.handle().clone();
    let (outcome, ()) = tokio::join!(
        execute_query(
            app.handle().clone(),
            request,
            app.state(),
            Channel::new(|_| Ok(()))
        ),
        async move {
            // The run waits for the driver that this test keeps.
            tokio::time::sleep(Duration::from_millis(50)).await;
            let state = stopper.state::<AppState>();
            let request = state.take_request("r1").await.unwrap();
            stop_requests(vec![request]).await;
        }
    );
    assert!(matches!(outcome, Err(Error::Cancelled)));
    drop(guard);
}

#[tokio::test]
async fn a_file_that_cannot_open_leaves_the_read_paused() {
    let state = state();
    let read = paused_chunk_read(RowsDriver::new(5), 2).await;
    let kept = KeptResult::new("c1", KeptSource::PausedRead(read));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing").join("all.csv");
    let options = export_options(10, 30);
    assert!(
        write_kept(&state, "e1", &kept, &path, ExportFormat::Csv, &options)
            .await
            .is_err()
    );
    assert!(kept.source.is_live());
    assert!(state.take_request("e1").await.is_none());
}

#[test]
fn a_run_that_spills_never_pauses() {
    assert_eq!(pause_seconds(600, false), 600);
    assert_eq!(pause_seconds(600, true), 0);
}

#[tokio::test]
async fn a_run_that_asks_for_a_spill_ends_at_the_limit() {
    let (_dir, descriptor) = temp_sqlite();
    let (app, state) = state_with_sqlite(descriptor).await;
    let open = state.connection("s1").await.unwrap();
    let tab = open
        .sessions
        .insert("t1", session(RowsDriver::new(5)))
        .await;
    app.manage(state);
    let request = ExecuteRequest {
        connection_id: "s1".into(),
        request_id: "r1".into(),
        query: "SELECT n".into(),
        tab_id: Some("t1".into()),
        query_params: None,
        options: Some(ExecOptions {
            max_rows: 2,
            timeout_secs: 30,
            one_statement: false,
        }),
        spill: Some(SpillRequest {
            max_rows: 10,
            max_bytes: 1 << 20,
        }),
        pause_secs: 60,
        messages_file: None,
    };
    execute_query(
        app.handle().clone(),
        request,
        app.state(),
        Channel::new(|_| Ok(())),
    )
    .await
    .unwrap();
    assert!(app.state::<AppState>().kept.get("r1:0").is_none());
    assert!(tab.driver.try_lock().is_ok());
}

#[tokio::test]
async fn a_run_that_logs_its_messages_still_pauses() {
    use crate::message_log::MessageLogs;
    let (_dir, descriptor) = temp_sqlite();
    let (app, state) = state_with_sqlite(descriptor).await;
    let open = state.connection("s1").await.unwrap();
    let paused = open
        .sessions
        .insert("t1", session(RowsDriver::new(5)))
        .await;
    app.manage(state);
    app.manage(MessageLogs::default());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("paused.txt");
    let id = app.state::<MessageLogs>().remember(path.clone());

    let request = ExecuteRequest {
        connection_id: "s1".into(),
        request_id: "r1".into(),
        query: "SELECT n".into(),
        tab_id: Some("t1".into()),
        query_params: None,
        options: Some(ExecOptions {
            max_rows: 2,
            timeout_secs: 30,
            one_statement: false,
        }),
        spill: None,
        pause_secs: 60,
        messages_file: Some(id),
    };
    execute_query(
        app.handle().clone(),
        request,
        app.state(),
        Channel::new(|_| Ok(())),
    )
    .await
    .unwrap();
    // The pause point and the wait of the pausing sink pass through the
    // copy of the messages, so the read paused and stays live.
    let registry = &app.state::<AppState>().kept;
    assert!(matches!(
        registry
            .get("r1:0")
            .map(|kept| matches!(&kept.source, KeptSource::PausedRead(read) if read.is_live())),
        Some(true)
    ));
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.ends_with("\nstart\n"));

    release_session("s1".into(), "t1".into(), app.state())
        .await
        .unwrap();
    drop(paused.driver.lock().await);
}
