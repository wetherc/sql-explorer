// backend/src/db/drivers.rs
pub mod mssql;

use crate::db::{Database, Schema, Table, AppColumn, QueryResponse};
use crate::error::Error;
use async_trait::async_trait;

#[async_trait]
pub trait DatabaseDriver {
    async fn execute_query(&mut self, query: &str) -> Result<QueryResponse, Error>;
    async fn list_databases(&mut self) -> Result<Vec<Database>, Error>;
    async fn list_schemas(&mut self) -> Result<Vec<Schema>, Error>;
    async fn list_tables(&mut self, schema: &str) -> Result<Vec<Table>, Error>;
    async fn list_columns(&mut self, schema: &str, table: &str) -> Result<Vec<AppColumn>, Error>;
}
