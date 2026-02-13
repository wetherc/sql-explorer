// backend/src/state.rs
use crate::db::drivers::DatabaseDriver;
use std::collections::HashMap;
use tokio::sync::Mutex;

pub struct AppState {
    pub connections: Mutex<HashMap<String, Box<dyn DatabaseDriver + Send + Sync>>>,
}
