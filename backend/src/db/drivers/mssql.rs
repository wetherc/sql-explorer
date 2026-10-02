//! The MS SQL Server driver.
//!
//! The configuration is built with the builder of `tiberius` and not from a
//! connection string. A connection string loses the port, because the
//! parser reads the port only from inside the `server` value, and it also
//! loses any password that holds a semicolon or a brace.

use crate::db::drivers::{
    add_constraint_column, add_included_column, add_index_column, add_snapshot_column,
    bytes_to_json, constraint_kind, f32_to_json, f64_to_json, number_out_of_range, number_value,
    parameter_type_refused, routine_kind, rows_affected_message, rows_returned_message,
    single_statement, size_text, table_kind, CancelHandle, DatabaseDriver, NumberValue,
};
use crate::db::sink::{BufferSink, RowSink, RunSummary, SinkControl};
use crate::db::{
    AppColumn, ColumnInfo, Constraint, CreateQuery, Database, DriverCapabilities, ExecOptions,
    IndexInfo, Message, MessageLevel, PlanKind, QueryParams, QueryResponse, ResultSet, Routine,
    Schema, SchemaSnapshot, SnapshotColumn, Table, TableFact, TableKind,
};
use crate::error::{Error, Result};
use crate::sql::{only_reads, split_batches, split_statements, Dialect};
use crate::storage::{MssqlAuth, SavedConnection, TlsMode};
use async_trait::async_trait;
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime};
use futures_util::TryStreamExt;
use serde_json::Value as JsonValue;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tiberius::numeric::Numeric;
use tiberius::xml::XmlData;
use tiberius::{
    AttentionHandle, AuthMethod, Client, ColumnData, ColumnType, Config, EncryptionLevel, FromSql,
    QueryItem, Row,
};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

type MssqlClient = Client<Compat<TcpStream>>;

pub struct MssqlDriver {
    client: MssqlClient,
}

/// What one walk of a batch found.
struct Walk {
    /// True when the sink stopped the run.
    stopped: bool,
    /// The sum of the counts of changed rows, or `None` when the server sent
    /// no count outside a result set.
    rows_affected: Option<u64>,
}

/// Builds the `tiberius` configuration from a saved connection.
pub async fn build_config(connection: &SavedConnection) -> Result<Config> {
    let mut config = if let Some(url) = connection.options.connection_url.as_deref() {
        let mut config = parse_string(url)?;
        add_login_of_record(&mut config, connection);
        config
    } else {
        Config::new()
    };

    if connection.options.connection_url.is_none() {
        config.host(connection.effective_host());
        if let Some(port) = connection.effective_port() {
            config.port(port);
        }
        if let Some(instance) = non_empty(connection.options.instance_name.as_deref()) {
            config.instance_name(instance);
        }
        if let Some(database) = non_empty(connection.database.as_deref()) {
            config.database(database);
        }
        if let Some(name) = non_empty(connection.options.application_name.as_deref()) {
            config.application_name(name);
        }
        config.authentication(auth_method(connection).await?);
        config.encryption(encryption_level(connection.options.tls_mode));
        // `tiberius` panics when it gets both `trust_cert` and
        // `trust_cert_ca`, so a CA file applies only to a mode that verifies.
        if !connection.options.tls_mode.verifies_certificate() {
            config.trust_cert();
        } else if let Some(path) = non_empty(connection.options.ca_cert_path.as_deref()) {
            config.trust_cert_ca(path);
        }
    }
    // The flag sets `ApplicationIntent=ReadOnly`, which sends the login to a
    // readable secondary of an availability group. It does not stop a write
    // on a primary or on a standalone server. A connection string can set
    // the same intent, and a switch that is off leaves that value as it is.
    if connection.options.read_only {
        config.readonly(true);
    }

    Ok(config)
}

/// Reads an ADO.NET or a JDBC connection string.
fn parse_string(url: &str) -> Result<Config> {
    let trimmed = url.trim();
    Ok(if trimmed.starts_with("jdbc:") {
        Config::from_jdbc_string(trimmed)?
    } else {
        Config::from_ado_string(trimmed)?
    })
}

/// True when a connection string gives a password.
pub fn string_has_password(url: &str) -> Result<bool> {
    let config = parse_string(url)?;
    Ok(config
        .get_authentication()
        .password()
        .is_some_and(|password| !password.is_empty()))
}

/// Puts the password of the record into the SQL login of a connection
/// string. The keychain keeps the password, so the string does not give one.
/// A string that names no user takes the user of the record. A string that
/// gives its own password, or that names another method, stays as it is.
fn add_login_of_record(config: &mut Config, connection: &SavedConnection) {
    if connection.options.mssql_auth != MssqlAuth::SqlLogin {
        return;
    }
    let Some(password) = connection.password.as_deref().filter(|v| !v.is_empty()) else {
        return;
    };
    let auth = config.get_authentication();
    let own_password = auth.password().is_some_and(|given| !given.is_empty());
    if !matches!(auth, AuthMethod::SqlServer(_)) || own_password {
        return;
    }
    let user = match auth.user().filter(|user| !user.is_empty()) {
        Some(user) => user.to_string(),
        None => connection.user.clone().unwrap_or_default(),
    };
    config.authentication(AuthMethod::sql_server(user, password));
}

/// Returns the text when it holds something other than blank space.
fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|text| !text.is_empty())
}

/// The resource that a token for a SQL database names.
pub const DATABASE_RESOURCE: &str = "https://database.windows.net/";

/// The places a desktop application looks for the Azure CLI. An application
/// that a desktop starts holds a short `PATH` that often misses these.
const AZURE_CLI_PLACES: [&str; 4] = [
    "az",
    "/opt/homebrew/bin/az",
    "/usr/local/bin/az",
    "/usr/bin/az",
];

/// Reads the access token out of the JSON that the Azure CLI writes.
pub fn token_from_cli_output(output: &str) -> Result<String> {
    let value: serde_json::Value = serde_json::from_str(output).map_err(|error| {
        Error::Authentication(format!("The Azure CLI gave no readable answer: {error}"))
    })?;
    value
        .get("accessToken")
        .and_then(|token| token.as_str())
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .ok_or_else(|| Error::Authentication("The Azure CLI gave no access token.".to_string()))
}

/// Reads the moment a JWT access token stops being valid.
///
/// The `exp` claim sits in the middle part of the token, which is base64url
/// text without padding. No signature check is made. The client acts on a
/// date that it reads for itself, and the server stays the judge of the
/// token. A token that cannot be read gives `None`, so an answer of `None`
/// means "ask the server".
fn token_expiry(token: &str) -> Option<SystemTime> {
    use base64::Engine;
    let mut parts = token.split('.');
    let (_header, payload, _signature) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let seconds = claims.get("exp")?.as_u64()?;
    Some(UNIX_EPOCH + Duration::from_secs(seconds))
}

/// The time that a clock which runs early may gain. A token that expired
/// inside this span still goes to the server.
const TOKEN_CLOCK_ALLOWANCE: Duration = Duration::from_secs(60);

/// The words that name an access token which is too old.
const EXPIRED_TOKEN_MESSAGE: &str = "The access token has expired. Paste a new one, or use the \
                                     Azure CLI method, which reads a fresh token on each \
                                     connection.";

/// The words that report a statement which the row limit ended.
const ENDED_AT_THE_LIMIT_MESSAGE: &str =
    "The read reached the row limit, so the statement was ended on the server. Raise the row \
     limit in the settings to read further.";

/// The statement that tells whether an attention packet keeps the work of
/// the session. The packet rolls back the statement that it ends. When the
/// session is inside a transaction and `XACT_ABORT` is on (bit 16384 of
/// `@@OPTIONS`), the packet also rolls back the whole transaction.
const ATTENTION_KEEPS_WORK: &str =
    "SELECT CASE WHEN @@TRANCOUNT = 0 OR @@OPTIONS & 16384 = 0 THEN 1 ELSE 0 END";

/// True when the token names a moment that is more than the allowance in the
/// past. A token that cannot be read is not refused here.
fn token_has_expired(token: &str, now: SystemTime) -> bool {
    match token_expiry(token) {
        Some(expiry) => now
            .duration_since(expiry)
            .is_ok_and(|age| age > TOKEN_CLOCK_ALLOWANCE),
        None => false,
    }
}

/// Asks the Azure CLI for a token for the SQL database resource.
///
/// The token lives for about one hour. The reconnection path builds the
/// configuration again, so a connection that is opened again asks the CLI
/// for a new token.
async fn azure_cli_token(configured_path: Option<&str>) -> Result<String> {
    let places: Vec<String> = match non_empty(configured_path) {
        Some(path) => vec![path.to_string()],
        None => AZURE_CLI_PLACES
            .iter()
            .map(|path| path.to_string())
            .collect(),
    };

    let mut last: Option<String> = None;
    for path in &places {
        let outcome = tokio::process::Command::new(path)
            .arg("account")
            .arg("get-access-token")
            .arg("--resource")
            .arg(DATABASE_RESOURCE)
            .arg("--output")
            .arg("json")
            .output()
            .await;

        match outcome {
            Ok(output) if output.status.success() => {
                let text = String::from_utf8_lossy(&output.stdout);
                return token_from_cli_output(&text);
            }
            Ok(output) => {
                // The CLI ran and refused. A further place would give the
                // same answer, so the reason is reported at once.
                let reason = String::from_utf8_lossy(&output.stderr).trim().to_string();
                return Err(Error::Authentication(format!(
                    "The Azure CLI could not give a token. Run `az login` first. {reason}"
                )));
            }
            Err(error) => last = Some(error.to_string()),
        }
    }

    Err(Error::Authentication(format!(
        "The Azure CLI was not found. Give its path in the connection. {}",
        last.unwrap_or_default()
    )))
}

/// Selects the authentication method. Windows Integrated Security needs the
/// `winauth` feature, which builds on Windows only.
async fn auth_method(connection: &SavedConnection) -> Result<AuthMethod> {
    match connection.options.mssql_auth {
        // Windows uses SSPI. Every other system uses Kerberos through
        // GSSAPI, which reads the ticket of the user from the credential
        // cache that `kinit` fills.
        MssqlAuth::Integrated => Ok(AuthMethod::Integrated),
        MssqlAuth::EntraAzureCli => {
            let token = azure_cli_token(connection.options.azure_cli_path.as_deref()).await?;
            Ok(AuthMethod::aad_token(token))
        }
        MssqlAuth::EntraAccessToken => {
            // The token is a credential, so it travels in the field that the
            // secret store holds and never reaches the settings file.
            let token = non_empty(connection.password.as_deref()).ok_or_else(|| {
                Error::Authentication(
                    "This connection needs an access token. Paste one, or read one from the Azure CLI."
                        .to_string(),
                )
            })?;
            // A token lives for about one hour, and the reconnection path
            // builds the configuration again with the stored token. The date
            // in the token is read here, so that an old token is named as one
            // before a socket opens.
            if token_has_expired(token, SystemTime::now()) {
                return Err(Error::Authentication(EXPIRED_TOKEN_MESSAGE.to_string()));
            }
            Ok(AuthMethod::aad_token(token))
        }
        MssqlAuth::SqlLogin => Ok(AuthMethod::sql_server(
            connection.user.as_deref().unwrap_or_default(),
            connection.password.as_deref().unwrap_or_default(),
        )),
    }
}

/// Maps the transport setting of the application onto the encryption level
/// of `tiberius`.
pub fn encryption_level(mode: TlsMode) -> EncryptionLevel {
    match mode {
        // `NotSupported` tells the server that this client has no TLS.
        TlsMode::Disable => EncryptionLevel::NotSupported,
        // `Off` encrypts the login packet only.
        TlsMode::Prefer => EncryptionLevel::Off,
        TlsMode::Require | TlsMode::VerifyFull => EncryptionLevel::Required,
    }
}

impl MssqlDriver {
    pub async fn connect(connection: &SavedConnection) -> Result<Box<dyn DatabaseDriver>> {
        let config = build_config(connection).await?;
        let limit = Duration::from_secs(connection.options.connect_timeout_secs.max(1));

        // A named instance listens on a port that the SQL Browser service
        // gives out, so the port in the configuration does not apply.
        let uses_instance = non_empty(connection.options.instance_name.as_deref()).is_some()
            || connection
                .options
                .connection_url
                .as_deref()
                .map(|url| url.contains('\\'))
                .unwrap_or(false);

        let tcp = if uses_instance {
            use tiberius::SqlBrowser;
            while_connecting(limit, TcpStream::connect_named(&config)).await??
        } else {
            while_connecting(limit, TcpStream::connect(config.get_addr())).await??
        };

        if let Err(error) = tcp.set_nodelay(true) {
            log::warn!("Could not disable the Nagle algorithm: {error}");
        }

        let client = while_connecting(limit, Client::connect(config, tcp.compat_write()))
            .await?
            .map_err(|error| describe_login(error, connection.options.mssql_auth))?;
        Ok(Box::new(MssqlDriver { client }))
    }

    /// Runs one batch through the path that keeps rows, and feeds each
    /// result set to the sink as it arrives.
    ///
    /// A stream that is dropped in the middle leaves the connection in the
    /// middle of a message, so the stream is always walked to its end. With
    /// `may_end_early`, the attention packet goes to the server once the read
    /// reaches the row limit or the sink stops. The server ends the
    /// statement, the stream ends with the cancel error of `tiberius`, and
    /// the walk then covers the rows in flight alone. The next statement of
    /// the session waits for the acknowledgement of the attention packet
    /// before it starts.
    ///
    /// The packet ends the whole batch and rolls back the statement that it
    /// ends, so a batch that [`Self::may_end_early`] refuses arrives with
    /// `may_end_early` false and keeps the walk.
    ///
    /// The `DONE` token of each statement ends its result set. A statement
    /// with no result set, such as an `UPDATE`, sends its count of changed
    /// rows in that token, and the count goes to the sink as a message. A
    /// statement that sends neither a set nor a count adds nothing.
    async fn stream_sets(
        &mut self,
        statement: &str,
        params: &[&dyn tiberius::ToSql],
        options: &ExecOptions,
        sink: &mut dyn RowSink,
        may_end_early: bool,
    ) -> Result<Walk> {
        // The handle is taken before the stream, because the stream holds
        // the client while it lives.
        let attention = self.client.attention_handle();
        let mut stream = self.client.query(statement, params).await?;
        let mut open = false;
        let mut count = 0usize;
        let mut truncated = false;
        let mut stopped = false;
        let mut asked_to_end = false;
        // True when the row limit brought the end, and not the sink.
        let mut ended_at_limit = false;
        let mut rows_affected: Option<u64> = None;
        let mut errors = 0usize;

        loop {
            let item = match stream.try_next().await {
                Ok(Some(item)) => item,
                Ok(None) => break,
                // The end that the attention packet brings is the wanted
                // end, so it carries no fault to the user.
                Err(tiberius::error::Error::Canceled) if asked_to_end => break,
                Err(error) => return Err(error.into()),
            };
            match item {
                QueryItem::Metadata(metadata) => {
                    if open {
                        end_set(sink, count, truncated)?;
                        open = false;
                    }
                    // After a stop the sets that remain drain without a feed.
                    if stopped {
                        continue;
                    }
                    sink.begin_set(
                        metadata
                            .columns()
                            .iter()
                            .map(|column| {
                                ColumnInfo::new(column.name(), type_name(column.column_type()))
                            })
                            .collect(),
                    )?;
                    open = true;
                    count = 0;
                    truncated = false;
                }
                // The text of PRINT and of a RAISERROR of a low severity
                // stands beside the rows, as the server sends it.
                QueryItem::Message(message) => {
                    sink.message(Message::info(message.text().to_string()));
                }
                // A batch can go on after an error. The stream ends with the
                // first error, which reaches the user as the error of the
                // run. Each later error goes among the messages.
                QueryItem::Error(error) => {
                    errors += 1;
                    if errors > 1 {
                        sink.message(later_error(&error));
                    }
                }
                QueryItem::Done(done_rows) => {
                    if open {
                        end_set(sink, count, truncated)?;
                        open = false;
                    } else if let (Some(changed), false) = (done_rows, stopped) {
                        rows_affected = Some(rows_affected.unwrap_or(0) + changed);
                        sink.message(rows_affected_message(changed));
                    }
                }
                QueryItem::Row(row) => {
                    if !open || stopped {
                        continue;
                    }
                    if count >= options.max_rows {
                        truncated = true;
                        if may_end_early && !asked_to_end {
                            attention.signal();
                            asked_to_end = true;
                            ended_at_limit = true;
                        }
                        continue;
                    }
                    if sink.row(row_to_json(&row))? == SinkControl::Stop {
                        truncated = true;
                        stopped = true;
                        if may_end_early && !asked_to_end {
                            attention.signal();
                            asked_to_end = true;
                        }
                        continue;
                    }
                    count += 1;
                }
            }
        }
        if open {
            end_set(sink, count, truncated)?;
        }
        if ended_at_limit {
            sink.message(Message::info(ENDED_AT_THE_LIMIT_MESSAGE.to_string()));
        }
        Ok(Walk {
            stopped,
            rows_affected,
        })
    }

