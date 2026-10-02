//! The MySQL and MariaDB driver.

use crate::db::drivers::{
    add_constraint_column, add_index_column, add_snapshot_column, bytes_to_json, constraint_kind,
    f32_to_json, f64_to_json, next_values, number_out_of_range, number_value,
    parameter_type_refused, prefixed_plan, routine_kind, rows_affected_message,
    rows_returned_message, size_text, system_roots, table_kind, CancelHandle, DatabaseDriver,
    NumberValue,
};
use crate::db::sink::{RowSink, RunSummary, SinkControl};
use crate::db::{
    AppColumn, ColumnInfo, Constraint, CreateQuery, Database, DriverCapabilities, ExecOptions,
    IndexInfo, Message, MessageLevel, PlanKind, QueryParams, QueryResponse, Routine, Schema,
    SchemaSnapshot, SnapshotColumn, Table, TableFact, TableKind,
};
use crate::error::{is_mysql_stop, Error, Result};
use crate::sql::{only_reads, split_statements, Dialect};
use crate::storage::{SavedConnection, TlsMode};
use async_trait::async_trait;
use mysql_async::consts::{ColumnFlags, ColumnType, StatusFlags};
use mysql_async::prelude::*;
use mysql_async::{Conn, Opts, OptsBuilder, Row as MysqlRow, SslOpts, Value as MysqlValue};
use serde_json::Value as JsonValue;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct MysqlDriver {
    conn: Option<Conn>,
    /// The identifier of the session on the server. A second connection
    /// uses it to stop a statement that runs.
    connection_id: u32,
    opts: Opts,
    /// The connect time limit of the record, which the stop also obeys.
    connect_limit: Duration,
}

