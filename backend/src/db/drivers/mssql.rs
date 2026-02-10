use crate::db::{
    AppColumn, Database, QueryResponse, ResultSet, Schema, Table,
    drivers::DatabaseDriver,
};
use crate::error::Error; // Use crate::error::Error directly
use async_trait::async_trait;
use futures_util::{StreamExt, TryStreamExt};
use log::{debug, error, info, warn};
use serde_json::Value as JsonValue;
use tiberius::{Client, Config, QueryItem, Row, Column, numeric::Numeric as TiberiusNumeric};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};
use std::error::Error as StdError;

pub type DbClient = Client<Compat<TcpStream>>; // Defined locally
pub type DbResult<T> = Result<T, Error>; // Use crate::error::Error for DbResult

pub struct MssqlDriver {
    client: DbClient,
}

impl MssqlDriver {
    pub async fn connect(connection_string: &str) -> DbResult<Box<dyn DatabaseDriver + Send>> {
        info!("Attempting to connect to the database.");
        debug!("Connection string: {}", connection_string);

        let config = match Config::from_ado_string(connection_string) {
            Ok(config) => {
                info!("Database configuration parsed successfully.");
                debug!("Configuration details: {:?}", config);
                config
            }
            Err(e) => {
                error!("Failed to parse connection string: {}", e);
                return Err(e.into());
            }
        };

        let addr = config.get_addr();
        info!("Connecting to TCP socket at {}", addr);
        let tcp = match TcpStream::connect(addr).await {
            Ok(tcp) => {
                info!("TCP socket connected successfully.");
                tcp
            }
            Err(e) => {
                error!("Failed to connect TCP socket: {}", e);
                return Err(e.into());
            }
        };

        if let Err(e) = tcp.set_nodelay(true) {
            warn!("Could not set TCP_NODELAY: {}", e);
        } else {
            info!("Nagle's algorithm disabled.");
        }

        info!("Starting TLS handshake and connecting client.");
        let client_result = Client::connect(config, tcp.compat_write()).await;

        match client_result {
            Ok(client) => {
                info!("Database client connected successfully.");
                Ok(Box::new(MssqlDriver { client }))
            }
            Err(e) => {
                error!("Failed to connect database client. Top-level error: {}", e);
                match &e {
                    tiberius::error::Error::Io { kind, message } => {
                        error!(
                            "Error details: An IO error occurred. Kind: {:?}, Message: {}",
                            kind, message
                        );
                    }
                    tiberius::error::Error::Tls(tls_msg) => {
                        error!(
                            "Error details: A TLS error occurred with message: '{}'",
                            tls_msg
                        );
                    }
                    tiberius::error::Error::Server(message) => {
                        error!(
                            "Error details: The server returned an error: {}",
                            message.to_string()
                        );
                    }
                    tiberius::error::Error::Routing { host, port } => {
                        error!(
                            "Error details: A routing error occurred for host '{}' on port {}",
                            host, port
                        );
                    }
                    _ => {
                        error!("Error details: An unhandled error variant occurred. Attempting to walk source chain generically.");
                        let mut source = e.source();
                        if source.is_none() {
                            error!("  No further `source` available on the error chain.");
                        }
                        while let Some(err) = source {
                            error!("  Caused by: {}", err);
                            source = err.source();
                        }
                    }
                }
                Err(e.into())
            }
        }
    }
}

#[async_trait]
impl DatabaseDriver for MssqlDriver {
    async fn execute_query(&mut self, query: &str) -> Result<QueryResponse, Error> {
        let mut stream = self.client.simple_query(query).await?;
        let mut all_results: Vec<ResultSet> = Vec::new();
        let mut current_result_set: Option<ResultSet> = None;
        let messages: Vec<String> = Vec::new(); // Will remain empty for now

        while let Some(item) = stream.try_next().await? {
            match item {
                QueryItem::Metadata(metadata) => {
                    // If there's an active result set, save it before starting a new one
                    if let Some(rs) = current_result_set.take() {
                        all_results.push(rs);
                    }

                    // Start a new result set with the new metadata
                    let columns: Vec<String> = metadata
                        .columns()
                        .iter()
                        .map(|c| c.name().to_string())
                        .collect();
                    current_result_set = Some(ResultSet {
                        columns,
                        rows: Vec::new(),
                    });
                }
                QueryItem::Row(row) => {
                    // Add row to the current result set
                    if let Some(ref mut rs) = current_result_set {
                        rs.rows.push(row_to_json(&row, row.columns()));
                    } else {
                        // Row received before any metadata, this is an unexpected state
                        error!("Row received before any result set metadata.");
                        // Decide how to handle, e.g., create a default result set
                    }
                }
            }
        }

        // Push the last active result set if any
        if let Some(rs) = current_result_set.take() {
            all_results.push(rs);
        }

        Ok(QueryResponse {
            results: all_results,
            messages,
        })
    }