    /// True when an attention packet at the row limit loses no work. The
    /// batch must hold one statement that only reads, and the probe
    /// [`ATTENTION_KEEPS_WORK`] must accept the state of the session.
    ///
    /// The packet ends the whole batch, so a batch of several statements
    /// would lose the statements after the one that reached the limit. The
    /// packet also rolls back the statement it ends, so an
    /// `INSERT ... OUTPUT` would write no row. A probe that fails gives
    /// false.
    async fn may_end_early(&mut self, statements: &[String]) -> bool {
        if statements.len() != 1 || !only_reads(&statements[0], Dialect::MsSql) {
            return false;
        }
        let row = match self.client.simple_query(ATTENTION_KEEPS_WORK).await {
            Ok(stream) => stream.into_row().await,
            Err(error) => Err(error),
        };
        matches!(row, Ok(Some(row)) if row.get::<i32, _>(0) == Some(1))
    }

    /// Runs one statement of the session that carries no rows back, such as
    /// the switch that turns the plan on or off.
    async fn run_switch(&mut self, statement: &str) -> Result<()> {
        let mut stream = self.client.simple_query(statement).await?;
        while stream.try_next().await?.is_some() {}
        Ok(())
    }
}

/// Names the reason a login failed. Kerberos reports a missing ticket in
/// words that mean nothing to a user of a database, so the message says what
/// to do instead.
fn describe_login(error: tiberius::error::Error, auth: MssqlAuth) -> Error {
    // A pasted token that the server refuses is old more often than it is
    // wrong, and the date check before the login lets an unreadable token
    // through.
    if auth == MssqlAuth::EntraAccessToken {
        let text = error.to_string();
        if names_a_refused_login(&text) {
            return Error::Authentication(format!("{EXPIRED_TOKEN_MESSAGE} {text}"));
        }
        return Error::from(error);
    }
    if auth != MssqlAuth::Integrated {
        return Error::from(error);
    }
    let text = error.to_string();
    if names_a_ticket_fault(&text) {
        Error::Authentication(format!(
            "The server refused the account of this user. On macOS and on Linux, run `kinit` to \
             get a Kerberos ticket, and name the server by its full host name so that the ticket \
             matches. {text}"
        ))
    } else {
        Error::from(error)
    }
}

/// True when the text of an error is the refusal of a login by the server.
/// The server gives the number 18456 for such a refusal, and the text of the
/// message names the login as well.
fn names_a_refused_login(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("login failed") || lower.contains("18456")
}

/// True when the text of a failed login points at the ticket of the user.
fn names_a_ticket_fault(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "credential",
        "gss",
        "kerberos",
        "ticket",
        "kdc",
        "sspi",
        "principal",
    ]
    .iter()
    .any(|mark| lower.contains(mark))
}

/// Runs a step of the opening of a connection under the time limit. A step
/// that does not finish reports the connection and not the statement, because
/// the advice for a slow statement does not fit a server that never answered.
async fn while_connecting<F: std::future::Future>(limit: Duration, future: F) -> Result<F::Output> {
    tokio::time::timeout(limit, future).await.map_err(|_| {
        Error::Connection(format!(
            "The server did not finish opening the connection inside {} seconds.",
            limit.as_secs()
        ))
    })
}

/// Builds the statement that reads the CREATE text of one view. MS SQL
/// Server keeps no text for a table, so a table gives no statement and the
/// command layer builds a draft instead. `OBJECT_DEFINITION` looks in the
/// current database, so the statement reads `sys.sql_modules` of the
/// database of the view. `OBJECT_ID` finds the view in that database from
/// its three-part name.
fn create_query_text(
    database: Option<&str>,
    schema: Option<&str>,
    table: &str,
    kind: TableKind,
) -> Option<CreateQuery> {
    if kind != TableKind::View {
        return None;
    }
    let name = Dialect::MsSql.qualified_name(database, schema, table);
    let catalog = database
        .map(|database| format!("{}.", Dialect::MsSql.quote_identifier(database)))
        .unwrap_or_default();
    Some(CreateQuery::new(
        format!(
            "SELECT m.definition FROM {catalog}sys.sql_modules AS m \
             WHERE m.object_id = OBJECT_ID({});",
            Dialect::MsSql.quote_literal(&name)
        ),
        0,
    ))
}

/// The name MS SQL Server gives the column that holds a plan. Both plan
/// switches use this name.
pub const PLAN_COLUMN: &str = "Microsoft SQL Server 2005 XML Showplan";

/// The switch that asks the session for a plan. `SHOWPLAN_XML` compiles the
/// statement and does not run it. `STATISTICS XML` runs it and adds the plan
/// after each result set of the statement.
pub fn plan_switch(kind: PlanKind) -> &'static str {
    match kind {
        PlanKind::Estimated => "SHOWPLAN_XML",
        PlanKind::Actual => "STATISTICS XML",
    }
}

/// True when one result set holds a plan.
pub fn is_plan_set(set: &ResultSet) -> bool {
    set.columns.len() == 1 && set.columns[0].name == PLAN_COLUMN
}

/// Keeps the plan sets of a run and drops the rows of the statement itself.
///
/// `STATISTICS XML` sends one plan after each result set, so a run gives the
/// data of the statement and its plan together. The second value of the answer
/// is false when the run held no plan at all, and the caller then keeps every
/// set and says so.
pub fn select_plan_sets(sets: Vec<ResultSet>) -> (Vec<ResultSet>, bool) {
    if sets.iter().any(is_plan_set) {
        (sets.into_iter().filter(is_plan_set).collect(), true)
    } else {
        (sets, false)
    }
}

/// The message for an error that the server sent after the first error of a
/// batch, with the number, the severity, the state and the line that SQL
/// Server Management Studio shows.
fn later_error(error: &tiberius::error::TokenError) -> Message {
    Message {
        level: MessageLevel::Error,
        text: error.message().to_string(),
        detail: Some(format!(
            "Msg {}, Level {}, State {}, Line {}",
            error.code(),
            error.class(),
            error.state(),
            error.line()
        )),
    }
}

/// Reports the count of rows of the open result set and ends the set.
fn end_set(sink: &mut dyn RowSink, count: usize, truncated: bool) -> Result<()> {
    sink.message(rows_returned_message(count, truncated));
    sink.end_set(truncated)
}

/// Turns the JSON parameters into values that `tiberius` can bind.
fn bind_params(params: Option<&QueryParams>) -> Result<Vec<Box<dyn tiberius::ToSql>>> {
    let mut bound: Vec<Box<dyn tiberius::ToSql>> = Vec::new();
    let Some(params) = params else {
        return Ok(bound);
    };
    for param in params {
        match &param.value {
            JsonValue::String(text) => bound.push(Box::new(text.clone())),
            JsonValue::Bool(flag) => bound.push(Box::new(*flag)),
            JsonValue::Null => bound.push(Box::new(Option::<String>::None)),
            JsonValue::Number(number) => match number_value(number) {
                Some(NumberValue::Integer(value)) => bound.push(Box::new(value)),
                Some(NumberValue::Float(value)) => bound.push(Box::new(value)),
                None => return Err(number_out_of_range(number)),
            },
            other => return Err(parameter_type_refused(other)),
        }
    }
    Ok(bound)
}

#[async_trait]
impl DatabaseDriver for MssqlDriver {
    fn capabilities(&self) -> DriverCapabilities {
        DriverCapabilities {
            supports_schemas: true,
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
        Dialect::MsSql
    }

    fn create_query(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        table: &str,
        kind: TableKind,
    ) -> Option<CreateQuery> {
        create_query_text(database, schema, table, kind)
    }

    async fn ping(&mut self) -> Result<()> {
        let mut stream = self.client.simple_query("SELECT 1").await?;
        while stream.try_next().await?.is_some() {}
        Ok(())
    }

    async fn holds_open_transaction(&mut self) -> Result<bool> {
        let row = self
            .client
            .simple_query("SELECT @@TRANCOUNT")
            .await?
            .into_row()
            .await?;
        Ok(row.and_then(|row| row.get::<i32, _>(0)).unwrap_or(0) > 0)
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

        // The batch is the unit that the server compiles, so a batch goes to
        // the server whole. A variable that one statement declares then holds
        // for the statements that follow it in the same batch.
        let batches = split_batches(query, Dialect::MsSql);
        // The placeholders of the parameters are numbered over the whole
        // text, and each batch is a request of its own, so the parameters of
        // a later batch cannot be named.
        if params.is_some() && batches.len() > 1 {
            return Err(Error::Configuration(
                "A run with parameters takes one batch. Remove the GO separators, or run one \
                 batch at a time."
                    .to_string(),
            ));
        }

        'batches: for batch in batches {
            let statements = split_statements(&batch.text, Dialect::MsSql);
            // A statement that only reads leaves the state of the session as
            // it found it, so one probe serves every run of the batch.
            let may_end_early = self.may_end_early(&statements).await;

            for _ in 0..batch.runs {
                let bound = bind_params(params)?;
                let borrowed: Vec<&dyn tiberius::ToSql> =
                    bound.iter().map(|value| value.as_ref()).collect();

                // Every batch goes through the path that keeps rows, because
                // an `INSERT ... OUTPUT` or a `BEGIN ... END` block can answer
                // with rows as well as with a count of changed rows.
                let walk = self
                    .stream_sets(
                        &batch.text,
                        borrowed.as_slice(),
                        options,
                        sink,
                        may_end_early,
                    )
                    .await?;
                if let Some(changed) = walk.rows_affected {
                    rows_affected = Some(rows_affected.unwrap_or(0) + changed);
                }
                if walk.stopped {
                    break 'batches;
                }
            }
        }

        Ok(RunSummary {
            rows_affected,
            elapsed_ms: started.elapsed().as_millis() as u64,
            stats: None,
        })
    }

    /// Reads the plan of one statement.
    ///
    /// The switch that asks for a plan must stand alone in its batch, and it
    /// holds for the whole session, so the switch goes on, the statement runs,
    /// and the switch goes off again even when the statement failed.
    ///
    /// The statement goes through the path that keeps rows, whatever its first
    /// keyword is, because with the plan switch on an INSERT also answers with
    /// a plan.
    ///
    /// With the actual plan the server sends the plan set after the rows of
    /// the statement. An attention packet at the row limit would end the batch
    /// before that set arrives, so the walk of the actual plan keeps to the end
    /// of the stream.
    async fn explain(
        &mut self,
        query: &str,
        params: Option<&QueryParams>,
        kind: PlanKind,
        options: &ExecOptions,
    ) -> Result<QueryResponse> {
        let statement = single_statement(query, Dialect::MsSql)?;
        let bound = bind_params(params)?;
        let borrowed: Vec<&dyn tiberius::ToSql> =
            bound.iter().map(|value| value.as_ref()).collect();
        let switch = plan_switch(kind);
        let started = Instant::now();

        self.run_switch(&format!("SET {switch} ON")).await?;
        // The plan sets are filtered after the run, so the rows buffer here.
        let mut sink = BufferSink::new(options.max_rows);
        // The server does not run a statement under the estimated plan, so
        // an attention packet there rolls back no work.
        let may_end_early = kind == PlanKind::Estimated;
        let outcome = self
            .stream_sets(
                &statement,
                borrowed.as_slice(),
                options,
                &mut sink,
                may_end_early,
            )
            .await;
        if let Err(error) = self.run_switch(&format!("SET {switch} OFF")).await {
            // The switch holds for the session, so a session that keeps it on
            // answers every later statement with a plan. The connection is
            // therefore no longer fit for use.
            log::warn!("The plan switch stayed on: {error}");
            return Err(Error::Connection(format!(
                "The plan switch '{switch}' could not be turned off again, so this connection was \
                 left in the plan state. Open the connection again. {error}"
            )));
        }

        outcome?;
        let sets = sink.into_response(RunSummary::default()).results;
        let (results, found) = select_plan_sets(sets);
        let mut response = QueryResponse {
            results,
            elapsed_ms: started.elapsed().as_millis() as u64,
            ..QueryResponse::default()
        };
        if !found {
            response.messages.push(Message::warning(
                "The server sent no plan, so these are the sets the statement returned.",
            ));
        }
        Ok(response)
    }

    async fn list_databases(&mut self) -> Result<Vec<Database>> {
        let query = "SELECT name FROM sys.databases \
                     WHERE state = 0 AND HAS_DBACCESS(name) = 1 ORDER BY name";
        let mut stream = self.client.simple_query(query).await?;
        let mut databases = Vec::new();
        while let Some(item) = stream.try_next().await? {
            if let QueryItem::Row(row) = item {
                if let Some(name) = row.try_get::<&str, _>(0)? {
                    databases.push(Database {
                        name: name.to_string(),
                    });
                }
            }
        }
        Ok(databases)
    }

    async fn list_schemas(&mut self, database: &str) -> Result<Vec<Schema>> {
        let query = schema_query(&Dialect::MsSql.quote_identifier(database));
        let mut stream = self.client.simple_query(query).await?;
        let mut schemas = Vec::new();
        while let Some(item) = stream.try_next().await? {
            if let QueryItem::Row(row) = item {
                if let Some(name) = row.try_get::<&str, _>(0)? {
                    schemas.push(Schema {
                        name: name.to_string(),
                    });
                }
            }
        }
        Ok(schemas)
    }

