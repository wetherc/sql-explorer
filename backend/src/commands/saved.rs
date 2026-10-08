//! The commands that act on the saved full results of the runs.

use crate::error::Result;
use crate::state::AppState;

/// Stops the spill of a running query and keeps the run. The read then
/// stops at the row limit of the grid. Gives false when the run does not
/// spill now, for example because it already ended.
#[tauri::command]
pub async fn stop_saving(request_id: String, state: tauri::State<'_, AppState>) -> Result<bool> {
    Ok(state.kept.stop_spill(&request_id))
}

/// The disk use and the number of the saved full results.
#[derive(Debug, serde::Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SavedUsage {
    pub bytes: u64,
    pub count: usize,
}

/// Gives the disk use and the number of the saved full results.
#[tauri::command]
pub async fn saved_results_usage(state: tauri::State<'_, AppState>) -> Result<SavedUsage> {
    Ok(SavedUsage {
        bytes: state.kept.disk_use().bytes(),
        count: state.kept.spill_count(),
    })
}

/// Removes each saved full result, and gives the number of the removed
/// results and the bytes that they used.
#[tauri::command]
pub async fn clear_saved_results(state: tauri::State<'_, AppState>) -> Result<SavedUsage> {
    let (count, bytes) = state.kept.release_spills();
    Ok(SavedUsage { bytes, count })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemoryStore;
    use std::sync::Arc;
    use tauri::Manager;

    #[tokio::test]
    async fn a_stop_reaches_the_spill_of_a_running_query_alone() {
        let app = tauri::test::mock_app();
        app.manage(AppState::new(Arc::new(MemoryStore::default())));
        let state = app.state::<AppState>();
        let stop = state.kept.watch_spill("r1");
        assert!(!stop_saving("r2".into(), app.state()).await.unwrap());
        assert!(!stop.flag().load(std::sync::atomic::Ordering::Relaxed));
        assert!(stop_saving("r1".into(), app.state()).await.unwrap());
        assert!(stop.flag().load(std::sync::atomic::Ordering::Relaxed));
        drop(stop);
        // The end of the run removes its flag.
        assert!(!stop_saving("r1".into(), app.state()).await.unwrap());
    }

    #[tokio::test]
    async fn the_usage_and_the_clear_count_the_spill_files() {
        let folder = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_app();
        app.manage(AppState::new(Arc::new(MemoryStore::default())));
        let state = app.state::<AppState>();
        let empty = SavedUsage { bytes: 0, count: 0 };
        assert_eq!(saved_results_usage(app.state()).await.unwrap(), empty);
        let file = crate::spill::tests::spill(folder.path(), &state.kept.disk_use(), 2);
        state.kept.keep(
            "r1",
            "c1",
            vec![(0, crate::kept::KeptSource::SpillFile(file))],
        );
        let usage = saved_results_usage(app.state()).await.unwrap();
        assert_eq!(usage.count, 1);
        assert!(usage.bytes > 0);
        assert_eq!(clear_saved_results(app.state()).await.unwrap(), usage);
        assert_eq!(saved_results_usage(app.state()).await.unwrap(), empty);
        assert_eq!(
            serde_json::to_value(&usage).unwrap(),
            serde_json::json!({ "bytes": usage.bytes, "count": 1 })
        );
    }
}
