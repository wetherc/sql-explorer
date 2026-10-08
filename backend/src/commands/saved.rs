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
}