    async fn list_tables(&mut self, database: &str, schema: Option<&str>) -> Result<Vec<Table>> {
        let schema = schema.unwrap_or("dbo");
        let query = format!(
            "SELECT TABLE_NAME, TABLE_TYPE FROM {}.INFORMATION_SCHEMA.TABLES \
             WHERE TABLE_SCHEMA = @P1 ORDER BY TABLE_TYPE, TABLE_NAME",
            Dialect::MsSql.quote_identifier(database)
        );
        let mut stream = self.client.query(query, &[&schema]).await?;
        let mut tables = Vec::new();
        while let Some(item) = stream.try_next().await? {
            if let QueryItem::Row(row) = item {
                let name = row.try_get::<&str, _>(0)?.unwrap_or_default().to_string();
                let kind = row.try_get::<&str, _>(1)?.unwrap_or_default();
                tables.push(if kind.eq_ignore_ascii_case("VIEW") {
                    Table::view(name)
                } else {
                    Table::table(name)
                });
            }
        }
        Ok(tables)
    }

    async fn list_columns(
        &mut self,
        database: &str,
        schema: Option<&str>,
        table: &str,
    ) -> Result<Vec<AppColumn>> {
        let schema = schema.unwrap_or("dbo");
        let catalog = Dialect::MsSql.quote_identifier(database);
        let query = format!(
            "SELECT c.COLUMN_NAME, \
                    c.DATA_TYPE, \
                    c.CHARACTER_MAXIMUM_LENGTH, \
                    c.NUMERIC_PRECISION, \
                    c.NUMERIC_SCALE, \
                    c.IS_NULLABLE, \
                    CASE WHEN k.COLUMN_NAME IS NULL THEN 0 ELSE 1 END AS IS_KEY \
             FROM {catalog}.INFORMATION_SCHEMA.COLUMNS AS c \
             LEFT JOIN ( \
                 SELECT ku.TABLE_SCHEMA, ku.TABLE_NAME, ku.COLUMN_NAME \
                 FROM {catalog}.INFORMATION_SCHEMA.TABLE_CONSTRAINTS AS tc \
                 JOIN {catalog}.INFORMATION_SCHEMA.KEY_COLUMN_USAGE AS ku \
                   ON tc.CONSTRAINT_NAME = ku.CONSTRAINT_NAME \
                  AND tc.CONSTRAINT_SCHEMA = ku.CONSTRAINT_SCHEMA \
                 WHERE tc.CONSTRAINT_TYPE = 'PRIMARY KEY' \
             ) AS k \
               ON k.TABLE_SCHEMA = c.TABLE_SCHEMA \
              AND k.TABLE_NAME = c.TABLE_NAME \
              AND k.COLUMN_NAME = c.COLUMN_NAME \
             WHERE c.TABLE_SCHEMA = @P1 AND c.TABLE_NAME = @P2 \
             ORDER BY c.ORDINAL_POSITION"
        );
        let mut stream = self.client.query(query, &[&schema, &table]).await?;
        let mut columns = Vec::new();
        while let Some(item) = stream.try_next().await? {
            if let QueryItem::Row(row) = item {
                let name = row.try_get::<&str, _>(0)?.unwrap_or_default().to_string();
                let base = row.try_get::<&str, _>(1)?.unwrap_or_default();
                let length = row.try_get::<i32, _>(2)?;
                let precision = row.try_get::<u8, _>(3)?;
                let scale = row.try_get::<i32, _>(4)?;
                let nullable = row.try_get::<&str, _>(5)?.unwrap_or("YES");
                let is_key = row.try_get::<i32, _>(6)?.unwrap_or(0);
                columns.push(AppColumn {
                    name,
                    data_type: format_type(base, length, precision, scale),
                    nullable: nullable.eq_ignore_ascii_case("YES"),
                    is_primary_key: is_key == 1,
                });
            }
        }
        Ok(columns)
    }

    /// Reads the rows and the size of a relation from the partition figures
    /// of the engine, together with the day the object last changed.
    async fn table_facts(
        &mut self,
        database: &str,
        schema: Option<&str>,
        table: &str,
    ) -> Result<Vec<TableFact>> {
        let name =
            Dialect::MsSql.qualified_name(Some(database), Some(schema.unwrap_or("dbo")), table);
        let query = fact_query(&Dialect::MsSql.quote_identifier(database));
        let mut stream = self.client.query(query, &[&name.as_str()]).await?;
        let mut facts = Vec::new();
        while let Some(item) = stream.try_next().await? {
            let QueryItem::Row(row) = item else { continue };
            if let Some(rows) = row.try_get::<i64, _>(0)? {
                facts.push(TableFact::new("Rows", rows.max(0).to_string()));
            }
            if let Some(pages) = row.try_get::<i64, _>(1)? {
                // One page of MS SQL Server holds eight kilobytes.
                facts.push(TableFact::new(
                    "Size",
                    size_text(pages.max(0) as u64 * 8 * 1024),
                ));
            }
            if let Some(changed) = row.try_get::<NaiveDateTime, _>(2)? {
                facts.push(TableFact::new("Last change", changed.to_string()));
            }
        }
        Ok(facts)
    }

    async fn schema_snapshot(
        &mut self,
        database: &str,
        max_columns: usize,
    ) -> Result<SchemaSnapshot> {
        let query = snapshot_query(&Dialect::MsSql.quote_identifier(database));
        let mut stream = self.client.query(query, &[]).await?;
        let mut snapshot = SchemaSnapshot {
            database: database.to_string(),
            complete: true,
            ..SchemaSnapshot::default()
        };
        while let Some(item) = stream.try_next().await? {
            let QueryItem::Row(row) = item else { continue };
            let kept = add_snapshot_column(
                &mut snapshot,
                max_columns,
                row.try_get::<&str, _>(0)?.map(str::to_string),
                row.try_get::<&str, _>(1)?.unwrap_or_default().to_string(),
                table_kind(row.try_get::<&str, _>(2)?.unwrap_or_default()),
                SnapshotColumn {
                    name: row.try_get::<&str, _>(3)?.unwrap_or_default().to_string(),
                    data_type: row.try_get::<&str, _>(4)?.unwrap_or_default().to_string(),
                },
            );
            if !kept {
                break;
            }
        }
        Ok(snapshot)
    }

    async fn list_routines(
        &mut self,
        database: &str,
        schema: Option<&str>,
    ) -> Result<Vec<Routine>> {
        let schema = schema.unwrap_or("dbo");
        let query = routine_query(&Dialect::MsSql.quote_identifier(database));
        let mut stream = self.client.query(query, &[&schema]).await?;
        let mut routines = Vec::new();
        while let Some(item) = stream.try_next().await? {
            if let QueryItem::Row(row) = item {
                routines.push(Routine {
                    name: row.try_get::<&str, _>(0)?.unwrap_or_default().to_string(),
                    kind: routine_kind(row.try_get::<&str, _>(1)?.unwrap_or_default()),
                });
            }
        }
        Ok(routines)
    }

    async fn list_indexes(
        &mut self,
        database: &str,
        schema: Option<&str>,
        table: &str,
    ) -> Result<Vec<IndexInfo>> {
        let name =
            Dialect::MsSql.qualified_name(Some(database), Some(schema.unwrap_or("dbo")), table);
        let query = index_query(&Dialect::MsSql.quote_identifier(database));
        let mut stream = self.client.query(query, &[&name.as_str()]).await?;
        let mut indexes = Vec::new();
        while let Some(item) = stream.try_next().await? {
            if let QueryItem::Row(row) = item {
                let name = row.try_get::<&str, _>(0)?.unwrap_or_default().to_string();
                let column = row.try_get::<&str, _>(1)?.unwrap_or_default().to_string();
                let unique = row.try_get::<bool, _>(2)?.unwrap_or(false);
                let primary = row.try_get::<bool, _>(3)?.unwrap_or(false);
                if row.try_get::<bool, _>(4)?.unwrap_or(false) {
                    add_included_column(&mut indexes, name, unique, primary, column);
                } else {
                    add_index_column(&mut indexes, name, unique, primary, Some(column));
                }
            }
        }
        Ok(indexes)
    }

    async fn list_constraints(
        &mut self,
        database: &str,
        schema: Option<&str>,
        table: &str,
    ) -> Result<Vec<Constraint>> {
        let schema = schema.unwrap_or("dbo");
        let query = constraint_query(&Dialect::MsSql.quote_identifier(database));
        let mut stream = self.client.query(query, &[&schema, &table]).await?;
        let mut constraints = Vec::new();
        while let Some(item) = stream.try_next().await? {
            if let QueryItem::Row(row) = item {
                let target = row.try_get::<&str, _>(3)?.map(str::to_string);
                let check = row.try_get::<&str, _>(4)?.map(str::to_string);
                add_constraint_column(
                    &mut constraints,
                    row.try_get::<&str, _>(0)?.unwrap_or_default().to_string(),
                    constraint_kind(row.try_get::<&str, _>(1)?.unwrap_or_default()),
                    row.try_get::<&str, _>(2)?.map(str::to_string),
                    target.or(check),
                );
            }
        }
        Ok(constraints)
    }

    fn cancel_handle(&self) -> Option<Arc<dyn CancelHandle>> {
        Some(Arc::new(MssqlCancel(self.client.attention_handle())))
    }
}

/// A handle that asks the server to stop the statement that runs on this
/// connection. The connection sends an attention packet, the server ends
/// the statement, and the stream of the statement ends with the cancel
/// error of `tiberius`. The connection then stays open for the next
/// statement.
struct MssqlCancel(Arc<AttentionHandle>);

#[async_trait]
impl CancelHandle for MssqlCancel {
    async fn cancel(&self) -> Result<()> {
        self.0.signal();
        Ok(())
    }
}

/// Reads the rows, the pages and the day of the last change of one relation.
fn fact_query(catalog: &str) -> String {
    format!(
        "SELECT SUM(CASE WHEN s.index_id IN (0, 1) THEN s.row_count ELSE 0 END), \
                SUM(s.used_page_count), \
                MAX(o.modify_date) \
         FROM {catalog}.sys.dm_db_partition_stats AS s \
         JOIN {catalog}.sys.objects AS o ON o.object_id = s.object_id \
         WHERE s.object_id = OBJECT_ID(@P1)"
    )
}

/// Reads every relation and every column of one database in one statement.
/// The rows arrive in the order of the relation, which the fold needs.
fn snapshot_query(catalog: &str) -> String {
    format!(
        "SELECT c.TABLE_SCHEMA, c.TABLE_NAME, t.TABLE_TYPE, c.COLUMN_NAME, c.DATA_TYPE \
         FROM {catalog}.INFORMATION_SCHEMA.COLUMNS AS c \
         JOIN {catalog}.INFORMATION_SCHEMA.TABLES AS t \
           ON t.TABLE_SCHEMA = c.TABLE_SCHEMA AND t.TABLE_NAME = c.TABLE_NAME \
         ORDER BY c.TABLE_SCHEMA, c.TABLE_NAME, c.ORDINAL_POSITION"
    )
}

/// Reads the procedures and the functions of one schema. The name of the
/// database is quoted into the statement, because a name of a database cannot
/// be bound as a parameter.
fn routine_query(catalog: &str) -> String {
    format!(
        "SELECT ROUTINE_NAME, ROUTINE_TYPE \
         FROM {catalog}.INFORMATION_SCHEMA.ROUTINES \
         WHERE ROUTINE_SCHEMA = @P1 \
         ORDER BY ROUTINE_TYPE, ROUTINE_NAME"
    )
}

/// Reads the schemas of one database. The catalog view shows the database
/// that the connection is attached to, so the name of the database goes in
/// front of it. The statement hides `INFORMATION_SCHEMA` (id 3), `sys` (id 4)
/// and the schemas of the fixed database roles, such as `db_owner`, which
/// have the ids 16384 to 16399. It hides no schema by its name, so a user
/// schema such as `db_sales` stays in the list. It does not join the owner,
/// because a user can see a schema whose owner is hidden from that user.
fn schema_query(catalog: &str) -> String {
    format!(
        "SELECT s.name FROM {catalog}.sys.schemas AS s \
         WHERE s.schema_id NOT IN (3, 4) AND s.schema_id NOT BETWEEN 16384 AND 16399 \
         ORDER BY s.name"
    )
}

/// Reads one column of one index for each row. The name of the relation
/// reaches `OBJECT_ID` as a parameter. An `INCLUDE` column has the key
/// ordinal 0, so the key columns come first in key order, and the included
/// columns follow in the order of the statement that made the index.
fn index_query(catalog: &str) -> String {
    format!(
        "SELECT i.name, c.name, i.is_unique, i.is_primary_key, ic.is_included_column \
         FROM {catalog}.sys.indexes AS i \
         JOIN {catalog}.sys.index_columns AS ic \
           ON ic.object_id = i.object_id AND ic.index_id = i.index_id \
         JOIN {catalog}.sys.columns AS c \
           ON c.object_id = ic.object_id AND c.column_id = ic.column_id \
         WHERE i.object_id = OBJECT_ID(@P1) AND i.name IS NOT NULL \
         ORDER BY i.name, ic.is_included_column, ic.key_ordinal, ic.index_column_id"
    )
}

/// Reads one column of one constraint for each row. A foreign key carries the
/// relation it points at, and a check carries its rule. `OBJECT_NAME` and
/// `SCHEMA_ID` look in the current database, so the statement joins the
/// `sys` views of the named database instead. A foreign key matches on its
/// schema and its name, because two schemas can each have a key of one name.
fn constraint_query(catalog: &str) -> String {
    format!(
        "SELECT tc.CONSTRAINT_NAME, \
                tc.CONSTRAINT_TYPE, \
                ku.COLUMN_NAME, \
                ro.name, \
                cc.CHECK_CLAUSE \
         FROM {catalog}.INFORMATION_SCHEMA.TABLE_CONSTRAINTS AS tc \
         LEFT JOIN {catalog}.INFORMATION_SCHEMA.KEY_COLUMN_USAGE AS ku \
                ON ku.CONSTRAINT_NAME = tc.CONSTRAINT_NAME \
               AND ku.CONSTRAINT_SCHEMA = tc.CONSTRAINT_SCHEMA \
         LEFT JOIN {catalog}.INFORMATION_SCHEMA.CHECK_CONSTRAINTS AS cc \
                ON cc.CONSTRAINT_NAME = tc.CONSTRAINT_NAME \
               AND cc.CONSTRAINT_SCHEMA = tc.CONSTRAINT_SCHEMA \
         LEFT JOIN ({catalog}.sys.foreign_keys AS fk \
                    JOIN {catalog}.sys.schemas AS fs ON fs.schema_id = fk.schema_id) \
                ON fk.name = tc.CONSTRAINT_NAME AND fs.name = tc.CONSTRAINT_SCHEMA \
         LEFT JOIN {catalog}.sys.objects AS ro ON ro.object_id = fk.referenced_object_id \
         WHERE tc.TABLE_SCHEMA = @P1 AND tc.TABLE_NAME = @P2 \
         ORDER BY tc.CONSTRAINT_NAME, ku.ORDINAL_POSITION"
    )
}