/// Builds the connection options from a saved connection.
pub fn build_opts(connection: &SavedConnection) -> Result<Opts> {
    if let Some(url) = connection.options.connection_url.as_deref() {
        let builder = add_fields_of_record(Opts::from_url(url.trim())?, connection);
        return Ok(Opts::from(read_only_setup(builder, connection)));
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

    Ok(Opts::from(read_only_setup(builder, connection)))
}

/// True when a connection string gives a password.
pub fn string_has_password(url: &str) -> Result<bool> {
    Ok(Opts::from_url(url.trim())?
        .pass()
        .is_some_and(|password| !password.is_empty()))
}

/// Adds the fields of the record that a connection string does not give.
/// The keychain keeps the password, so the string does not give one. The
/// transport mode of the form also applies when the string names no TLS.
fn add_fields_of_record(opts: Opts, connection: &SavedConnection) -> OptsBuilder {
    let mut builder = OptsBuilder::from_opts(opts.clone());
    if opts.user().is_none() {
        if let Some(user) = connection.user.as_deref().filter(|v| !v.is_empty()) {
            builder = builder.user(Some(user.to_string()));
        }
    }
    if opts.pass().is_none() {
        if let Some(password) = connection.password.as_deref().filter(|v| !v.is_empty()) {
            builder = builder.pass(Some(password.to_string()));
        }
    }
    if opts.ssl_opts().is_none() {
        builder = builder.ssl_opts(ssl_opts(connection));
    }
    builder
}

/// The statement that makes every later transaction of the session read-only.
const READ_ONLY_SESSION: &str = "SET SESSION TRANSACTION READ ONLY";

/// Adds the read-only statement to the setup of a read-only connection. The
/// driver runs the setup on each new login and after each reset of the
/// session, so a reset does not give back write access.
fn read_only_setup(builder: OptsBuilder, connection: &SavedConnection) -> OptsBuilder {
    if !connection.options.read_only {
        return builder;
    }
    let mut setup = Opts::from(builder.clone()).setup().to_vec();
    setup.push(READ_ONLY_SESSION.to_string());
    builder.setup(setup)
}

/// Selects the transport settings. A preference asks for TLS and accepts
/// any certificate, as a demand without verification does. `mysql_async`
/// has no setting that tries TLS and then continues without it, so
/// `clear_text_opts` gives the options of the second login. A demand with
/// verification trusts the roots of the operating system in place of the
/// Mozilla roots that `mysql_async` holds, and it keeps the Mozilla roots
/// when the system gives no usable root.
pub fn ssl_opts(connection: &SavedConnection) -> Option<SslOpts> {
    if connection.options.tls_mode == TlsMode::Disable {
        return None;
    }
    let mut opts = SslOpts::default();
    let mut roots = Vec::new();
    if connection.options.tls_mode.verifies_certificate() {
        let system = system_roots();
        roots.extend(system.iter().map(|root| root.to_vec().into()));
        opts = opts.with_disable_built_in_roots(!system.is_empty());
    } else {
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
        roots.push(std::path::PathBuf::from(path).into());
    }
    Some(opts.with_root_certs(roots))
}

/// Gives the options of a login in clear text when the record prefers TLS
/// and the TLS settings come from the record. A connection string that
/// names its own TLS settings gets no second login.
pub fn clear_text_opts(connection: &SavedConnection) -> Result<Option<Opts>> {
    if connection.options.tls_mode != TlsMode::Prefer {
        return Ok(None);
    }
    if let Some(url) = connection.options.connection_url.as_deref() {
        if Opts::from_url(url.trim())?.ssl_opts().is_some() {
            return Ok(None);
        }
    }
    let builder = OptsBuilder::from_opts(build_opts(connection)?).ssl_opts(None::<SslOpts>);
    Ok(Some(Opts::from(builder)))
}

/// True when the login failed because the server offers no TLS.
fn server_refuses_tls(error: &mysql_async::Error) -> bool {
    matches!(
        error,
        mysql_async::Error::Driver(mysql_async::DriverError::NoClientSslFlagFromServer)
    )
}

/// Opens a login and gives the options that it used. When the record
/// prefers TLS and the server offers none, the driver logs in again in
/// clear text.
async fn open_login(connection: &SavedConnection) -> Result<(Conn, Opts)> {
    let opts = build_opts(connection)?;
    match Conn::new(opts.clone()).await {
        Ok(conn) => Ok((conn, opts)),
        Err(error) if server_refuses_tls(&error) => match clear_text_opts(connection)? {
            Some(plain) => {
                let conn = Conn::new(plain.clone())
                    .await
                    .map_err(describe_connect_error)?;
                Ok((conn, plain))
            }
            None => Err(describe_connect_error(error)),
        },
        Err(error) => Err(describe_connect_error(error)),
    }
}

impl MysqlDriver {
    pub async fn connect(connection: &SavedConnection) -> Result<Box<dyn DatabaseDriver>> {
        let limit = Duration::from_secs(connection.options.connect_timeout_secs.max(1));
        let (conn, opts) = tokio::time::timeout(limit, open_login(connection))
            .await
            .map_err(|_| Error::Timeout(limit.as_secs()))??;
        let connection_id = conn.id();
        Ok(Box::new(MysqlDriver {
            conn: Some(conn),
            connection_id,
            opts,
            connect_limit: limit,
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

/// Turns the JSON parameters into values the driver can bind. A run
/// without parameters gives no list.
pub fn bind_params(params: Option<&QueryParams>) -> Result<Option<Vec<MysqlValue>>> {
    let Some(params) = params else {
        return Ok(None);
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
    Ok(Some(values))
}

/// Runs one statement and feeds each row to the sink as the driver reads
/// it. A set that passed the row limit or a stop of the sink drains one row
/// at a time, so the connection stays fit for the next set.
///
/// A statement without parameters goes through the text protocol. MySQL
/// refuses `CREATE PROCEDURE`, `CREATE TRIGGER`, `USE` and some other
/// statements in the prepared protocol with error 1295, and a prepared
/// statement costs a second round trip. A run that sets `one_statement`
/// goes through the prepared protocol, because there the server refuses a
/// text with a second statement. In a run with parameters, a statement that
/// holds a `?` goes there too. The prepared statement gives the number of its
/// places, and the statement takes that many of the values of the script.
/// `used` counts the values that earlier statements took.
///
/// MySQL holds no packet that ends a statement on the connection that runs
/// it. The drain reads each remaining row from the network, so a large
/// result takes the time to receive it. A statement that only reads gets
/// `kill`, which runs `KILL QUERY` from a second connection when the drain
/// takes longer than `KILL_GRACE`, as `drain_with_kill` describes. Any other
/// statement drains in full, because a stop in the middle of a procedure
/// would leave its later writes undone.
///
/// The flag `stopped` carries a stop of the sink back to the caller, and a
/// run that arrives with the flag set drains its sets without a feed.
#[allow(clippy::too_many_arguments)]
async fn stream_statement(
    conn: &mut Conn,
    statement: &str,
    values: Option<&[MysqlValue]>,
    used: &mut usize,
    options: &ExecOptions,
    sink: &mut dyn RowSink,
    rows_affected: &mut Option<u64>,
    stopped: &mut bool,
    kill: &MysqlCancel,
) -> Result<()> {
    let kill = only_reads(statement, Dialect::MySql).then_some(kill);
    let mut killed = false;
    if options.one_statement || (values.is_some() && statement.contains('?')) {
        let prepared = conn.prep(statement).await?;
        let count = usize::from(prepared.num_params());
        let bound = next_values(values.unwrap_or_default(), used, count);
        let params = if bound.is_empty() {
            mysql_async::Params::Empty
        } else {
            mysql_async::Params::Positional(bound)
        };
        let result = conn.exec_iter(prepared, params).await?;
        read_sets(
            result,
            options,
            sink,
            rows_affected,
            stopped,
            kill,
            &mut killed,
        )
        .await?;
    } else {
        let result = conn.query_iter(statement).await?;
        read_sets(
            result,
            options,
            sink,
            rows_affected,
            stopped,
            kill,
            &mut killed,
        )
        .await?;
    }
    if killed {
        absorb_kill(conn).await?;
    }
    Ok(())
}

/// The time that the drain of a stopped set may take before the driver
/// sends `KILL QUERY`. A short rest of a result drains faster than the login
/// of a second connection.
const KILL_GRACE: Duration = Duration::from_millis(200);

/// Drains the rest of a set that the driver stopped reading. A drain that
/// ends within `grace` needs nothing more. A longer drain runs `kill` beside
/// it, so the server ends the statement and sends a fault in place of the
/// remaining rows. That fault is the expected end of the drain. A kill that
/// fails leaves the drain to read every row.
///
/// Gives true when the kill started. The kill can reach the session after
/// the statement ended, so the caller then runs `absorb_kill`.
async fn drain_with_kill<D, K>(drain: D, kill: K, grace: Duration) -> mysql_async::Result<bool>
where
    D: Future<Output = mysql_async::Result<()>>,
    K: Future<Output = Result<()>>,
{
    tokio::pin!(drain);
    tokio::select! {
        drained = &mut drain => return drained.map(|()| false),
        () = tokio::time::sleep(grace) => {}
    }
    let (killed, drained) = tokio::join!(kill, drain);
    if let Err(error) = killed {
        log::warn!("The server did not stop the rest of the result: {error}");
    }
    match drained {
        Err(error) if !is_mysql_stop(&error) => Err(error),
        _ => Ok(true),
    }
}

/// Runs a statement that does nothing after a `KILL QUERY`. A server that
/// keeps a kill which arrived between two statements stops the next
/// statement of the session, so this statement takes the stop in place of
/// the next statement of the user.
async fn absorb_kill(conn: &mut Conn) -> Result<()> {
    match conn.query_drop("DO 0").await {
        Err(error) if is_mysql_stop(&error) => Ok(()),
        other => other.map_err(Error::from),
    }
}

/// Reads the rest of the current set of a result and drops the rows.
async fn drain_set<P: Protocol>(
    result: &mut mysql_async::QueryResult<'_, 'static, P>,
) -> mysql_async::Result<()> {
    while result.next().await?.is_some() {}
    Ok(())
}

/// Reads each set of one statement into the sink, as `stream_statement`
/// describes.
async fn read_sets<P: Protocol>(
    mut result: mysql_async::QueryResult<'_, 'static, P>,
    options: &ExecOptions,
    sink: &mut dyn RowSink,
    rows_affected: &mut Option<u64>,
    stopped: &mut bool,
    kill: Option<&MysqlCancel>,
    killed: &mut bool,
) -> Result<()> {
    loop {
        let wire = result.columns().unwrap_or_default();
        let columns: Vec<ColumnInfo> = wire
            .iter()
            .map(|column| {
                ColumnInfo::new(
                    column.name_str().to_string(),
                    type_label(column.column_type(), column.character_set(), column.flags()),
                )
            })
            .collect();

        if columns.is_empty() {
            // The statement changed rows instead of returning them.
            let affected = result.affected_rows();
            let info = result.info().to_string();
            *rows_affected = Some(rows_affected.unwrap_or(0) + affected);
            sink.message(rows_affected_message(affected));
            if !info.is_empty() {
                // The server sends the text of the statement, such as the
                // rows it matched and the warnings it counted.
                sink.message(Message::info(info));
            }
            if result.is_empty() {
                break;
            }
            while result.next().await?.is_some() {}
            continue;
        }

        if *stopped {
            while result.next().await?.is_some() {}
            if result.is_empty() {
                break;
            }
            continue;
        }

        let kinds: Vec<ValueKind> = wire
            .iter()
            .map(|column| value_kind(column.column_type(), column.character_set()))
            .collect();
        sink.begin_set(columns.clone())?;
        let mut count = 0usize;
        let mut truncated = false;
        while let Some(row) = result.next().await? {
            if count >= options.max_rows {
                truncated = true;
                break;
            }
            if sink.row(row_to_json(&row, &kinds))? == SinkControl::Stop {
                truncated = true;
                *stopped = true;
                break;
            }
            count += 1;
        }
        if truncated {
            let drain = drain_set(&mut result);
            match kill {
                Some(kill) => *killed |= drain_with_kill(drain, kill.cancel(), KILL_GRACE).await?,
                None => drain.await?,
            }
        }
        sink.message(rows_returned_message(count, truncated));
        sink.end_set(truncated)?;

        if result.is_empty() {
            break;
        }
    }

    Ok(())
}

/// Sends the warnings of the last statement to the sink.
///
/// The OK packet at the end of a statement gives only the count of its
/// warnings, so the text needs `SHOW WARNINGS`. The query runs only when the
/// count is above zero. The statement has already succeeded, so a failure of
/// this read becomes a message and does not fail the run.
async fn report_warnings(conn: &mut Conn, sink: &mut dyn RowSink) {
    if conn.get_warnings() == 0 {
        return;
    }
    match conn
        .query::<(String, u32, String), _>("SHOW WARNINGS")
        .await
    {
        Ok(rows) => {
            for (level, code, text) in rows {
                sink.message(warning_message(&level, code, text));
            }
        }
        Err(error) => sink.message(Message::warning(format!(
            "The warnings of the statement could not be read: {error}"
        ))),
    }
}

/// The message for one row of `SHOW WARNINGS`. The level of the row sets the
/// level of the message, and the detail gives the level and the code as the
/// server sent them.
fn warning_message(level: &str, code: u32, text: String) -> Message {
    let kind = match level {
        "Note" => MessageLevel::Info,
        "Error" => MessageLevel::Error,
        _ => MessageLevel::Warning,
    };
    Message {
        level: kind,
        text,
        detail: Some(format!("{level}, Code {code}")),
    }
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

    /// The server marks each OK packet with a flag while a transaction is
    /// open, so a statement that does nothing reads the state.
    async fn holds_open_transaction(&mut self) -> Result<bool> {
        let conn = self.conn()?;
        conn.query_drop("DO 0").await?;
        Ok(conn.last_ok_packet().is_some_and(|ok| {
            ok.status_flags()
                .contains(StatusFlags::SERVER_STATUS_IN_TRANS)
        }))
    }

    async fn execute_stream(
        &mut self,
        query: &str,
        params: Option<&QueryParams>,
        options: &ExecOptions,
        sink: &mut dyn RowSink,
    ) -> Result<RunSummary> {
        let started = Instant::now();
        let mut rows_affected: Option<u64> = None;
        let mut stopped = false;

        let values = bind_params(params)?;
        let mut used = 0;
        for statement in split_statements(query, Dialect::MySql) {
            if stopped {
                break;
            }
            let kill = MysqlCancel {
                opts: self.opts.clone(),
                connection_id: self.connection_id,
                limit: self.connect_limit,
            };
            let conn = self.conn()?;
            stream_statement(
                conn,
                &statement,
                values.as_deref(),
                &mut used,
                options,
                sink,
                &mut rows_affected,
                &mut stopped,
                &kill,
            )
            .await?;
            report_warnings(self.conn()?, sink).await;
        }

        Ok(RunSummary {
            rows_affected,
            elapsed_ms: started.elapsed().as_millis() as u64,
            stats: None,
        })
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
            limit: self.connect_limit,
        }))
    }
}

/// Opens a second connection and asks the server to stop the statement of
/// the first session. The login and the `KILL QUERY` together obey the
/// connect time limit, so on a lost network a stop gives up after that
/// limit and not after the connect timeout of the operating system, which
/// is about 75 s.
struct MysqlCancel {
    opts: Opts,
    connection_id: u32,
    limit: Duration,
}

#[async_trait]
impl CancelHandle for MysqlCancel {
    async fn cancel(&self) -> Result<()> {
        let kill = async {
            let mut conn = Conn::new(self.opts.clone()).await?;
            let outcome = conn
                .query_drop(format!("KILL QUERY {}", self.connection_id))
                .await;
            let _ = conn.disconnect().await;
            outcome.map_err(Error::from)
        };
        tokio::time::timeout(self.limit, kill)
            .await
            .map_err(|_| Error::Timeout(self.limit.as_secs()))?
    }
}

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

/// The kind of value that a column holds, as far as the conversion to JSON
/// needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    /// A DATE column. The driver gives the same value for DATE, DATETIME
    /// and TIMESTAMP, so the kind decides whether the text has a time.
    DateOnly,
    /// A whole number. The text protocol sends it as text.
    Integer,
    /// A floating-point number. The text protocol sends it as text.
    Float,
    /// A BIT column. The server sends its bits as bytes in both protocols,
    /// and those bytes as text would show control characters.
    Bit,
    /// A BINARY, VARBINARY or BLOB column. Its bytes become base64 even when
    /// they happen to be valid UTF-8, so one column does not mix text and
    /// base64.
    Binary,
    /// Any other column.
    Other,
}

/// The number of the `binary` character set. The server gives it to BINARY,
/// VARBINARY and BLOB columns, and also to numbers and dates.
const BINARY_CHARSET: u16 = 63;

/// Finds the kind of value from the wire type and the character set of a
/// column.
pub fn value_kind(column_type: ColumnType, charset: u16) -> ValueKind {
    use ColumnType::*;
    match column_type {
        MYSQL_TYPE_DATE | MYSQL_TYPE_NEWDATE => ValueKind::DateOnly,
        MYSQL_TYPE_TINY | MYSQL_TYPE_SHORT | MYSQL_TYPE_INT24 | MYSQL_TYPE_LONG
        | MYSQL_TYPE_LONGLONG | MYSQL_TYPE_YEAR => ValueKind::Integer,
        MYSQL_TYPE_FLOAT | MYSQL_TYPE_DOUBLE => ValueKind::Float,
        MYSQL_TYPE_BIT => ValueKind::Bit,
        MYSQL_TYPE_STRING
        | MYSQL_TYPE_VAR_STRING
        | MYSQL_TYPE_VARCHAR
        | MYSQL_TYPE_BLOB
        | MYSQL_TYPE_TINY_BLOB
        | MYSQL_TYPE_MEDIUM_BLOB
        | MYSQL_TYPE_LONG_BLOB
            if charset == BINARY_CHARSET =>
        {
            ValueKind::Binary
        }
        _ => ValueKind::Other,
    }
}

/// Names the type of a result column in the words of MySQL, such as
/// `varchar` or `int unsigned`. The wire type says less than the type of
/// the table: TEXT and MEDIUMTEXT both arrive as a BLOB type, ENUM and SET
/// arrive as a fixed string with a flag, and a binary string has the
/// binary character set.
pub fn type_label(column_type: ColumnType, charset: u16, flags: ColumnFlags) -> String {
    use ColumnType::*;
    let binary = charset == BINARY_CHARSET;
    let name = match column_type {
        MYSQL_TYPE_DECIMAL | MYSQL_TYPE_NEWDECIMAL => "decimal",
        MYSQL_TYPE_TINY => "tinyint",
        MYSQL_TYPE_SHORT => "smallint",
        MYSQL_TYPE_INT24 => "mediumint",
        MYSQL_TYPE_LONG => "int",
        MYSQL_TYPE_LONGLONG => "bigint",
        MYSQL_TYPE_FLOAT => "float",
        MYSQL_TYPE_DOUBLE => "double",
        MYSQL_TYPE_NULL => "null",
        MYSQL_TYPE_TIMESTAMP | MYSQL_TYPE_TIMESTAMP2 => "timestamp",
        MYSQL_TYPE_DATE | MYSQL_TYPE_NEWDATE => "date",
        MYSQL_TYPE_TIME | MYSQL_TYPE_TIME2 => "time",
        MYSQL_TYPE_DATETIME | MYSQL_TYPE_DATETIME2 => "datetime",
        MYSQL_TYPE_YEAR => "year",
        MYSQL_TYPE_BIT => "bit",
        MYSQL_TYPE_JSON => "json",
        MYSQL_TYPE_VECTOR => "vector",
        MYSQL_TYPE_GEOMETRY => "geometry",
        MYSQL_TYPE_ENUM => "enum",
        MYSQL_TYPE_SET => "set",
        MYSQL_TYPE_STRING if flags.contains(ColumnFlags::ENUM_FLAG) => "enum",
        MYSQL_TYPE_STRING if flags.contains(ColumnFlags::SET_FLAG) => "set",
        MYSQL_TYPE_STRING if binary => "binary",
        MYSQL_TYPE_STRING => "char",
        MYSQL_TYPE_VARCHAR | MYSQL_TYPE_VAR_STRING if binary => "varbinary",
        MYSQL_TYPE_VARCHAR | MYSQL_TYPE_VAR_STRING => "varchar",
        MYSQL_TYPE_TINY_BLOB | MYSQL_TYPE_MEDIUM_BLOB | MYSQL_TYPE_LONG_BLOB | MYSQL_TYPE_BLOB
            if binary =>
        {
            "blob"
        }
        MYSQL_TYPE_TINY_BLOB | MYSQL_TYPE_MEDIUM_BLOB | MYSQL_TYPE_LONG_BLOB | MYSQL_TYPE_BLOB => {
            "text"
        }
        MYSQL_TYPE_TYPED_ARRAY | MYSQL_TYPE_UNKNOWN => "unknown",
    };
    let numeric = matches!(
        column_type,
        MYSQL_TYPE_DECIMAL
            | MYSQL_TYPE_NEWDECIMAL
            | MYSQL_TYPE_TINY
            | MYSQL_TYPE_SHORT
            | MYSQL_TYPE_INT24
            | MYSQL_TYPE_LONG
            | MYSQL_TYPE_LONGLONG
            | MYSQL_TYPE_FLOAT
            | MYSQL_TYPE_DOUBLE
    );
    if numeric && flags.contains(ColumnFlags::UNSIGNED_FLAG) {
        format!("{name} unsigned")
    } else {
        name.to_string()
    }
}

/// Converts one row into an array of JSON values. The values stay in the row
/// while they are read, so a row of text costs no copy of that text.
///
/// `kinds` holds one kind for each column, as `value_kind` reads it.
pub fn row_to_json(row: &MysqlRow, kinds: &[ValueKind]) -> Vec<JsonValue> {
    kinds
        .iter()
        .enumerate()
        .map(|(index, kind)| {
            row.as_ref(index)
                .map_or(JsonValue::Null, |value| value_to_json(value, *kind))
        })
        .collect()
}

/// Converts a number that the text protocol sends as text. Text that does
/// not parse stays text.
fn number_text_to_json(text: &str, kind: ValueKind) -> Option<JsonValue> {
    match kind {
        ValueKind::Integer => text
            .parse::<i64>()
            .map(JsonValue::from)
            .or_else(|_| text.parse::<u64>().map(JsonValue::from))
            .ok(),
        ValueKind::Float => text.parse::<f64>().ok().map(f64_to_json),
        ValueKind::DateOnly | ValueKind::Bit | ValueKind::Binary | ValueKind::Other => None,
    }
}

/// Converts one value of the driver into JSON. `kind` is the kind of value
/// of the column.
pub fn value_to_json(value: &MysqlValue, kind: ValueKind) -> JsonValue {
    match value {
        MysqlValue::NULL => JsonValue::Null,
        MysqlValue::Int(number) => JsonValue::from(*number),
        MysqlValue::UInt(number) => JsonValue::from(*number),
        MysqlValue::Float(number) => f32_to_json(*number),
        MysqlValue::Double(number) => f64_to_json(*number),
        // A BIT value is a whole number of at most 64 bits, first byte
        // highest, so BIT(1) that holds 1 gives 1 and BIT(8) gives 65, not
        // "A".
        MysqlValue::Bytes(bytes) if kind == ValueKind::Bit && bytes.len() <= 8 => JsonValue::from(
            bytes
                .iter()
                .fold(0u64, |value, byte| (value << 8) | u64::from(*byte)),
        ),
        MysqlValue::Bytes(bytes) if kind == ValueKind::Binary => bytes_to_json(bytes),
        // The server sends text and decimals as bytes. Text that is not
        // valid UTF-8 becomes base64.
        MysqlValue::Bytes(bytes) => match std::str::from_utf8(bytes) {
            Ok(text) => {
                number_text_to_json(text, kind).unwrap_or_else(|| JsonValue::String(text.into()))
            }
            Err(_) => bytes_to_json(bytes),
        },
        MysqlValue::Date(year, month, day, hour, minute, second, microsecond) => {
            JsonValue::String(format_date(
                *year,
                *month,
                *day,
                *hour,
                *minute,
                *second,
                *microsecond,
                kind == ValueKind::DateOnly,
            ))
        }
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
/// is zero, which is what the server itself shows. A column that holds a
/// date alone gives the date alone, and a column that holds a time keeps
/// the time even at midnight, so a DATETIME reads as a DATETIME.
#[allow(clippy::too_many_arguments)]
pub fn format_date(
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    microsecond: u32,
    date_only: bool,
) -> String {
    let date = format!("{year:04}-{month:02}-{day:02}");
    if date_only {
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

    #[tokio::test]
    async fn a_stop_gives_up_at_the_connect_limit() {
        // The server takes the socket and never sends its greeting.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
            drop(socket);
        });
        let cancel = MysqlCancel {
            opts: Opts::from(
                OptsBuilder::default()
                    .ip_or_hostname("127.0.0.1")
                    .tcp_port(port),
            ),
            connection_id: 7,
            limit: Duration::from_millis(200),
        };
        let started = std::time::Instant::now();
        let error = cancel.cancel().await.unwrap_err();
        assert_eq!(error.kind(), crate::error::ErrorKind::Timeout);
        assert!(started.elapsed() < Duration::from_secs(5));
        server.abort();
    }

    fn server_error(code: u16) -> mysql_async::Error {
        mysql_async::Error::Server(mysql_async::ServerError {
            code,
            message: "stopped".into(),
            state: "70100".into(),
        })
    }

    /// A drain that ends after the given time with the given outcome.
    async fn slow_drain(
        wait: Duration,
        outcome: mysql_async::Result<()>,
    ) -> mysql_async::Result<()> {
        tokio::time::sleep(wait).await;
        outcome
    }

    #[tokio::test]
    async fn a_short_drain_sends_no_kill() {
        let asked = std::sync::atomic::AtomicBool::new(false);
        let kill = async {
            asked.store(true, std::sync::atomic::Ordering::Relaxed);
            Ok::<(), Error>(())
        };
        let started = drain_with_kill(
            async { Ok::<(), mysql_async::Error>(()) },
            kill,
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert!(!started);
        assert!(!asked.load(std::sync::atomic::Ordering::Relaxed));
    }

    #[tokio::test]
    async fn a_long_drain_sends_a_kill_and_takes_the_stop_as_its_end() {
        let grace = Duration::from_millis(10);
        let wait = Duration::from_millis(50);

        // The drain ends with the fault that the kill causes.
        let stopped = drain_with_kill(
            slow_drain(wait, Err(server_error(1317))),
            async { Ok::<(), Error>(()) },
            grace,
        )
        .await
        .unwrap();
        assert!(stopped);

        // The drain ends before the kill lands.
        let ended = drain_with_kill(
            slow_drain(wait, Ok(())),
            async { Ok::<(), Error>(()) },
            grace,
        )
        .await
        .unwrap();
        assert!(ended);

        // A kill that fails leaves the drain to read every row.
        let failed = drain_with_kill(
            slow_drain(wait, Ok(())),
            async { Err::<(), Error>(Error::Connection("no login".into())) },
            grace,
        )
        .await
        .unwrap();
        assert!(failed);

        // A different fault of the server stays a fault.
        let error = drain_with_kill(
            slow_drain(wait, Err(server_error(1146))),
            async { Ok::<(), Error>(()) },
            grace,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, mysql_async::Error::Server(server) if server.code == 1146));
    }

    #[test]
    fn a_warning_row_gives_a_message_of_its_level() {
        let note = warning_message("Note", 1051, "Unknown table 'a'".into());
        assert_eq!(note.level, MessageLevel::Info);
        assert_eq!(note.text, "Unknown table 'a'");
        assert_eq!(note.detail.as_deref(), Some("Note, Code 1051"));

        let warning = warning_message("Warning", 1265, "Data truncated".into());
        assert_eq!(warning.level, MessageLevel::Warning);
        assert_eq!(warning.detail.as_deref(), Some("Warning, Code 1265"));

        let error = warning_message("Error", 1146, "No such table".into());
        assert_eq!(error.level, MessageLevel::Error);

        // A level that the server adds later reads as a warning.
        let other = warning_message("Alert", 1, "text".into());
        assert_eq!(other.level, MessageLevel::Warning);
    }

    #[test]
    fn a_bit_column_gives_a_whole_number() {
        use serde_json::json;
        assert_eq!(
            value_kind(ColumnType::MYSQL_TYPE_BIT, BINARY_CHARSET),
            ValueKind::Bit
        );
        let bit = |bytes: &[u8]| value_to_json(&MysqlValue::Bytes(bytes.to_vec()), ValueKind::Bit);
        assert_eq!(bit(&[1]), json!(1));
        assert_eq!(bit(&[0x41]), json!(65));
        assert_eq!(bit(&[0x01, 0x00]), json!(256));
        assert_eq!(bit(&[0xFF; 8]), json!(u64::MAX));
        // Another column that holds the same bytes keeps its text.
        assert_eq!(
            value_to_json(&MysqlValue::Bytes(vec![0x41]), ValueKind::Other),
            json!("A")
        );
    }

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
    use crate::storage::{ConnectionOptions, DbType};

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
            aws_secret_access_key: None,
            aws_session_token: None,
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
    fn a_read_only_connection_sets_the_session_read_only() {
        let mut input = connection();
        assert!(build_opts(&input).unwrap().setup().is_empty());
        input.options.read_only = true;
        let expected = ["SET SESSION TRANSACTION READ ONLY".to_string()];
        assert_eq!(build_opts(&input).unwrap().setup(), expected);
        input.options.connection_url = Some("mysql://user@other.example.com/other".into());
        assert_eq!(build_opts(&input).unwrap().setup(), expected);
    }

    #[test]
    fn a_connection_string_takes_the_fields_that_it_does_not_give() {
        let mut input = connection();
        input.options.connection_url = Some("mysql://other.example.com/other".into());
        let opts = build_opts(&input).unwrap();
        assert_eq!(opts.user(), input.user.as_deref());
        assert_eq!(opts.pass(), input.password.as_deref());
        assert!(opts.ssl_opts().is_some());

        // The values of the string win over the fields of the record.
        input.options.connection_url =
            Some("mysql://u:own@other.example.com/other?require_ssl=true".into());
        input.options.tls_mode = TlsMode::Disable;
        let opts = build_opts(&input).unwrap();
        assert_eq!(opts.user(), Some("u"));
        assert_eq!(opts.pass(), Some("own"));
        assert!(opts.ssl_opts().is_some());

        // Empty fields of the record add nothing.
        input.options.connection_url = Some("mysql://other.example.com/other".into());
        input.user = Some(String::new());
        input.password = None;
        let opts = build_opts(&input).unwrap();
        assert_eq!(opts.user(), None);
        assert_eq!(opts.pass(), None);
        assert!(opts.ssl_opts().is_none());
    }

    #[test]
    fn a_password_in_a_connection_string_is_found() {
        assert!(string_has_password("mysql://u:p@h/d").unwrap());
        assert!(!string_has_password("mysql://u@h/d").unwrap());
        assert!(!string_has_password("mysql://u:@h/d").unwrap());
        assert!(string_has_password("not-a-url").is_err());
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
        let opts = ssl_opts(&input).unwrap();
        assert!(opts.accept_invalid_certs());
        assert!(opts.skip_domain_validation());

        input.options.tls_mode = TlsMode::Require;
        let opts = ssl_opts(&input).unwrap();
        assert!(opts.accept_invalid_certs());

        input.options.ca_cert_path = Some("/etc/ca.pem".into());
        assert_eq!(ssl_opts(&input).unwrap().root_certs().len(), 1);

        input.options.tls_mode = TlsMode::VerifyFull;
        input.options.ca_cert_path = None;
        let opts = ssl_opts(&input).unwrap();
        let system = system_roots().len();
        assert!(!opts.accept_invalid_certs());
        assert_eq!(opts.root_certs().len(), system);
        assert_eq!(opts.disable_built_in_roots(), system > 0);

        input.options.ca_cert_path = Some("/etc/ca.pem".into());
        assert_eq!(ssl_opts(&input).unwrap().root_certs().len(), system + 1);

        input.options.ca_cert_path = Some("  ".into());
        assert_eq!(ssl_opts(&input).unwrap().root_certs().len(), system);
    }

    #[test]
    fn a_preference_for_tls_gives_a_second_login_in_clear_text() {
        let mut input = connection();
        input.options.read_only = true;
        for mode in [TlsMode::Disable, TlsMode::Require, TlsMode::VerifyFull] {
            input.options.tls_mode = mode;
            assert!(clear_text_opts(&input).unwrap().is_none());
        }

        input.options.tls_mode = TlsMode::Prefer;
        let plain = clear_text_opts(&input).unwrap().unwrap();
        assert!(plain.ssl_opts().is_none());
        assert_eq!(plain.ip_or_hostname(), "mysql.example.com");
        assert_eq!(plain.setup(), ["SET SESSION TRANSACTION READ ONLY"]);

        // A string without TLS settings takes the preference of the record.
        input.options.connection_url = Some("mysql://other.example.com/other".into());
        let plain = clear_text_opts(&input).unwrap().unwrap();
        assert!(plain.ssl_opts().is_none());
        assert_eq!(plain.ip_or_hostname(), "other.example.com");

        // A string with its own TLS settings keeps them.
        input.options.connection_url =
            Some("mysql://other.example.com/other?require_ssl=true".into());
        assert!(clear_text_opts(&input).unwrap().is_none());

        input.options.connection_url = Some("not-a-url".into());
        assert!(clear_text_opts(&input).is_err());
    }

    #[test]
    fn only_a_server_without_tls_starts_the_second_login() {
        assert!(server_refuses_tls(&mysql_async::Error::Driver(
            mysql_async::DriverError::NoClientSslFlagFromServer
        )));
        assert!(!server_refuses_tls(&mysql_async::Error::Driver(
            mysql_async::DriverError::ConnectionClosed
        )));
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
        assert_eq!(bind_params(Some(&params)).unwrap().unwrap().len(), 5);
        assert!(bind_params(None).unwrap().is_none());
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
        assert_eq!(
            value_to_json(&MysqlValue::NULL, ValueKind::Other),
            JsonValue::Null
        );
        assert_eq!(
            value_to_json(&MysqlValue::Int(-4), ValueKind::Other),
            serde_json::json!(-4)
        );
        assert_eq!(
            value_to_json(&MysqlValue::UInt(4), ValueKind::Other),
            serde_json::json!(4)
        );
        assert_eq!(
            value_to_json(&MysqlValue::Double(1.25), ValueKind::Other),
            serde_json::json!(1.25)
        );
        assert_eq!(
            value_to_json(&MysqlValue::Float(0.5), ValueKind::Other),
            serde_json::json!(0.5)
        );
        assert_eq!(
            value_to_json(&MysqlValue::Float(0.1), ValueKind::Other),
            serde_json::json!(0.1)
        );
        assert_eq!(
            value_to_json(&MysqlValue::Bytes(b"hello".to_vec()), ValueKind::Other),
            serde_json::json!("hello")
        );
        // Bytes that are not valid text become base64.
        assert_eq!(
            value_to_json(&MysqlValue::Bytes(vec![0xff, 0xfe]), ValueKind::Other),
            serde_json::json!("//4=")
        );
    }

    #[test]
    fn a_date_shows_only_the_parts_that_carry_information() {
        // A DATE column gives the date alone.
        assert_eq!(
            value_to_json(
                &MysqlValue::Date(2026, 8, 10, 0, 0, 0, 0),
                ValueKind::DateOnly
            ),
            serde_json::json!("2026-08-10")
        );
        // A DATETIME column keeps the time, also at midnight.
        assert_eq!(
            value_to_json(&MysqlValue::Date(2026, 8, 10, 0, 0, 0, 0), ValueKind::Other),
            serde_json::json!("2026-08-10 00:00:00")
        );
        assert_eq!(
            value_to_json(
                &MysqlValue::Date(2026, 8, 10, 13, 5, 6, 0),
                ValueKind::Other
            ),
            serde_json::json!("2026-08-10 13:05:06")
        );
        assert_eq!(
            value_to_json(
                &MysqlValue::Date(2026, 8, 10, 13, 5, 6, 123456),
                ValueKind::Other
            ),
            serde_json::json!("2026-08-10 13:05:06.123456")
        );
    }

    #[test]
    fn the_type_of_a_column_gives_the_kind_of_value() {
        use ColumnType::*;
        let text = 255;
        assert_eq!(
            value_kind(MYSQL_TYPE_DATE, BINARY_CHARSET),
            ValueKind::DateOnly
        );
        assert_eq!(
            value_kind(MYSQL_TYPE_NEWDATE, BINARY_CHARSET),
            ValueKind::DateOnly
        );
        assert_eq!(
            value_kind(MYSQL_TYPE_DATETIME, BINARY_CHARSET),
            ValueKind::Other
        );
        assert_eq!(
            value_kind(MYSQL_TYPE_TIMESTAMP, BINARY_CHARSET),
            ValueKind::Other
        );
        for kind in [
            MYSQL_TYPE_TINY,
            MYSQL_TYPE_SHORT,
            MYSQL_TYPE_INT24,
            MYSQL_TYPE_LONG,
            MYSQL_TYPE_LONGLONG,
            MYSQL_TYPE_YEAR,
        ] {
            assert_eq!(value_kind(kind, BINARY_CHARSET), ValueKind::Integer);
        }
        assert_eq!(
            value_kind(MYSQL_TYPE_FLOAT, BINARY_CHARSET),
            ValueKind::Float
        );
        assert_eq!(
            value_kind(MYSQL_TYPE_DOUBLE, BINARY_CHARSET),
            ValueKind::Float
        );
        assert_eq!(
            value_kind(MYSQL_TYPE_NEWDECIMAL, BINARY_CHARSET),
            ValueKind::Other
        );
        for kind in [
            MYSQL_TYPE_STRING,
            MYSQL_TYPE_VAR_STRING,
            MYSQL_TYPE_VARCHAR,
            MYSQL_TYPE_BLOB,
            MYSQL_TYPE_TINY_BLOB,
            MYSQL_TYPE_MEDIUM_BLOB,
            MYSQL_TYPE_LONG_BLOB,
        ] {
            assert_eq!(value_kind(kind, BINARY_CHARSET), ValueKind::Binary);
            assert_eq!(value_kind(kind, text), ValueKind::Other);
        }
    }

    #[test]
    fn the_grid_names_each_type_in_the_words_of_mysql() {
        use ColumnType::*;
        let none = ColumnFlags::empty();
        let unsigned = ColumnFlags::UNSIGNED_FLAG;
        let text = 255;
        let cases = [
            (MYSQL_TYPE_DECIMAL, BINARY_CHARSET, none, "decimal"),
            (
                MYSQL_TYPE_NEWDECIMAL,
                BINARY_CHARSET,
                unsigned,
                "decimal unsigned",
            ),
            (MYSQL_TYPE_TINY, BINARY_CHARSET, none, "tinyint"),
            (MYSQL_TYPE_SHORT, BINARY_CHARSET, none, "smallint"),
            (MYSQL_TYPE_INT24, BINARY_CHARSET, none, "mediumint"),
            (MYSQL_TYPE_LONG, BINARY_CHARSET, unsigned, "int unsigned"),
            (MYSQL_TYPE_LONGLONG, BINARY_CHARSET, none, "bigint"),
            (MYSQL_TYPE_FLOAT, BINARY_CHARSET, none, "float"),
            (MYSQL_TYPE_DOUBLE, BINARY_CHARSET, none, "double"),
            (MYSQL_TYPE_NULL, BINARY_CHARSET, none, "null"),
            (MYSQL_TYPE_TIMESTAMP, BINARY_CHARSET, none, "timestamp"),
            (MYSQL_TYPE_TIMESTAMP2, BINARY_CHARSET, none, "timestamp"),
            (MYSQL_TYPE_DATE, BINARY_CHARSET, none, "date"),
            (MYSQL_TYPE_NEWDATE, BINARY_CHARSET, none, "date"),
            (MYSQL_TYPE_TIME, BINARY_CHARSET, none, "time"),
            (MYSQL_TYPE_TIME2, BINARY_CHARSET, none, "time"),
            (MYSQL_TYPE_DATETIME, BINARY_CHARSET, none, "datetime"),
            (MYSQL_TYPE_DATETIME2, BINARY_CHARSET, none, "datetime"),
            (MYSQL_TYPE_YEAR, BINARY_CHARSET, unsigned, "year"),
            (MYSQL_TYPE_BIT, BINARY_CHARSET, unsigned, "bit"),
            (MYSQL_TYPE_JSON, BINARY_CHARSET, none, "json"),
            (MYSQL_TYPE_VECTOR, BINARY_CHARSET, none, "vector"),
            (MYSQL_TYPE_GEOMETRY, BINARY_CHARSET, none, "geometry"),
            (MYSQL_TYPE_ENUM, text, none, "enum"),
            (MYSQL_TYPE_SET, text, none, "set"),
            (MYSQL_TYPE_STRING, text, ColumnFlags::ENUM_FLAG, "enum"),
            (MYSQL_TYPE_STRING, text, ColumnFlags::SET_FLAG, "set"),
            (MYSQL_TYPE_STRING, BINARY_CHARSET, none, "binary"),
            (MYSQL_TYPE_STRING, text, none, "char"),
            (MYSQL_TYPE_VAR_STRING, BINARY_CHARSET, none, "varbinary"),
            (MYSQL_TYPE_VARCHAR, text, none, "varchar"),
            (MYSQL_TYPE_BLOB, BINARY_CHARSET, none, "blob"),
            (MYSQL_TYPE_LONG_BLOB, text, none, "text"),
            (MYSQL_TYPE_TYPED_ARRAY, BINARY_CHARSET, none, "unknown"),
            (MYSQL_TYPE_UNKNOWN, BINARY_CHARSET, none, "unknown"),
        ];
        for (column_type, charset, flags, label) in cases {
            assert_eq!(type_label(column_type, charset, flags), label);
        }
    }

    #[test]
    fn a_binary_column_gives_base64_for_every_value() {
        assert_eq!(
            value_to_json(&MysqlValue::Bytes(b"AB".to_vec()), ValueKind::Binary),
            serde_json::json!("QUI=")
        );
        assert_eq!(
            value_to_json(&MysqlValue::Bytes(vec![0xFF]), ValueKind::Binary),
            value_to_json(&MysqlValue::Bytes(vec![0xFF]), ValueKind::Other)
        );
    }

    #[test]
    fn a_number_in_the_text_protocol_becomes_a_json_number() {
        let text = |value: &str, kind| value_to_json(&MysqlValue::Bytes(value.into()), kind);
        assert_eq!(text("-4", ValueKind::Integer), serde_json::json!(-4));
        assert_eq!(
            text("18446744073709551615", ValueKind::Integer),
            serde_json::json!(u64::MAX)
        );
        assert_eq!(text("0.1", ValueKind::Float), serde_json::json!(0.1));
        assert_eq!(text("1e20", ValueKind::Float), serde_json::json!(1e20));
        // A DECIMAL keeps its text, so no digit is lost.
        assert_eq!(text("1.50", ValueKind::Other), serde_json::json!("1.50"));
        // A date in the text protocol is already text.
        assert_eq!(
            text("2026-08-10", ValueKind::DateOnly),
            serde_json::json!("2026-08-10")
        );
        // Text that does not parse stays text.
        assert_eq!(text("x", ValueKind::Integer), serde_json::json!("x"));
        assert_eq!(text("x", ValueKind::Float), serde_json::json!("x"));
    }

    #[test]
    fn an_interval_folds_the_days_into_the_hours() {
        assert_eq!(
            value_to_json(&MysqlValue::Time(false, 1, 2, 3, 4, 0), ValueKind::Other),
            serde_json::json!("26:03:04")
        );
        assert_eq!(
            value_to_json(&MysqlValue::Time(true, 0, 2, 3, 4, 500), ValueKind::Other),
            serde_json::json!("-02:03:04.000500")
        );
    }

    #[test]
    fn a_row_of_values_gives_one_json_value_for_each_column() {
        let values = [
            MysqlValue::Int(1),
            MysqlValue::Bytes(b"a".to_vec()),
            MysqlValue::NULL,
        ];
        let json: Vec<JsonValue> = values
            .iter()
            .map(|value| value_to_json(value, ValueKind::Other))
            .collect();
        assert_eq!(
            json,
            vec![
                serde_json::json!(1),
                serde_json::json!("a"),
                JsonValue::Null
            ]
        );
    }
}
