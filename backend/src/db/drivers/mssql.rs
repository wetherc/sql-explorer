// backend/src/db/drivers/mssql.rs
use crate::db::{
    AppColumn, Database, DbClient, DbResult, QueryResponse, Schema, Table,
    drivers::DatabaseDriver,
};
use crate::error::Error;
use async_trait::async_trait;

pub struct MssqlDriver {
    client: DbClient,
}

impl MssqlDriver {
    pub async fn connect(connection_string: &str) -> DbResult<Box<dyn DatabaseDriver + Send>> {
        let client = crate::db::db_connect(connection_string).await?;
        Ok(Box::new(MssqlDriver { client }))
    }
}

#[async_trait]
impl DatabaseDriver for MssqlDriver {
    async fn execute_query(&mut self, query: &str) -> Result<QueryResponse, Error> {
        crate::db::db_execute_query(&mut self.client, query).await
    }

    async fn list_databases(&mut self) -> Result<Vec<Database>, Error> {
        crate::db::db_list_databases(&mut self.client).await
    }

    async fn list_schemas(&mut self) -> Result<Vec<Schema>, Error> {
        crate::db::db_list_schemas(&mut self.client).await
    }

    async fn list_tables(&mut self, schema: &str) -> Result<Vec<Table>, Error> {
        crate::db::db_list_tables(&mut self.client, schema).await
    }

    async fn list_columns(&mut self, schema: &str, table: &str) -> Result<Vec<AppColumn>, Error> {
        crate::db::db_list_columns(&mut self.client, schema, table).await
    }
}
