//! The SQLite driver.
//!
//! `rusqlite` is a blocking library, so every call runs on the thread pool
//! for blocking work. The connection is held behind a mutex that the
//! closure takes for the length of one call.

use crate::db::drivers::{
    bytes_to_json, f64_to_json, number_out_of_range, number_value, rows_affected_message,
    rows_returned_message, DatabaseDriver, NumberValue,
};
use crate::db::{
    AppColumn, ColumnInfo, Database, DriverCapabilities, ExecOptions, QueryParams, QueryResponse,
    ResultSet, Schema, Table,
};
use crate::error::{Error, Result};
use crate::sql::{split_statements, Dialect};
use crate::storage::SavedConnection;
use async_trait::async_trait;
use rusqlite::types::{Value as SqliteValue, ValueRef};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value as JsonValue;
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub struct SqliteDriver {
    connection: Arc<Mutex<Connection>>,
    path: String,
}

impl SqliteDriver {
    pub async fn connect(connection: &SavedConnection) -> Result<Box<dyn DatabaseDriver>> {
        let path = connection
            .options
            .file_path
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                Error::Configuration("A SQLite connection needs the path of a file.".to_string())
            })?
            .to_string();

        let flags = if connection.options.read_only {
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI
        } else {
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_URI
        };

        let open_path = path.clone();
        let handle =
            tokio::task::spawn_blocking(move || Connection::open_with_flags(&open_path, flags))
                .await
                .map_err(|error| Error::Connection(error.to_string()))??;

        Ok(Box::new(SqliteDriver {
            connection: Arc::new(Mutex::new(handle)),
            path,
        }))
    }

    /// Runs a closure against the connection on the pool for blocking work.
    async fn with_connection<T, F>(&self, work: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
    {
        let connection = self.connection.clone();
        tokio::task::spawn_blocking(move || {
            let guard = connection
                .lock()
                .map_err(|_| Error::Connection("The SQLite connection is not usable.".into()))?;
            work(&guard)
        })
        .await
        .map_err(|error| Error::Connection(error.to_string()))?
    }
}

#[async_trait]
impl DatabaseDriver for SqliteDriver {
    fn capabilities(&self) -> DriverCapabilities {
        DriverCapabilities {
            supports_schemas: false,
            supports_multiple_databases: false,
            supports_cancel: false,
            supports_transactions: true,
        }
    }

    fn dialect(&self) -> Dialect {
        Dialect::Sqlite
    }

    async fn ping(&mut self) -> Result<()> {
        self.with_connection(|connection| {
            connection.query_row("SELECT 1", [], |_| Ok(()))?;
            Ok(())
        })
        .await
    }

    async fn execute_query(
        &mut self,
        query: &str,
        params: Option<&QueryParams>,
        options: &ExecOptions,
    ) -> Result<QueryResponse> {
        let started = Instant::now();
        let bound = bind_params(params)?;
        let statements: Vec<String> = if params.is_some() {
            vec![query.to_string()]
        } else {
            split_statements(query, Dialect::Sqlite)
        };
        let options = *options;

        let mut response = self
            .with_connection(move |connection| {
                let mut response = QueryResponse::default();
                for statement in statements {
                    run_statement(connection, &statement, &bound, &options, &mut response)?;
                }
                Ok(response)
            })
            .await?;

        response.elapsed_ms = started.elapsed().as_millis() as u64;
        Ok(response)
    }

