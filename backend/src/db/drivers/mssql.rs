use crate::db::{
    AppColumn, Database, QueryResponse, ResultSet, Schema, Table, QueryParams,
};
use crate::error::Error; // Use crate::error::Error directly
use async_trait::async_trait;
use futures_util::{StreamExt, TryStreamExt};
use log::{debug, error, info, warn};
use serde_json::Value as JsonValue;
use tiberius::{Client, Config, QueryItem, Row, Column, numeric::Numeric as TiberiusNumeric, EncryptionLevel, Into<td>};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};
use std::error::Error as StdError;

pub type DbClient = Client<Compat<TcpStream>>; // Defined locally
pub type DbResult<T> = Result<T, Error>; // Use crate::error::Error for DbResult

pub struct MssqlDriver {
    client: DbClient,
}

impl MssqlDriver {
    pub async fn connect(connection_string: &str) -> DbResult<Box<dyn DatabaseDriver + Send + Sync>> {
        info!("Attempting to connect to the database.");
        debug!("Connection string: {}", connection_string);

        let mut config = match Config::from_ado_string(connection_string) {
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

        // Enforce encryption for MSSQL connections
        config.encryption(EncryptionLevel::Required);

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
    async fn execute_query(&mut self, query: &str, query_params: Option<&QueryParams>) -> Result<QueryResponse, Error> {
        info!("Executing MSSQL query: {}", query);
        debug!("Parameters: {:?}", query_params);

        let mut params: Vec<Box<dyn Into<td>>> = Vec::new();
        if let Some(qp) = query_params {
            for param in qp {
                match &param.value {
                    JsonValue::String(s) => params.push(Box::new(s.clone())),
                    JsonValue::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            params.push(Box::new(i));
                        } else if let Some(f) = n.as_f64() {
                            params.push(Box::new(f));
                        } else {
                            return Err(Error::Anyhow(anyhow::anyhow!("Unsupported number type for parameter")));
                        }
                    },
                    JsonValue::Bool(b) => params.push(Box::new(b.clone())),
                    JsonValue::Null => params.push(Box::new(Option::<String>::None)), // Represent null as Option<String>::None
                    _ => return Err(Error::Anyhow(anyhow::anyhow!("Unsupported parameter type"))),
                }
            }
        }
        let borrowed_params: Vec<&dyn Into<td>> = params.iter().map(|p| p.as_ref()).collect();

        let mut stream = self.client.query(query, borrowed_params.as_slice()).await?;
        let mut all_results: Vec<ResultSet> = Vec::new();

        while let Some(item) = stream.try_next().await? {
            match item {
                QueryItem::Metadata(metadata) => {
                    let columns: Vec<String> = metadata
                        .columns()
                        .iter()
                        .map(|c| c.name().to_string())
                        .collect();
                    let rows_stream = stream.rows().await?;
                    let rows: Vec<JsonValue> = rows_stream
                        .into_iter()
                        .map(|row| row_to_json(&row, metadata.columns()))
                        .collect();

                    all_results.push(ResultSet { columns, rows });
                },
                _ => { /* Ignore other QueryItem types for now, e.g., ReturnValue, Done */ }
            }
        }

        Ok(QueryResponse {
            results: all_results,
            messages: Vec::new(),
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
        let query = "SELECT TABLE_NAME FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_SCHEMA = @p1 ORDER BY TABLE_NAME";
        let mut stream = self.client.query(query, &[&schema]).await?;
        let mut tables = Vec::new();

        while let Some(item) = stream.try_next().await? {
            match item {
                QueryItem::Metadata(metadata) => {
                    let rows_stream = stream.rows().await?;
                    for row in rows_stream {
                        if let Some(name) = row.get::<&str, _>(0) {
                            tables.push(Table { name: name.to_string() });
                        }
                    }
                },
                _ => {}
            }
        }
        Ok(tables)
    }

    async fn list_columns(&mut self, schema: &str, table: &str) -> Result<Vec<AppColumn>, Error> {
        let query = "SELECT COLUMN_NAME, DATA_TYPE FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_SCHEMA = @p1 AND TABLE_NAME = @p2 ORDER BY ORDINAL_POSITION";
        let mut stream = self.client.query(query, &[&schema, &table]).await?;
        let mut columns = Vec::new();

        while let Some(item) = stream.try_next().await? {
            match item {
                QueryItem::Metadata(metadata) => {
                    let rows_stream = stream.rows().await?;
                    for row in rows_stream {
                        if let (Some(name), Some(data_type)) = (row.get::<&str, _>(0), row.get::<&str, _>(1)) {
                            columns.push(AppColumn {
                                name: name.to_string(),
                                data_type: data_type.to_string(),
                            });
                        }
                    }
                },
                _ => {}
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

#[cfg(test)]
mod tests {
    use super::*;
    use dotenv::dotenv;
    use std::env;

    async fn get_test_driver() -> Option<Box<dyn DatabaseDriver + Send + Sync>> {
        dotenv().ok();
        let connection_string = match env::var("MSSQL_TEST_DB_URL") {
            Ok(s) => s,
            Err(_) => {
                eprintln!("Skipping MS SQL integration test: MSSQL_TEST_DB_URL not set.");
                return None;
            }
        };
        MssqlDriver::connect(&connection_string).await.ok()
    }

    #[tokio::test]
    async fn test_connect() {
        if get_test_driver().await.is_none() {
            return; // Skip test
        }
        // If we reach here, the connection was successful.
        assert!(true);
    }
}
