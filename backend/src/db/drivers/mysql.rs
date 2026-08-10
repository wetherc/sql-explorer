//! The MySQL and MariaDB driver.

use crate::db::drivers::{
    add_constraint_column, add_index_column, add_snapshot_column, bytes_to_json, constraint_kind,
    f64_to_json, number_out_of_range, number_value, parameter_type_refused, prefixed_plan,
    routine_kind, rows_affected_message, rows_returned_message, size_text, table_kind,
    CancelHandle, DatabaseDriver, NumberValue,
};
use crate::db::{
    AppColumn, ColumnInfo, Constraint, CreateQuery, Database, DriverCapabilities, ExecOptions,
    IndexInfo, Message, PlanKind, QueryParams, QueryResponse, ResultSet, Routine, Schema,
    SchemaSnapshot, SnapshotColumn, Table, TableFact, TableKind,
};
use crate::error::{Error, Result};
use crate::sql::{split_statements, Dialect};
use crate::storage::SavedConnection;
use async_trait::async_trait;
use mysql_async::prelude::*;
use mysql_async::{Conn, Opts, OptsBuilder, Row as MysqlRow, SslOpts, Value as MysqlValue};
use serde_json::Value as JsonValue;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct MysqlDriver {
    conn: Option<Conn>,
    /// The identifier of the session on the server. A second connection
    /// uses it to stop a statement that runs.
    connection_id: u32,
    opts: Opts,
}

/// Builds the connection options from a saved connection.
pub fn build_opts(connection: &SavedConnection) -> Result<Opts> {
    if let Some(url) = connection.options.connection_url.as_deref() {
        return Ok(Opts::from_url(url.trim())?);
    }

    let mut builder = OptsBuilder::default()
        .ip_or_hostname(connection.effective_host().to_string())
        .prefer_socket(false);

    if let Some(port) = connection.effective_port() {
        builder = builder.tcp_port(port);
    }
    if let Some(user) = connection.user.as_deref().filter(|v| !v.is_empty()) {
        builder = builder.user(Some(user.to_string()));
    }
    if let Some(password) = connection.password.as_deref().filter(|v| !v.is_empty()) {
        builder = builder.pass(Some(password.to_string()));
    }
    if let Some(database) = connection.database.as_deref().filter(|v| !v.is_empty()) {
        builder = builder.db_name(Some(database.to_string()));
    }
    builder = builder.ssl_opts(ssl_opts(connection));

    Ok(Opts::from(builder))
}

/// Selects the transport settings. `mysql_async` has no setting that tries
/// TLS and then continues without it, so a preference is served without
/// TLS and a demand is served with it.
pub fn ssl_opts(connection: &SavedConnection) -> Option<SslOpts> {
    if !connection.options.tls_mode.is_required() {
        return None;
    }
    let mut opts = SslOpts::default();
    if !connection.options.tls_mode.verifies_certificate() {
        opts = opts
            .with_danger_accept_invalid_certs(true)
            .with_danger_skip_domain_validation(true);
    }
    if let Some(path) = connection
        .options
        .ca_cert_path
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        opts = opts.with_root_certs(vec![std::path::PathBuf::from(path).into()]);
    }
    Some(opts)
}

impl MysqlDriver {
    pub async fn connect(connection: &SavedConnection) -> Result<Box<dyn DatabaseDriver>> {
        let opts = build_opts(connection)?;
        let limit = Duration::from_secs(connection.options.connect_timeout_secs.max(1));
        let conn = tokio::time::timeout(limit, Conn::new(opts.clone()))
            .await
            .map_err(|_| Error::Timeout(limit.as_secs()))?
            .map_err(describe_connect_error)?;
        let connection_id = conn.id();
        Ok(Box::new(MysqlDriver {
            conn: Some(conn),
            connection_id,
            opts,
        }))
    }

    /// Borrows the open connection.
    fn conn(&mut self) -> Result<&mut Conn> {
        self.conn.as_mut().ok_or(Error::Connection(
            "The MySQL connection is closed.".to_string(),
        ))
    }
}