    async fn list_databases(&mut self) -> Result<Vec<Database>, Error> {
        let query = "SELECT name FROM sys.databases WHERE database_id > 4 ORDER BY name";
        let mut stream = self.client.simple_query(query).await?;
        let mut databases = Vec::new();

        while let Some(result) = stream.next().await {
            if let Ok(QueryItem::Row(row)) = result {
                if let Some(name) = row.get::<&str, _>(0) {
                    databases.push(Database { name: name.to_string() });
                }
            }
        }
        Ok(databases)
    }

    async fn list_schemas(&mut self) -> Result<Vec<Schema>, Error> {
        let query = "SELECT name FROM sys.schemas ORDER BY name";
        let mut stream = self.client.simple_query(query).await?;
        let mut schemas = Vec::new();

        while let Some(result) = stream.next().await {
            if let Ok(QueryItem::Row(row)) = result {
                if let Some(name) = row.get::<&str, _>(0) {
                    schemas.push(Schema { name: name.to_string() });
                }
            }
        }
        Ok(schemas)
    }

    async fn list_tables(&mut self, schema: &str) -> Result<Vec<Table>, Error> {
        let query = format!(
            "SELECT TABLE_NAME FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_SCHEMA = '{}' ORDER BY TABLE_NAME",
            schema
        );
        let mut stream = self.client.simple_query(query.as_str()).await?;
        let mut tables = Vec::new();

        while let Some(result) = stream.next().await {
            if let Ok(QueryItem::Row(row)) = result {
                if let Some(name) = row.get::<&str, _>(0) {
                    tables.push(Table { name: name.to_string() });
                }
            }
        }
        Ok(tables)
    }

    async fn list_columns(&mut self, schema: &str, table: &str) -> Result<Vec<AppColumn>, Error> {
        let query = format!(
            "SELECT COLUMN_NAME, DATA_TYPE FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_SCHEMA = '{}' AND TABLE_NAME = '{}' ORDER BY ORDINAL_POSITION",
            schema, table
        );
        let mut stream = self.client.simple_query(query.as_str()).await?;
        let mut columns = Vec::new();

        while let Some(result) = stream.next().await {
            if let Ok(QueryItem::Row(row)) = result {
                if let (Some(name), Some(data_type)) = (row.get::<&str, _>(0), row.get::<&str, _>(1)) {
                    columns.push(AppColumn {
                        name: name.to_string(),
                        data_type: data_type.to_string(),
                    });
                }
            }
        }
        Ok(columns)
    }
}

fn row_to_json(row: &Row, columns: &[Column]) -> JsonValue {
    let mut map = serde_json::Map::new();
    for (i, col) in columns.iter().enumerate() {
        let col_name = col.name().to_string();
        // This is a simplified conversion. A more robust solution would handle different types.
        let val = match col.column_type() {
            tiberius::ColumnType::Bit => row.get::<bool, _>(i).map(JsonValue::Bool).unwrap_or(JsonValue::Null),
            tiberius::ColumnType::Intn | tiberius::ColumnType::Int4 => row.get::<i32, _>(i).map(JsonValue::from).unwrap_or(JsonValue::Null),
            tiberius::ColumnType::Int8 => row.get::<i64, _>(i).map(JsonValue::from).unwrap_or(JsonValue::Null),
            tiberius::ColumnType::Float4 => row.get::<f32, _>(i).map(JsonValue::from).unwrap_or(JsonValue::Null),
            tiberius::ColumnType::Float8 => row.get::<f64, _>(i).map(JsonValue::from).unwrap_or(JsonValue::Null),
            tiberius::ColumnType::Numericn => {
                let s: Option<TiberiusNumeric> = row.get(i);
                s.map(|d| JsonValue::from(f64::from(d))).unwrap_or(JsonValue::Null)
            },
            // Add more type conversions as needed
            _ => row.get::<&str, _>(i).map(|s| JsonValue::String(s.to_string())).unwrap_or(JsonValue::Null),
        };
        map.insert(col_name, val);
    }
    JsonValue::Object(map)
}
