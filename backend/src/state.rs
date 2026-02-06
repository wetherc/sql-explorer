// backend/src/state.rs
use crate::db::DbClient;
use tokio::sync::Mutex;

pub struct AppState {
    pub db: Mutex<Option<DbClient>>,
}
