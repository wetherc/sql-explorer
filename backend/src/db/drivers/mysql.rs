//! The MySQL and MariaDB driver.

use crate::db::drivers::{
    add_constraint_column, add_index_column, add_snapshot_column, bytes_to_json, connect_within,
    constraint_type, f32_to_json, f64_to_json, finish_set, next_values, non_empty,
    number_out_of_range, number_value, parameter_type_refused, prefixed_plan, relation_type,
    routine_type, rows_affected_message, size_text, system_roots, trigger_event, trigger_timing,
    CancelHandle, DatabaseDriver, NumberValue, KEEPALIVE_IDLE,
};
use crate::db::sink::{RowSink, RunSummary, SinkControl};
use crate::db::{
    AppColumn, ColumnInfo, Constraint, CreateQuery, Database, DriverCapabilities, ExecOptions,
    IndexInfo, Message, MessageLevel, ObjectType, PlanMode, QueryParams, QueryResponse,
    RelationType, Routine, ScheduledEvent, Schema, SchemaSnapshot, SnapshotColumn, Table,
    TableFact, Trigger,
};
use crate::error::{is_mysql_stop, Error, Result};
use crate::error::{offset_place, place_of_byte_offset};
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
    if let Some(user) = non_empty(&connection.user) {
        builder = builder.user(Some(user.to_string()));
    }
    if let Some(password) = connection.password.as_deref().filter(|v| !v.is_empty()) {
        builder = builder.pass(Some(password.to_string()));
    }
    if let Some(database) = non_empty(&connection.database) {
        builder = builder.db_name(Some(database.to_string()));
    }
    builder = builder
        .ssl_opts(ssl_opts(connection))
        .tcp_keepalive(Some(KEEPALIVE_IDLE));

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
        if let Some(user) = non_empty(&connection.user) {
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
    // The driver can set the idle time alone, so the operating system
    // decides the time between two probes.
    if opts.tcp_keepalive().is_none() {
        builder = builder.tcp_keepalive(Some(KEEPALIVE_IDLE));
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
        let (conn, opts) = connect_within(limit.as_secs(), open_login(connection)).await??;
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
            "The server requested the '{name}' authentication plugin, which this client doesn't \
             support. Switch the user on the server to 'caching_sha2_password' or \
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
    if options.one_statement
        || (values.is_some() && crate::sql::has_placeholder(statement, Dialect::MySql))
    {
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

        let formats: Vec<ValueFormat> = wire
            .iter()
            .map(|column| value_format(column.column_type(), column.character_set()))
            .collect();
        sink.begin_set(columns.clone())?;
        let mut count = 0usize;
        let mut truncated = false;
        while let Some(row) = result.next().await? {
            if count >= options.max_rows {
                truncated = true;
                break;
            }
            if sink.row(row_to_json(&row, &formats))? == SinkControl::Stop {
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
        finish_set(sink, count, truncated)?;

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
            "Couldn't read the statement's warnings: {error}"
        ))),
    }
}