/// Turns the authentication plugin error of the server into advice the user
/// can act on.
pub fn describe_connect_error(error: mysql_async::Error) -> Error {
    if let mysql_async::Error::Driver(mysql_async::DriverError::UnknownAuthPlugin { name }) = &error
    {
        return Error::Connection(format!(
            "The server asked for the '{name}' authentication plugin, which this client does not \
             have. Change the user on the server to 'caching_sha2_password' or to \
             'mysql_native_password'."
        ));
    }
    Error::MySql(error)
}

/// Turns the JSON parameters into values the driver can bind.
pub fn bind_params(params: Option<&QueryParams>) -> Result<mysql_async::Params> {
    let Some(params) = params else {
        return Ok(mysql_async::Params::Empty);
    };
    let mut values: Vec<MysqlValue> = Vec::new();
    for param in params {
        values.push(match &param.value {
            JsonValue::String(text) => MysqlValue::from(text.clone()),
            JsonValue::Bool(flag) => MysqlValue::from(*flag),
            JsonValue::Null => MysqlValue::NULL,
            JsonValue::Number(number) => match number_value(number) {
                Some(NumberValue::Integer(value)) => MysqlValue::from(value),
                Some(NumberValue::Float(value)) => MysqlValue::from(value),
                None => return Err(number_out_of_range(number)),
            },
            other => return Err(parameter_type_refused(other)),
        });
    }
    Ok(mysql_async::Params::Positional(values))
}

/// Runs one statement and collects everything it produced.
async fn run_statement(
    conn: &mut Conn,
    statement: &str,
    params: mysql_async::Params,
    options: &ExecOptions,
    response: &mut QueryResponse,
) -> Result<()> {
    let mut result = conn.exec_iter(statement, params).await?;

    loop {
        let columns: Vec<ColumnInfo> = result.columns().map_or_else(Vec::new, |columns| {
            columns
                .iter()
                .map(|column| {
                    ColumnInfo::new(
                        column.name_str().to_string(),
                        format!("{:?}", column.column_type()).to_lowercase(),
                    )
                })
                .collect()
        });

        if columns.is_empty() {
            // The statement changed rows instead of returning them.
            let affected = result.affected_rows();
            let info = result.info().to_string();
            response.rows_affected = Some(response.rows_affected.unwrap_or(0) + affected);
            response.messages.push(rows_affected_message(affected));
            if !info.is_empty() {
                // The server sends the text of the statement, such as the
                // rows it matched and the warnings it counted.
                response.messages.push(Message::info(info));
            }
            if result.is_empty() {
                break;
            }
            let _: Vec<MysqlRow> = result.collect().await?;
            continue;
        }

        let mut set = ResultSet::new(columns.clone());
        let rows: Vec<MysqlRow> = result.collect().await?;
        for row in rows {
            if set.rows.len() >= options.max_rows {
                set.truncated = true;
                break;
            }
            set.rows.push(row_to_json(&row, columns.len()));
        }
        response
            .messages
            .push(rows_returned_message(set.rows.len(), set.truncated));
        response.results.push(set);

        if result.is_empty() {
            break;
        }
    }

    Ok(())
}

#[async_trait]
impl DatabaseDriver for MysqlDriver {
    fn capabilities(&self) -> DriverCapabilities {
        DriverCapabilities {
            supports_schemas: false,
            supports_multiple_databases: true,
            supports_cancel: true,
            supports_transactions: true,
            supports_routines: true,
            supports_indexes: true,
            supports_constraints: true,
            supports_partitions: false,
            supports_explain: true,
        }
    }

    fn dialect(&self) -> Dialect {
        Dialect::MySql
    }

    fn create_query(
        &self,
        database: Option<&str>,
        _schema: Option<&str>,
        table: &str,
        kind: TableKind,
    ) -> Option<CreateQuery> {
        Some(create_query_text(database, table, kind))
    }

    async fn ping(&mut self) -> Result<()> {
        self.conn()?.ping().await?;
        Ok(())
    }

    async fn execute_query(
        &mut self,
        query: &str,
        params: Option<&QueryParams>,
        options: &ExecOptions,
    ) -> Result<QueryResponse> {
        let started = Instant::now();
        let mut response = QueryResponse::default();

        let statements: Vec<String> = if params.is_some() {
            vec![query.to_string()]
        } else {
            split_statements(query, Dialect::MySql)
        };

        for statement in statements {
            let bound = bind_params(params)?;
            let conn = self.conn()?;
            run_statement(conn, &statement, bound, options, &mut response).await?;
        }

        response.elapsed_ms = started.elapsed().as_millis() as u64;
        Ok(response)
    }