/// Joins the base type with the length or the precision, so that the
/// explorer shows `varchar(50)` and not `varchar`.
pub fn format_type(
    base: &str,
    length: Option<i32>,
    precision: Option<u8>,
    scale: Option<i32>,
) -> String {
    match base.to_lowercase().as_str() {
        "char" | "varchar" | "nchar" | "nvarchar" | "binary" | "varbinary" => match length {
            Some(-1) => format!("{base}(max)"),
            Some(value) => format!("{base}({value})"),
            None => base.to_string(),
        },
        "decimal" | "numeric" => match (precision, scale) {
            (Some(precision), Some(scale)) => format!("{base}({precision},{scale})"),
            (Some(precision), None) => format!("{base}({precision})"),
            _ => base.to_string(),
        },
        _ => base.to_string(),
    }
}

/// Gives the name of a column type for the header of the results grid.
pub fn type_name(column_type: ColumnType) -> &'static str {
    match column_type {
        ColumnType::Null => "null",
        ColumnType::Bit | ColumnType::Bitn => "bit",
        ColumnType::Int1 => "tinyint",
        ColumnType::Int2 => "smallint",
        ColumnType::Int4 => "int",
        ColumnType::Int8 => "bigint",
        ColumnType::Intn => "integer",
        ColumnType::Float4 => "real",
        ColumnType::Float8 => "float",
        ColumnType::Floatn => "float",
        ColumnType::Money | ColumnType::Money4 => "money",
        ColumnType::Decimaln => "decimal",
        ColumnType::Numericn => "numeric",
        ColumnType::Guid => "uniqueidentifier",
        ColumnType::Datetime | ColumnType::Datetimen => "datetime",
        ColumnType::Datetime4 => "smalldatetime",
        ColumnType::Datetime2 => "datetime2",
        ColumnType::DatetimeOffsetn => "datetimeoffset",
        ColumnType::Daten => "date",
        ColumnType::Timen => "time",
        ColumnType::BigVarChar => "varchar",
        ColumnType::BigChar => "char",
        ColumnType::NVarchar => "nvarchar",
        ColumnType::NChar => "nchar",
        ColumnType::Text => "text",
        ColumnType::NText => "ntext",
        ColumnType::BigVarBin => "varbinary",
        ColumnType::BigBinary => "binary",
        ColumnType::Image => "image",
        ColumnType::Xml => "xml",
        ColumnType::Udt => "udt",
        ColumnType::SSVariant => "sql_variant",
    }
}

/// Writes a decimal value as text, so that no precision is lost. The
/// `Display` of `tiberius` puts a second minus sign in front of the
/// fraction of a negative value, so the digits are laid out here.
pub fn numeric_to_string(value: Numeric) -> String {
    let scale = value.scale() as usize;
    let raw = value.value();
    let sign = if raw < 0 { "-" } else { "" };
    let digits = raw.unsigned_abs().to_string();
    if scale == 0 {
        return format!("{sign}{digits}");
    }
    let digits = if digits.len() <= scale {
        format!("{}{}", "0".repeat(scale + 1 - digits.len()), digits)
    } else {
        digits
    };
    let split = digits.len() - scale;
    format!("{sign}{}.{}", &digits[..split], &digits[split..])
}

/// Converts one row into an array of JSON values.
///
/// Every read uses `try_get`. The `get` of `tiberius` panics when the type
/// of the column does not match the target type, and the type of a column
/// is not known before the server answers.
pub fn row_to_json(row: &Row) -> Vec<JsonValue> {
    row.cells()
        .enumerate()
        .map(|(index, (column, data))| cell_to_json(row, index, column.column_type(), data))
        .collect()
}

/// Reads one cell. A read that fails falls back on the next target type,
/// and at the end on text, so that an unknown type shows a value and does
/// not stop the whole result.
///
/// A type of the server that covers several widths of value carries no
/// width in the type of the column. Such a cell gives the data it holds,
/// and the form of that data gives the JSON value, so the read pays no
/// conversion that fails.
fn cell_to_json(
    row: &Row,
    index: usize,
    column_type: ColumnType,
    data: &ColumnData<'static>,
) -> JsonValue {
    match column_type {
        ColumnType::Bit | ColumnType::Bitn => {
            read(row.try_get::<bool, _>(index)).map_or(JsonValue::Null, JsonValue::Bool)
        }
        ColumnType::Int1 => read(row.try_get::<u8, _>(index)).map_or(JsonValue::Null, Into::into),
        ColumnType::Int2 => read(row.try_get::<i16, _>(index)).map_or(JsonValue::Null, Into::into),
        ColumnType::Int4 => read(row.try_get::<i32, _>(index)).map_or(JsonValue::Null, Into::into),
        ColumnType::Int8 => read(row.try_get::<i64, _>(index)).map_or(JsonValue::Null, Into::into),
        // A nullable integer covers every width from one to eight bytes,
        // which the type of the column does not name.
        ColumnType::Intn => column_data_to_json(data),
        ColumnType::Float4 => {
            read(row.try_get::<f32, _>(index)).map_or(JsonValue::Null, f32_to_json)
        }
        ColumnType::Float8 => {
            read(row.try_get::<f64, _>(index)).map_or(JsonValue::Null, f64_to_json)
        }
        // The copy of `tiberius` reads a money value as a decimal, so each of
        // its 19 digits stays.
        ColumnType::Money | ColumnType::Money4 => column_data_to_json(data),
        // A nullable float holds four bytes or eight, which the type of the
        // column does not name either.
        ColumnType::Floatn => column_data_to_json(data),
        ColumnType::Decimaln | ColumnType::Numericn => read(row.try_get::<Numeric, _>(index))
            .map(|value| JsonValue::String(numeric_to_string(value)))
            .unwrap_or(JsonValue::Null),
        ColumnType::Guid => read(row.try_get::<uuid::Uuid, _>(index))
            .map(|value| JsonValue::String(value.to_string()))
            .unwrap_or(JsonValue::Null),
        ColumnType::Daten => read(row.try_get::<NaiveDate, _>(index))
            .map(|value| JsonValue::String(value.to_string()))
            .unwrap_or(JsonValue::Null),
        ColumnType::Timen => read(row.try_get::<NaiveTime, _>(index))
            .map(|value| JsonValue::String(value.to_string()))
            .unwrap_or(JsonValue::Null),
        ColumnType::Datetime
        | ColumnType::Datetimen
        | ColumnType::Datetime4
        | ColumnType::Datetime2 => read(row.try_get::<NaiveDateTime, _>(index))
            .map(|value| JsonValue::String(value.to_string()))
            .unwrap_or(JsonValue::Null),
        ColumnType::DatetimeOffsetn => column_data_to_json(data),
        ColumnType::BigVarBin | ColumnType::BigBinary | ColumnType::Image | ColumnType::Udt => {
            read(row.try_get::<&[u8], _>(index)).map_or(JsonValue::Null, bytes_to_json)
        }
        // The server sends an XML value in its own column type, which holds
        // text. The reads for text refuse that type, so the value needs the
        // type of the driver.
        ColumnType::Xml => read(row.try_get::<&XmlData, _>(index))
            .map(|value| JsonValue::String(value.as_ref().to_string()))
            .unwrap_or(JsonValue::Null),
        // Each value of a `sql_variant` column carries its own type, so the
        // type of the column says nothing about the cell. The cell gives its
        // own data, and the type of that data gives the JSON value.
        ColumnType::SSVariant => column_data_to_json(data),
        _ => text_or_bytes(row, index),
    }
}

/// Turns the data of one cell into JSON. The reads of `cell_to_json` ask for
/// a target type, which a `sql_variant` cell does not have before the value
/// arrives. This function reads the value that the cell already holds.
fn column_data_to_json(data: &ColumnData<'static>) -> JsonValue {
    match data {
        ColumnData::U8(value) => value.map_or(JsonValue::Null, Into::into),
        ColumnData::I16(value) => value.map_or(JsonValue::Null, Into::into),
        ColumnData::I32(value) => value.map_or(JsonValue::Null, Into::into),
        ColumnData::I64(value) => value.map_or(JsonValue::Null, Into::into),
        ColumnData::F32(value) => value.map_or(JsonValue::Null, f32_to_json),
        ColumnData::F64(value) => value.map_or(JsonValue::Null, f64_to_json),
        ColumnData::Bit(value) => value.map_or(JsonValue::Null, JsonValue::Bool),
        ColumnData::String(value) => value
            .as_ref()
            .map_or(JsonValue::Null, |text| JsonValue::String(text.to_string())),
        ColumnData::Guid(value) => value.map_or(JsonValue::Null, |value| {
            JsonValue::String(value.to_string())
        }),
        ColumnData::Binary(value) => value
            .as_ref()
            .map_or(JsonValue::Null, |bytes| bytes_to_json(bytes)),
        ColumnData::Numeric(value) => value.map_or(JsonValue::Null, |value| {
            JsonValue::String(numeric_to_string(value))
        }),
        ColumnData::Xml(value) => value.as_ref().map_or(JsonValue::Null, |value| {
            JsonValue::String(value.to_string())
        }),
        ColumnData::DateTime(_) | ColumnData::SmallDateTime(_) | ColumnData::DateTime2(_) => {
            read(NaiveDateTime::from_sql(data)).map_or(JsonValue::Null, |value| {
                JsonValue::String(value.to_string())
            })
        }
        ColumnData::Date(_) => read(NaiveDate::from_sql(data)).map_or(JsonValue::Null, |value| {
            JsonValue::String(value.to_string())
        }),
        ColumnData::Time(_) => read(NaiveTime::from_sql(data)).map_or(JsonValue::Null, |value| {
            JsonValue::String(value.to_string())
        }),
        // The value keeps the offset that the server sent with it.
        ColumnData::DateTimeOffset(_) => read(DateTime::<FixedOffset>::from_sql(data))
            .map_or(JsonValue::Null, |value| {
                JsonValue::String(value.to_rfc3339())
            }),
    }
}

/// Reads a value as text, and falls back on the binary form.
fn text_or_bytes(row: &Row, index: usize) -> JsonValue {
    if let Some(text) = read(row.try_get::<&str, _>(index)) {
        return JsonValue::String(text.to_string());
    }
    read(row.try_get::<&[u8], _>(index)).map_or(JsonValue::Null, bytes_to_json)
}

