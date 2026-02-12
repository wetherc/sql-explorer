#![allow(dead_code)]

use crate::db::{AppColumn, Database, QueryResponse, ResultSet, Schema, Table, QueryParams};
use crate::error::Error;
use async_trait::async_trait;
use log::{debug, info};
use mysql_async::{prelude::*, Conn, Opts, OptsBuilder, Row as MySqlRow, QueryResult, Value as MySqlValue, BinaryProtocol};
use serde_json::Value as JsonValue;


pub struct MysqlDriver {
    conn: Option<Conn>,
}

impl MysqlDriver {
    pub async fn connect(connection_string: &str) -> Result<Box<dyn super::DatabaseDriver + Send + Sync>, Error> {
        info!("Attempting to connect to MySQL database.");
        debug!("Connection string: {}", connection_string);

        let opts = Opts::from_url(connection_string)?;
        let opts_builder = OptsBuilder::from_opts(opts.clone());

        let conn = match Conn::new(opts_builder).await {
            Ok(c) => c,
            Err(e) => {
                if let mysql_async::Error::Driver(driver_err) = &e {
                    if let mysql_async::DriverError::UnknownAuthPlugin { name } = driver_err {
                        if name == "sha256_password" {
                            return Err(Error::Anyhow(anyhow::anyhow!(
                                "Failed to connect to MySQL database: The client does not support the '{}' authentication plugin.
                                Please consider changing the MySQL user's authentication method to 'caching_sha2_password' or 'mysql_native_password' on the server.",
                                name
                            )));
                        }
                    }
                }
                // For other errors, re-raise the original error
                return Err(e.into());
            }
        };

        info!("MySQL database client connected successfully.");
        Ok(Box::new(MysqlDriver { conn: Some(conn) }))
    }

    async fn with_conn<F, T, Fut>(&mut self, op: F) -> Result<T, Error>
    where
        F: FnOnce(Conn) -> Fut + Send,
        Fut: std::future::Future<Output = Result<(Conn, T), Error>> + Send,
    {
        let conn = self.conn.take().ok_or(Error::NotConnected)?;
        let (conn_after, result) = op(conn).await?;
        self.conn = Some(conn_after);
        Ok(result)
    }
}

#[async_trait]
impl super::DatabaseDriver for MysqlDriver {
    async fn execute_query(&mut self, query: &str, query_params: Option<&QueryParams>) -> Result<QueryResponse, Error> {
        info!("Executing MySQL query: {}", query);
        self.with_conn(|conn| execute_internal(conn, query.to_string(), query_params)).await
    }