    async fn explain(
        &mut self,
        query: &str,
        params: Option<&QueryParams>,
        kind: PlanKind,
        options: &ExecOptions,
    ) -> Result<QueryResponse> {
        let statement = prefixed_plan(query, Dialect::MySql, plan_prefix(kind))?;
        self.execute_query(&statement, params, options).await
    }

    async fn list_databases(&mut self) -> Result<Vec<Database>> {
        let names: Vec<String> = self
            .conn()?
            .query(
                "SELECT SCHEMA_NAME FROM information_schema.SCHEMATA \
                 ORDER BY SCHEMA_NAME",
            )
            .await?;
        Ok(names.into_iter().map(|name| Database { name }).collect())
    }

    /// MySQL has no level between the database and the table, so the list
    /// of schemas is empty and the explorer puts the tables directly below
    /// the database.
    async fn list_schemas(&mut self, _database: &str) -> Result<Vec<Schema>> {
        Ok(Vec::new())
    }

    async fn list_tables(&mut self, database: &str, _schema: Option<&str>) -> Result<Vec<Table>> {
        let rows: Vec<(String, String)> = self
            .conn()?
            .exec(
                "SELECT TABLE_NAME, TABLE_TYPE FROM information_schema.TABLES \
                 WHERE TABLE_SCHEMA = ? ORDER BY TABLE_TYPE, TABLE_NAME",
                (database,),
            )
            .await?;
        Ok(rows
            .into_iter()
            .map(|(name, kind)| {
                if kind.eq_ignore_ascii_case("VIEW") {
                    Table::view(name)
                } else {
                    Table::table(name)
                }
            })
            .collect())
    }

    async fn list_columns(
        &mut self,
        database: &str,
        _schema: Option<&str>,
        table: &str,
    ) -> Result<Vec<AppColumn>> {
        let rows: Vec<(String, String, String, String)> = self
            .conn()?
            .exec(
                "SELECT COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_KEY \
                 FROM information_schema.COLUMNS \
                 WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? \
                 ORDER BY ORDINAL_POSITION",
                (database, table),
            )
            .await?;
        Ok(rows
            .into_iter()
            .map(|(name, data_type, nullable, key)| AppColumn {
                name,
                data_type,
                nullable: nullable.eq_ignore_ascii_case("YES"),
                is_primary_key: key.eq_ignore_ascii_case("PRI"),
            })
            .collect())
    }

    /// Reads the facts that `information_schema` holds for one relation. The
    /// number of rows of a table of InnoDB is an estimate of the engine.
    async fn table_facts(
        &mut self,
        database: &str,
        _schema: Option<&str>,
        table: &str,
    ) -> Result<Vec<TableFact>> {
        let rows: Vec<(
            Option<u64>,
            Option<u64>,
            Option<u64>,
            Option<String>,
            Option<String>,
            Option<String>,
        )> = self
            .conn()?
            .exec(
                "SELECT TABLE_ROWS, DATA_LENGTH, INDEX_LENGTH, ENGINE, TABLE_COLLATION, \
                        CAST(UPDATE_TIME AS CHAR) \
                 FROM information_schema.TABLES \
                 WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?",
                (database, table),
            )
            .await?;
        let Some((count, data, index, engine, collation, changed)) = rows.into_iter().next() else {
            return Ok(Vec::new());
        };

        let mut facts = Vec::new();
        if let Some(count) = count {
            facts.push(TableFact::new("Rows", format!("about {count}")));
        }
        if data.is_some() || index.is_some() {
            let total = data.unwrap_or(0) + index.unwrap_or(0);
            facts.push(TableFact::new("Size", size_text(total)));
        }
        if let Some(engine) = engine {
            facts.push(TableFact::new("Engine", engine));
        }
        if let Some(collation) = collation {
            facts.push(TableFact::new("Collation", collation));
        }
        if let Some(changed) = changed {
            facts.push(TableFact::new("Last change", changed));
        }
        Ok(facts)
    }