/// Turns the result of a read into an option. A read that fails is written
/// to the log and counts as an absent value, so the next target type gets a
/// turn.
fn read<T>(result: tiberius::Result<Option<T>>) -> Option<T> {
    match result {
        Ok(value) => value,
        Err(error) => {
            log::debug!("A column did not match the target type: {error}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// The kinds of packet that the test reads from the client.
    const PACKET_SQL_BATCH: u8 = 1;
    const PACKET_RPC: u8 = 3;
    const PACKET_ATTENTION: u8 = 6;
    /// The `Attention` flag of a `DONE` token.
    const DONE_ATTENTION: u16 = 1 << 5;
    /// The flag of a packet that ends its message.
    const END_OF_MESSAGE: u8 = 1;
    /// The `More` and `Count` flags of a `DONE` token.
    const DONE_MORE: u16 = 1;
    const DONE_COUNT: u16 = 1 << 4;

    /// Reads one message of the client and gives back the kind of its first
    /// packet. A message can arrive in several packets, and the last of them
    /// carries the end flag.
    async fn read_message(server: &mut tokio::net::TcpStream) -> u8 {
        let mut kind = None;
        loop {
            let mut header = [0u8; 8];
            server.read_exact(&mut header).await.unwrap();
            let length = u16::from_be_bytes([header[2], header[3]]) as usize;
            let mut body = vec![0u8; length - 8];
            server.read_exact(&mut body).await.unwrap();
            kind.get_or_insert(header[0]);
            if header[1] & END_OF_MESSAGE == END_OF_MESSAGE {
                return kind.unwrap();
            }
        }
    }

    /// Writes one packet of the server, with the kind `TabularResult`.
    async fn write_packet(server: &mut tokio::net::TcpStream, status: u8, payload: &[u8]) {
        let length = (payload.len() + 8) as u16;
        let mut packet = vec![4, status];
        packet.extend_from_slice(&length.to_be_bytes());
        packet.extend_from_slice(&[0, 0, 0, 0]);
        packet.extend_from_slice(payload);
        server.write_all(&packet).await.unwrap();
    }

    /// A `DONEINPROC` token with the given count of rows. The token ends one
    /// result set of a batch that holds more sets behind it.
    fn done_in_proc(rows: u64) -> Vec<u8> {
        let mut token = vec![0xFF];
        token.extend_from_slice(&0u16.to_le_bytes());
        token.extend_from_slice(&0u16.to_le_bytes());
        token.extend_from_slice(&rows.to_le_bytes());
        token
    }

    /// A `DONE` token with the given flags and count of rows.
    fn done_token(status: u16, rows: u64) -> Vec<u8> {
        let mut token = vec![0xFD];
        token.extend_from_slice(&status.to_le_bytes());
        token.extend_from_slice(&0u16.to_le_bytes());
        token.extend_from_slice(&rows.to_le_bytes());
        token
    }

    /// An `ERROR` token of severity 16 with the given number and text.
    fn error_token(code: u32, text: &str) -> Vec<u8> {
        let mut body = code.to_le_bytes().to_vec();
        body.push(1);
        body.push(16);
        let units: Vec<u16> = text.encode_utf16().collect();
        body.extend_from_slice(&(units.len() as u16).to_le_bytes());
        units
            .iter()
            .for_each(|unit| body.extend_from_slice(&unit.to_le_bytes()));
        body.push(0);
        body.push(0);
        body.extend_from_slice(&2u32.to_le_bytes());
        let mut token = vec![0xAA];
        token.extend_from_slice(&(body.len() as u16).to_le_bytes());
        token.extend(body);
        token
    }

    /// A `COLMETADATA` token of one `int` column with the name `a`.
    fn int_metadata() -> Vec<u8> {
        let mut token = vec![0x81];
        token.extend_from_slice(&1u16.to_le_bytes());
        token.extend_from_slice(&0u32.to_le_bytes());
        token.extend_from_slice(&0u16.to_le_bytes());
        token.push(0x38);
        token.push(1);
        token.extend_from_slice(&('a' as u16).to_le_bytes());
        token
    }

    /// A `ROW` token that carries one `int`.
    fn int_row(value: i32) -> Vec<u8> {
        let mut token = vec![0xD1];
        token.extend_from_slice(&value.to_le_bytes());
        token
    }

    /// A `COLMETADATA` token for one column of the type `xml` with the given
    /// name. The type carries a byte that says whether a schema follows, and
    /// no schema follows here.
    fn xml_metadata_named(name: &str) -> Vec<u8> {
        let utf16: Vec<u16> = name.encode_utf16().collect();
        let mut token = vec![0x81];
        token.extend_from_slice(&1u16.to_le_bytes());
        token.extend_from_slice(&0u32.to_le_bytes());
        token.extend_from_slice(&0u16.to_le_bytes());
        token.push(0xF1);
        token.push(0);
        token.push(utf16.len() as u8);
        for unit in utf16 {
            token.extend_from_slice(&unit.to_le_bytes());
        }
        token
    }

    /// A `COLMETADATA` token for one column of the type `xml` with the name
    /// `x`.
    fn xml_metadata() -> Vec<u8> {
        xml_metadata_named("x")
    }

    /// A `ROW` token that carries one `xml` value. The value goes in the
    /// form of a blob of unknown size: one chunk of UTF-16 text and a length
    /// of zero that ends the value.
    fn xml_row(text: &str) -> Vec<u8> {
        let utf16: Vec<u8> = text
            .encode_utf16()
            .flat_map(|unit| unit.to_le_bytes())
            .collect();
        let mut token = vec![0xD1];
        token.extend_from_slice(&0xfffffffffffffffe_u64.to_le_bytes());
        token.extend_from_slice(&(utf16.len() as u32).to_le_bytes());
        token.extend_from_slice(&utf16);
        token.extend_from_slice(&0u32.to_le_bytes());
        token
    }

    /// A `COLMETADATA` token for one column of the type `sql_variant`. The
    /// type carries the greatest length of a value in four bytes.
    fn variant_metadata() -> Vec<u8> {
        let mut token = vec![0x81];
        token.extend_from_slice(&1u16.to_le_bytes());
        token.extend_from_slice(&0u32.to_le_bytes());
        token.extend_from_slice(&0u16.to_le_bytes());
        token.push(0x62);
        token.extend_from_slice(&8009u32.to_le_bytes());
        token.push(1);
        token.extend_from_slice(&('v' as u16).to_le_bytes());
        token
    }

    /// A `ROW` token that carries one `sql_variant` value. The value holds
    /// its total length, the token of its base type, the count of its
    /// property bytes, the property bytes, and the value itself.
    fn variant_row(base: u8, props: &[u8], value: &[u8]) -> Vec<u8> {
        let mut token = vec![0xD1];
        let total = (2 + props.len() + value.len()) as u32;
        token.extend_from_slice(&total.to_le_bytes());
        token.push(base);
        token.push(props.len() as u8);
        token.extend_from_slice(props);
        token.extend_from_slice(value);
        token
    }

    /// A `ROW` token that carries a `sql_variant` value of no length, which
    /// is the null value.
    fn variant_null_row() -> Vec<u8> {
        let mut token = vec![0xD1];
        token.extend_from_slice(&0u32.to_le_bytes());
        token
    }

    /// Answers the prelogin and the login of a client that connects. The
    /// answer to the prelogin holds the terminator alone, which leaves the
    /// connection without encryption.
    async fn accept_login(server: &mut tokio::net::TcpStream) {
        read_message(server).await;
        write_packet(server, END_OF_MESSAGE, &[0xFF]).await;
        read_message(server).await;
        write_packet(server, END_OF_MESSAGE, &done_token(0, 0)).await;
    }

    /// The configuration of a client that speaks to the fake server.
    fn test_config() -> Config {
        let mut config = Config::new();
        config.authentication(AuthMethod::sql_server("user", "password"));
        config.encryption(EncryptionLevel::NotSupported);
        config
    }

    /// A server that answers one statement with five rows and keeps the
    /// statement running. It then waits for the attention packet, ends the
    /// statement with the acknowledgement, and answers one more statement.
    /// A test that never signals waits here for ever, so the wait itself
    /// proves that the driver asks the server to stop.
    async fn serve_rows_until_attention(listener: TcpListener) {
        let (mut socket, _) = listener.accept().await.unwrap();
        accept_login(&mut socket).await;

        let kind = read_message(&mut socket).await;
        assert!(kind == PACKET_RPC || kind == PACKET_SQL_BATCH);
        let mut answer = int_metadata();
        for value in 0..5 {
            answer.extend_from_slice(&int_row(value));
        }
        write_packet(&mut socket, 0, &answer).await;

        assert_eq!(read_message(&mut socket).await, PACKET_ATTENTION);
        write_packet(&mut socket, END_OF_MESSAGE, &done_token(DONE_ATTENTION, 0)).await;

        // The connection takes the next statement of the session.
        let kind = read_message(&mut socket).await;
        assert!(kind == PACKET_RPC || kind == PACKET_SQL_BATCH);
        write_packet(&mut socket, END_OF_MESSAGE, &done_token(0, 0)).await;
    }

    #[tokio::test]
    async fn the_row_limit_ends_the_statement_and_keeps_the_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(serve_rows_until_attention(listener));

        let tcp = TcpStream::connect(address).await.unwrap();
        let client = Client::connect(test_config(), tcp.compat_write())
            .await
            .unwrap();
        let mut driver = MssqlDriver { client };

        let options = ExecOptions {
            max_rows: 2,
            timeout_secs: 30,
            one_statement: false,
        };
        let mut sink = BufferSink::new(options.max_rows);
        let stopped = driver
            .stream_sets("SELECT a FROM b", &[], &options, &mut sink, true)
            .await
            .unwrap()
            .stopped;
        let response = sink.into_response(RunSummary::default());

        assert!(!stopped);
        // The set holds the rows of the limit and no more, and it reports
        // that the limit stopped the read.
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].rows.len(), 2);
        assert!(response.results[0].truncated);
        let messages: Vec<&str> = response
            .messages
            .iter()
            .map(|message| message.text.as_str())
            .collect();
        assert!(messages
            .iter()
            .any(|text| text.contains("The row limit stopped the read")));
        assert!(messages.contains(&ENDED_AT_THE_LIMIT_MESSAGE));

        // The session runs a second statement on the same connection.
        let mut next = BufferSink::new(options.max_rows);
        driver
            .stream_sets("SELECT 1", &[], &options, &mut next, true)
            .await
            .unwrap();

        server.await.unwrap();
    }

    /// Answers a run that asks for the actual plan. The server sends five
    /// rows of the statement and then waits. An attention packet ends the
    /// batch without the plan set, in the way the real server ends it. When
    /// no attention arrives, the plan set follows the rows. Gives back true
    /// when the attention packet came.
    async fn serve_plan_run(listener: TcpListener) -> bool {
        let (mut socket, _) = listener.accept().await.unwrap();
        accept_login(&mut socket).await;

        // The switch that turns the plan on.
        read_message(&mut socket).await;
        write_packet(&mut socket, END_OF_MESSAGE, &done_token(0, 0)).await;

        let kind = read_message(&mut socket).await;
        assert!(kind == PACKET_RPC || kind == PACKET_SQL_BATCH);
        let mut rows = int_metadata();
        for value in 0..5 {
            rows.extend_from_slice(&int_row(value));
        }
        write_packet(&mut socket, 0, &rows).await;

        let pause = Duration::from_millis(300);
        let ended_early = match tokio::time::timeout(pause, read_message(&mut socket)).await {
            Ok(kind) => {
                assert_eq!(kind, PACKET_ATTENTION);
                write_packet(&mut socket, END_OF_MESSAGE, &done_token(DONE_ATTENTION, 0)).await;
                true
            }
            Err(_) => {
                let mut plan = done_in_proc(5);
                plan.extend_from_slice(&xml_metadata_named(PLAN_COLUMN));
                plan.extend_from_slice(&xml_row("<ShowPlanXML />"));
                plan.extend_from_slice(&done_token(0, 1));
                write_packet(&mut socket, END_OF_MESSAGE, &plan).await;
                false
            }
        };

        // The switch that turns the plan off.
        read_message(&mut socket).await;
        write_packet(&mut socket, END_OF_MESSAGE, &done_token(0, 0)).await;
        ended_early
    }

    #[tokio::test]
    async fn the_actual_plan_arrives_when_the_rows_pass_the_limit() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(serve_plan_run(listener));

        let tcp = TcpStream::connect(address).await.unwrap();
        let client = Client::connect(test_config(), tcp.compat_write())
            .await
            .unwrap();
        let mut driver = MssqlDriver { client };

        let options = ExecOptions {
            max_rows: 2,
            timeout_secs: 30,
            one_statement: false,
        };
        let response = driver
            .explain("SELECT a FROM b", None, PlanKind::Actual, &options)
            .await
            .unwrap();

        // The walk keeps to the end of the stream, so no attention packet
        // reaches the server and the plan set arrives behind the rows.
        assert!(!server.await.unwrap());
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].columns[0].name, PLAN_COLUMN);
        assert_eq!(
            response.results[0].rows[0][0],
            JsonValue::String("<ShowPlanXML />".into())
        );
        assert!(response.messages.is_empty());
    }

    /// Answers each request with one result set of one row, and gives back
    /// how many requests arrived. The wait ends when no request comes for a
    /// moment, so a driver that sends too few does not hold the test.
    async fn serve_and_count(listener: TcpListener) -> usize {
        let (mut socket, _) = listener.accept().await.unwrap();
        accept_login(&mut socket).await;

        let mut count = 0usize;
        let pause = Duration::from_millis(300);
        while let Ok(kind) = tokio::time::timeout(pause, read_message(&mut socket)).await {
            assert!(kind == PACKET_RPC || kind == PACKET_SQL_BATCH);
            let mut answer = int_metadata();
            answer.extend_from_slice(&int_row(count as i32));
            answer.extend_from_slice(&done_token(0, 1));
            write_packet(&mut socket, END_OF_MESSAGE, &answer).await;
            count += 1;
        }
        count
    }

    /// Opens a driver against a fake server that counts the requests.
    async fn driver_that_counts() -> (MssqlDriver, tokio::task::JoinHandle<usize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(serve_and_count(listener));
        let tcp = TcpStream::connect(address).await.unwrap();
        let client = Client::connect(test_config(), tcp.compat_write())
            .await
            .unwrap();
        (MssqlDriver { client }, server)
    }

    #[tokio::test]
    async fn a_batch_goes_to_the_server_whole_and_the_word_go_ends_it() {
        let (mut driver, server) = driver_that_counts().await;
        let options = ExecOptions {
            max_rows: 10,
            timeout_secs: 30,
            one_statement: false,
        };
        let mut sink = BufferSink::new(options.max_rows);

        let summary = driver
            .execute_stream(
                "DECLARE @x int = 1;\nSELECT @x;\nGO\nSELECT 2;",
                None,
                &options,
                &mut sink,
            )
            .await
            .unwrap();

        // The two statements of the first batch travel together, so the
        // variable of the first holds for the second. The second batch reads
        // alone, so the probe of the row limit goes before it.
        assert_eq!(server.await.unwrap(), 3);
        assert_eq!(sink.into_response(summary).results.len(), 2);
    }

    #[tokio::test]
    async fn a_count_after_the_word_go_runs_the_batch_again() {
        let (mut driver, server) = driver_that_counts().await;
        let options = ExecOptions {
            max_rows: 10,
            timeout_secs: 30,
            one_statement: false,
        };
        let mut sink = BufferSink::new(options.max_rows);

        driver
            .execute_stream("SELECT 1;\nGO 3\n", None, &options, &mut sink)
            .await
            .unwrap();

        // One probe serves the three runs of the batch.
        assert_eq!(server.await.unwrap(), 4);
    }

    /// An `ERROR` or an `INFO` token with the given text and severity.
    fn text_token(kind: u8, class: u8, text: &str) -> Vec<u8> {
        let utf16: Vec<u8> = text
            .encode_utf16()
            .flat_map(|unit| unit.to_le_bytes())
            .collect();
        let mut body = Vec::new();
        body.extend_from_slice(&50000u32.to_le_bytes());
        body.push(1);
        body.push(class);
        body.extend_from_slice(&((utf16.len() / 2) as u16).to_le_bytes());
        body.extend_from_slice(&utf16);
        body.push(0);
        body.push(0);
        body.extend_from_slice(&1u32.to_le_bytes());
        let mut token = vec![kind];
        token.extend_from_slice(&(body.len() as u16).to_le_bytes());
        token.extend_from_slice(&body);
        token
    }

    /// An `ERROR` token with the given text and a `DONE` token with the
    /// error flag after it.
    fn error_answer(text: &str) -> Vec<u8> {
        let mut token = text_token(0xAA, 16, text);
        token.extend_from_slice(&done_token(2, 0));
        token
    }

    /// How the fake server answers the probe of the row limit.
    enum Probe {
        Absent,
        Answer(i32),
        Fault,
    }

    /// Answers the probe as told, then answers the statement with five rows.
    /// With `attention`, the server keeps the statement running until the
    /// attention packet arrives. Without it, the whole answer arrives at
    /// once, and a test that sends the packet leaves it unread.
    async fn serve_probe_then_rows(listener: TcpListener, probe: Probe, attention: bool) {
        let (mut socket, _) = listener.accept().await.unwrap();
        accept_login(&mut socket).await;

        match probe {
            Probe::Absent => {}
            Probe::Answer(value) => {
                assert_eq!(read_message(&mut socket).await, PACKET_SQL_BATCH);
                let mut answer = int_metadata();
                answer.extend_from_slice(&int_row(value));
                answer.extend_from_slice(&done_token(0, 1));
                write_packet(&mut socket, END_OF_MESSAGE, &answer).await;
            }
            Probe::Fault => {
                assert_eq!(read_message(&mut socket).await, PACKET_SQL_BATCH);
                write_packet(&mut socket, END_OF_MESSAGE, &error_answer("no")).await;
            }
        }

        let kind = read_message(&mut socket).await;
        assert!(kind == PACKET_RPC || kind == PACKET_SQL_BATCH);
        let mut answer = int_metadata();
        for value in 0..5 {
            answer.extend_from_slice(&int_row(value));
        }
        if attention {
            write_packet(&mut socket, 0, &answer).await;
            assert_eq!(read_message(&mut socket).await, PACKET_ATTENTION);
            write_packet(&mut socket, END_OF_MESSAGE, &done_token(DONE_ATTENTION, 0)).await;
        } else {
            answer.extend_from_slice(&done_token(0, 5));
            write_packet(&mut socket, END_OF_MESSAGE, &answer).await;
        }
    }

    /// Runs the query against a fake server that answers the probe as told,
    /// and gives back the response.
    async fn run_with_probe(query: &str, probe: Probe, attention: bool) -> QueryResponse {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(serve_probe_then_rows(listener, probe, attention));
        let tcp = TcpStream::connect(address).await.unwrap();
        let client = Client::connect(test_config(), tcp.compat_write())
            .await
            .unwrap();
        let mut driver = MssqlDriver { client };

        let options = ExecOptions {
            max_rows: 2,
            timeout_secs: 30,
            one_statement: false,
        };
        let mut sink = BufferSink::new(options.max_rows);
        let summary = driver
            .execute_stream(query, None, &options, &mut sink)
            .await
            .unwrap();
        server.await.unwrap();
        sink.into_response(summary)
    }

    fn ended_at_the_limit(response: &QueryResponse) -> bool {
        response
            .messages
            .iter()
            .any(|message| message.text == ENDED_AT_THE_LIMIT_MESSAGE)
    }

    #[tokio::test]
    async fn a_read_outside_a_transaction_ends_at_the_row_limit() {
        let response = run_with_probe("SELECT a FROM b", Probe::Answer(1), true).await;

        assert_eq!(response.results[0].rows.len(), 2);
        assert!(response.results[0].truncated);
        assert!(ended_at_the_limit(&response));
    }

    #[tokio::test]
    async fn a_read_that_would_roll_back_the_transaction_walks_to_its_end() {
        let response = run_with_probe("SELECT a FROM b", Probe::Answer(0), false).await;

        assert_eq!(response.results[0].rows.len(), 2);
        assert!(response.results[0].truncated);
        assert!(!ended_at_the_limit(&response));
    }

    #[tokio::test]
    async fn a_probe_that_fails_keeps_the_walk() {
        let response = run_with_probe("SELECT a FROM b", Probe::Fault, false).await;

        assert_eq!(response.results[0].rows.len(), 2);
        assert!(!ended_at_the_limit(&response));
    }

    #[tokio::test]
    async fn a_write_that_returns_rows_needs_no_probe_and_walks_to_its_end() {
        let response = run_with_probe(
            "SELECT a INTO c FROM b; SELECT a FROM c",
            Probe::Absent,
            false,
        )
        .await;
        assert!(!ended_at_the_limit(&response));

        let response = run_with_probe("SELECT a INTO c FROM b", Probe::Absent, false).await;
        assert!(!ended_at_the_limit(&response));
    }

    /// A `DONEINPROC` token with the `Count` flag and the given count.
    fn counted_done_in_proc(rows: u64) -> Vec<u8> {
        let mut token = vec![0xFF];
        token.extend_from_slice(&(DONE_MORE | DONE_COUNT).to_le_bytes());
        token.extend_from_slice(&0u16.to_le_bytes());
        token.extend_from_slice(&rows.to_le_bytes());
        token
    }

    /// Runs the query against a fake server that sends the given answer to
    /// the first request. The query must not be one that only reads, so that
    /// no probe of the row limit goes before it.
    async fn run_against_answer(query: &str, answer: Vec<u8>, sink_rows: usize) -> QueryResponse {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            accept_login(&mut socket).await;
            let kind = read_message(&mut socket).await;
            assert!(kind == PACKET_RPC || kind == PACKET_SQL_BATCH);
            write_packet(&mut socket, END_OF_MESSAGE, &answer).await;
        });
        let tcp = TcpStream::connect(address).await.unwrap();
        let client = Client::connect(test_config(), tcp.compat_write())
            .await
            .unwrap();
        let mut driver = MssqlDriver { client };

        let options = ExecOptions {
            max_rows: 10,
            timeout_secs: 30,
            one_statement: false,
        };
        let mut sink = BufferSink::new(sink_rows);
        let summary = driver
            .execute_stream(query, None, &options, &mut sink)
            .await
            .unwrap();
        server.await.unwrap();
        sink.into_response(summary)
    }

    fn message_texts(response: &QueryResponse) -> Vec<&str> {
        response
            .messages
            .iter()
            .map(|message| message.text.as_str())
            .collect()
    }

    #[tokio::test]
    async fn an_insert_with_output_shows_its_rows() {
        let mut answer = int_metadata();
        answer.extend_from_slice(&int_row(7));
        answer.extend_from_slice(&done_token(DONE_COUNT, 1));

        let response =
            run_against_answer("INSERT INTO t(a) OUTPUT inserted.a VALUES (7)", answer, 10).await;

        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].rows, vec![vec![JsonValue::from(7)]]);
        assert_eq!(message_texts(&response), ["1 row returned."]);
        assert_eq!(response.rows_affected, None);
    }

    /// A `COLMETADATA` token for one column `g` of the user-defined type
    /// `sys.hierarchyid`.
    fn udt_metadata() -> Vec<u8> {
        let mut token = vec![0x81];
        token.extend_from_slice(&1u16.to_le_bytes());
        token.extend_from_slice(&0u32.to_le_bytes());
        token.extend_from_slice(&0u16.to_le_bytes());
        token.push(0xF0);
        token.extend_from_slice(&892u16.to_le_bytes());
        for name in ["db", "sys", "hierarchyid"] {
            token.push(name.len() as u8);
            name.encode_utf16()
                .for_each(|unit| token.extend_from_slice(&unit.to_le_bytes()));
        }
        token.extend_from_slice(&1u16.to_le_bytes());
        token.extend_from_slice(&('h' as u16).to_le_bytes());
        token.push(1);
        token.extend_from_slice(&('g' as u16).to_le_bytes());
        token
    }

    #[tokio::test]
    async fn a_user_defined_type_shows_its_bytes() {
        let mut answer = udt_metadata();
        // A value of a user-defined type comes in chunks, as a `varbinary(max)`
        // value does, whatever the maximum size of the type is.
        answer.push(0xD1);
        answer.extend_from_slice(&2u64.to_le_bytes());
        answer.extend_from_slice(&2u32.to_le_bytes());
        answer.extend_from_slice(&[0x58, 0x40]);
        answer.extend_from_slice(&0u32.to_le_bytes());
        answer.push(0xD1);
        answer.extend_from_slice(&u64::MAX.to_le_bytes());
        answer.extend_from_slice(&done_token(DONE_COUNT, 2));

        let response = run_against_answer(
            "INSERT INTO t(g) OUTPUT inserted.g VALUES (0x5840)",
            answer,
            10,
        )
        .await;

        assert_eq!(response.results[0].columns[0].type_name, "udt");
        assert_eq!(
            response.results[0].rows,
            vec![vec![JsonValue::from("WEA=")], vec![JsonValue::Null]]
        );
    }

    #[tokio::test]
    async fn a_block_shows_its_counts_and_its_rows() {
        // The server sends a `DONEINPROC` token for each statement of a
        // request that runs through `sp_executesql`.
        let mut answer = counted_done_in_proc(3);
        answer.extend_from_slice(&int_metadata());
        answer.extend_from_slice(&int_row(1));
        answer.extend_from_slice(&counted_done_in_proc(1));
        answer.extend_from_slice(&counted_done_in_proc(2));
        answer.extend_from_slice(&done_token(0, 0));

        let response = run_against_answer(
            "BEGIN TRY UPDATE t SET a = 1; SELECT a FROM t; DELETE FROM u END TRY \
             BEGIN CATCH SELECT 0 END CATCH",
            answer,
            10,
        )
        .await;

        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].rows.len(), 1);
        assert_eq!(
            message_texts(&response),
            ["3 rows affected.", "1 row returned.", "2 rows affected."]
        );
        assert_eq!(response.rows_affected, Some(5));
    }

    #[tokio::test]
    async fn a_message_and_a_count_before_the_first_set_arrive() {
        let mut answer = text_token(0xAB, 0, "hello");
        answer.extend_from_slice(&counted_done_in_proc(0));
        answer.extend_from_slice(&int_metadata());
        answer.extend_from_slice(&int_row(1));
        answer.extend_from_slice(&done_token(DONE_COUNT, 1));

        let response = run_against_answer(
            "PRINT 'hello'; DELETE FROM t WHERE 1 = 0; SELECT a FROM t",
            answer,
            10,
        )
        .await;

        assert_eq!(response.results.len(), 1);
        assert_eq!(
            message_texts(&response),
            ["hello", "0 rows affected.", "1 row returned."]
        );
        assert_eq!(response.rows_affected, Some(0));
    }

    #[tokio::test]
    async fn a_statement_without_a_count_adds_no_message() {
        let response = run_against_answer("CREATE TABLE t (a int)", done_token(0, 0), 10).await;

        assert!(response.results.is_empty());
        assert!(response.messages.is_empty());
        assert_eq!(response.rows_affected, None);
    }

    #[tokio::test]
    async fn a_count_after_the_sink_stops_adds_no_message() {
        let mut answer = int_metadata();
        answer.extend_from_slice(&int_row(1));
        answer.extend_from_slice(&int_row(2));
        answer.extend_from_slice(&counted_done_in_proc(2));
        answer.extend_from_slice(&counted_done_in_proc(4));
        answer.extend_from_slice(&done_token(0, 0));

        let response = run_against_answer("SELECT a FROM t; UPDATE t SET a = 1", answer, 1).await;

        assert_eq!(response.results[0].rows.len(), 1);
        assert!(response.results[0].truncated);
        assert_eq!(
            message_texts(&response),
            ["1 row returned. The row limit stopped the read."]
        );
        assert_eq!(response.rows_affected, None);
    }

    #[tokio::test]
    async fn a_run_with_parameters_refuses_a_script_of_several_batches() {
        let (mut driver, server) = driver_that_counts().await;
        let options = ExecOptions {
            max_rows: 10,
            timeout_secs: 30,
            one_statement: false,
        };
        let mut sink = BufferSink::new(options.max_rows);
        let params = vec![crate::db::QueryParam {
            value: JsonValue::from(1),
        }];

        let error = driver
            .execute_stream(
                "SELECT @P1;\nGO\nSELECT @P1;",
                Some(&params),
                &options,
                &mut sink,
            )
            .await
            .unwrap_err();

        assert!(error.to_string().contains("GO"), "{error}");
        assert_eq!(server.await.unwrap(), 0);
    }

    #[tokio::test]
    async fn an_xml_column_shows_its_text() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            accept_login(&mut socket).await;

            let kind = read_message(&mut socket).await;
            assert!(kind == PACKET_RPC || kind == PACKET_SQL_BATCH);
            let mut answer = xml_metadata();
            answer.extend_from_slice(&xml_row("<a>1</a>"));
            answer.extend_from_slice(&done_token(0, 1));
            write_packet(&mut socket, END_OF_MESSAGE, &answer).await;
        });

        let tcp = TcpStream::connect(address).await.unwrap();
        let client = Client::connect(test_config(), tcp.compat_write())
            .await
            .unwrap();
        let mut driver = MssqlDriver { client };

        let options = ExecOptions {
            max_rows: 10,
            timeout_secs: 30,
            one_statement: false,
        };
        let mut sink = BufferSink::new(options.max_rows);
        driver
            .stream_sets("SELECT x FROM b", &[], &options, &mut sink, false)
            .await
            .unwrap();
        let response = sink.into_response(RunSummary::default());

        assert_eq!(
            response.results[0].rows[0][0],
            JsonValue::String("<a>1</a>".into())
        );

        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_sql_variant_column_shows_the_value_of_each_row() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            accept_login(&mut socket).await;

            let kind = read_message(&mut socket).await;
            assert!(kind == PACKET_RPC || kind == PACKET_SQL_BATCH);
            let text: Vec<u8> = "hi".encode_utf16().flat_map(u16::to_le_bytes).collect();
            let mut answer = variant_metadata();
            answer.extend_from_slice(&variant_row(0x38, &[], &42i32.to_le_bytes()));
            answer.extend_from_slice(&variant_row(0xE7, &[0, 0, 0, 0, 0, 0x40, 0x1F], &text));
            answer.extend_from_slice(&variant_null_row());
            answer.extend_from_slice(&done_token(0, 3));
            write_packet(&mut socket, END_OF_MESSAGE, &answer).await;
        });

        let tcp = TcpStream::connect(address).await.unwrap();
        let client = Client::connect(test_config(), tcp.compat_write())
            .await
            .unwrap();
        let mut driver = MssqlDriver { client };

        let options = ExecOptions {
            max_rows: 10,
            timeout_secs: 30,
            one_statement: false,
        };
        let mut sink = BufferSink::new(options.max_rows);
        driver
            .stream_sets("SELECT v FROM b", &[], &options, &mut sink, false)
            .await
            .unwrap();
        let response = sink.into_response(RunSummary::default());

        assert_eq!(response.results[0].columns[0].type_name, "sql_variant");
        assert_eq!(response.results[0].rows[0][0], JsonValue::from(42));
        assert_eq!(
            response.results[0].rows[1][0],
            JsonValue::String("hi".into())
        );
        assert_eq!(response.results[0].rows[2][0], JsonValue::Null);

        server.await.unwrap();
    }

    #[test]
    fn the_data_of_a_cell_covers_every_form_of_value() {
        use std::borrow::Cow;
        use tiberius::time::{Date, DateTime, DateTime2, DateTimeOffset, SmallDateTime, Time};

        assert_eq!(
            column_data_to_json(&ColumnData::U8(Some(1))),
            JsonValue::from(1)
        );
        assert_eq!(
            column_data_to_json(&ColumnData::I16(Some(-2))),
            JsonValue::from(-2)
        );
        assert_eq!(
            column_data_to_json(&ColumnData::I32(Some(3))),
            JsonValue::from(3)
        );
        assert_eq!(
            column_data_to_json(&ColumnData::I64(Some(4))),
            JsonValue::from(4)
        );
        assert_eq!(
            column_data_to_json(&ColumnData::F32(Some(1.5))),
            JsonValue::from(1.5)
        );
        assert_eq!(
            column_data_to_json(&ColumnData::F64(Some(2.5))),
            JsonValue::from(2.5)
        );
        assert_eq!(
            column_data_to_json(&ColumnData::Bit(Some(true))),
            JsonValue::Bool(true)
        );
        assert_eq!(
            column_data_to_json(&ColumnData::String(Some(Cow::from("a")))),
            JsonValue::String("a".into())
        );
        assert_eq!(
            column_data_to_json(&ColumnData::Guid(Some(uuid::Uuid::nil()))),
            JsonValue::String("00000000-0000-0000-0000-000000000000".into())
        );
        assert_eq!(
            column_data_to_json(&ColumnData::Binary(Some(Cow::from(vec![1u8])))),
            bytes_to_json(&[1])
        );
        assert_eq!(
            column_data_to_json(&ColumnData::Numeric(Some(Numeric::new_with_scale(125, 2)))),
            JsonValue::String("1.25".into())
        );
        assert_eq!(
            column_data_to_json(&ColumnData::Xml(Some(Cow::Owned(XmlData::new("<a/>"))))),
            JsonValue::String("<a/>".into())
        );
        assert_eq!(
            column_data_to_json(&ColumnData::DateTime(Some(DateTime::new(0, 0)))),
            JsonValue::String("1900-01-01 00:00:00".into())
        );
        assert_eq!(
            column_data_to_json(&ColumnData::SmallDateTime(Some(SmallDateTime::new(0, 0)))),
            JsonValue::String("1900-01-01 00:00:00".into())
        );
        assert_eq!(
            column_data_to_json(&ColumnData::DateTime2(Some(DateTime2::new(
                Date::new(0),
                Time::new(0, 0)
            )))),
            JsonValue::String("0001-01-01 00:00:00".into())
        );
        assert_eq!(
            column_data_to_json(&ColumnData::Date(Some(Date::new(0)))),
            JsonValue::String("0001-01-01".into())
        );
        assert_eq!(
            column_data_to_json(&ColumnData::Time(Some(Time::new(0, 0)))),
            JsonValue::String("00:00:00".into())
        );
        assert_eq!(
            column_data_to_json(&ColumnData::DateTimeOffset(Some(DateTimeOffset::new(
                DateTime2::new(Date::new(730119), Time::new(0, 0)),
                0
            )))),
            JsonValue::String("2000-01-01T00:00:00+00:00".into())
        );
        // The server sends the moment in UTC with the offset beside it, and
        // the text keeps the offset.
        assert_eq!(
            column_data_to_json(&ColumnData::DateTimeOffset(Some(DateTimeOffset::new(
                DateTime2::new(Date::new(730119), Time::new(14 * 3600 * 10_000_000, 7)),
                -300
            )))),
            JsonValue::String("2000-01-01T09:00:00-05:00".into())
        );
        // A four-byte float gives the digits that it shows, and a money
        // value keeps each of its digits.
        assert_eq!(
            column_data_to_json(&ColumnData::F32(Some(0.1))),
            serde_json::json!(0.1)
        );
        assert_eq!(
            column_data_to_json(&ColumnData::Numeric(Some(Numeric::new_with_scale(
                i64::MAX as i128,
                4
            )))),
            JsonValue::String("922337203685477.5807".into())
        );
        assert_eq!(
            column_data_to_json(&ColumnData::Numeric(Some(Numeric::new_with_scale(
                -15000, 4
            )))),
            JsonValue::String("-1.5000".into())
        );

        // A value that is absent gives the null of JSON in every form.
        for data in [
            ColumnData::U8(None),
            ColumnData::I16(None),
            ColumnData::I32(None),
            ColumnData::I64(None),
            ColumnData::F32(None),
            ColumnData::F64(None),
            ColumnData::Bit(None),
            ColumnData::String(None),
            ColumnData::Guid(None),
            ColumnData::Binary(None),
            ColumnData::Numeric(None),
            ColumnData::Xml(None),
            ColumnData::DateTime(None),
            ColumnData::SmallDateTime(None),
            ColumnData::DateTime2(None),
            ColumnData::Date(None),
            ColumnData::Time(None),
            ColumnData::DateTimeOffset(None),
        ] {
            assert_eq!(column_data_to_json(&data), JsonValue::Null);
        }
    }

    #[tokio::test]
    async fn a_batch_of_several_statements_walks_the_rest_of_the_result() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            accept_login(&mut socket).await;

            // The whole answer arrives at once, and the server waits for no
            // attention packet.
            let kind = read_message(&mut socket).await;
            assert!(kind == PACKET_RPC || kind == PACKET_SQL_BATCH);
            let mut answer = int_metadata();
            for value in 0..5 {
                answer.extend_from_slice(&int_row(value));
            }
            answer.extend_from_slice(&done_token(0, 5));
            write_packet(&mut socket, END_OF_MESSAGE, &answer).await;
        });

        let tcp = TcpStream::connect(address).await.unwrap();
        let client = Client::connect(test_config(), tcp.compat_write())
            .await
            .unwrap();
        let mut driver = MssqlDriver { client };

        let options = ExecOptions {
            max_rows: 2,
            timeout_secs: 30,
            one_statement: false,
        };
        let mut sink = BufferSink::new(options.max_rows);
        driver
            .stream_sets("SELECT a FROM b", &[], &options, &mut sink, false)
            .await
            .unwrap();
        let response = sink.into_response(RunSummary::default());

        // The set still holds the rows of the limit and reports the warning.
        assert_eq!(response.results[0].rows.len(), 2);
        assert!(response.results[0].truncated);
        // The statement ran to its end, so no message names an early end.
        assert!(!response
            .messages
            .iter()
            .any(|message| message.text == ENDED_AT_THE_LIMIT_MESSAGE));

        server.await.unwrap();
    }

    #[tokio::test]
    async fn an_error_after_the_first_one_of_a_batch_goes_among_the_messages() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            accept_login(&mut socket).await;
            read_message(&mut socket).await;
            let mut answer = error_token(8134, "Divide by zero error encountered.");
            answer.extend_from_slice(&done_token(0x02, 0));
            answer.extend_from_slice(&done_token(0x10, 3));
            answer.extend_from_slice(&error_token(547, "The DELETE statement conflicted."));
            answer.extend_from_slice(&done_token(0x02, 0));
            write_packet(&mut socket, END_OF_MESSAGE, &answer).await;
        });

        let tcp = TcpStream::connect(address).await.unwrap();
        let client = Client::connect(test_config(), tcp.compat_write())
            .await
            .unwrap();
        let mut driver = MssqlDriver { client };
        let options = ExecOptions {
            max_rows: 100,
            timeout_secs: 30,
            one_statement: false,
        };
        let mut sink = BufferSink::new(options.max_rows);
        let outcome = driver
            .stream_sets(
                "INSERT INTO t VALUES (1/0); UPDATE t SET a = 1; DELETE FROM u",
                &[],
                &options,
                &mut sink,
                false,
            )
            .await;
        let response = sink.into_response(RunSummary::default());

        // The first error is the error of the run.
        let error = outcome.err().unwrap();
        assert!(error.to_string().contains("Divide by zero"));
        // The count of the statement between the errors and the later error
        // stand among the messages.
        assert!(response
            .messages
            .iter()
            .any(|message| message.text == rows_affected_message(3).text));
        let later = response
            .messages
            .iter()
            .find(|message| message.level == MessageLevel::Error)
            .unwrap();
        assert_eq!(later.text, "The DELETE statement conflicted.");
        assert_eq!(
            later.detail.as_deref(),
            Some("Msg 547, Level 16, State 1, Line 2")
        );

        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_sink_that_stops_ends_the_statement_as_well() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(serve_rows_until_attention(listener));

        let tcp = TcpStream::connect(address).await.unwrap();
        let client = Client::connect(test_config(), tcp.compat_write())
            .await
            .unwrap();
        let mut driver = MssqlDriver { client };

        // The limit of the sink is below the limit of the run, so the sink
        // stops the read before the driver reaches its own limit.
        let options = ExecOptions {
            max_rows: 10,
            timeout_secs: 30,
            one_statement: false,
        };
        let mut sink = BufferSink::new(2);
        let stopped = driver
            .stream_sets("SELECT a FROM b", &[], &options, &mut sink, true)
            .await
            .unwrap()
            .stopped;
        let response = sink.into_response(RunSummary::default());

        assert!(stopped);
        assert_eq!(response.results[0].rows.len(), 2);
        assert!(response.results[0].truncated);
        // The words of the row limit belong to the limit of the run alone.
        assert!(!response
            .messages
            .iter()
            .any(|message| message.text == ENDED_AT_THE_LIMIT_MESSAGE));

        let mut next = BufferSink::new(options.max_rows);
        driver
            .stream_sets("SELECT 1", &[], &options, &mut next, true)
            .await
            .unwrap();

        server.await.unwrap();
    }

    #[tokio::test]
    async fn the_cancel_handle_signals_and_reports_no_fault() {
        let handle = MssqlCancel(Arc::new(AttentionHandle::default()));
        handle.cancel().await.unwrap();
        // A second request for a statement that already stopped does no harm.
        handle.cancel().await.unwrap();
    }

    #[test]
    fn each_plan_has_its_own_session_switch() {
        assert_eq!(plan_switch(PlanKind::Estimated), "SHOWPLAN_XML");
        assert_eq!(plan_switch(PlanKind::Actual), "STATISTICS XML");
    }

    #[test]
    fn the_plan_sets_of_a_run_are_kept_and_the_rows_are_dropped() {
        let mut rows = ResultSet::new(vec![ColumnInfo::new("id", "int")]);
        rows.rows.push(vec![serde_json::json!(1)]);
        let plan = ResultSet::new(vec![ColumnInfo::new(PLAN_COLUMN, "xml")]);

        assert!(is_plan_set(&plan));
        assert!(!is_plan_set(&rows));

        let (kept, found) = select_plan_sets(vec![rows.clone(), plan]);
        assert!(found);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].columns[0].name, PLAN_COLUMN);

        // A run that held no plan keeps every set, and the caller says so.
        let (kept, found) = select_plan_sets(vec![rows]);
        assert!(!found);
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn the_fact_statement_reads_the_rows_the_pages_and_the_change() {
        let text = fact_query("[Sales]");
        assert!(text.contains("FROM [Sales].sys.dm_db_partition_stats AS s"));
        assert!(text.contains("JOIN [Sales].sys.objects AS o"));
        assert!(text.contains("WHERE s.object_id = OBJECT_ID(@P1)"));
    }

    #[test]
    fn the_snapshot_statement_reads_the_columns_and_the_kind() {
        let text = snapshot_query("[Sales]");
        assert!(text.contains("FROM [Sales].INFORMATION_SCHEMA.COLUMNS AS c"));
        assert!(text.contains("JOIN [Sales].INFORMATION_SCHEMA.TABLES AS t"));
        assert!(text.contains("ORDER BY c.TABLE_SCHEMA, c.TABLE_NAME, c.ORDINAL_POSITION"));
    }

    #[test]
    fn the_catalog_statements_name_the_database_of_the_connection() {
        let schemas = schema_query("[Sales]");
        assert!(schemas.contains("FROM [Sales].sys.schemas AS s"));
        assert!(schemas.contains("NOT BETWEEN 16384 AND 16399"));
        assert!(!schemas.contains("database_principals"));
        assert!(!schemas.contains("LIKE"));

        let routines = routine_query("[Sales]");
        assert!(routines.contains("FROM [Sales].INFORMATION_SCHEMA.ROUTINES"));
        assert!(routines.contains("WHERE ROUTINE_SCHEMA = @P1"));

        let indexes = index_query("[Sales]");
        assert!(indexes.contains("FROM [Sales].sys.indexes AS i"));
        assert!(indexes.contains("OBJECT_ID(@P1)"));
        assert!(indexes.contains("ic.is_included_column FROM"));
        assert!(indexes.contains(
            "ORDER BY i.name, ic.is_included_column, ic.key_ordinal, ic.index_column_id"
        ));

        let constraints = constraint_query("[Sales]");
        assert!(constraints.contains("FROM [Sales].INFORMATION_SCHEMA.TABLE_CONSTRAINTS AS tc"));
        assert!(constraints.contains("cc.CHECK_CLAUSE"));
        assert!(!constraints.contains("OBJECT_NAME"));
        assert!(constraints.contains("JOIN [Sales].sys.schemas AS fs"));
        assert!(constraints.contains("fs.name = tc.CONSTRAINT_SCHEMA"));
        assert!(constraints.contains(
            "LEFT JOIN [Sales].sys.objects AS ro ON ro.object_id = fk.referenced_object_id"
        ));
        assert!(constraints.contains("WHERE tc.TABLE_SCHEMA = @P1 AND tc.TABLE_NAME = @P2"));
    }

    #[test]
    fn the_create_statement_covers_a_view_alone() {
        let view = create_query_text(Some("db"), Some("dbo"), "v", TableKind::View).unwrap();
        assert_eq!(
            view.sql,
            "SELECT m.definition FROM [db].sys.sql_modules AS m \
             WHERE m.object_id = OBJECT_ID('[db].[dbo].[v]');"
        );
        assert_eq!(view.column, 0);
        // With no database the statement reads the current one.
        let current = create_query_text(None, Some("dbo"), "v", TableKind::View).unwrap();
        assert_eq!(
            current.sql,
            "SELECT m.definition FROM sys.sql_modules AS m \
             WHERE m.object_id = OBJECT_ID('[dbo].[v]');"
        );
        assert!(create_query_text(Some("db"), Some("dbo"), "t", TableKind::Table).is_none());
    }
    use crate::storage::{ConnectionOptions, DbType};

    fn connection() -> SavedConnection {
        SavedConnection {
            id: "id".into(),
            name: "name".into(),
            db_type: DbType::Mssql,
            host: Some("sql.example.com".into()),
            port: Some(14330),
            user: Some("sa".into()),
            database: Some("Sales".into()),
            password: Some("p;a{s}s".into()),
            aws_secret_access_key: None,
            aws_session_token: None,
            options: ConnectionOptions::default(),
            color: None,
            group: None,
        }
    }

    #[tokio::test]
    async fn the_configuration_keeps_the_port() {
        let config = build_config(&connection()).await.unwrap();
        assert_eq!(config.get_addr(), "sql.example.com:14330");
    }

    #[tokio::test]
    async fn the_configuration_uses_the_default_port_when_none_is_given() {
        let mut input = connection();
        input.port = None;
        let config = build_config(&input).await.unwrap();
        assert_eq!(config.get_addr(), "sql.example.com:1433");
    }

    #[tokio::test]
    async fn the_configuration_accepts_an_empty_host() {
        let mut input = connection();
        input.host = None;
        let config = build_config(&input).await.unwrap();
        assert_eq!(config.get_addr(), "localhost:14330");
    }

    #[tokio::test]
    async fn a_connection_string_replaces_the_fields() {
        let mut input = connection();
        input.options.connection_url = Some("server=tcp:other.example.com,4200".into());
        let config = build_config(&input).await.unwrap();
        assert_eq!(config.get_addr(), "other.example.com:4200");
    }

    #[tokio::test]
    async fn a_jdbc_connection_string_is_accepted() {
        let mut input = connection();
        input.options.connection_url =
            Some("jdbc:sqlserver://other.example.com:4300;database=Sales".into());
        let config = build_config(&input).await.unwrap();
        assert_eq!(config.get_addr(), "other.example.com:4300");
    }

    #[tokio::test]
    async fn a_connection_string_that_is_not_valid_gives_an_error() {
        let mut input = connection();
        input.options.connection_url = Some("server=a,b,c".into());
        assert!(build_config(&input).await.is_err());
    }

    #[tokio::test]
    async fn a_connection_string_with_both_trust_options_gives_an_error() {
        let url = "server=tcp:a,1433;TrustServerCertificate=true;TrustServerCertificateCA=ca.crt";
        let mut input = connection();
        input.options.connection_url = Some(url.into());
        assert!(build_config(&input).await.is_err());
        assert!(string_has_password(url).is_err());
    }

    #[tokio::test]
    async fn a_connection_string_takes_the_login_of_the_record() {
        let login = |config: &Config| {
            let auth = config.get_authentication();
            (
                auth.user().unwrap_or("-").to_string(),
                auth.password().unwrap_or("-").to_string(),
            )
        };
        let mut input = connection();
        input.options.connection_url = Some("server=tcp:a,1433".into());
        let config = build_config(&input).await.unwrap();
        assert_eq!(login(&config), ("sa".into(), "p;a{s}s".into()));

        // The user of the string wins over the user of the record.
        input.options.connection_url = Some("server=tcp:a,1433;User ID=app".into());
        let config = build_config(&input).await.unwrap();
        assert_eq!(login(&config), ("app".into(), "p;a{s}s".into()));

        // A string that gives its own password keeps it.
        input.options.connection_url = Some("server=tcp:a,1433;uid=app;pwd=own".into());
        let config = build_config(&input).await.unwrap();
        assert_eq!(login(&config), ("app".into(), "own".into()));

        // With no password in the record, the string stays as it is.
        input.options.connection_url = Some("server=tcp:a,1433".into());
        input.password = Some(String::new());
        let config = build_config(&input).await.unwrap();
        assert_eq!(login(&config), ("".into(), "".into()));

        // A record without a user gives an empty user.
        input.password = Some("secret".into());
        input.user = None;
        let config = build_config(&input).await.unwrap();
        assert_eq!(login(&config), ("".into(), "secret".into()));
    }

    #[tokio::test]
    async fn a_connection_string_keeps_a_method_other_than_the_sql_login() {
        let mut input = connection();
        input.options.connection_url = Some("server=tcp:a,1433".into());
        input.options.mssql_auth = MssqlAuth::Integrated;
        let config = build_config(&input).await.unwrap();
        assert_eq!(config.get_authentication().password(), Some(""));

        // The string names a method that sends no password.
        input.options.mssql_auth = MssqlAuth::SqlLogin;
        let mut config = parse_string("server=tcp:a,1433").unwrap();
        config.authentication(AuthMethod::aad_token("t"));
        add_login_of_record(&mut config, &input);
        assert_eq!(config.get_authentication(), &AuthMethod::aad_token("t"));
    }

    #[test]
    fn a_password_in_a_connection_string_is_found() {
        assert!(string_has_password("server=tcp:a,1433;uid=u;Password=x").unwrap());
        assert!(string_has_password("jdbc:sqlserver://a:1433;pwd=x").unwrap());
        assert!(!string_has_password("server=tcp:a,1433;uid=u").unwrap());
        assert!(!string_has_password("server=tcp:a,1433;pwd=").unwrap());
        assert!(string_has_password("server=a,b,c").is_err());
    }

    #[tokio::test]
    async fn the_optional_fields_are_accepted() {
        let mut input = connection();
        input.options.instance_name = Some("SQLEXPRESS".into());
        input.options.ca_cert_path = Some("/etc/ca.pem".into());
        input.options.read_only = true;
        input.database = Some(String::new());
        input.options.application_name = Some("  ".into());
        assert!(build_config(&input).await.is_ok());
    }

    #[tokio::test]
    async fn the_read_only_switch_sets_the_intent_also_with_a_connection_string() {
        let mut input = connection();
        input.options.read_only = true;
        let debug = |config: Config| format!("{config:?}");
        assert!(debug(build_config(&input).await.unwrap()).contains("readonly: true"));
        input.options.connection_url = Some("server=tcp:a,1433".into());
        assert!(debug(build_config(&input).await.unwrap()).contains("readonly: true"));
        // A switch that is off keeps the intent that the string gives.
        input.options.read_only = false;
        input.options.connection_url = Some("server=tcp:a,1433;ApplicationIntent=ReadOnly".into());
        assert!(debug(build_config(&input).await.unwrap()).contains("readonly: true"));
        input.options.connection_url = Some("server=tcp:a,1433".into());
        assert!(debug(build_config(&input).await.unwrap()).contains("readonly: false"));
    }

    #[tokio::test]
    async fn a_ca_file_is_ignored_when_the_mode_does_not_verify() {
        let mut input = connection();
        input.options.tls_mode = TlsMode::Require;
        input.options.ca_cert_path = Some("/etc/ca.pem".into());
        assert!(build_config(&input).await.is_ok());
    }

    #[test]
    fn the_transport_setting_selects_the_encryption_level() {
        assert_eq!(
            encryption_level(TlsMode::Disable),
            EncryptionLevel::NotSupported
        );
        assert_eq!(encryption_level(TlsMode::Prefer), EncryptionLevel::Off);
        assert_eq!(
            encryption_level(TlsMode::Require),
            EncryptionLevel::Required
        );
        assert_eq!(
            encryption_level(TlsMode::VerifyFull),
            EncryptionLevel::Required
        );
    }

    #[tokio::test]
    async fn integrated_security_works_on_every_system() {
        let mut input = connection();
        input.options.mssql_auth = MssqlAuth::Integrated;
        // Windows reaches SSPI and every other system reaches Kerberos, so
        // the method is available everywhere.
        assert!(auth_method(&input).await.is_ok());
    }

    #[tokio::test]
    async fn a_missing_user_gives_an_empty_credential() {
        let mut input = connection();
        input.user = None;
        input.password = None;
        assert!(auth_method(&input).await.is_ok());
    }

    #[test]
    fn non_empty_removes_blank_values() {
        assert_eq!(non_empty(Some(" a ")), Some("a"));
        assert_eq!(non_empty(Some("   ")), None);
        assert_eq!(non_empty(None), None);
    }

    #[test]
    fn the_first_keyword_steps_over_comments_that_never_close() {
        use crate::sql::leading_keyword;
        assert_eq!(
            leading_keyword("-- only a comment", crate::sql::Dialect::MsSql),
            ""
        );
        assert_eq!(
            leading_keyword("/* never closed", crate::sql::Dialect::MsSql),
            ""
        );
        assert_eq!(
            leading_keyword("/* a */ /* b */ select", crate::sql::Dialect::MsSql),
            "select"
        );
    }

    #[test]
    fn the_parameters_accept_the_simple_json_types() {
        let params = vec![
            crate::db::QueryParam {
                value: serde_json::json!("text"),
            },
            crate::db::QueryParam {
                value: serde_json::json!(7),
            },
            crate::db::QueryParam {
                value: serde_json::json!(1.5),
            },
            crate::db::QueryParam {
                value: serde_json::json!(true),
            },
            crate::db::QueryParam {
                value: serde_json::Value::Null,
            },
        ];
        assert_eq!(bind_params(Some(&params)).unwrap().len(), 5);
        assert!(bind_params(None).unwrap().is_empty());
    }

    #[test]
    fn a_parameter_with_a_structured_type_is_refused() {
        let params = vec![crate::db::QueryParam {
            value: serde_json::json!({ "a": 1 }),
        }];
        assert_eq!(
            bind_params(Some(&params)).err().unwrap().kind(),
            crate::error::ErrorKind::Configuration
        );

        let params = vec![crate::db::QueryParam {
            value: serde_json::json!([1, 2]),
        }];
        assert!(bind_params(Some(&params)).is_err());
    }

    #[test]
    fn a_whole_number_outside_the_range_is_refused() {
        let params = vec![crate::db::QueryParam {
            value: serde_json::json!(18446744073709551615u64),
        }];
        assert_eq!(
            bind_params(Some(&params)).err().unwrap().kind(),
            crate::error::ErrorKind::Configuration
        );
    }

    #[test]
    fn the_type_name_covers_every_column_type() {
        let all = [
            ColumnType::Null,
            ColumnType::Bit,
            ColumnType::Bitn,
            ColumnType::Int1,
            ColumnType::Int2,
            ColumnType::Int4,
            ColumnType::Int8,
            ColumnType::Intn,
            ColumnType::Float4,
            ColumnType::Float8,
            ColumnType::Floatn,
            ColumnType::Money,
            ColumnType::Money4,
            ColumnType::Decimaln,
            ColumnType::Numericn,
            ColumnType::Guid,
            ColumnType::Datetime,
            ColumnType::Datetimen,
            ColumnType::Datetime4,
            ColumnType::Datetime2,
            ColumnType::DatetimeOffsetn,
            ColumnType::Daten,
            ColumnType::Timen,
            ColumnType::BigVarChar,
            ColumnType::BigChar,
            ColumnType::NVarchar,
            ColumnType::NChar,
            ColumnType::Text,
            ColumnType::NText,
            ColumnType::BigVarBin,
            ColumnType::BigBinary,
            ColumnType::Image,
            ColumnType::Xml,
            ColumnType::Udt,
            ColumnType::SSVariant,
        ];
        for column_type in all {
            assert!(!type_name(column_type).is_empty());
        }
        assert_eq!(type_name(ColumnType::Int8), "bigint");
        assert_eq!(type_name(ColumnType::Guid), "uniqueidentifier");
    }

    #[test]
    fn a_decimal_keeps_every_digit() {
        assert_eq!(
            numeric_to_string(Numeric::new_with_scale(12345, 2)),
            "123.45"
        );
        assert_eq!(
            numeric_to_string(Numeric::new_with_scale(-12345, 2)),
            "-123.45"
        );
        assert_eq!(numeric_to_string(Numeric::new_with_scale(5, 3)), "0.005");
        assert_eq!(numeric_to_string(Numeric::new_with_scale(-5, 3)), "-0.005");
        assert_eq!(numeric_to_string(Numeric::new_with_scale(42, 0)), "42");
        assert_eq!(numeric_to_string(Numeric::new_with_scale(0, 2)), "0.00");
        assert_eq!(
            numeric_to_string(Numeric::new_with_scale(
                170141183460469231731687303715884105727,
                0
            )),
            "170141183460469231731687303715884105727"
        );
    }

    #[test]
    fn the_column_type_gets_its_length_or_precision() {
        assert_eq!(format_type("varchar", Some(50), None, None), "varchar(50)");
        assert_eq!(
            format_type("nvarchar", Some(-1), None, None),
            "nvarchar(max)"
        );
        assert_eq!(format_type("varbinary", None, None, None), "varbinary");
        assert_eq!(
            format_type("decimal", None, Some(18), Some(4)),
            "decimal(18,4)"
        );
        assert_eq!(format_type("numeric", None, Some(9), None), "numeric(9)");
        assert_eq!(format_type("decimal", None, None, None), "decimal");
        assert_eq!(format_type("int", None, None, None), "int");
    }

    #[test]
    fn a_failed_read_counts_as_an_absent_value() {
        let ok: tiberius::Result<Option<i32>> = Ok(Some(1));
        assert_eq!(read(ok), Some(1));
        let empty: tiberius::Result<Option<i32>> = Ok(None);
        assert_eq!(read(empty), None);
        let failed: tiberius::Result<Option<i32>> =
            Err(tiberius::error::Error::Conversion("no".into()));
        assert_eq!(read(failed), None);
    }
    #[test]
    fn the_token_is_read_from_the_answer_of_the_cli() {
        let good = r#"{"accessToken":"abc","expiresOn":"2026-01-01"}"#;
        assert_eq!(token_from_cli_output(good).unwrap(), "abc");

        for bad in [r#"{"accessToken":""}"#, r#"{"other":1}"#, "not json"] {
            assert_eq!(
                token_from_cli_output(bad).err().unwrap().kind(),
                crate::error::ErrorKind::Authentication
            );
        }
    }

    /// Builds a token whose middle part holds the given claims.
    fn token_with_claims(claims: &str) -> String {
        use base64::Engine;
        let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        format!(
            "{}.{}.{}",
            engine.encode(r#"{"alg":"RS256"}"#),
            engine.encode(claims),
            engine.encode("signature")
        )
    }

    #[test]
    fn the_date_of_a_token_is_read_from_its_middle_part() {
        let token = token_with_claims(r#"{"exp":1735689600,"aud":"sql"}"#);
        assert_eq!(
            token_expiry(&token),
            Some(UNIX_EPOCH + Duration::from_secs(1_735_689_600))
        );
    }

    #[test]
    fn a_token_the_reader_cannot_use_gives_no_date() {
        for token in [
            // No `exp` claim.
            token_with_claims(r#"{"aud":"sql"}"#),
            // The claim is not a number.
            token_with_claims(r#"{"exp":"soon"}"#),
            // The claim is a number that no date can hold.
            token_with_claims(r#"{"exp":-1}"#),
            // The middle part is not JSON.
            token_with_claims("not json"),
            // The middle part is not base64url text.
            "a.!!.c".to_string(),
            // Fewer than three parts.
            "a.b".to_string(),
            // More than three parts.
            "a.b.c.d".to_string(),
            // Not a token at all.
            "a-token".to_string(),
        ] {
            assert_eq!(token_expiry(&token), None, "{token}");
        }
    }

    #[test]
    fn a_token_is_old_only_past_the_allowance() {
        let expiry = UNIX_EPOCH + Duration::from_secs(2_000_000_000);
        let token = token_with_claims(r#"{"exp":2000000000}"#);

        assert!(!token_has_expired(&token, expiry - Duration::from_secs(1)));
        assert!(!token_has_expired(&token, expiry + TOKEN_CLOCK_ALLOWANCE));
        assert!(token_has_expired(
            &token,
            expiry + TOKEN_CLOCK_ALLOWANCE + Duration::from_secs(1)
        ));

        // A token that the reader cannot use goes to the server.
        assert!(!token_has_expired("a-token", expiry));
    }

    #[tokio::test]
    async fn a_token_that_the_user_gives_becomes_the_credential() {
        let mut input = connection();
        input.options.mssql_auth = MssqlAuth::EntraAccessToken;
        input.password = Some("a-token".into());
        assert!(auth_method(&input).await.is_ok());

        input.password = None;
        assert_eq!(
            auth_method(&input).await.err().unwrap().kind(),
            crate::error::ErrorKind::Authentication
        );
    }

    #[tokio::test]
    async fn a_token_with_a_date_in_the_past_is_refused_before_the_socket() {
        let mut input = connection();
        input.options.mssql_auth = MssqlAuth::EntraAccessToken;
        input.password = Some(token_with_claims(r#"{"exp":1000000000}"#));

        let error = auth_method(&input).await.err().unwrap();
        assert_eq!(error.kind(), crate::error::ErrorKind::Authentication);
        assert!(error.to_string().contains("has expired"));

        // A date far ahead passes the check.
        input.password = Some(token_with_claims(r#"{"exp":4000000000}"#));
        assert!(auth_method(&input).await.is_ok());
    }

    #[tokio::test]
    async fn a_missing_azure_cli_is_reported() {
        let mut input = connection();
        input.options.mssql_auth = MssqlAuth::EntraAzureCli;
        input.options.azure_cli_path = Some("/nowhere/az".into());
        let error = auth_method(&input).await.err().unwrap();
        assert_eq!(error.kind(), crate::error::ErrorKind::Authentication);
        assert!(error.to_string().contains("was not found"));
    }

    #[test]
    fn the_resource_of_the_token_names_the_database_service() {
        assert_eq!(DATABASE_RESOURCE, "https://database.windows.net/");
    }
    #[test]
    fn a_fault_of_the_ticket_is_named_as_one() {
        assert!(names_a_ticket_fault(
            "Login failed. No credentials were supplied for GSS"
        ));
        assert!(names_a_ticket_fault("Cannot reach the KDC"));
        assert!(names_a_ticket_fault("SSPI handshake failed"));
        assert!(!names_a_ticket_fault("Login failed for user 'sa'."));
    }

    #[test]
    fn a_login_that_names_no_ticket_keeps_the_error_of_the_driver() {
        use tiberius::error::Error as TiberiusError;

        // The text of this error names no ticket, so it stays a database
        // error even for the integrated method.
        assert_eq!(
            describe_login(TiberiusError::Utf8, MssqlAuth::Integrated).kind(),
            crate::error::ErrorKind::Database
        );

        // A fault of the ticket becomes an error about the credentials.
        let error = describe_login(
            TiberiusError::Protocol("no credentials were supplied".into()),
            MssqlAuth::Integrated,
        );
        assert_eq!(error.kind(), crate::error::ErrorKind::Authentication);
        assert!(error.to_string().contains("kinit"));

        // Another method keeps the error of the driver as it is.
        assert_eq!(
            describe_login(
                TiberiusError::Protocol("no credentials were supplied".into()),
                MssqlAuth::SqlLogin
            )
            .kind(),
            crate::error::ErrorKind::Database
        );
    }

    #[test]
    fn a_server_that_refuses_a_pasted_token_points_at_the_date() {
        use tiberius::error::Error as TiberiusError;

        assert!(names_a_refused_login("Login failed for user 'sa'."));
        assert!(names_a_refused_login("Msg 18456, Level 14"));
        assert!(!names_a_refused_login("The stream ended."));

        let error = describe_login(
            TiberiusError::Protocol("Login failed for the user.".into()),
            MssqlAuth::EntraAccessToken,
        );
        assert_eq!(error.kind(), crate::error::ErrorKind::Authentication);
        assert!(error.to_string().contains("has expired"));

        // Another fault keeps the error of the driver, because a token that
        // is old is not its cause.
        assert_eq!(
            describe_login(TiberiusError::Utf8, MssqlAuth::EntraAccessToken).kind(),
            crate::error::ErrorKind::Database
        );
    }
}
