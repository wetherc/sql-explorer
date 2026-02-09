// backend/src/db.rs
use crate::error::Error as AppError;
use futures_util::StreamExt;
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::error::Error;
use tiberius::{Client, Config, QueryItem, Row};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

pub type DbClient = Client<Compat<TcpStream>>;
pub type DbResult<T> = Result<T, AppError>;

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
pub struct Column {
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


pub async fn db_connect(connection_string: &str) -> DbResult<DbClient> {
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
            Ok(client)
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

pub async fn db_execute_query(client: &mut DbClient, query: &str) -> DbResult<QueryResponse> {
    let mut stream = client.simple_query(query).await?;
    let mut all_results: Vec<ResultSet> = Vec::new();
    let mut messages: Vec<String> = Vec::new();

    loop {
        // Get columns for the current result set before processing its rows
        let columns: Vec<String> = stream
            .columns()
            .to_vec()
            .iter()
            .map(|c| c.name().to_string())
            .collect();
        let mut rows: Vec<JsonValue> = Vec::new();
        let mut has_rows_in_current_set = false;

        // Iterate through rows and QueryItems of the current result set
        // The stream yields QueryItem::Row or QueryItem::Message until QueryItem::Done for the current set
        while let Some(result) = stream.next().await {
            match result? {
                QueryItem::Row(row) => {
                    rows.push(row_to_json(row));
                    has_rows_in_current_set = true;
                }
                QueryItem::Message(message) => {
                    messages.push(message.message().to_string());
                }
                QueryItem::Done => {
                    // End of current result set, break from inner loop
                    break;
                }
                _ => {} // Ignore other QueryItem types
            }
        }

        // If there were rows or columns in the current result set, add it to all_results
        if has_rows_in_current_set || !columns.is_empty() {
             all_results.push(ResultSet { columns, rows });
        }

        // Check if there are more result sets in the stream
        if stream.has_more_results() {
            // Advance to the next result set
            stream.next_resultset().await?;
        } else {
            // No more result sets, break from outer loop
            break;
        }
    }

    Ok(QueryResponse {
        results: all_results,
        messages,
    })
}

pub async fn db_list_databases(client: &mut DbClient) -> DbResult<Vec<Database>> {
    let query = "SELECT name FROM sys.databases WHERE database_id > 4 ORDER BY name";
    let mut stream = client.simple_query(query).await?;
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

pub async fn db_list_schemas(client: &mut DbClient) -> DbResult<Vec<Schema>> {
    let query = "SELECT name FROM sys.schemas ORDER BY name";
    let mut stream = client.simple_query(query).await?;
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

pub async fn db_list_tables(
    client: &mut DbClient,
    schema_name: &str,
) -> DbResult<Vec<Table>> {
    let query = format!(
        "SELECT TABLE_NAME FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_SCHEMA = '{}' ORDER BY TABLE_NAME",
        schema_name
    );
    let mut stream = client.simple_query(query).await?;
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

pub async fn db_list_columns(
    client: &mut DbClient,
    schema_name: &str,
    table_name: &str,
) -> DbResult<Vec<Column>> {
    let query = format!(
        "SELECT COLUMN_NAME, DATA_TYPE FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_SCHEMA = '{}' AND TABLE_NAME = '{}' ORDER BY ORDINAL_POSITION",
        schema_name, table_name
    );
    let mut stream = client.simple_query(query).await?;
    let mut columns = Vec::new();

    while let Some(result) = stream.next().await {
        if let Ok(QueryItem::Row(row)) = result {
            if let (Some(name), Some(data_type)) = (row.get::<&str, _>(0), row.get::<&str, _>(1)) {
                columns.push(Column {
                    name: name.to_string(),
                    data_type: data_type.to_string(),
                });
            }
        }
    }
    Ok(columns)
}

fn row_to_json(row: Row) -> JsonValue {
    let mut map = serde_json::Map::new();
    for (i, col) in row.columns().iter().enumerate() {
        let col_name = col.name().to_string();
        // This is a simplified conversion. A more robust solution would handle different types.
        let val = match row.get::<&str, _>(i) {
            Some(s) => JsonValue::String(s.to_string()),
            None => JsonValue::Null,
        };
        map.insert(col_name, val);
    }
    JsonValue::Object(map)
}