    /// Reads every relation and every column of one database in one
    /// statement. MySQL holds no schema level, so the schema of a relation
    /// stays absent.
    async fn schema_snapshot(
        &mut self,
        database: &str,
        max_columns: usize,
    ) -> Result<SchemaSnapshot> {
        let rows: Vec<(String, String, String, String)> = self
            .conn()?
            .exec(
                "SELECT c.TABLE_NAME, t.TABLE_TYPE, c.COLUMN_NAME, c.COLUMN_TYPE \
                 FROM information_schema.COLUMNS AS c \
                 JOIN information_schema.TABLES AS t \
                   ON t.TABLE_SCHEMA = c.TABLE_SCHEMA AND t.TABLE_NAME = c.TABLE_NAME \
                 WHERE c.TABLE_SCHEMA = ? \
                 ORDER BY c.TABLE_NAME, c.ORDINAL_POSITION",
                (database,),
            )
            .await?;
        let mut snapshot = SchemaSnapshot {
            database: database.to_string(),
            complete: true,
            ..SchemaSnapshot::default()
        };
        for (relation, kind, name, data_type) in rows {
            if !add_snapshot_column(
                &mut snapshot,
                max_columns,
                None,
                relation,
                table_kind(&kind),
                SnapshotColumn { name, data_type },
            ) {
                break;
            }
        }
        Ok(snapshot)
    }

    async fn list_routines(
        &mut self,
        database: &str,
        _schema: Option<&str>,
    ) -> Result<Vec<Routine>> {
        let rows: Vec<(String, String)> = self
            .conn()?
            .exec(
                "SELECT ROUTINE_NAME, ROUTINE_TYPE FROM information_schema.ROUTINES \
                 WHERE ROUTINE_SCHEMA = ? ORDER BY ROUTINE_TYPE, ROUTINE_NAME",
                (database,),
            )
            .await?;
        Ok(rows
            .into_iter()
            .map(|(name, kind)| Routine {
                name,
                kind: routine_kind(&kind),
            })
            .collect())
    }

    /// Reads the indexes from `STATISTICS`, which holds one column of one
    /// index in each row. MySQL names the index of the primary key `PRIMARY`.
    async fn list_indexes(
        &mut self,
        database: &str,
        _schema: Option<&str>,
        table: &str,
    ) -> Result<Vec<IndexInfo>> {
        let rows: Vec<(String, Option<String>, i64)> = self
            .conn()?
            .exec(
                "SELECT INDEX_NAME, COLUMN_NAME, NON_UNIQUE FROM information_schema.STATISTICS \
                 WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? \
                 ORDER BY INDEX_NAME, SEQ_IN_INDEX",
                (database, table),
            )
            .await?;
        let mut indexes = Vec::new();
        for (name, column, not_unique) in rows {
            let primary = name == "PRIMARY";
            add_index_column(&mut indexes, name, not_unique == 0, primary, column);
        }
        Ok(indexes)
    }

    async fn list_constraints(
        &mut self,
        database: &str,
        _schema: Option<&str>,
        table: &str,
    ) -> Result<Vec<Constraint>> {
        let rows: Vec<(
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
        )> = self
            .conn()?
            .exec(
                "SELECT tc.CONSTRAINT_NAME, \
                        tc.CONSTRAINT_TYPE, \
                        ku.COLUMN_NAME, \
                        ku.REFERENCED_TABLE_NAME, \
                        ku.REFERENCED_COLUMN_NAME \
                 FROM information_schema.TABLE_CONSTRAINTS AS tc \
                 LEFT JOIN information_schema.KEY_COLUMN_USAGE AS ku \
                        ON ku.CONSTRAINT_NAME = tc.CONSTRAINT_NAME \
                       AND ku.TABLE_SCHEMA = tc.TABLE_SCHEMA \
                       AND ku.TABLE_NAME = tc.TABLE_NAME \
                 WHERE tc.TABLE_SCHEMA = ? AND tc.TABLE_NAME = ? \
                 ORDER BY tc.CONSTRAINT_NAME, ku.ORDINAL_POSITION",
                (database, table),
            )
            .await?;
        let mut constraints = Vec::new();
        for (name, kind, column, target, target_column) in rows {
            let detail = target.map(|target| match target_column {
                Some(column) => format!("{target}({column})"),
                None => target,
            });
            add_constraint_column(
                &mut constraints,
                name,
                constraint_kind(&kind),
                column,
                detail,
            );
        }
        Ok(constraints)
    }