    /// One SQLite connection holds one file. The name of the file stands
    /// for the database.
    async fn list_databases(&mut self) -> Result<Vec<Database>> {
        let name = std::path::Path::new(&self.path)
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.clone());
        Ok(vec![Database { name }])
    }

    async fn list_schemas(&mut self, _database: &str) -> Result<Vec<Schema>> {
        Ok(Vec::new())
    }

    async fn list_tables(&mut self, _database: &str, _schema: Option<&str>) -> Result<Vec<Table>> {
        self.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT name, type FROM sqlite_master \
                 WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' \
                 ORDER BY type, name",
            )?;
            let rows = statement.query_map([], |row| {
                let name: String = row.get(0)?;
                let kind: String = row.get(1)?;
                Ok(if kind == "view" {
                    Table::view(name)
                } else {
                    Table::table(name)
                })
            })?;
            let mut tables = Vec::new();
            for table in rows {
                tables.push(table?);
            }
            Ok(tables)
        })
        .await
    }

    async fn list_columns(
        &mut self,
        _database: &str,
        _schema: Option<&str>,
        table: &str,
    ) -> Result<Vec<AppColumn>> {
        let table = table.to_string();
        self.with_connection(move |connection| {
            let mut statement = connection
                .prepare("SELECT name, type, \"notnull\", pk FROM pragma_table_info(?1)")?;
            let rows = statement.query_map([&table], |row| {
                let name: String = row.get(0)?;
                let data_type: String = row.get(1)?;
                let not_null: i64 = row.get(2)?;
                let primary_key: i64 = row.get(3)?;
                Ok(AppColumn {
                    name,
                    data_type: if data_type.is_empty() {
                        "any".to_string()
                    } else {
                        data_type
                    },
                    nullable: not_null == 0,
                    is_primary_key: primary_key > 0,
                })
            })?;
            let mut columns = Vec::new();
            for column in rows {
                columns.push(column?);
            }
            Ok(columns)
        })
        .await
    }
}

/// Runs one statement and adds what it produced to the response.
fn run_statement(
    connection: &Connection,
    statement_text: &str,
    params: &[SqliteValue],
    options: &ExecOptions,
    response: &mut QueryResponse,
) -> Result<()> {
    let mut statement = connection.prepare(statement_text)?;
    let column_count = statement.column_count();

    if column_count == 0 {
        let affected = statement.execute(rusqlite::params_from_iter(params.iter()))? as u64;
        response.rows_affected = Some(response.rows_affected.unwrap_or(0) + affected);
        response.messages.push(rows_affected_message(affected));
        return Ok(());
    }

    let columns: Vec<ColumnInfo> = (0..column_count)
        .map(|index| {
            ColumnInfo::new(
                statement.column_name(index).unwrap_or("?").to_string(),
                statement
                    .columns()
                    .get(index)
                    .and_then(|column| column.decl_type())
                    .unwrap_or("any")
                    .to_string(),
            )
        })
        .collect();

    let mut set = ResultSet::new(columns);
    let mut rows = statement.query(rusqlite::params_from_iter(params.iter()))?;
    while let Some(row) = rows.next()? {
        if set.rows.len() >= options.max_rows {
            set.truncated = true;
            break;
        }
        set.rows.push(
            (0..column_count)
                .map(|index| value_to_json(row.get_ref(index).unwrap_or(ValueRef::Null)))
                .collect(),
        );
    }
    response
        .messages
        .push(rows_returned_message(set.rows.len(), set.truncated));
    response.results.push(set);
    Ok(())
}

/// Turns the JSON parameters into values the driver can bind.
pub fn bind_params(params: Option<&QueryParams>) -> Result<Vec<SqliteValue>> {
    let Some(params) = params else {
        return Ok(Vec::new());
    };
    let mut bound = Vec::new();
    for param in params {
        bound.push(match &param.value {
            JsonValue::Null => SqliteValue::Null,
            JsonValue::Bool(flag) => SqliteValue::Integer(i64::from(*flag)),
            JsonValue::String(text) => SqliteValue::Text(text.clone()),
            JsonValue::Number(number) => match number_value(number) {
                Some(NumberValue::Integer(value)) => SqliteValue::Integer(value),
                Some(NumberValue::Float(value)) => SqliteValue::Real(value),
                None => return Err(number_out_of_range(number)),
            },
            // SQLite has no structured type, so the value goes in as text.
            other => SqliteValue::Text(other.to_string()),
        });
    }
    Ok(bound)
}

