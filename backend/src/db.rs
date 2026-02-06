// backend/src/db.rs
use crate::error::Error;
use futures_util::StreamExt;
use serde_json::Value as JsonValue;
use tiberius::{Client, Config, QueryItem, Row};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

pub type DbClient = Client<Compat<TcpStream>>;
pub type DbResult<T> = Result<T, Error>;

pub async fn db_connect(connection_string: &str) -> DbResult<DbClient> {
    let config = Config::from_ado_string(connection_string)?;
    let tcp = TcpStream::connect(config.get_addr()).await?;
    tcp.set_nodelay(true)?;
    let client = Client::connect(config, tcp.compat_write()).await?;
    Ok(client)
}

pub async fn db_execute_query(client: &mut DbClient, query: &str) -> DbResult<JsonValue> {
    let mut stream = client.simple_query(query).await?;
    let mut results = Vec::new();

    while let Some(result) = stream.next().await {
        if let Ok(QueryItem::Row(row)) = result {
            results.push(row_to_json(row));
        }
    }

    Ok(JsonValue::Array(results))
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
