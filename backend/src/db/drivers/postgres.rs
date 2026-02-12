use crate::db::{
    drivers::DatabaseDriver, AppColumn, Database, QueryResponse, ResultSet, Schema, Table, QueryParams,
};
use crate::error::Error;
use async_trait::async_trait;
use log::{info, debug}; // Import debug macro
use serde_json::Value as JsonValue;
use tokio_postgres::{Client, Row, Config as PgConfig};

use rustls::{ClientConfig, RootCertStore};

use webpki_roots;
use std::str::FromStr; // Import FromStr trait

pub struct PostgresDriver {
    client: Client,
}

impl PostgresDriver {
    pub async fn connect(
        connection_string: &str,
    ) -> Result<Box<dyn DatabaseDriver + Send + Sync>, Error> {
        info!("Attempting to connect to PostgreSQL database.");
        
        let _pg_config = PgConfig::from_str(connection_string)?;

        info!("Connecting with TLS.");
        let mut root_cert_store = RootCertStore::empty();
        root_cert_store.extend(
            webpki_roots::TLS_SERVER_ROOTS.iter().map(|ta| ta.to_owned())
        );
        let tls_config = ClientConfig::builder()
            .with_root_certificates(root_cert_store)
            .with_no_client_auth();
        let tls_connector = tokio_postgres_rustls_improved::MakeRustlsConnect::new(tls_config);
        let (client, connection) = tokio_postgres::connect(connection_string, tls_connector).await?;

        // The connection object performs the actual I/O, so it must be spawned.
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                eprintln!("connection error: {}", e);
            }
        });

        info!("PostgreSQL database client connected successfully.");
        Ok(Box::new(PostgresDriver { client }))
    }
}

#[async_trait]
impl DatabaseDriver for PostgresDriver {
    async fn execute_query(&mut self, query: &str, query_params: Option<&QueryParams>) -> Result<QueryResponse, Error> {
        info!("Executing PostgreSQL query: {}", query);
        debug!("Parameters: {:?}", query_params);

        let mut params: Vec<Box<dyn tokio_postgres::types::ToSql + Send + Sync>> = Vec::new();
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
                    JsonValue::Null => params.push(Box::new(Option::<String>::None)),
                    _ => return Err(Error::Anyhow(anyhow::anyhow!("Unsupported parameter type"))),
                }
            }
        }
        let borrowed_params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params.iter().map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync)).collect();

        let rows = self.client.query(query, borrowed_params.as_slice()).await?;
        let mut results: Vec<ResultSet> = Vec::new();

        if !rows.is_empty() {
            let columns = rows[0]
                .columns()
                .iter()
                .map(|c| c.name().to_string())
                .collect::<Vec<String>>();

            let json_rows = rows.iter().map(row_to_json).collect::<Vec<JsonValue>>();

            results.push(ResultSet {
                columns,
                rows: json_rows,
            });
        }

        Ok(QueryResponse {
            results,
            messages: vec![], // tokio-postgres doesn't expose notice messages in the same way
        })
    }

    async fn list_databases(&mut self) -> Result<Vec<Database>, Error> {
        let rows = self
            .client
            .query("SELECT datname FROM pg_database WHERE datistemplate = false;", &[])
            .await?;
        let databases = rows
            .iter()
            .map(|row| Database {
                name: row.get(0),
            })
            .collect();
        Ok(databases)
    }

    async fn list_schemas(&mut self, _database: &str) -> Result<Vec<Schema>, Error> {
        let rows = self
            .client
            .query(
                "SELECT nspname FROM pg_catalog.pg_namespace WHERE nspname NOT IN ('pg_toast', 'pg_catalog', 'information_schema') AND NOT nspname.starts_with('pg_temp_');",
                &[],
            )
            .await?;
        let schemas = rows
            .iter()
            .map(|row| Schema {
                name: row.get(0),
            })
            .collect();
        Ok(schemas)
    }

    async fn list_tables(&mut self, schema: &str) -> Result<Vec<Table>, Error> {
        let rows = self
            .client
            .query(
                "SELECT tablename FROM pg_tables WHERE schemaname = $1;",
                &[&schema],
            )
            .await?;
        let tables = rows
            .iter()
            .map(|row| Table {
                name: row.get(0),
            })
            .collect();
        Ok(tables)
    }

    async fn list_columns(&mut self, schema: &str, table: &str) -> Result<Vec<AppColumn>, Error> {
        let rows = self
            .client
            .query(
                "SELECT column_name, data_type FROM information_schema.columns WHERE table_schema = $1 AND table_name = $2;",
                &[&schema, &table],
            )
            .await?;
        let columns = rows
            .iter()
            .map(|row| AppColumn {
                name: row.get(0),
                data_type: row.get(1),
            })
            .collect();
        Ok(columns)
    }
}

fn row_to_json(row: &Row) -> JsonValue {
    use postgres_types::Type;
    use serde_json::Map;

    let mut map = Map::new();
    for (i, column) in row.columns().iter().enumerate() {
        let col_name = column.name().to_string();
        let value = match *column.type_() {
            Type::BOOL => row.get::<_, bool>(i).into(),
            Type::INT2 => row.get::<_, i16>(i).into(),
            Type::INT4 => row.get::<_, i32>(i).into(),
            Type::INT8 => row.get::<_, i64>(i).into(),
            Type::FLOAT4 => row.get::<_, f32>(i).into(),
            Type::FLOAT8 => row.get::<_, f64>(i).into(),
            Type::TEXT | Type::VARCHAR | Type::NAME => {
                row.get::<_, String>(i).into()
            }
            // Add more type mappings as needed
            _ => JsonValue::String(format!(
                "Unsupported type: {}",
                column.type_().name()
            )),
        };
        map.insert(col_name, value);
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
        let connection_string = match env::var("POSTGRES_TEST_DB_URL") {
            Ok(s) => s,
            Err(_) => {
                eprintln!("Skipping PostgreSQL integration test: POSTGRES_TEST_DB_URL not set.");
                return None;
            }
        };
        PostgresDriver::connect(&connection_string).await.ok()
    }

    #[tokio::test]
    async fn test_connect() {
        if get_test_driver().await.is_none() {
            return; // Skip test
        }
        assert!(true);
    }
}