    fn cancel_handle(&self) -> Option<Arc<dyn CancelHandle>> {
        Some(Arc::new(MysqlCancel {
            opts: self.opts.clone(),
            connection_id: self.connection_id,
        }))
    }
}

/// Opens a second connection and asks the server to stop the statement of
/// the first session.
struct MysqlCancel {
    opts: Opts,
    connection_id: u32,
}

#[async_trait]
impl CancelHandle for MysqlCancel {
    async fn cancel(&self) -> Result<()> {
        let mut conn = Conn::new(self.opts.clone()).await?;
        let outcome = conn
            .query_drop(format!("KILL QUERY {}", self.connection_id))
            .await;
        let _ = conn.disconnect().await;
        outcome?;
        Ok(())
    }
}

/// Converts one row into an array of JSON values.
/// The keyword that asks MySQL or MariaDB for a plan. `EXPLAIN ANALYZE` runs
/// the statement, and it needs MySQL 8.0.18 or MariaDB 10.1 or a later
/// version.
pub fn plan_prefix(kind: PlanKind) -> &'static str {
    match kind {
        PlanKind::Estimated => "EXPLAIN",
        PlanKind::Actual => "EXPLAIN ANALYZE",
    }
}

/// Builds the statement that reads the CREATE text of one object. MySQL and
/// MariaDB answer `SHOW CREATE` with the name in the first column and the
/// text in the second one.
fn create_query_text(database: Option<&str>, table: &str, kind: TableKind) -> CreateQuery {
    let name = Dialect::MySql.qualified_name(database, None, table);
    let word = match kind {
        TableKind::Table => "TABLE",
        TableKind::View => "VIEW",
    };
    CreateQuery::new(format!("SHOW CREATE {word} {name};"), 1)
}

pub fn row_to_json(row: &MysqlRow, column_count: usize) -> Vec<JsonValue> {
    let values: Vec<MysqlValue> = (0..column_count)
        .map(|index| row.as_ref(index).cloned().unwrap_or(MysqlValue::NULL))
        .collect();
    values_to_json(&values)
}

/// Converts a whole row of values into JSON.
pub fn values_to_json(values: &[MysqlValue]) -> Vec<JsonValue> {
    values.iter().map(value_to_json).collect()
}

/// Converts one value of the driver into JSON.
pub fn value_to_json(value: &MysqlValue) -> JsonValue {
    match value {
        MysqlValue::NULL => JsonValue::Null,
        MysqlValue::Int(number) => JsonValue::from(*number),
        MysqlValue::UInt(number) => JsonValue::from(*number),
        MysqlValue::Float(number) => f64_to_json(*number as f64),
        MysqlValue::Double(number) => f64_to_json(*number),
        // The server sends text, decimals and binary data as bytes. Text
        // that is not valid UTF-8 is binary, so it becomes base64.
        MysqlValue::Bytes(bytes) => match std::str::from_utf8(bytes) {
            Ok(text) => JsonValue::String(text.to_string()),
            Err(_) => bytes_to_json(bytes),
        },
        MysqlValue::Date(year, month, day, hour, minute, second, microsecond) => JsonValue::String(
            format_date(*year, *month, *day, *hour, *minute, *second, *microsecond),
        ),
        MysqlValue::Time(negative, days, hours, minutes, seconds, microseconds) => {
            JsonValue::String(format_time(
                *negative,
                *days,
                *hours,
                *minutes,
                *seconds,
                *microseconds,
            ))
        }
    }
}

/// Writes a date and a time. The fraction of a second is left out when it
/// is zero, which is what the server itself shows.
pub fn format_date(
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    microsecond: u32,
) -> String {
    let date = format!("{year:04}-{month:02}-{day:02}");
    if hour == 0 && minute == 0 && second == 0 && microsecond == 0 {
        return date;
    }
    let time = format!("{hour:02}:{minute:02}:{second:02}");
    if microsecond == 0 {
        format!("{date} {time}")
    } else {
        format!("{date} {time}.{microsecond:06}")
    }
}