/// Converts one value of the driver into JSON.
pub fn value_to_json(value: ValueRef<'_>) -> JsonValue {
    match value {
        ValueRef::Null => JsonValue::Null,
        ValueRef::Integer(number) => JsonValue::from(number),
        ValueRef::Real(number) => f64_to_json(number),
        ValueRef::Text(bytes) => match std::str::from_utf8(bytes) {
            Ok(text) => JsonValue::String(text.to_string()),
            Err(_) => bytes_to_json(bytes),
        },
        ValueRef::Blob(bytes) => bytes_to_json(bytes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::QueryParam;
    use crate::storage::{ConnectionOptions, DbType};

    fn connection_for(path: &str) -> SavedConnection {
        SavedConnection {
            id: "id".into(),
            name: "name".into(),
            db_type: DbType::Sqlite,
            host: None,
            port: None,
            user: None,
            database: None,
            password: None,
            options: ConnectionOptions {
                file_path: Some(path.to_string()),
                ..ConnectionOptions::default()
            },
            color: None,
            group: None,
        }
    }

    async fn open_memory() -> Box<dyn DatabaseDriver> {
        SqliteDriver::connect(&connection_for("file:sqlite_test?mode=memory&cache=shared"))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_connection_needs_a_file_path() {
        let mut input = connection_for("  ");
        assert_eq!(
            SqliteDriver::connect(&input).await.err().unwrap().kind(),
            crate::error::ErrorKind::Configuration
        );
        input.options.file_path = None;
        assert!(SqliteDriver::connect(&input).await.is_err());
    }

    #[tokio::test]
    async fn a_read_only_connection_refuses_a_write() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("read_only.db");
        let path_text = path.to_string_lossy().into_owned();

        let mut driver = SqliteDriver::connect(&connection_for(&path_text))
            .await
            .unwrap();
        driver
            .execute_query("CREATE TABLE t (a INTEGER)", None, &ExecOptions::default())
            .await
            .unwrap();
        drop(driver);

        let mut input = connection_for(&path_text);
        input.options.read_only = true;
        let mut driver = SqliteDriver::connect(&input).await.unwrap();
        assert!(driver
            .execute_query("INSERT INTO t VALUES (1)", None, &ExecOptions::default())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn a_missing_file_in_a_missing_folder_gives_an_error() {
        let result = SqliteDriver::connect(&connection_for("/does/not/exist/a.db")).await;
        assert_eq!(
            result.err().unwrap().kind(),
            crate::error::ErrorKind::Database
        );
    }

    #[tokio::test]
    async fn the_driver_reports_what_it_can_do() {
        let driver = open_memory().await;
        assert_eq!(driver.dialect(), Dialect::Sqlite);
        let capabilities = driver.capabilities();
        assert!(!capabilities.supports_schemas);
        assert!(!capabilities.supports_multiple_databases);
        assert!(capabilities.supports_transactions);
        assert!(driver.cancel_handle().is_none());
    }

    #[tokio::test]
    async fn a_query_returns_rows_and_a_message() {
        let mut driver = open_memory().await;
        let response = driver
            .execute_query(
                "SELECT 1 AS a, 'x' AS b, NULL AS c",
                None,
                &ExecOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].columns[0], ColumnInfo::new("a", "any"));
        assert_eq!(
            response.results[0].rows[0],
            vec![
                serde_json::json!(1),
                serde_json::json!("x"),
                JsonValue::Null
            ]
        );
        assert_eq!(response.messages, vec!["1 row returned."]);
        assert!(!response.results[0].truncated);
    }

    #[tokio::test]
    async fn a_script_runs_every_statement_and_counts_the_changes() {
        let mut driver = open_memory().await;
        let response = driver
            .execute_query(
                "DROP TABLE IF EXISTS people; \
                 CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT NOT NULL); \
                 INSERT INTO people (name) VALUES ('Ada'), ('Grace'); \
                 SELECT name FROM people ORDER BY name;",
                None,
                &ExecOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].rows.len(), 2);
        assert_eq!(response.rows_affected, Some(2));
        assert!(response.messages.iter().any(|m| m == "2 rows affected."));
    }

    #[tokio::test]
    async fn the_row_limit_stops_the_read() {
        let mut driver = open_memory().await;
        driver
            .execute_query(
                "DROP TABLE IF EXISTS numbers; CREATE TABLE numbers (n INTEGER);\
                 INSERT INTO numbers VALUES (1),(2),(3);",
                None,
                &ExecOptions::default(),
            )
            .await
            .unwrap();
        let response = driver
            .execute_query(
                "SELECT n FROM numbers ORDER BY n",
                None,
                &ExecOptions {
                    max_rows: 2,
                    timeout_secs: 10,
                },
            )
            .await
            .unwrap();
        assert_eq!(response.results[0].rows.len(), 2);
        assert!(response.results[0].truncated);
        assert!(response.messages[0].contains("row limit"));
    }

    #[tokio::test]
    async fn a_statement_with_parameters_binds_them() {
        let mut driver = open_memory().await;
        let params: QueryParams = vec![
            QueryParam {
                value: serde_json::json!(4),
            },
            QueryParam {
                value: serde_json::json!("four"),
            },
        ];
        let response = driver
            .execute_query(
                "SELECT ?1 AS n, ?2 AS t",
                Some(&params),
                &ExecOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.results[0].rows[0],
            vec![serde_json::json!(4), serde_json::json!("four")]
        );
    }

    #[tokio::test]
    async fn a_statement_that_is_not_valid_gives_a_database_error() {
        let mut driver = open_memory().await;
        let error = driver
            .execute_query("SELECT FROM", None, &ExecOptions::default())
            .await
            .unwrap_err();
        assert_eq!(error.kind(), crate::error::ErrorKind::Database);
    }

    #[tokio::test]
    async fn the_metadata_lists_the_tables_the_views_and_the_columns() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("meta.db");
        let mut driver = SqliteDriver::connect(&connection_for(&path.to_string_lossy()))
            .await
            .unwrap();
        driver
            .execute_query(
                "CREATE TABLE orders (id INTEGER PRIMARY KEY, total REAL, note TEXT); \
                 CREATE VIEW big_orders AS SELECT * FROM orders WHERE total > 10;",
                None,
                &ExecOptions::default(),
            )
            .await
            .unwrap();

        let databases = driver.list_databases().await.unwrap();
        assert_eq!(
            databases,
            vec![Database {
                name: "meta.db".into()
            }]
        );
        assert!(driver.list_schemas("meta.db").await.unwrap().is_empty());

        let tables = driver.list_tables("meta.db", None).await.unwrap();
        assert!(tables.contains(&Table::table("orders")));
        assert!(tables.contains(&Table::view("big_orders")));

        let columns = driver
            .list_columns("meta.db", None, "orders")
            .await
            .unwrap();
        assert_eq!(columns.len(), 3);
        assert_eq!(columns[0].name, "id");
        assert!(columns[0].is_primary_key);
        assert_eq!(columns[1].data_type, "REAL");
        assert!(columns[1].nullable);

        driver.ping().await.unwrap();
    }

    #[tokio::test]
    async fn a_database_name_falls_back_to_the_whole_path() {
        let mut driver = SqliteDriver::connect(&connection_for("file:x?mode=memory"))
            .await
            .unwrap();
        assert_eq!(driver.list_databases().await.unwrap().len(), 1);
    }

    #[test]
    fn the_parameters_accept_every_json_type() {
        let params: QueryParams = vec![
            QueryParam {
                value: JsonValue::Null,
            },
            QueryParam {
                value: serde_json::json!(true),
            },
            QueryParam {
                value: serde_json::json!("a"),
            },
            QueryParam {
                value: serde_json::json!(2),
            },
            QueryParam {
                value: serde_json::json!(2.5),
            },
            QueryParam {
                value: serde_json::json!([1, 2]),
            },
        ];
        let bound = bind_params(Some(&params)).unwrap();
        assert_eq!(bound.len(), 6);
        assert_eq!(bound[0], SqliteValue::Null);
        assert_eq!(bound[1], SqliteValue::Integer(1));
        assert_eq!(bound[3], SqliteValue::Integer(2));
        assert_eq!(bound[4], SqliteValue::Real(2.5));
        assert_eq!(bound[5], SqliteValue::Text("[1,2]".into()));
        assert!(bind_params(None).unwrap().is_empty());
    }

    #[test]
    fn a_number_that_is_too_large_is_refused() {
        let params: QueryParams = vec![QueryParam {
            value: serde_json::json!(18446744073709551615u64),
        }];
        assert_eq!(
            bind_params(Some(&params)).unwrap_err().kind(),
            crate::error::ErrorKind::Configuration
        );
    }

    #[test]
    fn every_value_type_becomes_json() {
        assert_eq!(value_to_json(ValueRef::Null), JsonValue::Null);
        assert_eq!(value_to_json(ValueRef::Integer(3)), serde_json::json!(3));
        assert_eq!(value_to_json(ValueRef::Real(0.5)), serde_json::json!(0.5));
        assert_eq!(
            value_to_json(ValueRef::Text(b"hi")),
            serde_json::json!("hi")
        );
        assert_eq!(
            value_to_json(ValueRef::Text(&[0xff, 0xfe])),
            serde_json::json!("//4=")
        );
        assert_eq!(
            value_to_json(ValueRef::Blob(b"hi")),
            serde_json::json!("aGk=")
        );
    }
}
