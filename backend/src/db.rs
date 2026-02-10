// backend/src/db.rs
pub mod drivers;


use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;



#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Database {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schema {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Table {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppColumn {
    pub name: String,
    pub data_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultSet {
    pub columns: Vec<String>,
    pub rows: Vec<JsonValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResponse {
    pub results: Vec<ResultSet>,
    pub messages: Vec<String>,
}