/// Writes an interval. The day count folds into the hours, which is the
/// form the server itself shows.
pub fn format_time(
    negative: bool,
    days: u32,
    hours: u8,
    minutes: u8,
    seconds: u8,
    microseconds: u32,
) -> String {
    let sign = if negative { "-" } else { "" };
    let total_hours = days * 24 + hours as u32;
    if microseconds == 0 {
        format!("{sign}{total_hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{sign}{total_hours:02}:{minutes:02}:{seconds:02}.{microseconds:06}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_analysed_plan_runs_the_statement() {
        assert_eq!(plan_prefix(PlanKind::Estimated), "EXPLAIN");
        assert_eq!(plan_prefix(PlanKind::Actual), "EXPLAIN ANALYZE");
    }

    #[test]
    fn the_create_statement_names_the_kind_of_the_object() {
        let table = create_query_text(Some("db"), "t", TableKind::Table);
        assert_eq!(table.sql, "SHOW CREATE TABLE `db`.`t`;");
        assert_eq!(table.column, 1);

        let view = create_query_text(None, "v", TableKind::View);
        assert_eq!(view.sql, "SHOW CREATE VIEW `v`;");
    }
    use crate::storage::{ConnectionOptions, DbType, TlsMode};

    fn connection() -> SavedConnection {
        SavedConnection {
            id: "id".into(),
            name: "name".into(),
            db_type: DbType::Mysql,
            host: Some("mysql.example.com".into()),
            port: Some(3307),
            user: Some("root".into()),
            database: Some("shop".into()),
            password: Some("p@ss:word/with?chars".into()),
            options: ConnectionOptions::default(),
            color: None,
            group: None,
        }
    }

    #[test]
    fn the_options_keep_the_host_the_port_and_the_credentials() {
        let opts = build_opts(&connection()).unwrap();
        assert_eq!(opts.ip_or_hostname(), "mysql.example.com");
        assert_eq!(opts.tcp_port(), 3307);
        assert_eq!(opts.user(), Some("root"));
        assert_eq!(opts.pass(), Some("p@ss:word/with?chars"));
        assert_eq!(opts.db_name(), Some("shop"));
    }

    #[test]
    fn the_options_fall_back_to_the_default_port() {
        let mut input = connection();
        input.port = None;
        assert_eq!(build_opts(&input).unwrap().tcp_port(), 3306);
    }

    #[test]
    fn empty_credentials_are_left_out() {
        let mut input = connection();
        input.user = Some(String::new());
        input.password = Some(String::new());
        input.database = Some(String::new());
        let opts = build_opts(&input).unwrap();
        assert_eq!(opts.db_name(), None);
    }

    #[test]
    fn a_connection_string_replaces_the_fields() {
        let mut input = connection();
        input.options.connection_url = Some("mysql://user:pw@other.example.com:3399/other".into());
        let opts = build_opts(&input).unwrap();
        assert_eq!(opts.ip_or_hostname(), "other.example.com");
        assert_eq!(opts.tcp_port(), 3399);
        assert_eq!(opts.db_name(), Some("other"));
    }

    #[test]
    fn a_connection_string_that_is_not_valid_gives_an_error() {
        let mut input = connection();
        input.options.connection_url = Some("not-a-url".into());
        assert!(build_opts(&input).is_err());
    }

    #[test]
    fn the_transport_setting_selects_the_tls_options() {
        let mut input = connection();

        input.options.tls_mode = TlsMode::Disable;
        assert!(ssl_opts(&input).is_none());

        input.options.tls_mode = TlsMode::Prefer;
        assert!(ssl_opts(&input).is_none());

        input.options.tls_mode = TlsMode::Require;
        let opts = ssl_opts(&input).unwrap();
        assert!(opts.accept_invalid_certs());

        input.options.tls_mode = TlsMode::VerifyFull;
        let opts = ssl_opts(&input).unwrap();
        assert!(!opts.accept_invalid_certs());

        input.options.ca_cert_path = Some("/etc/ca.pem".into());
        assert_eq!(ssl_opts(&input).unwrap().root_certs().len(), 1);

        input.options.ca_cert_path = Some("  ".into());
        assert!(ssl_opts(&input).unwrap().root_certs().is_empty());
    }

    #[test]
    fn an_unknown_authentication_plugin_gives_advice() {
        let error = describe_connect_error(mysql_async::Error::Driver(
            mysql_async::DriverError::UnknownAuthPlugin {
                name: "sha256_password".into(),
            },
        ));
        assert_eq!(error.kind(), crate::error::ErrorKind::Connection);
        assert!(error.to_string().contains("sha256_password"));
        assert!(error.to_string().contains("caching_sha2_password"));
    }

    #[test]
    fn another_connection_error_keeps_its_own_text() {
        let error = describe_connect_error(mysql_async::Error::Other("boom".into()));
        assert_eq!(error.kind(), crate::error::ErrorKind::Database);
    }

    #[test]
    fn the_parameters_accept_the_simple_json_types() {
        let params = vec![
            crate::db::QueryParam {
                value: serde_json::json!("text"),
            },
            crate::db::QueryParam {
                value: serde_json::json!(-7),
            },
            crate::db::QueryParam {
                value: serde_json::json!(1.5),
            },
            crate::db::QueryParam {
                value: serde_json::json!(false),
            },
            crate::db::QueryParam {
                value: serde_json::Value::Null,
            },
        ];
        match bind_params(Some(&params)).unwrap() {
            mysql_async::Params::Positional(values) => assert_eq!(values.len(), 5),
            other => panic!("unexpected parameters: {other:?}"),
        }
        assert!(matches!(
            bind_params(None).unwrap(),
            mysql_async::Params::Empty
        ));
    }

    #[test]
    fn a_parameter_with_a_structured_type_is_refused() {
        let params = vec![crate::db::QueryParam {
            value: serde_json::json!({ "a": 1 }),
        }];
        assert_eq!(
            bind_params(Some(&params)).unwrap_err().kind(),
            crate::error::ErrorKind::Configuration
        );
    }

    #[test]
    fn a_whole_number_outside_the_range_is_refused() {
        let params = vec![crate::db::QueryParam {
            value: serde_json::json!(18446744073709551615u64),
        }];
        assert_eq!(
            bind_params(Some(&params)).unwrap_err().kind(),
            crate::error::ErrorKind::Configuration
        );
    }

    #[test]
    fn every_value_type_becomes_json() {
        assert_eq!(value_to_json(&MysqlValue::NULL), JsonValue::Null);
        assert_eq!(value_to_json(&MysqlValue::Int(-4)), serde_json::json!(-4));
        assert_eq!(value_to_json(&MysqlValue::UInt(4)), serde_json::json!(4));
        assert_eq!(
            value_to_json(&MysqlValue::Double(1.25)),
            serde_json::json!(1.25)
        );
        assert_eq!(
            value_to_json(&MysqlValue::Float(0.5)),
            serde_json::json!(0.5)
        );
        assert_eq!(
            value_to_json(&MysqlValue::Bytes(b"hello".to_vec())),
            serde_json::json!("hello")
        );
        // Bytes that are not valid text become base64.
        assert_eq!(
            value_to_json(&MysqlValue::Bytes(vec![0xff, 0xfe])),
            serde_json::json!("//4=")
        );
    }

    #[test]
    fn a_date_shows_only_the_parts_that_carry_information() {
        assert_eq!(
            value_to_json(&MysqlValue::Date(2026, 8, 10, 0, 0, 0, 0)),
            serde_json::json!("2026-08-10")
        );
        assert_eq!(
            value_to_json(&MysqlValue::Date(2026, 8, 10, 13, 5, 6, 0)),
            serde_json::json!("2026-08-10 13:05:06")
        );
        assert_eq!(
            value_to_json(&MysqlValue::Date(2026, 8, 10, 13, 5, 6, 123456)),
            serde_json::json!("2026-08-10 13:05:06.123456")
        );
    }

    #[test]
    fn an_interval_folds_the_days_into_the_hours() {
        assert_eq!(
            value_to_json(&MysqlValue::Time(false, 1, 2, 3, 4, 0)),
            serde_json::json!("26:03:04")
        );
        assert_eq!(
            value_to_json(&MysqlValue::Time(true, 0, 2, 3, 4, 500)),
            serde_json::json!("-02:03:04.000500")
        );
    }

    #[test]
    fn a_row_of_values_gives_one_json_value_for_each_column() {
        let values = vec![
            MysqlValue::Int(1),
            MysqlValue::Bytes(b"a".to_vec()),
            MysqlValue::NULL,
        ];
        assert_eq!(
            values_to_json(&values),
            vec![
                serde_json::json!(1),
                serde_json::json!("a"),
                JsonValue::Null
            ]
        );
        assert!(values_to_json(&[]).is_empty());
    }
}