    async fn list_databases(&mut self) -> Result<Vec<Database>, Error> {
        info!("Listing MySQL databases.");
        self.with_conn(|conn| async {
            let (conn, response) = execute_internal(conn, "SHOW DATABASES".to_string(), None).await?;
            let databases: Vec<String> = response.results.into_iter()
                .flat_map(|rs| rs.rows)
                .filter_map(|row_json| {
                    row_json.get("Database") // Access by column name "Database"
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
                .collect();
            Ok((conn, databases.into_iter().map(|name| Database { name }).collect()))
        }).await
    }

    async fn list_schemas(&mut self, _database: &str) -> Result<Vec<Schema>, Error> {
        info!("Listing MySQL schemas (returning empty as MySQL uses databases).");
        // MySQL doesn't have schemas in the same way as MS SQL or PostgreSQL.
        // It uses databases. So, we return an empty list or adapt this.
        Ok(Vec::new())
    }

    async fn list_tables(&mut self, database: &str) -> Result<Vec<Table>, Error> {
        info!("Listing MySQL tables for database: {}", database);
        let query = format!("SHOW TABLES FROM `{}`", database);
        self.with_conn(|conn| async {
            let (conn, response) = execute_internal(conn, query, None).await?;
            let tables: Vec<String> = response.results.into_iter()
                .flat_map(|rs| {
                    let column_name = rs.columns.first().map(|s| s.to_string()).unwrap_or_default(); // Get the first column name as an owned String
                    rs.rows.into_iter().filter_map(move |row_json| {
                        row_json.get(&column_name) // Access by the actual column name, borrowing it
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                    })
                })
                .collect();
            Ok((conn, tables.into_iter().map(|name| Table { name }).collect()))
        }).await
    }

    async fn list_columns(&mut self, database: &str, table: &str) -> Result<Vec<AppColumn>, Error> {
        info!("Listing MySQL columns for database: {} and table: {}", database, table);
        let query = format!("SHOW COLUMNS FROM `{}` FROM `{}`", table, database);
        self.with_conn(|conn| async {
            let (conn, response) = execute_internal(conn, query, None).await?;
            let columns: Vec<AppColumn> = response.results.into_iter().flat_map(|rs| rs.rows).map(|row| AppColumn {
                name: row.get(0).and_then(|v| v.as_str().map(|s| s.to_string())).unwrap_or_default(),
                data_type: row.get(1).and_then(|v| v.as_str().map(|s| s.to_string())).unwrap_or_default(),
            }).collect();
            Ok((conn, columns))
        }).await
    }
}

pub(crate) async fn execute_internal(
    mut conn: Conn,
    query: String,
    query_params: Option<&QueryParams>,
) -> Result<(Conn, QueryResponse), Error> {
    let mut all_results: Vec<ResultSet> = Vec::new();
    let messages: Vec<String> = Vec::new(); // MySQL doesn't have direct messages like MSSQL

    let params = if let Some(qp) = query_params {
        let mut mysql_params: Vec<mysql_async::Value> = Vec::new();
        for param in qp {
            match &param.value {
                JsonValue::String(s) => mysql_params.push(mysql_async::Value::from(s.clone())),
                JsonValue::Number(n) => {
                    if let Some(i) = n.as_i64() {
                        mysql_params.push(mysql_async::Value::from(i));
                    } else if let Some(f) = n.as_f64() {
                        mysql_params.push(mysql_async::Value::from(f));
                    } else {
                        return Err(Error::Anyhow(anyhow::anyhow!("Unsupported number type for parameter")));
                    }
                },
                JsonValue::Bool(b) => mysql_params.push(mysql_async::Value::from(b.clone())),
                JsonValue::Null => mysql_params.push(mysql_async::Value::NULL),
                _ => return Err(Error::Anyhow(anyhow::anyhow!("Unsupported parameter type"))),
            }
        }
        mysql_async::Params::Positional(mysql_params)
    } else {
        mysql_async::Params::Empty
    };

    let mut query_result: QueryResult<'_, '_, BinaryProtocol> = conn.exec_iter(&query, params).await?;

    while !query_result.is_empty() {
        let columns: Vec<String> = query_result.columns().unwrap_or_default().iter().map(|c: &mysql_async::Column| c.name_str().to_string()).collect();
        let rows: Vec<JsonValue> = query_result
            .map(|row| row_to_json(&row, &columns))
            .await?;

        all_results.push(crate::db::ResultSet { columns, rows });
    }

    Ok((conn, QueryResponse { results: all_results, messages }))
}

pub(crate) fn row_to_json(row: &MySqlRow, columns: &[String]) -> JsonValue {
    let mut map = serde_json::Map::new();
    for (i, col_name) in columns.iter().enumerate() {
        let value: Option<MySqlValue> = row.get(i); // Use get(i)
        let json_value = match value {
            None | Some(MySqlValue::NULL) => JsonValue::Null,
            Some(MySqlValue::Int(i)) => JsonValue::from(i), // Remove dereference
            Some(MySqlValue::UInt(u)) => JsonValue::from(u), // Remove dereference
            Some(MySqlValue::Float(f)) => JsonValue::from(f), // Remove dereference
            Some(MySqlValue::Double(d)) => JsonValue::from(d), // Remove dereference
            Some(MySqlValue::Bytes(b)) => { // Use Bytes for strings
                JsonValue::String(String::from_utf8_lossy(&b).into_owned()) // Correctly borrow b
            }
            Some(MySqlValue::Date(y, m, d, h, min, s, us)) => {
                JsonValue::String(format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:06}", y, m, d, h, min, s, us))
            }
            Some(MySqlValue::Time(is_neg, d, h, m, s, us)) => {
                let sign = if is_neg { "-" } else { "" }; // Remove dereference
                JsonValue::String(format!("{}{} {:02}:{:02}:{:02}.{:06}", sign, d, h, m, s, us))
            }
        };
        map.insert(col_name.clone(), json_value);
    }
    JsonValue::Object(map)
}


#[cfg(test)]
mod tests {
    use super::*;
    use super::super::DatabaseDriver; // Import the trait
    use dotenv::dotenv;
    use std::env;

    async fn get_test_driver() -> Option<Box<dyn DatabaseDriver + Send + Sync>> {
        dotenv().ok();
        let connection_string = match env::var("MYSQL_TEST_DB_URL") {
            Ok(s) => s,
            Err(_) => {
                eprintln!("Skipping MySQL integration test: MYSQL_TEST_DB_URL not set.");
                return None;
            }
        };
        MysqlDriver::connect(&connection_string).await.ok()
    }

    #[tokio::test]
    async fn test_connect() {
        let _driver = match get_test_driver().await {
            Some(d) => d,
            None => return, // Skip test if env var not set
        };
        // If we reach here, the connection was successful.
        assert!(true);
    }

    #[tokio::test]
    async fn test_list_databases() {
        let mut driver = match get_test_driver().await {
            Some(d) => d,
            None => return, // Skip test if env var not set
        };
        let databases = driver.list_databases().await.unwrap();
        assert!(!databases.is_empty());
        assert!(databases.iter().any(|db| db.name == "mysql"));
    }

    #[tokio::test]
    async fn test_execute_query_select_version() {
        let mut driver = match get_test_driver().await {
            Some(d) => d,
            None => return, // Skip test if env var not set
        };
        let response = driver.execute_query("SELECT VERSION();", None).await.unwrap();
        assert!(!response.results.is_empty());
        let result_set = &response.results[0];
        assert_eq!(result_set.columns.len(), 1);
        assert!(!result_set.rows.is_empty());
        // Check if the version string is present
        let version_value = result_set.rows[0][&result_set.columns[0]].as_str().unwrap();
        assert!(version_value.contains("mysql") || version_value.contains("MariaDB"));
    }

    #[tokio::test]
    async fn test_execute_query_multiple_result_sets() {
        let mut driver = match get_test_driver().await {
            Some(d) => d,
            None => return, // Skip test if env var not set
        };
        let query = "SELECT 1; SELECT 'hello';";
        let response = driver.execute_query(query, None).await.unwrap();
        assert_eq!(response.results.len(), 2);

        // First result set
        let rs1 = &response.results[0];
        assert_eq!(rs1.columns.len(), 1);
        assert_eq!(rs1.rows.len(), 1);
        assert_eq!(rs1.rows[0][&rs1.columns[0]].as_i64().unwrap(), 1);

        // Second result set
        let rs2 = &response.results[1];
        assert_eq!(rs2.columns.len(), 1);
        assert_eq!(rs2.rows.len(), 1);
        assert_eq!(rs2.rows[0][&rs2.columns[0]].as_str().unwrap(), "hello");
    }

    #[tokio::test]
    async fn test_execute_query_insert_select() {
        let mut driver = match get_test_driver().await {
            Some(d) => d,
            None => return, // Skip test if env var not set
        };
        let db_name = "test_db_for_insert";
        let table_name = "test_table";
        let create_db_query = format!("CREATE DATABASE IF NOT EXISTS {}", db_name);
        driver.execute_query(&create_db_query, None).await.unwrap();

        let use_db_query = format!("USE {}", db_name);
        driver.execute_query(&use_db_query, None).await.unwrap();

        let create_table_query = format!(
            "CREATE TABLE IF NOT EXISTS `{}`.`{}` (
                id INT AUTO_INCREMENT PRIMARY KEY,
                name VARCHAR(255)
            );",
            db_name, table_name
        );
        driver.execute_query(&create_table_query, None).await.unwrap();

        let insert_query = format!("INSERT INTO `{}`.`{}` (name) VALUES ('TestName');", db_name, table_name);
        driver.execute_query(&insert_query, None).await.unwrap();

        let select_query = format!("SELECT id, name FROM `{}`.`{}` WHERE name = 'TestName';", db_name, table_name);
        let response = driver.execute_query(&select_query, None).await.unwrap();

        assert!(!response.results.is_empty());
        let result_set = &response.results[0];
        assert_eq!(result_set.columns.len(), 2);
        assert_eq!(result_set.rows.len(), 1);
        assert_eq!(result_set.rows[0]["name"].as_str().unwrap(), "TestName");

        // Clean up
        let drop_db_query = format!("DROP DATABASE {}", db_name);
        driver.execute_query(&drop_db_query, None).await.unwrap();
    }

    #[tokio::test]
    async fn test_list_tables() {
        let mut driver = match get_test_driver().await {
            Some(d) => d,
            None => return, // Skip test if env var not set
        };
        let db_name = "test_db_for_tables";
        let table_name1 = "table_one";
        let table_name2 = "table_two"; // ADDED THIS LINE
        driver.execute_query(&format!("CREATE DATABASE IF NOT EXISTS {}", db_name), None).await.unwrap();
        driver.execute_query(&format!("CREATE TABLE `{}`.`{}` (id INT)", db_name, table_name1), None).await.unwrap(); // CHANGED table_name2 to table_name1
        driver.execute_query(&format!("CREATE TABLE `{}`.`{}` (id INT)", db_name, table_name2), None).await.unwrap(); // ADDED THIS LINE to create the second table

        let tables = driver.list_tables(db_name).await.unwrap();
        assert_eq!(tables.len(), 2);
        assert!(tables.iter().any(|t| t.name == table_name1));
        assert!(tables.iter().any(|t| t.name == table_name2));

        // Clean up
        driver.execute_query(&format!("DROP DATABASE {}", db_name), None).await.unwrap();
    }

    #[tokio::test]
    async fn test_list_columns() {
        let mut driver = match get_test_driver().await {
            Some(d) => d,
            None => return, // Skip test if env var not set
        };
        let db_name = "test_db_for_columns";
        let table_name = "columns_table";

        driver.execute_query(&format!("CREATE DATABASE IF NOT EXISTS {}", db_name), None).await.unwrap();
        driver.execute_query(&format!("USE {}", db_name), None).await.unwrap();
        driver.execute_query(&format!(
            "CREATE TABLE `{}`.`{}` (
                id INT PRIMARY KEY,
                name VARCHAR(255),
                age INT,
                created_at DATETIME
            )", db_name, table_name
        ), None).await.unwrap();

        let columns = driver.list_columns(db_name, table_name).await.unwrap();
        assert_eq!(columns.len(), 4);
        assert!(columns.iter().any(|c| c.name == "id" && c.data_type == "int"));
        assert!(columns.iter().any(|c| c.name == "name" && c.data_type == "varchar(255)"));
        assert!(columns.iter().any(|c| c.name == "age" && c.data_type == "int"));
        assert!(columns.iter().any(|c| c.name == "created_at" && c.data_type == "datetime"));

        // Clean up
        driver.execute_query(&format!("DROP DATABASE {}", db_name), None).await.unwrap();
    }
}
