// backend/src/state.rs
use crate::db::drivers::DatabaseDriver;
use tokio::sync::Mutex;

pub struct AppState {
    pub db: Mutex<Option<Box<dyn DatabaseDriver + Send>>>,
}