/// The message for one row of `SHOW WARNINGS`. The level of the row sets the
/// level of the message, and the detail gives the level and the code as the
/// server sent them.
fn warning_message(level: &str, code: u32, text: String) -> Message {
    let message_level = match level {
        "Note" => MessageLevel::Info,
        "Error" => MessageLevel::Error,
        _ => MessageLevel::Warning,
    };
    Message {
        level: message_level,
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
            supports_materialized_views: false,
            supports_foreign_tables: false,
            supports_synonyms: false,
            supports_triggers: true,
            supports_view_triggers: false,
            supports_events: true,
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
        relation_type: RelationType,
    ) -> Option<CreateQuery> {
        Some(create_query_text(database, table, relation_type))
    }

    fn object_create_query(
        &self,
        database: Option<&str>,
        _schema: Option<&str>,
        _parent: Option<&str>,
        name: &str,
        object_type: ObjectType,
    ) -> Option<CreateQuery> {
        Some(object_query_text(database, name, object_type))
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
        // The end of the last statement found in the text, so that a
        // statement that stands twice is found at its own place.
        let mut cursor = 0;
        for statement in split_statements(query, Dialect::MySql) {
            if stopped {
                break;
            }
            let start = query[cursor..]
                .find(statement.as_str())
                .map(|at| cursor + at);
            if let Some(at) = start {
                cursor = at + statement.len();
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
            .await
            .map_err(|error| locate_error(error, query, start))?;
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
        mode: PlanMode,
        options: &ExecOptions,
    ) -> Result<QueryResponse> {
        let statement = prefixed_plan(query, Dialect::MySql, plan_prefix(mode))?;
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
            .map(|(name, word)| {
                if relation_type(&word).is_view() {
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
        let rows: Vec<(String, String, String, String, String)> = self
            .conn()?
            .exec(
                "SELECT COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_KEY, EXTRA \
                 FROM information_schema.COLUMNS \
                 WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? \
                 ORDER BY ORDINAL_POSITION",
                (database, table),
            )
            .await?;
        Ok(rows
            .into_iter()
            .map(|(name, data_type, nullable, key, extra)| AppColumn {
                name,
                data_type,
                nullable: nullable.eq_ignore_ascii_case("YES"),
                is_primary_key: key.eq_ignore_ascii_case("PRI"),
                is_generated: generated_extra(&extra),
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
            facts.push(TableFact::new("Last modified", changed));
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
        for (relation, word, name, data_type) in rows {
            if !add_snapshot_column(
                &mut snapshot,
                max_columns,
                None,
                relation,
                relation_type(&word),
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
            .map(|(name, word)| Routine {
                name,
                routine_type: routine_type(&word),
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
        for (name, word, column, target, target_column) in rows {
            let detail = target.map(|target| match target_column {
                Some(column) => format!("{target}({column})"),
                None => target,
            });
            add_constraint_column(
                &mut constraints,
                name,
                constraint_type(&word),
                column,
                detail,
            );
        }
        Ok(constraints)
    }

    /// Reads the triggers from `TRIGGERS`. A trigger of MySQL fires on one
    /// event alone, and the server has no switch that disables it.
    async fn list_triggers(
        &mut self,
        database: &str,
        _schema: Option<&str>,
        table: &str,
    ) -> Result<Vec<Trigger>> {
        let rows: Vec<(String, String, String)> =
            self.conn()?.exec(TRIGGERS_QUERY, (database, table)).await?;
        Ok(rows
            .into_iter()
            .map(|(name, timing, event)| trigger_of(name, &timing, &event))
            .collect())
    }

    /// Reads the scheduled events from `EVENTS`, with the parts of each
    /// schedule.
    async fn list_events(
        &mut self,
        database: &str,
        _schema: Option<&str>,
    ) -> Result<Vec<ScheduledEvent>> {
        let rows: Vec<EventRow> = self.conn()?.exec(EVENTS_QUERY, (database,)).await?;
        Ok(rows.into_iter().map(event_of).collect())
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
pub fn plan_prefix(mode: PlanMode) -> &'static str {
    match mode {
        PlanMode::Estimated => "EXPLAIN",
        PlanMode::Actual => "EXPLAIN ANALYZE",
    }
}

/// Builds the statement that reads the CREATE text of one object. MySQL and
/// MariaDB answer `SHOW CREATE` with the name in the first column and the
/// text in the second one.
fn create_query_text(
    database: Option<&str>,
    table: &str,
    relation_type: RelationType,
) -> CreateQuery {
    let name = Dialect::MySql.qualified_name(database, None, table);
    let word = if relation_type.is_view() {
        "VIEW"
    } else {
        "TABLE"
    };
    CreateQuery::new(format!("SHOW CREATE {word} {name};"), 1)
}

/// Builds the statement that reads the CREATE text of one trigger or one
/// event. `SHOW CREATE TRIGGER` gives the text in its third column, and
/// `SHOW CREATE EVENT` gives it in its fourth one. A body of more than one
/// statement goes between `DELIMITER` commands, so the splitter of the app
/// sends the text to the server whole.
///
/// `SHOW CREATE TRIGGER` gives no `FOLLOWS` or `PRECEDES` clause on MySQL
/// 8.4 or on MariaDB 11.4. A trigger made again from its text goes last
/// among the triggers with its timing and its event, so the firing order
/// can change.
fn object_query_text(database: Option<&str>, name: &str, object_type: ObjectType) -> CreateQuery {
    let name = Dialect::MySql.qualified_name(database, None, name);
    let query = match object_type {
        ObjectType::Trigger => CreateQuery::new(format!("SHOW CREATE TRIGGER {name};"), 2),
        ObjectType::Event => CreateQuery::new(format!("SHOW CREATE EVENT {name};"), 3),
    };
    query.with_delimiter()
}

/// Lists the triggers of one table in the order that they fire. The
/// triggers with the same timing and the same event fire in the order of
/// `ACTION_ORDER`, which `FOLLOWS` and `PRECEDES` set. `FIELD` puts `BEFORE`
/// in front of `AFTER`, and the events in the order insert, update, delete.
const TRIGGERS_QUERY: &str = "SELECT TRIGGER_NAME, ACTION_TIMING, EVENT_MANIPULATION \
     FROM information_schema.TRIGGERS \
     WHERE EVENT_OBJECT_SCHEMA = ? AND EVENT_OBJECT_TABLE = ? \
     ORDER BY FIELD(ACTION_TIMING, 'BEFORE', 'AFTER'), \
              FIELD(EVENT_MANIPULATION, 'INSERT', 'UPDATE', 'DELETE'), ACTION_ORDER";

/// Builds the record of one trigger from the words of `TRIGGERS`.
fn trigger_of(name: String, timing: &str, event: &str) -> Trigger {
    Trigger {
        name,
        timing: trigger_timing(timing),
        events: trigger_event(event).into_iter().collect(),
        enabled: true,
        replica: false,
        update_columns: Vec::new(),
    }
}

/// Lists the scheduled events of one database. The time of a single run
/// goes out as text, so the row reads every column as text.
const EVENTS_QUERY: &str = "SELECT EVENT_NAME, STATUS, EVENT_TYPE, \
            CAST(EXECUTE_AT AS CHAR), INTERVAL_VALUE, INTERVAL_FIELD \
     FROM information_schema.EVENTS \
     WHERE EVENT_SCHEMA = ? ORDER BY EVENT_NAME";

/// One row of [`EVENTS_QUERY`]: the name, the status, the type of the
/// schedule, the time of a single run, and the value and the unit of a
/// repeat.
type EventRow = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// Builds the record of one event. The status `ENABLED` marks an event that
/// runs. `DISABLED` and `SLAVESIDE_DISABLED`, which marks an event that a
/// replica copied from its source, mark an event that does not run. A
/// recurring event gives `EVERY` with its interval, and a single run gives
/// `AT` with its time.
fn event_of(row: EventRow) -> ScheduledEvent {
    let (name, status, schedule_type, at, value, unit) = row;
    let schedule = if schedule_type.eq_ignore_ascii_case("RECURRING") {
        value.map(|value| match unit {
            Some(unit) => format!("EVERY {value} {unit}"),
            None => format!("EVERY {value}"),
        })
    } else {
        at.map(|at| format!("AT {at}"))
    };
    ScheduledEvent {
        name,
        enabled: status.eq_ignore_ascii_case("ENABLED"),
        schedule,
    }
}

/// The format of the values of a column, as far as the conversion to JSON
/// needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueFormat {
    /// A DATE column. The driver gives the same value for DATE, DATETIME
    /// and TIMESTAMP, so the format decides whether the text has a time.
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

/// Finds the format of the values from the wire type and the character set
/// of a column.
pub fn value_format(column_type: ColumnType, charset: u16) -> ValueFormat {
    use ColumnType::*;
    match column_type {
        MYSQL_TYPE_DATE | MYSQL_TYPE_NEWDATE => ValueFormat::DateOnly,
        MYSQL_TYPE_TINY | MYSQL_TYPE_SHORT | MYSQL_TYPE_INT24 | MYSQL_TYPE_LONG
        | MYSQL_TYPE_LONGLONG | MYSQL_TYPE_YEAR => ValueFormat::Integer,
        MYSQL_TYPE_FLOAT | MYSQL_TYPE_DOUBLE => ValueFormat::Float,
        MYSQL_TYPE_BIT => ValueFormat::Bit,
        // A GEOMETRY value is the SRID and the WKB bytes of the shape, which
        // the server sends with no character set of text.
        MYSQL_TYPE_GEOMETRY => ValueFormat::Binary,
        MYSQL_TYPE_STRING
        | MYSQL_TYPE_VAR_STRING
        | MYSQL_TYPE_VARCHAR
        | MYSQL_TYPE_BLOB
        | MYSQL_TYPE_TINY_BLOB
        | MYSQL_TYPE_MEDIUM_BLOB
        | MYSQL_TYPE_LONG_BLOB
            if charset == BINARY_CHARSET =>
        {
            ValueFormat::Binary
        }
        _ => ValueFormat::Other,
    }
}

/// Reads the line from the `... at line N` that ends a syntax error of the
/// server. The server counts the lines of the statement from 1.
fn error_line(message: &str) -> Option<u32> {
    let at = message.rfind("at line ")?;
    let digits: String = message[at + "at line ".len()..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// Marks a server error that names a line of the statement with its place
/// in the whole text. `start` is the byte offset of the statement in `query`.
fn locate_error(error: Error, query: &str, start: Option<usize>) -> Error {
    let line = match (&error, start) {
        (Error::MySql(mysql_async::Error::Server(server)), Some(_)) => error_line(&server.message),
        _ => None,
    };
    match (line, start) {
        (Some(line), Some(start)) => {
            let (line, column) = offset_place(place_of_byte_offset(query, start), (line.max(1), 1));
            error.at(line, column)
        }
        _ => error,
    }
}

/// True when the `EXTRA` text of a column marks a value that the server
/// gives: a generated column (`VIRTUAL GENERATED`, `STORED GENERATED`, or
/// `PERSISTENT GENERATED` on MariaDB) or an `auto_increment` column. The word
/// `DEFAULT_GENERATED` marks a default expression, and such a column takes a
/// value from an INSERT.
fn generated_extra(extra: &str) -> bool {
    extra.split_whitespace().any(|word| {
        word.eq_ignore_ascii_case("generated") || word.eq_ignore_ascii_case("auto_increment")
    })
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
/// `formats` holds one format for each column, as `value_format` reads it.
pub fn row_to_json(row: &MysqlRow, formats: &[ValueFormat]) -> Vec<JsonValue> {
    formats
        .iter()
        .enumerate()
        .map(|(index, format)| {
            row.as_ref(index)
                .map_or(JsonValue::Null, |value| value_to_json(value, *format))
        })
        .collect()
}

/// Converts a number that the text protocol sends as text. Text that does
/// not parse stays text.
fn number_text_to_json(text: &str, format: ValueFormat) -> Option<JsonValue> {
    match format {
        ValueFormat::Integer => text
            .parse::<i64>()
            .map(JsonValue::from)
            .or_else(|_| text.parse::<u64>().map(JsonValue::from))
            .ok(),
        ValueFormat::Float => text.parse::<f64>().ok().map(f64_to_json),
        ValueFormat::DateOnly | ValueFormat::Bit | ValueFormat::Binary | ValueFormat::Other => None,
    }
}

/// Converts one value of the driver into JSON. `format` is the format of
/// the values of the column.
pub fn value_to_json(value: &MysqlValue, format: ValueFormat) -> JsonValue {
    match value {
        MysqlValue::NULL => JsonValue::Null,
        MysqlValue::Int(number) => JsonValue::from(*number),
        MysqlValue::UInt(number) => JsonValue::from(*number),
        MysqlValue::Float(number) => f32_to_json(*number),
        MysqlValue::Double(number) => f64_to_json(*number),
        // A BIT value is a whole number of at most 64 bits, first byte
        // highest, so BIT(1) that holds 1 gives 1 and BIT(8) gives 65, not
        // "A".
        MysqlValue::Bytes(bytes) if format == ValueFormat::Bit && bytes.len() <= 8 => {
            JsonValue::from(
                bytes
                    .iter()
                    .fold(0u64, |value, byte| (value << 8) | u64::from(*byte)),
            )
        }
        MysqlValue::Bytes(bytes) if format == ValueFormat::Binary => bytes_to_json(bytes),
        // The server sends text and decimals as bytes. Text that is not
        // valid UTF-8 becomes base64.
        MysqlValue::Bytes(bytes) => match std::str::from_utf8(bytes) {
            Ok(text) => {
                number_text_to_json(text, format).unwrap_or_else(|| JsonValue::String(text.into()))
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
                format == ValueFormat::DateOnly,
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
mod live;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_syntax_error_names_its_line_in_the_whole_text() {
        let query = "SELECT 1;\n\nSELECT\n  FROM";
        let start = query.find("SELECT\n").unwrap();
        let error = Error::MySql(server_error_with(
            1064,
            "You have an error in your SQL syntax; check the manual near 'FROM' at line 2",
        ));
        let located = locate_error(error, query, Some(start));
        assert!(matches!(
            located,
            Error::Located {
                line: 4,
                column: 1,
                ..
            }
        ));
        // The first line of the statement keeps the column of its start.
        let error = Error::MySql(server_error_with(1064, "near '' at line 1"));
        let located = locate_error(error, "SELECT 1; SELEC", Some(10));
        assert!(matches!(
            located,
            Error::Located {
                line: 1,
                column: 11,
                ..
            }
        ));
        // A message without a line, and a statement not found, stay as they are.
        let plain = locate_error(
            Error::MySql(server_error_with(1146, "no table")),
            query,
            Some(0),
        );
        assert!(matches!(plain, Error::MySql(_)));
        let lost = locate_error(
            Error::MySql(server_error_with(1064, "at line 2")),
            query,
            None,
        );
        assert!(matches!(lost, Error::MySql(_)));
        assert_eq!(error_line("at line x"), None);
    }

    #[test]
    fn a_server_given_value_counts_as_generated() {
        for extra in [
            "VIRTUAL GENERATED",
            "STORED GENERATED",
            "PERSISTENT GENERATED",
            "auto_increment",
        ] {
            assert!(generated_extra(extra), "{extra}");
        }
        for extra in ["", "DEFAULT_GENERATED", "on update CURRENT_TIMESTAMP"] {
            assert!(!generated_extra(extra), "{extra}");
        }
    }

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
        assert_eq!(error.category(), crate::error::ErrorCategory::Timeout);
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

    fn server_error_with(code: u16, message: &str) -> mysql_async::Error {
        mysql_async::Error::Server(mysql_async::ServerError {
            code,
            message: message.into(),
            state: "42000".into(),
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
            value_format(ColumnType::MYSQL_TYPE_BIT, BINARY_CHARSET),
            ValueFormat::Bit
        );
        let bit =
            |bytes: &[u8]| value_to_json(&MysqlValue::Bytes(bytes.to_vec()), ValueFormat::Bit);
        assert_eq!(bit(&[1]), json!(1));
        assert_eq!(bit(&[0x41]), json!(65));
        assert_eq!(bit(&[0x01, 0x00]), json!(256));
        assert_eq!(bit(&[0xFF; 8]), json!(u64::MAX));
        // Another column that holds the same bytes keeps its text.
        assert_eq!(
            value_to_json(&MysqlValue::Bytes(vec![0x41]), ValueFormat::Other),
            json!("A")
        );
    }

    #[test]
    fn the_analysed_plan_runs_the_statement() {
        assert_eq!(plan_prefix(PlanMode::Estimated), "EXPLAIN");
        assert_eq!(plan_prefix(PlanMode::Actual), "EXPLAIN ANALYZE");
    }

    #[test]
    fn the_create_statement_names_the_type_of_the_object() {
        let table = create_query_text(Some("db"), "t", RelationType::Table);
        assert_eq!(table.sql, "SHOW CREATE TABLE `db`.`t`;");
        assert_eq!(table.column, 1);

        let view = create_query_text(None, "v", RelationType::View);
        assert_eq!(view.sql, "SHOW CREATE VIEW `v`;");
    }

    #[test]
    fn the_create_statement_of_a_trigger_and_an_event_names_its_column() {
        let trigger = object_query_text(Some("db"), "audit", ObjectType::Trigger);
        assert_eq!(trigger.sql, "SHOW CREATE TRIGGER `db`.`audit`;");
        assert_eq!(trigger.column, 2);
        let event = object_query_text(None, "nightly", ObjectType::Event);
        assert_eq!(event.sql, "SHOW CREATE EVENT `nightly`;");
        assert_eq!(event.column, 3);
        assert!(trigger.delimited);
        assert!(event.delimited);
    }

    #[test]
    fn a_trigger_of_mysql_has_one_event_and_is_always_enabled() {
        let trigger = trigger_of("audit".into(), "BEFORE", "UPDATE");
        assert_eq!(trigger.name, "audit");
        assert_eq!(trigger.timing, crate::db::TriggerTiming::Before);
        assert_eq!(trigger.events, vec![crate::db::TriggerEvent::Update]);
        assert!(trigger.enabled);
        let after = trigger_of("log".into(), "AFTER", "INSERT");
        assert_eq!(after.timing, crate::db::TriggerTiming::After);
        assert!(trigger_of("x".into(), "AFTER", "OTHER").events.is_empty());
    }

    #[test]
    fn the_triggers_of_a_table_come_in_the_order_that_they_fire() {
        assert!(TRIGGERS_QUERY.contains("WHERE EVENT_OBJECT_SCHEMA = ? AND EVENT_OBJECT_TABLE = ?"));
        assert!(TRIGGERS_QUERY.ends_with(
            "ORDER BY FIELD(ACTION_TIMING, 'BEFORE', 'AFTER'), \
             FIELD(EVENT_MANIPULATION, 'INSERT', 'UPDATE', 'DELETE'), ACTION_ORDER"
        ));
    }

    #[test]
    fn an_event_gives_its_status_and_its_schedule() {
        let text = |value: &str| Some(value.to_string());
        let recurring = event_of((
            "nightly".into(),
            "ENABLED".into(),
            "RECURRING".into(),
            None,
            text("1"),
            text("DAY"),
        ));
        assert_eq!(recurring.name, "nightly");
        assert!(recurring.enabled);
        assert_eq!(recurring.schedule.as_deref(), Some("EVERY 1 DAY"));

        let once = event_of((
            "once".into(),
            "DISABLED".into(),
            "ONE TIME".into(),
            text("2026-01-01 00:00:00"),
            None,
            None,
        ));
        assert!(!once.enabled);
        assert_eq!(once.schedule.as_deref(), Some("AT 2026-01-01 00:00:00"));

        let copied = event_of((
            "copied".into(),
            "SLAVESIDE_DISABLED".into(),
            "RECURRING".into(),
            None,
            text("5"),
            None,
        ));
        assert!(!copied.enabled);
        assert_eq!(copied.schedule.as_deref(), Some("EVERY 5"));

        let bare = event_of((
            "bare".into(),
            "ENABLED".into(),
            "RECURRING".into(),
            None,
            None,
            None,
        ));
        assert_eq!(bare.schedule, None);
        assert!(EVENTS_QUERY.contains("WHERE EVENT_SCHEMA = ?"));
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

    #[tokio::test]
    async fn a_server_that_never_answers_gives_a_connection_error() {
        // The server takes the socket and never sends its greeting.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
            drop(socket);
        });
        let mut input = connection();
        input.host = Some("127.0.0.1".into());
        input.port = Some(port);
        input.options.connect_timeout_secs = 1;
        let Err(error) = MysqlDriver::connect(&input).await else {
            panic!("the connection opened");
        };
        assert!(matches!(error, Error::Connection(ref text) if text.contains("within 1 seconds")));
        server.abort();
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
    fn a_connection_probes_its_socket_after_a_minute_of_silence() {
        let mut input = connection();
        assert_eq!(
            build_opts(&input).unwrap().tcp_keepalive(),
            Some(KEEPALIVE_IDLE)
        );

        // A connection string with no keepalive value takes the one of the
        // application.
        input.options.connection_url = Some("mysql://u@h/d".into());
        assert_eq!(
            build_opts(&input).unwrap().tcp_keepalive(),
            Some(KEEPALIVE_IDLE)
        );

        // The value of the string wins.
        input.options.connection_url = Some("mysql://u@h/d?tcp_keepalive=300000".into());
        assert_eq!(
            build_opts(&input).unwrap().tcp_keepalive(),
            Some(Duration::from_secs(300))
        );
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
        assert_eq!(error.category(), crate::error::ErrorCategory::Connection);
        assert!(error.to_string().contains("sha256_password"));
        assert!(error.to_string().contains("caching_sha2_password"));
    }

    #[test]
    fn another_connection_error_keeps_its_own_text() {
        let error = describe_connect_error(mysql_async::Error::Other("boom".into()));
        assert_eq!(error.category(), crate::error::ErrorCategory::Database);
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
            bind_params(Some(&params)).unwrap_err().category(),
            crate::error::ErrorCategory::Configuration
        );
    }

    #[test]
    fn a_whole_number_outside_the_range_is_refused() {
        let params = vec![crate::db::QueryParam {
            value: serde_json::json!(18446744073709551615u64),
        }];
        assert_eq!(
            bind_params(Some(&params)).unwrap_err().category(),
            crate::error::ErrorCategory::Configuration
        );
    }

    #[test]
    fn every_value_type_becomes_json() {
        assert_eq!(
            value_to_json(&MysqlValue::NULL, ValueFormat::Other),
            JsonValue::Null
        );
        assert_eq!(
            value_to_json(&MysqlValue::Int(-4), ValueFormat::Other),
            serde_json::json!(-4)
        );
        assert_eq!(
            value_to_json(&MysqlValue::UInt(4), ValueFormat::Other),
            serde_json::json!(4)
        );
        assert_eq!(
            value_to_json(&MysqlValue::Double(1.25), ValueFormat::Other),
            serde_json::json!(1.25)
        );
        assert_eq!(
            value_to_json(&MysqlValue::Float(0.5), ValueFormat::Other),
            serde_json::json!(0.5)
        );
        assert_eq!(
            value_to_json(&MysqlValue::Float(0.1), ValueFormat::Other),
            serde_json::json!(0.1)
        );
        assert_eq!(
            value_to_json(&MysqlValue::Bytes(b"hello".to_vec()), ValueFormat::Other),
            serde_json::json!("hello")
        );
        // Bytes that are not valid text become base64.
        assert_eq!(
            value_to_json(&MysqlValue::Bytes(vec![0xff, 0xfe]), ValueFormat::Other),
            serde_json::json!("//4=")
        );
    }

    #[test]
    fn a_date_shows_only_the_parts_that_carry_information() {
        // A DATE column gives the date alone.
        assert_eq!(
            value_to_json(
                &MysqlValue::Date(2026, 8, 10, 0, 0, 0, 0),
                ValueFormat::DateOnly
            ),
            serde_json::json!("2026-08-10")
        );
        // A DATETIME column keeps the time, also at midnight.
        assert_eq!(
            value_to_json(
                &MysqlValue::Date(2026, 8, 10, 0, 0, 0, 0),
                ValueFormat::Other
            ),
            serde_json::json!("2026-08-10 00:00:00")
        );
        assert_eq!(
            value_to_json(
                &MysqlValue::Date(2026, 8, 10, 13, 5, 6, 0),
                ValueFormat::Other
            ),
            serde_json::json!("2026-08-10 13:05:06")
        );
        assert_eq!(
            value_to_json(
                &MysqlValue::Date(2026, 8, 10, 13, 5, 6, 123456),
                ValueFormat::Other
            ),
            serde_json::json!("2026-08-10 13:05:06.123456")
        );
    }

    #[test]
    fn the_type_of_a_column_gives_the_format_of_its_values() {
        use ColumnType::*;
        let text = 255;
        assert_eq!(
            value_format(MYSQL_TYPE_DATE, BINARY_CHARSET),
            ValueFormat::DateOnly
        );
        assert_eq!(
            value_format(MYSQL_TYPE_NEWDATE, BINARY_CHARSET),
            ValueFormat::DateOnly
        );
        assert_eq!(
            value_format(MYSQL_TYPE_DATETIME, BINARY_CHARSET),
            ValueFormat::Other
        );
        assert_eq!(
            value_format(MYSQL_TYPE_TIMESTAMP, BINARY_CHARSET),
            ValueFormat::Other
        );
        for column_type in [
            MYSQL_TYPE_TINY,
            MYSQL_TYPE_SHORT,
            MYSQL_TYPE_INT24,
            MYSQL_TYPE_LONG,
            MYSQL_TYPE_LONGLONG,
            MYSQL_TYPE_YEAR,
        ] {
            assert_eq!(
                value_format(column_type, BINARY_CHARSET),
                ValueFormat::Integer
            );
        }
        assert_eq!(
            value_format(MYSQL_TYPE_FLOAT, BINARY_CHARSET),
            ValueFormat::Float
        );
        assert_eq!(
            value_format(MYSQL_TYPE_DOUBLE, BINARY_CHARSET),
            ValueFormat::Float
        );
        assert_eq!(
            value_format(MYSQL_TYPE_NEWDECIMAL, BINARY_CHARSET),
            ValueFormat::Other
        );
        for column_type in [
            MYSQL_TYPE_STRING,
            MYSQL_TYPE_VAR_STRING,
            MYSQL_TYPE_VARCHAR,
            MYSQL_TYPE_BLOB,
            MYSQL_TYPE_TINY_BLOB,
            MYSQL_TYPE_MEDIUM_BLOB,
            MYSQL_TYPE_LONG_BLOB,
        ] {
            assert_eq!(
                value_format(column_type, BINARY_CHARSET),
                ValueFormat::Binary
            );
            assert_eq!(value_format(column_type, text), ValueFormat::Other);
        }
        // A GEOMETRY value is bytes whatever the character set says.
        assert_eq!(value_format(MYSQL_TYPE_GEOMETRY, text), ValueFormat::Binary);
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
            value_to_json(&MysqlValue::Bytes(b"AB".to_vec()), ValueFormat::Binary),
            serde_json::json!("QUI=")
        );
        assert_eq!(
            value_to_json(&MysqlValue::Bytes(vec![0xFF]), ValueFormat::Binary),
            value_to_json(&MysqlValue::Bytes(vec![0xFF]), ValueFormat::Other)
        );
    }

    #[test]
    fn a_number_in_the_text_protocol_becomes_a_json_number() {
        let text = |value: &str, format| value_to_json(&MysqlValue::Bytes(value.into()), format);
        assert_eq!(text("-4", ValueFormat::Integer), serde_json::json!(-4));
        assert_eq!(
            text("18446744073709551615", ValueFormat::Integer),
            serde_json::json!(u64::MAX)
        );
        assert_eq!(text("0.1", ValueFormat::Float), serde_json::json!(0.1));
        assert_eq!(text("1e20", ValueFormat::Float), serde_json::json!(1e20));
        // A DECIMAL keeps its text, so no digit is lost.
        assert_eq!(text("1.50", ValueFormat::Other), serde_json::json!("1.50"));
        // A date in the text protocol is already text.
        assert_eq!(
            text("2026-08-10", ValueFormat::DateOnly),
            serde_json::json!("2026-08-10")
        );
        // Text that does not parse stays text.
        assert_eq!(text("x", ValueFormat::Integer), serde_json::json!("x"));
        assert_eq!(text("x", ValueFormat::Float), serde_json::json!("x"));
    }

    #[test]
    fn an_interval_folds_the_days_into_the_hours() {
        assert_eq!(
            value_to_json(&MysqlValue::Time(false, 1, 2, 3, 4, 0), ValueFormat::Other),
            serde_json::json!("26:03:04")
        );
        assert_eq!(
            value_to_json(&MysqlValue::Time(true, 0, 2, 3, 4, 500), ValueFormat::Other),
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
            .map(|value| value_to_json(value, ValueFormat::Other))
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
