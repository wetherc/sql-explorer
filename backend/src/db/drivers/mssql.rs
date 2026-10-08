//! The MS SQL Server driver.
//!
//! The configuration is built with the builder of `tiberius` and not from a
//! connection string. A connection string loses the port, because the
//! parser reads the port only from inside the `server` value, and it also
//! loses any password that holds a semicolon or a brace.

use crate::db::drivers::{
    add_constraint_column, add_included_column, add_index_column, add_snapshot_column,
    add_snapshot_relation, add_trigger_event, connect_within, constraint_type, f32_to_json,
    f64_to_json, finish_set, hex_text, non_empty, number_out_of_range, number_value,
    parameter_type_refused, relation_type, routine_type, rows_affected_message, single_statement,
    size_text, trigger_event, CancelHandle, DatabaseDriver, NumberValue, KEEPALIVE_IDLE,
    KEEPALIVE_INTERVAL,
};
use crate::db::sink::{feed, BufferSink, PauseFacts, RowSink, RunSummary, SinkControl};
use crate::db::{
    AppColumn, ColumnInfo, Constraint, CreateQuery, Database, DriverCapabilities, ExecOptions,
    IndexInfo, Message, MessageLevel, ObjectType, PlanMode, QueryParams, QueryResponse,
    RelationType, ResultSet, Routine, Schema, SchemaSnapshot, SnapshotColumn, Table, TableFact,
    Trigger, TriggerTiming,
};
use crate::error::{mssql_error_detail, offset_place, place_of_byte_offset, Error, Result};
use crate::sql::{only_reads, split_batches, split_statements, Dialect};
use crate::storage::{MssqlAuth, SavedConnection, TlsMode};
use async_trait::async_trait;
use chrono::{NaiveDate, NaiveDateTime};
use futures_util::TryStreamExt;
use serde_json::Value as JsonValue;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tiberius::numeric::Numeric;
use tiberius::{
    AttentionHandle, AuthMethod, Client, ColumnData, ColumnType, Config, EncryptionLevel,
    QueryItem, Row,
};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

mod blocking;

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
    /// Error 266, when it was the only error of the batch. The caller
    /// decides if it fails the run, see [`MssqlDriver::settle_mismatch`].
    mismatch: Option<tiberius::error::TokenError>,
}

/// The error that SQL Server sends when a procedure or an
/// `sp_executesql` call ends with a count of open transactions that is not
/// the count at its start.
const TRANSACTION_COUNT_MISMATCH: u32 = 266;

/// Gives a walk that error 266 ended as that error. Only a run with
/// parameters can show the error as a warning.
fn mismatch_as_error(walk: Walk) -> Result<Walk> {
    match walk.mismatch {
        Some(token) => Err(Error::from(tiberius::error::Error::Server(token))),
        None => Ok(walk),
    }
}

/// The warning that takes the place of error 266 after a run with
/// parameters changed the count of open transactions. The detail keeps the
/// text of the server.
fn mismatch_warning(token: &tiberius::error::TokenError, opened: bool) -> Message {
    let text = if opened {
        "This batch started a transaction and left it open. Run COMMIT or ROLLBACK to end it."
    } else {
        "This batch ended a transaction that an earlier run started."
    };
    Message {
        level: MessageLevel::Warning,
        text: text.to_string(),
        detail: Some(format!(
            "{}\n{}",
            mssql_error_detail(token),
            token.message()
        )),
    }
}

/// Builds the `tiberius` configuration from a saved connection.
pub async fn build_config(connection: &SavedConnection) -> Result<Config> {
    let mut config = if let Some(url) = connection.options.connection_url.as_deref() {
        let mut config = parse_string(url)?;
        add_form_settings(&mut config, connection, &string_keys(url)).await?;
        add_login_of_record(&mut config, connection);
        config
    } else {
        Config::new()
    };

    if connection.options.connection_url.is_none() {
        config.host(connection.effective_host());
        // The SQL Browser service gives the port of a named instance, and
        // `tiberius` sends the question of the instance to the configured
        // port. A port of the record then sends the question to the instance
        // itself, so a named instance leaves the port out.
        if let Some(instance) = non_empty(&connection.options.instance_name) {
            config.instance_name(instance);
        } else if let Some(port) = connection.effective_port() {
            config.port(port);
        }
        if let Some(database) = non_empty(&connection.database) {
            config.database(database);
        }
        if let Some(name) = non_empty(&connection.options.application_name) {
            config.application_name(name);
        }
        config.authentication(auth_method(connection).await?);
        config.encryption(encryption_level(connection.options.tls_mode));
        // `tiberius` panics when it gets both `trust_cert` and
        // `trust_cert_ca`, so a CA file applies only to a mode that verifies.
        if !connection.options.tls_mode.verifies_certificate() {
            config.trust_cert();
        } else if let Some(path) = non_empty(&connection.options.ca_cert_path) {
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

/// Applies the settings of the form that a connection string does not give.
/// A key of the string keeps its value. The transport mode applies when the
/// string names no `encrypt` key, and the trust of the certificate when it
/// names no trust key. The method of the form applies when the string names
/// no credential and the method is not the SQL login, whose password
/// [`add_login_of_record`] adds.
async fn add_form_settings(
    config: &mut Config,
    connection: &SavedConnection,
    keys: &[String],
) -> Result<()> {
    let has = |names: &[&str]| keys.iter().any(|key| names.contains(&key.as_str()));
    let mode = connection.options.tls_mode;
    if !has(&["encrypt"]) {
        config.encryption(encryption_level(mode));
    }
    // `tiberius` panics when it gets both `trust_cert` and `trust_cert_ca`,
    // so the form adds neither when the string names one of them.
    if !has(&["trustservercertificate", "trustservercertificateca"]) {
        if !mode.verifies_certificate() {
            config.trust_cert();
        } else if let Some(path) = non_empty(&connection.options.ca_cert_path) {
            config.trust_cert_ca(path);
        }
    }
    if !has(&["application name", "applicationname"]) {
        if let Some(name) = non_empty(&connection.options.application_name) {
            config.application_name(name);
        }
    }
    let credentials = [
        "uid",
        "username",
        "user",
        "user id",
        "userid",
        "password",
        "pwd",
        "integratedsecurity",
        "integrated security",
        "trusted_connection",
    ];
    if connection.options.mssql_auth != MssqlAuth::SqlLogin && !has(&credentials) {
        config.authentication(auth_method(connection).await?);
    }
    Ok(())
}

/// Reads the keys of an ADO.NET or a JDBC connection string, in lower case.
/// A value in braces or in quotes may contain a semicolon or an equals sign, so
/// the walk skips such a value whole.
fn string_keys(url: &str) -> Vec<String> {
    let trimmed = url.trim();
    let body = match trimmed.strip_prefix("jdbc:") {
        Some(rest) => rest.split_once(';').map_or("", |(_, keys)| keys),
        None => trimmed,
    };
    let mut keys = Vec::new();
    let mut chars = body.chars().peekable();
    while chars.peek().is_some() {
        let mut key = String::new();
        let mut has_value = false;
        for c in chars.by_ref() {
            match c {
                '=' => {
                    has_value = true;
                    break;
                }
                ';' => break,
                _ => key.push(c),
            }
        }
        let key = key.trim().to_lowercase();
        if !key.is_empty() {
            keys.push(key);
        }
        if !has_value {
            continue;
        }
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        let close = match chars.peek() {
            Some('{') => Some('}'),
            Some('"') => Some('"'),
            Some('\'') => Some('\''),
            _ => None,
        };
        if let Some(close) = close {
            chars.next();
            for c in chars.by_ref() {
                if c == close {
                    break;
                }
            }
        }
        for c in chars.by_ref() {
            if c == ';' {
                break;
            }
        }
    }
    keys
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

/// The resource that a token for a SQL database names.
pub const DATABASE_RESOURCE: &str = "https://database.windows.net/";

/// The places a desktop application looks for the Azure CLI. An application
/// that a desktop starts holds a short `PATH` that often misses these.
#[cfg(not(windows))]
const AZURE_CLI_PLACES: [&str; 4] = [
    "az",
    "/opt/homebrew/bin/az",
    "/usr/local/bin/az",
    "/usr/bin/az",
];

/// The places of the Azure CLI on Windows. The CLI is a batch file there,
/// so it runs through `cmd /C`.
#[cfg(windows)]
const AZURE_CLI_PLACES: [&str; 3] = [
    "az.cmd",
    r"C:\Program Files\Microsoft SDKs\Azure\CLI2\wbin\az.cmd",
    r"C:\Program Files (x86)\Microsoft SDKs\Azure\CLI2\wbin\az.cmd",
];

/// The longest time that the driver waits for the Azure CLI.
const AZURE_CLI_WAIT: Duration = Duration::from_secs(30);

/// A cached token is used only while it stays valid for this long.
const TOKEN_REUSE_MARGIN: Duration = Duration::from_secs(5 * 60);

/// The tokens that the Azure CLI gave, by the path of the CLI. A new
/// connection then needs no new run of the CLI while the token is valid.
static AZURE_CLI_TOKENS: std::sync::Mutex<Vec<(String, String)>> =
    std::sync::Mutex::new(Vec::new());

/// One lock for each key of [`AZURE_CLI_TOKENS`]. Connections that open at
/// the same time with no valid token in the cache then wait for one run of the
/// CLI and share its token. Without the lock, each of them starts its own
/// `az` process.
static AZURE_CLI_RUNS: std::sync::Mutex<Vec<(String, Arc<tokio::sync::Mutex<()>>)>> =
    std::sync::Mutex::new(Vec::new());

/// Gives the lock for the runs of the CLI at `key`.
fn cli_run_lock(key: &str) -> Arc<tokio::sync::Mutex<()>> {
    let mut runs = AZURE_CLI_RUNS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((_, lock)) = runs.iter().find(|(path, _)| path == key) {
        return lock.clone();
    }
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    runs.push((key.to_string(), lock.clone()));
    lock
}

/// Builds the command that runs the Azure CLI at `path`.
fn azure_cli_command(path: &str) -> tokio::process::Command {
    #[cfg(windows)]
    {
        /// The flag that stops Windows from opening a console window.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut command = tokio::process::Command::new("cmd");
        command.arg("/C").arg(path).creation_flags(CREATE_NO_WINDOW);
        command
    }
    #[cfg(not(windows))]
    {
        tokio::process::Command::new(path)
    }
}

/// Gives the cached token of the CLI at `key` while it stays valid for
/// [`TOKEN_REUSE_MARGIN`].
fn cached_cli_token(key: &str, now: SystemTime) -> Option<String> {
    let tokens = AZURE_CLI_TOKENS.lock().ok()?;
    tokens
        .iter()
        .find(|(path, _)| path == key)
        .map(|(_, token)| token.clone())
        .filter(|token| {
            token_expiry(token).is_some_and(|expiry| {
                expiry
                    .duration_since(now)
                    .is_ok_and(|left| left > TOKEN_REUSE_MARGIN)
            })
        })
}

/// Keeps the token of the CLI at `key` for the next connection.
fn cache_cli_token(key: &str, token: &str) {
    if let Ok(mut tokens) = AZURE_CLI_TOKENS.lock() {
        tokens.retain(|(path, _)| path != key);
        tokens.push((key.to_string(), token.to_string()));
    }
}

/// Reads the access token out of the JSON that the Azure CLI writes.
pub fn token_from_cli_output(output: &str) -> Result<String> {
    let value: serde_json::Value = serde_json::from_str(output).map_err(|error| {
        Error::Authentication(format!("Couldn't read the Azure CLI output: {error}"))
    })?;
    value
        .get("accessToken")
        .and_then(|token| token.as_str())
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            Error::Authentication("The Azure CLI didn't return an access token.".to_string())
        })
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
const EXPIRED_TOKEN_MESSAGE: &str = "The access token has expired. Paste a new one, or switch to \
                                     the Azure CLI method, which gets a fresh token for each \
                                     connection.";

/// The words that report a statement which the row limit ended.
const ENDED_AT_THE_LIMIT_MESSAGE: &str =
    "Reached the row limit, so the statement was cancelled on the server. To fetch more rows, \
     raise the row limit in the settings.";

/// The statement that gives the number of the session on the server, as
/// the report of the sessions that block others names it. `@@SPID` is a
/// `smallint`, and the cast gives the `int` that the driver reads.
const SESSION_NUMBER: &str = "SELECT CAST(@@SPID AS int)";

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
/// The token lives for about one hour. A token that stays valid for more
/// than [`TOKEN_REUSE_MARGIN`] serves the next connection too, so the CLI
/// runs about once an hour. A CLI that does not answer within
/// [`AZURE_CLI_WAIT`] is stopped. Only one run for each CLI path is active at
/// a time. A caller that waits for the run of another caller then takes the
/// token of that run from the cache.
async fn azure_cli_token(configured_path: &Option<String>) -> Result<String> {
    let configured = non_empty(configured_path);
    let key = configured.unwrap_or_default().to_string();
    if let Some(token) = cached_cli_token(&key, SystemTime::now()) {
        return Ok(token);
    }
    let lock = cli_run_lock(&key);
    let _run = lock.lock().await;
    if let Some(token) = cached_cli_token(&key, SystemTime::now()) {
        return Ok(token);
    }
    let places: Vec<String> = match configured {
        Some(path) => vec![path.to_string()],
        None => AZURE_CLI_PLACES
            .iter()
            .map(|path| path.to_string())
            .collect(),
    };

    let mut last: Option<String> = None;
    for path in &places {
        let run = azure_cli_command(path)
            .arg("account")
            .arg("get-access-token")
            .arg("--resource")
            .arg(DATABASE_RESOURCE)
            .arg("--output")
            .arg("json")
            .kill_on_drop(true)
            .output();
        let outcome = tokio::time::timeout(AZURE_CLI_WAIT, run)
            .await
            .map_err(|_| {
                Error::Authentication(format!(
                    "The Azure CLI didn't answer within {} seconds.",
                    AZURE_CLI_WAIT.as_secs()
                ))
            })?;

        match outcome {
            Ok(output) if output.status.success() => {
                let text = String::from_utf8_lossy(&output.stdout);
                let token = token_from_cli_output(&text)?;
                cache_cli_token(&key, &token);
                return Ok(token);
            }
            Ok(output) => {
                // The CLI ran and refused. A further place would give the
                // same answer, so the reason is reported at once.
                let reason = String::from_utf8_lossy(&output.stderr).trim().to_string();
                return Err(Error::Authentication(cli_refusal(&reason)));
            }
            Err(error) if configured.is_some() => {
                return Err(Error::Authentication(format!(
                    "Couldn't run the Azure CLI at {path}. Check the Azure CLI path. {error}"
                )));
            }
            Err(error) => last = Some(error.to_string()),
        }
    }

    Err(Error::Authentication(format!(
        "Couldn't find the Azure CLI. Enter its path in the connection settings. {}",
        last.unwrap_or_default()
    )))
}

/// The text for a CLI that ran and gave no token. The advice to sign in
/// applies only when the CLI itself asks for `az login`.
fn cli_refusal(reason: &str) -> String {
    if reason.to_lowercase().contains("az login") {
        format!("The Azure CLI couldn't get a token. Run `az login` and try again. {reason}")
    } else {
        format!("The Azure CLI couldn't get a token. {reason}")
    }
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
            let token = azure_cli_token(&connection.options.azure_cli_path).await?;
            Ok(AuthMethod::aad_token(token))
        }
        MssqlAuth::EntraAccessToken => {
            // The token is a credential, so it travels in the field that the
            // secret store holds and never reaches the settings file.
            let token = non_empty(&connection.password).ok_or_else(|| {
                Error::Authentication(
                    "This connection needs an access token. Paste one, or switch to the Azure CLI method."
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

/// Turns on TCP keepalive for the socket of one connection. A socket where
/// the option cannot be set still works, so the failure is a warning and
/// gives false.
fn keep_alive(socket: socket2::SockRef<'_>) -> bool {
    let keepalive = socket2::TcpKeepalive::new()
        .with_time(KEEPALIVE_IDLE)
        .with_interval(KEEPALIVE_INTERVAL);
    let outcome = socket.set_tcp_keepalive(&keepalive);
    if let Err(error) = &outcome {
        log::warn!("Could not turn on TCP keepalive: {error}");
    }
    outcome.is_ok()
}

/// The count of redirects that one login follows. An availability group
/// listener or an Azure gateway sends one redirect.
const MAX_REDIRECTS: usize = 3;

/// Opens the socket and logs in. A named instance asks the SQL Browser
/// service for its port. A server that answers the login with a redirect,
/// such as the read-only routing of an availability group, gets a new socket
/// to the host and the port that it names.
async fn open_client(
    mut config: Config,
) -> std::result::Result<MssqlClient, tiberius::error::Error> {
    let mut routed: Option<String> = None;
    for _ in 0..=MAX_REDIRECTS {
        let tcp = match &routed {
            Some(address) => TcpStream::connect(address.as_str()).await?,
            None if config.get_instance_name().is_some() => {
                use tiberius::SqlBrowser;
                TcpStream::connect_named(&config).await?
            }
            None => TcpStream::connect(config.get_addr()).await?,
        };
        if let Err(error) = tcp.set_nodelay(true) {
            log::warn!("Could not disable the Nagle algorithm: {error}");
        }
        keep_alive(socket2::SockRef::from(&tcp));
        match Client::connect(config.clone(), tcp.compat_write()).await {
            Err(tiberius::error::Error::Routing { host, port }) => {
                log::info!("The server sent the login to {host}:{port}.");
                routed = Some(format!("{host}:{port}"));
                config.host(host);
                config.port(port);
            }
            other => return other,
        }
    }
    Err(tiberius::error::Error::Protocol(
        "The server redirected the login too many times.".into(),
    ))
}

/// Maps the transport setting of the application onto the encryption level
/// of `tiberius`.
pub fn encryption_level(mode: TlsMode) -> EncryptionLevel {
    match mode {
        // `NotSupported` tells the server that this client has no TLS.
        TlsMode::Disable => EncryptionLevel::NotSupported,
        // `Off` encrypts the login packet only, so `Prefer` asks for full
        // encryption. See [`MssqlDriver::connect`] for a server without TLS.
        TlsMode::Prefer | TlsMode::Require | TlsMode::VerifyFull => EncryptionLevel::Required,
    }
}

impl MssqlDriver {
    /// Opens a connection. One time limit covers the token of the Azure CLI,
    /// the socket, the TLS handshake, the login, and each redirect and retry
    /// of the login.
    ///
    /// The mode `Prefer` asks for full encryption and trusts any
    /// certificate. A server without TLS refuses that request, and the
    /// driver then opens one more connection without encryption.
    ///
    /// A Windows Authentication login asks the Kerberos server (KDC) for a
    /// ticket. The configuration gives `tiberius` a flag that is true while
    /// that call waits, so a time limit that passes during the wait names
    /// the Kerberos server and not the SQL Server.
    pub async fn connect(connection: &SavedConnection) -> Result<Box<dyn DatabaseDriver>> {
        let limit = connection.options.connect_timeout_secs.max(1);
        let auth = connection.options.mssql_auth;
        let waiting_for_kerberos = Arc::new(AtomicBool::new(false));
        let flag = waiting_for_kerberos.clone();
        let falls_back = connection.options.tls_mode == TlsMode::Prefer
            && connection
                .options
                .connection_url
                .as_deref()
                .is_none_or(|url| !string_keys(url).iter().any(|key| key == "encrypt"));
        let opened = connect_within(limit, async move {
            let mut config = build_config(connection).await?;
            config.watch_gssapi(flag);
            match open_client(config.clone()).await {
                Err(tiberius::error::Error::Tls(text))
                    if falls_back && text == tiberius::error::ENCRYPTION_NOT_SUPPORTED =>
                {
                    log::info!("The server has no TLS, so the connection is not encrypted.");
                    let mut plain = config;
                    plain.encryption(EncryptionLevel::NotSupported);
                    open_client(plain).await
                }
                other => other,
            }
            .map_err(|error| describe_login(error, auth))
        })
        .await;
        let client = name_kerberos_wait(opened, waiting_for_kerberos.load(Ordering::SeqCst))??;
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
    /// `may_end_early` false and keeps the walk. The walk then receives every
    /// row of each set from the server and drops the rows past the limit.
    /// A batch of more than one statement, and a session inside a
    /// transaction with `XACT_ABORT` on, take this path. TDS has no message
    /// that ends one set and lets the rest of the batch run. A server cursor
    /// would change the locks and the plan of the statement, and a change of
    /// `XACT_ABORT` for the read would stay on the session when a time limit
    /// drops the run.
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
        // A batch without parameters goes as a plain SQL batch, as SQL Server
        // Management Studio sends it. Inside `sp_executesql`, a `BEGIN
        // TRANSACTION` without its `COMMIT` ends with error 266, and a `USE`,
        // a `SET` or a temporary table ends with the call.
        let mut stream = if params.is_empty() {
            self.client.simple_query(statement).await?
        } else {
            self.client.query(statement, params).await?
        };
        let mut open = false;
        let mut count = 0usize;
        let mut truncated = false;
        let mut stopped = false;
        let mut asked_to_end = false;
        // True when the row limit brought the end, and not the sink.
        let mut ended_at_limit = false;
        // True after the sink heard that the walk drops the rows past the
        // limit.
        let mut told_past_limit = false;
        let mut rows_affected: Option<u64> = None;
        let mut errors = 0usize;
        let mut mismatch = None;

        loop {
            let item = match stream.try_next().await {
                Ok(Some(item)) => item,
                Ok(None) => break,
                // The end that the attention packet brings is the wanted
                // end, so it carries no fault to the user.
                Err(tiberius::error::Error::Canceled) if asked_to_end => break,
                // The stream gives its first error of the server at its end,
                // so the batch has done all its work.
                Err(tiberius::error::Error::Server(token))
                    if errors == 1 && token.code() == TRANSACTION_COUNT_MISMATCH =>
                {
                    mismatch = Some(token);
                    break;
                }
                Err(error) => return Err(error.into()),
            };
            match item {
                QueryItem::Metadata(metadata) => {
                    if open {
                        finish_set(sink, count, truncated)?;
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
                                // A user-defined type, such as `geography`,
                                // gives its own name.
                                let type_name =
                                    column.udt_name().unwrap_or(type_name(column.column_type()));
                                ColumnInfo::new(column.name(), type_name)
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
                        finish_set(sink, count, truncated)?;
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
                        tell_past_limit(sink, may_end_early, &mut told_past_limit);
                        continue;
                    }
                    if feed(sink, row_to_json(&row)).await? == SinkControl::Stop {
                        truncated = true;
                        stopped = true;
                        if may_end_early && !asked_to_end {
                            attention.signal();
                            asked_to_end = true;
                        }
                        tell_past_limit(sink, may_end_early, &mut told_past_limit);
                        continue;
                    }
                    count += 1;
                }
            }
        }
        if open {
            finish_set(sink, count, truncated)?;
        }
        if ended_at_limit {
            sink.message(Message::info(ENDED_AT_THE_LIMIT_MESSAGE.to_string()));
        }
        Ok(Walk {
            stopped,
            rows_affected,
            mismatch,
        })
    }

    /// Reads the count of open transactions of the session.
    async fn transaction_count(&mut self) -> Result<i32> {
        let row = self
            .client
            .simple_query("SELECT @@TRANCOUNT")
            .await?
            .into_row()
            .await?;
        Ok(row.and_then(|row| row.get::<i32, _>(0)).unwrap_or(0))
    }

    /// Decides what error 266 means after a run with parameters, which goes
    /// through `sp_executesql`. The server sends the error when the call
    /// changed the count of open transactions, but the change stays. When
    /// the count after the run differs from `before`, the error becomes a
    /// warning. Otherwise, for example when `XACT_ABORT` rolled the
    /// transaction back, the error fails the run.
    async fn settle_mismatch(
        &mut self,
        mut walk: Walk,
        before: i32,
        sink: &mut dyn RowSink,
    ) -> Result<Walk> {
        let Some(token) = walk.mismatch.take() else {
            return Ok(walk);
        };
        let after = self.transaction_count().await?;
        if after == before {
            walk.mismatch = Some(token);
            return mismatch_as_error(walk);
        }
        sink.message(mismatch_warning(&token, after > before));
        Ok(walk)
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

/// Tells the sink once that the walk reads and drops the rows past the
/// limit. A walk that may end early sends the attention packet instead, so
/// the sink hears nothing.
fn tell_past_limit(sink: &mut dyn RowSink, may_end_early: bool, told: &mut bool) {
    if !may_end_early && !*told {
        sink.reading_past_limit();
        *told = true;
    }
}

/// Names the Kerberos server as the cause of a connect time limit that
/// passed while the login waited for a GSSAPI call. Any other result stays
/// as it is.
fn name_kerberos_wait<T>(opened: Result<T>, waiting_for_kerberos: bool) -> Result<T> {
    match opened {
        Err(error @ Error::Connection(_)) if waiting_for_kerberos => {
            Err(Error::KerberosUnreachable(Box::new(error)))
        }
        other => other,
    }
}

/// True when a GSSAPI error says that no Kerberos server (KDC) of the realm
/// could be found or reached. MIT Kerberos and Heimdal, which macOS uses,
/// word this in different ways. An error that does not come from GSSAPI is
/// never such a failure, so a server fault with similar words does not
/// match.
fn names_an_unreachable_kdc(error: &tiberius::error::Error) -> bool {
    let tiberius::error::Error::Gssapi(text) = error else {
        return false;
    };
    let lower = text.to_lowercase();
    [
        "cannot contact any kdc",
        "unable to reach any kdc",
        "cannot find kdc",
        "cannot resolve network address for kdc",
    ]
    .iter()
    .any(|mark| lower.contains(mark))
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
    if names_an_unreachable_kdc(&error) {
        return Error::KerberosUnreachable(Box::new(Error::from(error)));
    }
    let text = error.to_string();
    if names_a_ticket_fault(&text) {
        Error::Authentication(format!(
            "The server rejected your Windows credentials. On macOS and Linux, run `kinit` to get a \
             Kerberos ticket, and use the server's fully qualified host name so the ticket \
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

/// Builds the statement that reads the CREATE text of one view or one
/// synonym. MS SQL Server keeps no text for a table, so a table gives no
/// statement and the command layer builds a draft instead.
///
/// `OBJECT_DEFINITION` looks in the current database, so the statement reads
/// the catalog of the database of the object. `OBJECT_ID` finds the object
/// in that database from its three-part name. A view gives its text from
/// `sys.sql_modules`. A synonym keeps no text, so the statement builds
/// `CREATE SYNONYM` from the name of the object it points at.
fn create_query_text(
    database: Option<&str>,
    schema: Option<&str>,
    table: &str,
    relation_type: RelationType,
) -> Option<CreateQuery> {
    let name = Dialect::MsSql.qualified_name(database, schema, table);
    let catalog = database
        .map(|database| format!("{}.", Dialect::MsSql.quote_identifier(database)))
        .unwrap_or_default();
    let object = Dialect::MsSql.quote_literal(&name);
    let sql = match relation_type {
        RelationType::View => format!(
            "SELECT m.definition FROM {catalog}sys.sql_modules AS m \
             WHERE m.object_id = OBJECT_ID({object});"
        ),
        RelationType::Synonym => format!(
            "SELECT N'CREATE SYNONYM ' + QUOTENAME(s.name) + N'.' + QUOTENAME(sy.name) + \
             N' FOR ' + sy.base_object_name + N';' \
             FROM {catalog}sys.synonyms AS sy \
             JOIN {catalog}sys.schemas AS s ON s.schema_id = sy.schema_id \
             WHERE sy.object_id = OBJECT_ID({object});"
        ),
        _ => return None,
    };
    Some(CreateQuery::new(sql, 0))
}

/// Builds the statement that reads the text of a trigger. The text comes
/// from `sys.sql_modules`, as for a view. A trigger belongs to the schema of
/// its relation, so its name in that schema finds it. MS SQL Server has no
/// scheduled events.
fn object_query_text(
    database: Option<&str>,
    schema: Option<&str>,
    name: &str,
    object_type: ObjectType,
) -> Option<CreateQuery> {
    match object_type {
        ObjectType::Trigger => create_query_text(database, schema, name, RelationType::View),
        ObjectType::Event => None,
    }
}

/// The name MS SQL Server gives the column that holds a plan. Both plan
/// switches use this name.
pub const PLAN_COLUMN: &str = "Microsoft SQL Server 2005 XML Showplan";

/// The switch that asks the session for a plan. `SHOWPLAN_XML` compiles the
/// statement and does not run it. `STATISTICS XML` runs it and adds the plan
/// after each result set of the statement.
pub fn plan_switch(mode: PlanMode) -> &'static str {
    match mode {
        PlanMode::Estimated => "SHOWPLAN_XML",
        PlanMode::Actual => "STATISTICS XML",
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

/// Marks a server error with its line in the whole text. The server counts
/// the lines of a batch from 1, and `start` is the byte offset of the batch
/// in `query`. An error without a line, or a batch that was not found in the
/// text, keeps no place.
fn locate_error(error: Error, query: &str, start: Option<usize>) -> Error {
    let line = match &error {
        Error::Tiberius(tiberius::error::Error::Server(token)) if token.line() > 0 => token.line(),
        _ => return error,
    };
    match start {
        Some(start) => {
            let (line, column) = offset_place(place_of_byte_offset(query, start), (line, 1));
            error.at(line, column)
        }
        None => error,
    }
}

/// The statement that ends each later statement of the session that waits
/// for a lock longer than `limit`. The server takes whole milliseconds, and
/// error 1222 ends such a statement. The statement goes as a batch of its
/// own, because a `SET` inside `sp_executesql` ends with that call.
fn lock_timeout_statement(limit: Duration) -> String {
    format!("SET LOCK_TIMEOUT {}", limit.as_millis())
}

/// The message for an error that the server sent after the first error of a
/// batch, with the number, the severity, the state and the line that SQL
/// Server Management Studio shows.
fn later_error(error: &tiberius::error::TokenError) -> Message {
    Message {
        level: MessageLevel::Error,
        text: error.message().to_string(),
        detail: Some(mssql_error_detail(error)),
    }
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
            supports_materialized_views: false,
            supports_foreign_tables: false,
            supports_synonyms: true,
            supports_triggers: true,
            supports_view_triggers: true,
            supports_events: false,
        }
    }

    fn dialect(&self) -> Dialect {
        Dialect::MsSql
    }

    /// The server waits with no time limit while the client does not read
    /// the rows, so a read can pause.
    fn pauses_reads(&self) -> bool {
        true
    }

    async fn pause_facts(&mut self) -> Result<PauseFacts> {
        let row = self
            .client
            .simple_query(SESSION_NUMBER)
            .await?
            .into_row()
            .await?;
        Ok(PauseFacts {
            server_session: row
                .and_then(|row| row.get::<i32, _>(0))
                .and_then(|spid| u64::try_from(spid).ok()),
            ..PauseFacts::default()
        })
    }

    fn create_query(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        table: &str,
        relation_type: RelationType,
    ) -> Option<CreateQuery> {
        create_query_text(database, schema, table, relation_type)
    }

    fn object_create_query(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        _parent: Option<&str>,
        name: &str,
        object_type: ObjectType,
    ) -> Option<CreateQuery> {
        object_query_text(database, schema, name, object_type)
    }

    async fn ping(&mut self) -> Result<()> {
        let mut stream = self.client.simple_query("SELECT 1").await?;
        while stream.try_next().await?.is_some() {}
        Ok(())
    }

    async fn limit_lock_waits(&mut self, limit: Duration) -> Result<()> {
        let statement = lock_timeout_statement(limit);
        let mut stream = self.client.simple_query(statement).await?;
        while stream.try_next().await?.is_some() {}
        Ok(())
    }

    async fn blocking_sessions(&mut self) -> Result<crate::db::blocking::BlockingReport> {
        self.blocking_report().await
    }

    async fn holds_open_transaction(&mut self) -> Result<bool> {
        Ok(self.transaction_count().await? > 0)
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
                "Parameters only work with a single batch. Remove the GO separators, or run one \
                 batch at a time."
                    .to_string(),
            ));
        }

        // The end of the last batch found in the text, so that a batch that
        // stands twice is found at its own place.
        let mut cursor = 0;
        'batches: for batch in batches {
            let start = query[cursor..]
                .find(batch.text.as_str())
                .map(|at| cursor + at);
            if let Some(at) = start {
                cursor = at + batch.text.len();
            }
            let statements = split_statements(&batch.text, Dialect::MsSql);
            // A statement that only reads leaves the state of the session as
            // it found it, so one probe serves every run of the batch.
            let may_end_early = self.may_end_early(&statements).await;

            for _ in 0..batch.runs {
                let bound = bind_params(params)?;
                let borrowed: Vec<&dyn tiberius::ToSql> =
                    bound.iter().map(|value| value.as_ref()).collect();

                // A run with parameters goes through `sp_executesql`, so the
                // count of open transactions before it tells what a later
                // error 266 means.
                let before = if borrowed.is_empty() {
                    None
                } else {
                    Some(self.transaction_count().await?)
                };
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
                    .await;
                let walk = match (walk, before) {
                    (Ok(walk), Some(before)) => self.settle_mismatch(walk, before, sink).await,
                    (walk, _) => walk.and_then(mismatch_as_error),
                }
                .map_err(|error| locate_error(error, query, start))?;
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
        mode: PlanMode,
        options: &ExecOptions,
    ) -> Result<QueryResponse> {
        let statement = single_statement(query, Dialect::MsSql)?;
        let bound = bind_params(params)?;
        let borrowed: Vec<&dyn tiberius::ToSql> =
            bound.iter().map(|value| value.as_ref()).collect();
        let switch = plan_switch(mode);
        let started = Instant::now();

        self.run_switch(&format!("SET {switch} ON")).await?;
        // The plan sets are filtered after the run, so the rows buffer here.
        let mut sink = BufferSink::new(options.max_rows);
        // The server does not run a statement under the estimated plan, so
        // an attention packet there rolls back no work.
        let may_end_early = mode == PlanMode::Estimated;
        let outcome = self
            .stream_sets(
                &statement,
                borrowed.as_slice(),
                options,
                &mut sink,
                may_end_early,
            )
            .await;
        let outcome = outcome.and_then(mismatch_as_error);
        if let Err(error) = self.run_switch(&format!("SET {switch} OFF")).await {
            // The switch holds for the session, so a session that keeps it on
            // answers every later statement with a plan. The connection is
            // therefore no longer fit for use.
            log::warn!("The plan switch stayed on: {error}");
            return Err(Error::Connection(format!(
                "Couldn't turn {switch} back off, so this connection still returns plans instead of \
                 results. Reconnect to fix it. {error}"
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
                "The server didn't return a plan, so these are the result sets the statement returned.",
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
        let query = tables_query(&Dialect::MsSql.quote_identifier(database));
        let mut stream = self.client.query(query, &[&schema]).await?;
        let mut tables = Vec::new();
        while let Some(item) = stream.try_next().await? {
            if let QueryItem::Row(row) = item {
                tables.push(relation_of(
                    row.try_get::<&str, _>(0)?.unwrap_or_default(),
                    row.try_get::<&str, _>(1)?.unwrap_or_default(),
                    row.try_get::<&str, _>(2)?,
                ));
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
                    c.DATETIME_PRECISION, \
                    CASE WHEN k.COLUMN_NAME IS NULL THEN 0 ELSE 1 END AS IS_KEY, \
                    CASE WHEN sc.is_identity = 1 OR sc.is_computed = 1 \
                           OR c.DATA_TYPE IN ('timestamp', 'rowversion') \
                         THEN 1 ELSE 0 END AS IS_GENERATED \
             FROM {catalog}.INFORMATION_SCHEMA.COLUMNS AS c \
             LEFT JOIN {catalog}.sys.columns AS sc \
               ON sc.object_id = OBJECT_ID(QUOTENAME(c.TABLE_CATALOG) + '.' \
                      + QUOTENAME(c.TABLE_SCHEMA) + '.' + QUOTENAME(c.TABLE_NAME)) \
              AND sc.name = c.COLUMN_NAME \
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
                let fraction = row.try_get::<i16, _>(6)?;
                let is_key = row.try_get::<i32, _>(7)?.unwrap_or(0);
                let is_generated = row.try_get::<i32, _>(8)?.unwrap_or(0);
                columns.push(AppColumn {
                    name,
                    data_type: format_type(
                        base,
                        length,
                        precision,
                        scale.or(fraction.map(i32::from)),
                    ),
                    nullable: nullable.eq_ignore_ascii_case("YES"),
                    is_primary_key: is_key == 1,
                    is_generated: is_generated == 1,
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
                facts.push(TableFact::new("Last modified", changed.to_string()));
            }
        }
        Ok(facts)
    }

    async fn schema_snapshot(
        &mut self,
        database: &str,
        max_columns: usize,
    ) -> Result<SchemaSnapshot> {
        let query = snapshot_query(&Dialect::MsSql.quote_identifier(database), max_columns);
        let mut stream = self.client.query(query, &[&database]).await?;
        let mut snapshot = SchemaSnapshot {
            database: database.to_string(),
            complete: true,
            ..SchemaSnapshot::default()
        };
        while let Some(item) = stream.try_next().await? {
            let QueryItem::Row(row) = item else { continue };
            let schema = row.try_get::<&str, _>(0)?.map(str::to_string);
            let relation = row.try_get::<&str, _>(1)?.unwrap_or_default().to_string();
            let relation_type = relation_type_of(row.try_get::<&str, _>(2)?.unwrap_or_default());
            let kept = match snapshot_column(row.try_get(3)?, row.try_get(4)?) {
                Some(column) => add_snapshot_column(
                    &mut snapshot,
                    max_columns,
                    schema,
                    relation,
                    relation_type,
                    column,
                ),
                None => add_snapshot_relation(
                    &mut snapshot,
                    max_columns,
                    schema,
                    relation,
                    relation_type,
                ),
            };
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
                    routine_type: routine_type(row.try_get::<&str, _>(1)?.unwrap_or_default()),
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

    /// Reads the triggers with [`trigger_query`], one event of one trigger
    /// in each row.
    async fn list_triggers(
        &mut self,
        database: &str,
        schema: Option<&str>,
        table: &str,
    ) -> Result<Vec<Trigger>> {
        let name =
            Dialect::MsSql.qualified_name(Some(database), Some(schema.unwrap_or("dbo")), table);
        let query = trigger_query(&Dialect::MsSql.quote_identifier(database));
        let mut stream = self.client.query(query, &[&name.as_str()]).await?;
        let mut triggers = Vec::new();
        while let Some(item) = stream.try_next().await? {
            if let QueryItem::Row(row) = item {
                add_trigger_event(
                    &mut triggers,
                    row.try_get::<&str, _>(0)?.unwrap_or_default().to_string(),
                    trigger_timing_of(row.try_get::<bool, _>(1)?.unwrap_or(false)),
                    !row.try_get::<bool, _>(2)?.unwrap_or(false),
                    row.try_get::<&str, _>(3)?.and_then(trigger_event),
                );
            }
        }
        Ok(triggers)
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
                    constraint_type(row.try_get::<&str, _>(1)?.unwrap_or_default()),
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

/// Lists the relations and the synonyms of one schema. `INFORMATION_SCHEMA`
/// shows no synonym, so the synonyms come from `sys.synonyms`, with the
/// name of the object that each one points at.
fn tables_query(catalog: &str) -> String {
    format!(
        "SELECT TABLE_NAME, TABLE_TYPE, CAST(NULL AS nvarchar(1035)) \
         FROM {catalog}.INFORMATION_SCHEMA.TABLES WHERE TABLE_SCHEMA = @P1 \
         UNION ALL SELECT sy.name, N'SYNONYM', sy.base_object_name \
         FROM {catalog}.sys.synonyms AS sy \
         JOIN {catalog}.sys.schemas AS s ON s.schema_id = sy.schema_id \
         WHERE s.name = @P1 \
         ORDER BY 2, 1"
    )
}

/// Reads the word of the type of a relation in [`tables_query`] and
/// [`snapshot_query`]. The word is `BASE TABLE`, `VIEW` or `SYNONYM`.
fn relation_type_of(word: &str) -> RelationType {
    if word.eq_ignore_ascii_case("SYNONYM") {
        RelationType::Synonym
    } else {
        relation_type(word)
    }
}

/// Turns one row of [`tables_query`] into its entry.
fn relation_of(name: &str, word: &str, target: Option<&str>) -> Table {
    match relation_type_of(word) {
        RelationType::Synonym => Table::synonym(name, target.unwrap_or_default()),
        relation_type => Table::new(name, relation_type),
    }
}

/// Reads every relation, every synonym and every column of one database in
/// one statement. `@P1` is the name of the database. The rows arrive in the
/// order of the relation, which the fold needs.
///
/// A synonym gets the columns of its target when the target is a table or a
/// view of the same database. `OBJECT_ID` finds the target through a name in
/// three parts, so the result does not depend on the database of the
/// connection. A target with no schema gets the empty part of `[db]..[t]`,
/// which gives the default schema of the user. A synonym with a target on
/// another server, in another database, or of another type gets one row with
/// no column, so its name stays in the snapshot.
///
/// The server sends the first `max_columns + 1` rows with a column, and each
/// row with no column. The fold stops at the first row after the count of
/// columns reaches the bound, and that row is among the rows sent, so the
/// fold still marks the snapshot as not complete. The rows past it do not
/// cross the network. `ROW_NUMBER` counts only the rows with a column, so a
/// synonym with no column does not use up the bound.
fn snapshot_query(catalog: &str, max_columns: usize) -> String {
    let bound = max_columns.saturating_add(1).min(i64::MAX as usize);
    format!(
        "SELECT w.sch, w.rel, w.typ, w.col, w.dtype, w.ord \
         FROM (SELECT u.*, ROW_NUMBER() OVER ( \
             PARTITION BY CASE WHEN u.col IS NULL THEN 0 ELSE 1 END \
             ORDER BY u.sch, u.rel, u.ord) AS n \
           FROM ({}) AS u (sch, rel, typ, col, dtype, ord)) AS w \
         WHERE w.col IS NULL OR w.n <= {bound} \
         ORDER BY w.sch, w.rel, w.ord",
        snapshot_rows(catalog)
    )
}

/// The rows of [`snapshot_query`] before the bound: one row for each column
/// of a relation, and the rows of the synonyms.
fn snapshot_rows(catalog: &str) -> String {
    format!(
        "SELECT c.TABLE_SCHEMA, c.TABLE_NAME, t.TABLE_TYPE, c.COLUMN_NAME, c.DATA_TYPE, \
         c.ORDINAL_POSITION \
         FROM {catalog}.INFORMATION_SCHEMA.COLUMNS AS c \
         JOIN {catalog}.INFORMATION_SCHEMA.TABLES AS t \
           ON t.TABLE_SCHEMA = c.TABLE_SCHEMA AND t.TABLE_NAME = c.TABLE_NAME \
         UNION ALL SELECT s.name, sy.name, N'SYNONYM', bc.COLUMN_NAME, bc.DATA_TYPE, \
         bc.ORDINAL_POSITION \
         FROM {catalog}.sys.synonyms AS sy \
         JOIN {catalog}.sys.schemas AS s ON s.schema_id = sy.schema_id \
         OUTER APPLY (SELECT OBJECT_ID(QUOTENAME(@P1) + N'.' \
             + ISNULL(QUOTENAME(PARSENAME(sy.base_object_name, 2)), N'') + N'.' \
             + QUOTENAME(PARSENAME(sy.base_object_name, 1))) AS id \
           WHERE PARSENAME(sy.base_object_name, 4) IS NULL \
           AND ISNULL(PARSENAME(sy.base_object_name, 3), @P1) = @P1) AS b \
         LEFT JOIN ({catalog}.INFORMATION_SCHEMA.COLUMNS AS bc \
           JOIN {catalog}.INFORMATION_SCHEMA.TABLES AS bt \
             ON bt.TABLE_SCHEMA = bc.TABLE_SCHEMA AND bt.TABLE_NAME = bc.TABLE_NAME) \
           ON bc.TABLE_SCHEMA = OBJECT_SCHEMA_NAME(b.id, DB_ID(@P1)) \
           AND bc.TABLE_NAME = OBJECT_NAME(b.id, DB_ID(@P1))"
    )
}

/// Reads the column of one row of [`snapshot_query`]. A synonym with no
/// known column gives a row with no column name, and so no column.
fn snapshot_column(name: Option<&str>, data_type: Option<&str>) -> Option<SnapshotColumn> {
    name.map(|name| SnapshotColumn {
        name: name.to_string(),
        data_type: data_type.unwrap_or_default().to_string(),
    })
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

/// Reads one event of one trigger of one relation for each row. The name
/// of the relation reaches `OBJECT_ID` as a parameter. The parent class 1
/// selects the triggers of a table or a view, so a trigger of the database,
/// which fires on a change of the schema, stays out of the list.
fn trigger_query(catalog: &str) -> String {
    format!(
        "SELECT tr.name, tr.is_instead_of_trigger, tr.is_disabled, te.type_desc \
         FROM {catalog}.sys.triggers AS tr \
         LEFT JOIN {catalog}.sys.trigger_events AS te ON te.object_id = tr.object_id \
         WHERE tr.parent_class = 1 AND tr.parent_id = OBJECT_ID(@P1) \
         ORDER BY tr.name, te.type"
    )
}

/// Reads the time of a trigger from its `is_instead_of_trigger` flag. MS
/// SQL Server has no `BEFORE` trigger, and a `FOR` trigger runs after the
/// change.
fn trigger_timing_of(instead_of: bool) -> TriggerTiming {
    if instead_of {
        TriggerTiming::InsteadOf
    } else {
        TriggerTiming::After
    }
}

/// Reads one column of one constraint for each row. A foreign key carries the
/// relation it points at, and a check carries its rule. `OBJECT_NAME` and
/// `SCHEMA_ID` look in the current database, so the statement joins the
/// `sys` views of the named database instead. A foreign key matches on its
/// schema and its name, because two schemas can each have a key of one name.
/// The relation of a foreign key gets its schema, because a key can point at
/// a table of a different schema.
///
/// `INFORMATION_SCHEMA` does not list a `DEFAULT` constraint, so the second
/// part of the statement reads `sys.default_constraints`. A default gives
/// its expression as the detail. The `ORDER BY` of a `UNION` can name only
/// the columns that the statement returns, so the statement also returns the
/// position of the column.
fn constraint_query(catalog: &str) -> String {
    format!(
        "SELECT tc.CONSTRAINT_NAME AS constraint_name, \
                tc.CONSTRAINT_TYPE, \
                ku.COLUMN_NAME, \
                rs.name + N'.' + ro.name, \
                cc.CHECK_CLAUSE, \
                ku.ORDINAL_POSITION AS position \
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
         LEFT JOIN ({catalog}.sys.objects AS ro \
                    JOIN {catalog}.sys.schemas AS rs ON rs.schema_id = ro.schema_id) \
                ON ro.object_id = fk.referenced_object_id \
         WHERE tc.TABLE_SCHEMA = @P1 AND tc.TABLE_NAME = @P2 \
         UNION ALL \
         SELECT dc.name, N'DEFAULT', c.name, NULL, dc.definition, 1 \
         FROM {catalog}.sys.default_constraints AS dc \
         JOIN {catalog}.sys.columns AS c \
           ON c.object_id = dc.parent_object_id AND c.column_id = dc.parent_column_id \
         JOIN {catalog}.sys.tables AS t ON t.object_id = dc.parent_object_id \
         JOIN {catalog}.sys.schemas AS s ON s.schema_id = t.schema_id \
         WHERE s.name = @P1 AND t.name = @P2 \
         ORDER BY constraint_name, position"
    )
}

/// Joins the base type with the length or the precision, so that the
/// explorer shows `varchar(50)` and not `varchar`.
///
/// A `datetime2`, `time` or `datetimeoffset` column takes the count of the
/// digits of its fraction of a second as `scale`. INFORMATION_SCHEMA gives
/// that count as DATETIME_PRECISION. A `float` of 24 bits of precision or
/// less is stored as `real`, so it shows as `real`.
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
        "datetime2" | "time" | "datetimeoffset" => match scale {
            Some(digits) => format!("{base}({digits})"),
            None => base.to_string(),
        },
        "float" if precision.is_some_and(|bits| bits <= 24) => "real".to_string(),
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
pub fn row_to_json(row: &Row) -> Vec<JsonValue> {
    row.cells()
        .map(|(_, data)| column_data_to_json(data))
        .collect()
}

/// Turns the data of one cell into JSON. The data gives the JSON value, so a
/// read needs no target type that can fail to match.
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
        ColumnData::Binary(value) => value.as_ref().map_or(JsonValue::Null, |bytes| {
            JsonValue::String(bytes_to_hex(bytes))
        }),
        // The copy of `tiberius` reads a money value as a decimal, so each of
        // its 19 digits stays.
        ColumnData::Numeric(value) => value.map_or(JsonValue::Null, |value| {
            JsonValue::String(numeric_to_string(value))
        }),
        ColumnData::Xml(value) => value.as_ref().map_or(JsonValue::Null, |value| {
            JsonValue::String(value.to_string())
        }),
        // The server keeps a `datetime` value in units of 1/300 of a second.
        // The text rounds the value to the nearest millisecond, as SQL
        // Server Management Studio does, so the value `.007` stays `.007`.
        ColumnData::DateTime(value) => text_or_null(value.and_then(|value| {
            let millis = (u64::from(value.seconds_fragments()) * 10 + 1) / 3;
            moment_text(DAYS_TO_1900 + i64::from(value.days()), millis as i64, 3)
        })),
        // A `smalldatetime` value counts the minutes after midnight.
        ColumnData::SmallDateTime(value) => text_or_null(value.and_then(|value| {
            moment_text(
                DAYS_TO_1900 + i64::from(value.days()),
                i64::from(value.seconds_fragments()) * 60,
                0,
            )
        })),
        ColumnData::DateTime2(value) => text_or_null(value.and_then(|value| {
            let time = value.time();
            moment_text(
                i64::from(value.date().days()),
                time.increments() as i64,
                time.scale(),
            )
        })),
        ColumnData::Date(value) => text_or_null(
            value
                .and_then(|value| date_after(i64::from(value.days())).map(|date| date.to_string())),
        ),
        ColumnData::Time(value) => {
            text_or_null(value.map(|value| time_text(value.increments(), value.scale())))
        }
        // The server sends the moment in UTC with the offset in minutes. The
        // text shows the local time and the offset, as SQL Server Management
        // Studio does.
        ColumnData::DateTimeOffset(value) => text_or_null(value.and_then(|value| {
            let datetime = value.datetime2();
            let time = datetime.time();
            let offset = i64::from(value.offset());
            let unit = 10i64.pow(u32::from(time.scale()));
            let local = moment_text(
                i64::from(datetime.date().days()),
                time.increments() as i64 + offset * 60 * unit,
                time.scale(),
            )?;
            let sign = if offset < 0 { '-' } else { '+' };
            let minutes = offset.abs();
            Some(format!(
                "{local} {sign}{:02}:{:02}",
                minutes / 60,
                minutes % 60
            ))
        })),
    }
}

/// The days from 0001-01-01 to 1900-01-01, the first day of `datetime` and
/// `smalldatetime`.
const DAYS_TO_1900: i64 = 693_595;

/// Gives the date that lies the given number of days after 0001-01-01, or
/// `None` when the date is outside the range of `chrono`.
fn date_after(days: i64) -> Option<NaiveDate> {
    let days = u64::try_from(days).ok()?;
    NaiveDate::from_ymd_opt(1, 1, 1)?.checked_add_days(chrono::Days::new(days))
}

/// Writes a time of day, given as increments of 10^-scale seconds after
/// midnight, with as many digits of fraction as the scale of the column.
/// A `time(7)` value shows as `12:00:00.1234567` and a `time(0)` value as
/// `12:00:00`, as in SQL Server Management Studio.
fn time_text(increments: u64, scale: u8) -> String {
    let unit = 10u64.pow(u32::from(scale));
    let seconds = increments / unit;
    let clock = format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    );
    if scale == 0 {
        return clock;
    }
    format!(
        "{clock}.{:0width$}",
        increments % unit,
        width = usize::from(scale)
    )
}

/// Writes a date and a time with a space between them. The increments of
/// 10^-scale seconds count from midnight of the given day, and a count
/// below zero or past one day moves the date.
fn moment_text(days: i64, increments: i64, scale: u8) -> Option<String> {
    let per_day = 86_400 * 10i64.pow(u32::from(scale));
    let date = date_after(days + increments.div_euclid(per_day))?;
    let time = time_text(increments.rem_euclid(per_day) as u64, scale);
    Some(format!("{date} {time}"))
}

/// Gives a JSON text, or the null of JSON for no text.
fn text_or_null(text: Option<String>) -> JsonValue {
    text.map_or(JsonValue::Null, JsonValue::String)
}

/// Writes bytes as the hexadecimal text that SQL Server Management Studio
/// shows, such as `0xE6100000`. The server sends `binary`, `varbinary`,
/// `image`, `rowversion` and each user-defined type, such as `geography`, as
/// bytes. The driver cannot decode the format of each user-defined type.
fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(2 + bytes.len() * 2);
    text.push_str("0x");
    hex_text(&mut text, bytes, true);
    text
}

#[cfg(test)]
mod live;

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn the_lock_limit_goes_to_the_server_in_milliseconds() {
        assert_eq!(
            lock_timeout_statement(Duration::from_millis(5_500)),
            "SET LOCK_TIMEOUT 5500"
        );
    }

    #[tokio::test]
    async fn the_socket_of_a_connection_sends_keepalive_probes() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tcp = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        assert!(keep_alive(socket2::SockRef::from(&tcp)));
        assert!(socket2::SockRef::from(&tcp).keepalive().unwrap());

        // A socket without TCP refuses the option, and the open goes on.
        let udp = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        assert!(!keep_alive(socket2::SockRef::from(&udp)));
    }

    /// The types of packet that the test reads from the client.
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

    /// Reads one message of the client and gives back the type of its first
    /// packet. A message can arrive in several packets, and the last of them
    /// carries the end flag.
    async fn read_message(server: &mut tokio::net::TcpStream) -> u8 {
        let mut packet_type = None;
        loop {
            let mut header = [0u8; 8];
            server.read_exact(&mut header).await.unwrap();
            let length = u16::from_be_bytes([header[2], header[3]]) as usize;
            let mut body = vec![0u8; length - 8];
            server.read_exact(&mut body).await.unwrap();
            packet_type.get_or_insert(header[0]);
            if header[1] & END_OF_MESSAGE == END_OF_MESSAGE {
                return packet_type.unwrap();
            }
        }
    }

    /// Writes one packet of the server, with the type `TabularResult`.
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

        let packet_type = read_message(&mut socket).await;
        assert_eq!(packet_type, PACKET_SQL_BATCH);
        let mut answer = int_metadata();
        for value in 0..5 {
            answer.extend_from_slice(&int_row(value));
        }
        write_packet(&mut socket, 0, &answer).await;

        assert_eq!(read_message(&mut socket).await, PACKET_ATTENTION);
        write_packet(&mut socket, END_OF_MESSAGE, &done_token(DONE_ATTENTION, 0)).await;

        // The connection takes the next statement of the session.
        let packet_type = read_message(&mut socket).await;
        assert_eq!(packet_type, PACKET_SQL_BATCH);
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
            .any(|text| text.contains("Stopped at the row limit")));
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

        let packet_type = read_message(&mut socket).await;
        assert_eq!(packet_type, PACKET_SQL_BATCH);
        let mut rows = int_metadata();
        for value in 0..5 {
            rows.extend_from_slice(&int_row(value));
        }
        write_packet(&mut socket, 0, &rows).await;

        let pause = Duration::from_millis(300);
        let ended_early = match tokio::time::timeout(pause, read_message(&mut socket)).await {
            Ok(packet_type) => {
                assert_eq!(packet_type, PACKET_ATTENTION);
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
            .explain("SELECT a FROM b", None, PlanMode::Actual, &options)
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
        while let Ok(packet_type) = tokio::time::timeout(pause, read_message(&mut socket)).await {
            assert_eq!(packet_type, PACKET_SQL_BATCH);
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
    async fn the_facts_of_a_pause_name_the_session_on_the_server() {
        let (mut driver, server) = driver_that_counts().await;
        // The fake server answers the first request with the number 0.
        let facts = driver.pause_facts().await.unwrap();
        assert_eq!(facts.server_session, Some(0));
        assert_eq!(server.await.unwrap(), 1);
        drop(driver);
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
    fn text_token(token_type: u8, class: u8, text: &str) -> Vec<u8> {
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
        let mut token = vec![token_type];
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

        let packet_type = read_message(&mut socket).await;
        assert_eq!(packet_type, PACKET_SQL_BATCH);
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
            let packet_type = read_message(&mut socket).await;
            assert_eq!(packet_type, PACKET_SQL_BATCH);
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

    /// Runs the query against a fake server that checks the type of each
    /// request and sends the answer that goes with it. The query must not be
    /// one that only reads, so that no probe of the row limit goes before
    /// it.
    async fn run_scripted(
        query: &str,
        params: Option<QueryParams>,
        exchanges: Vec<(u8, Vec<u8>)>,
    ) -> Result<QueryResponse> {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            accept_login(&mut socket).await;
            for (packet_type, answer) in exchanges {
                assert_eq!(read_message(&mut socket).await, packet_type);
                write_packet(&mut socket, END_OF_MESSAGE, &answer).await;
            }
        });
        let tcp = TcpStream::connect(address).await.unwrap();
        let client = Client::connect(test_config(), tcp.compat_write())
            .await
            .unwrap();
        let mut driver = MssqlDriver { client };
        let options = ExecOptions::default();
        let mut sink = BufferSink::new(10);
        let outcome = driver
            .execute_stream(query, params.as_ref(), &options, &mut sink)
            .await;
        server.await.unwrap();
        outcome.map(|summary| sink.into_response(summary))
    }

    /// The answer to `SELECT @@TRANCOUNT`.
    fn transaction_count_answer(count: i32) -> Vec<u8> {
        let mut answer = int_metadata();
        answer.extend_from_slice(&int_row(count));
        answer.extend_from_slice(&done_token(DONE_COUNT, 1));
        answer
    }

    /// The answer of a call of `sp_executesql` whose count of open
    /// transactions at its end differs from the count at its start.
    fn mismatch_answer() -> Vec<u8> {
        let mut answer = counted_done_in_proc(1);
        answer.extend_from_slice(&error_token(
            TRANSACTION_COUNT_MISMATCH,
            "Transaction count after EXECUTE indicates a mismatching number of BEGIN and \
             COMMIT statements.",
        ));
        answer.extend_from_slice(&done_token(0x02, 0));
        answer
    }

    fn one_param() -> Option<QueryParams> {
        Some(vec![crate::db::QueryParam {
            value: JsonValue::from(1),
        }])
    }

    #[tokio::test]
    async fn a_run_with_parameters_that_opens_a_transaction_gives_a_warning() {
        let response = run_scripted(
            "BEGIN TRANSACTION; UPDATE t SET a = @P1",
            one_param(),
            vec![
                (PACKET_SQL_BATCH, transaction_count_answer(0)),
                (PACKET_RPC, mismatch_answer()),
                (PACKET_SQL_BATCH, transaction_count_answer(1)),
            ],
        )
        .await
        .unwrap();

        let warning = response.messages.last().unwrap();
        assert_eq!(warning.level, MessageLevel::Warning);
        assert!(warning.text.contains("left it open"), "{}", warning.text);
        let detail = warning.detail.as_deref().unwrap();
        assert!(detail.contains("Msg 266"), "{detail}");
        assert!(
            detail.contains("Transaction count after EXECUTE"),
            "{detail}"
        );
        assert_eq!(response.rows_affected, Some(1));
    }

    #[tokio::test]
    async fn a_run_with_parameters_that_ends_a_transaction_gives_a_warning() {
        let response = run_scripted(
            "UPDATE t SET a = @P1; COMMIT",
            one_param(),
            vec![
                (PACKET_SQL_BATCH, transaction_count_answer(1)),
                (PACKET_RPC, mismatch_answer()),
                (PACKET_SQL_BATCH, transaction_count_answer(0)),
            ],
        )
        .await
        .unwrap();

        let warning = response.messages.last().unwrap();
        assert_eq!(warning.level, MessageLevel::Warning);
        assert!(
            warning.text.contains("ended a transaction"),
            "{}",
            warning.text
        );
    }

    #[tokio::test]
    async fn error_266_fails_the_run_when_the_count_did_not_change() {
        // XACT_ABORT, for example, can roll the transaction back after the
        // error, so the count is the same as before the run.
        let error = run_scripted(
            "BEGIN TRANSACTION; UPDATE t SET a = @P1",
            one_param(),
            vec![
                (PACKET_SQL_BATCH, transaction_count_answer(0)),
                (PACKET_RPC, mismatch_answer()),
                (PACKET_SQL_BATCH, transaction_count_answer(0)),
            ],
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("Transaction count"), "{error}");
    }

    #[tokio::test]
    async fn error_266_of_a_plain_batch_fails_the_run() {
        // A procedure that a plain batch runs can leave a transaction open,
        // and the server then names the procedure. The error stays an error,
        // as in SQL Server Management Studio.
        let error = run_scripted(
            "EXEC opens_a_transaction",
            None,
            vec![(PACKET_SQL_BATCH, mismatch_answer())],
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("Transaction count"), "{error}");
    }

    #[test]
    fn a_walk_without_error_266_passes() {
        let walk = Walk {
            stopped: false,
            rows_affected: None,
            mismatch: None,
        };
        assert!(mismatch_as_error(walk).is_ok());
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
    async fn a_user_defined_type_shows_its_name_and_its_bytes_in_hexadecimal() {
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

        assert_eq!(response.results[0].columns[0].type_name, "hierarchyid");
        assert_eq!(
            response.results[0].rows,
            vec![vec![JsonValue::from("0x5840")], vec![JsonValue::Null]]
        );
    }

    #[tokio::test]
    async fn a_rowversion_and_a_chunked_varbinary_show_their_bytes_in_hexadecimal() {
        // A `COLMETADATA` token for a `rowversion` column `r` and a
        // `varbinary(max)` column `v`. The user type 80 marks a `rowversion`
        // column, whose data is a `binary(8)` value.
        let mut answer = vec![0x81];
        answer.extend_from_slice(&2u16.to_le_bytes());
        answer.extend_from_slice(&80u32.to_le_bytes());
        answer.extend_from_slice(&0u16.to_le_bytes());
        answer.push(0xAD);
        answer.extend_from_slice(&8u16.to_le_bytes());
        answer.push(1);
        answer.extend_from_slice(&('r' as u16).to_le_bytes());
        answer.extend_from_slice(&0u32.to_le_bytes());
        answer.extend_from_slice(&0u16.to_le_bytes());
        answer.push(0xA5);
        answer.extend_from_slice(&u16::MAX.to_le_bytes());
        answer.push(1);
        answer.extend_from_slice(&('v' as u16).to_le_bytes());
        // The `varbinary(max)` value comes in two chunks.
        answer.push(0xD1);
        answer.extend_from_slice(&8u16.to_le_bytes());
        answer.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0x07, 0xD1]);
        answer.extend_from_slice(&3u64.to_le_bytes());
        answer.extend_from_slice(&1u32.to_le_bytes());
        answer.push(0xAB);
        answer.extend_from_slice(&2u32.to_le_bytes());
        answer.extend_from_slice(&[0xCD, 0xEF]);
        answer.extend_from_slice(&0u32.to_le_bytes());
        answer.extend_from_slice(&done_token(DONE_COUNT, 1));

        let response = run_against_answer(
            "INSERT INTO t(v) OUTPUT inserted.r, inserted.v VALUES (0xABCDEF)",
            answer,
            10,
        )
        .await;

        assert_eq!(
            response.results[0].rows,
            vec![vec![
                JsonValue::from("0x00000000000007D1"),
                JsonValue::from("0xABCDEF")
            ]]
        );
    }

    #[tokio::test]
    async fn a_block_shows_its_counts_and_its_rows() {
        // The fake answer ends each statement with a `DONEINPROC` token, as
        // the server does for a statement inside a `BEGIN TRY` block.
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
            ["1 row returned. Stopped at the row limit."]
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

            let packet_type = read_message(&mut socket).await;
            assert_eq!(packet_type, PACKET_SQL_BATCH);
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

            let packet_type = read_message(&mut socket).await;
            assert_eq!(packet_type, PACKET_SQL_BATCH);
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
        use tiberius::xml::XmlData;

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
            column_data_to_json(&ColumnData::Binary(Some(Cow::from(vec![0x0Au8, 0x1B])))),
            JsonValue::String("0x0A1B".into())
        );
        // An empty value shows only the prefix, as SQL Server Management
        // Studio shows it.
        assert_eq!(
            column_data_to_json(&ColumnData::Binary(Some(Cow::from(Vec::<u8>::new())))),
            JsonValue::String("0x".into())
        );
        assert_eq!(
            column_data_to_json(&ColumnData::Numeric(Some(Numeric::new_with_scale(125, 2)))),
            JsonValue::String("1.25".into())
        );
        assert_eq!(
            column_data_to_json(&ColumnData::Xml(Some(Cow::Owned(XmlData::new("<a/>"))))),
            JsonValue::String("<a/>".into())
        );
        // The server keeps 1/300 of a second, and the grid shows the three
        // digits of milliseconds that the server shows.
        assert_eq!(
            column_data_to_json(&ColumnData::DateTime(Some(DateTime::new(0, 0)))),
            JsonValue::String("1900-01-01 00:00:00.000".into())
        );
        assert_eq!(
            column_data_to_json(&ColumnData::DateTime(Some(DateTime::new(0, 2)))),
            JsonValue::String("1900-01-01 00:00:00.007".into())
        );
        assert_eq!(
            column_data_to_json(&ColumnData::DateTime(Some(DateTime::new(0, 299)))),
            JsonValue::String("1900-01-01 00:00:00.997".into())
        );
        // A `datetime` value before 1900 has a count of days below zero.
        assert_eq!(
            column_data_to_json(&ColumnData::DateTime(Some(DateTime::new(-53690, 0)))),
            JsonValue::String("1753-01-01 00:00:00.000".into())
        );
        // A `smalldatetime` value counts minutes and shows no fraction.
        assert_eq!(
            column_data_to_json(&ColumnData::SmallDateTime(Some(SmallDateTime::new(1, 61)))),
            JsonValue::String("1900-01-02 01:01:00".into())
        );
        assert_eq!(
            column_data_to_json(&ColumnData::DateTime2(Some(DateTime2::new(
                Date::new(0),
                Time::new(0, 0)
            )))),
            JsonValue::String("0001-01-01 00:00:00".into())
        );
        // A value shows as many digits of fraction as the scale of the
        // column, as in SQL Server Management Studio.
        assert_eq!(
            column_data_to_json(&ColumnData::DateTime2(Some(DateTime2::new(
                Date::new(730119),
                Time::new(432_001_234_567, 7)
            )))),
            JsonValue::String("2000-01-01 12:00:00.1234567".into())
        );
        assert_eq!(
            column_data_to_json(&ColumnData::DateTime2(Some(DateTime2::new(
                Date::new(730119),
                Time::new(4_320_005, 2)
            )))),
            JsonValue::String("2000-01-01 12:00:00.05".into())
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
            column_data_to_json(&ColumnData::Time(Some(Time::new(432_001_234_567, 7)))),
            JsonValue::String("12:00:00.1234567".into())
        );
        assert_eq!(
            column_data_to_json(&ColumnData::Time(Some(Time::new(45_296_120, 3)))),
            JsonValue::String("12:34:56.120".into())
        );
        assert_eq!(
            column_data_to_json(&ColumnData::DateTimeOffset(Some(DateTimeOffset::new(
                DateTime2::new(Date::new(730119), Time::new(0, 0)),
                0
            )))),
            JsonValue::String("2000-01-01 00:00:00 +00:00".into())
        );
        // The server sends the moment in UTC with the offset beside it. The
        // text shows the local time and the offset.
        assert_eq!(
            column_data_to_json(&ColumnData::DateTimeOffset(Some(DateTimeOffset::new(
                DateTime2::new(Date::new(730119), Time::new(14 * 3600 * 10_000_000, 7)),
                -300
            )))),
            JsonValue::String("2000-01-01 09:00:00.0000000 -05:00".into())
        );
        // The local time can fall on the day before or the day after the
        // day in UTC.
        assert_eq!(
            column_data_to_json(&ColumnData::DateTimeOffset(Some(DateTimeOffset::new(
                DateTime2::new(Date::new(730119), Time::new(3600, 0)),
                -330
            )))),
            JsonValue::String("1999-12-31 19:30:00 -05:30".into())
        );
        assert_eq!(
            column_data_to_json(&ColumnData::DateTimeOffset(Some(DateTimeOffset::new(
                DateTime2::new(Date::new(730119), Time::new(23 * 36_000, 1)),
                60
            )))),
            JsonValue::String("2000-01-02 00:00:00.0 +01:00".into())
        );
        // A date outside the range of `chrono` gives the null of JSON.
        assert_eq!(
            column_data_to_json(&ColumnData::DateTime(Some(DateTime::new(i32::MIN, 0)))),
            JsonValue::Null
        );
        assert_eq!(
            column_data_to_json(&ColumnData::DateTimeOffset(Some(DateTimeOffset::new(
                DateTime2::new(Date::new(0), Time::new(0, 0)),
                -60
            )))),
            JsonValue::Null
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

    /// A server that sends five rows and the end of the statement at once,
    /// and waits for no attention packet.
    async fn serve_five_rows(listener: TcpListener) {
        let (mut socket, _) = listener.accept().await.unwrap();
        accept_login(&mut socket).await;
        let packet_type = read_message(&mut socket).await;
        assert_eq!(packet_type, PACKET_SQL_BATCH);
        let mut answer = int_metadata();
        for value in 0..5 {
            answer.extend_from_slice(&int_row(value));
        }
        answer.extend_from_slice(&done_token(0, 5));
        write_packet(&mut socket, END_OF_MESSAGE, &answer).await;
    }

    #[tokio::test]
    async fn a_batch_of_several_statements_walks_the_rest_of_the_result() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(serve_five_rows(listener));

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
        let mut sink = crate::db::sink::probe::Telling::new(options.max_rows);
        driver
            .stream_sets("SELECT a FROM b", &[], &options, &mut sink, false)
            .await
            .unwrap();
        // The walk drops three rows past the limit and tells the sink once.
        assert_eq!(sink.told, 1);
        let response = sink.buffer.into_response(RunSummary::default());

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
    async fn a_sink_that_stops_a_batch_hears_that_the_walk_goes_on() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(serve_five_rows(listener));

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
        let mut sink = crate::db::sink::probe::Telling::new(1);
        let walk = driver
            .stream_sets("SELECT a FROM b", &[], &options, &mut sink, false)
            .await
            .unwrap();
        assert!(walk.stopped);
        assert_eq!(sink.told, 1);

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
        let mut sink = crate::db::sink::probe::Telling::new(2);
        let stopped = driver
            .stream_sets("SELECT a FROM b", &[], &options, &mut sink, true)
            .await
            .unwrap()
            .stopped;
        // The attention packet ends the statement, so the walk reads on
        // through the rows in flight alone and tells the sink nothing.
        assert_eq!(sink.told, 0);
        let response = sink.buffer.into_response(RunSummary::default());

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
        let attention = Arc::new(AttentionHandle::default());
        let handle = MssqlCancel(attention.clone());
        assert!(!attention.is_signalled());
        handle.cancel().await.unwrap();
        assert!(attention.is_signalled());
        // A second request for a statement that already stopped does no harm.
        handle.cancel().await.unwrap();
        assert!(attention.is_signalled());
    }

    #[test]
    fn each_plan_has_its_own_session_switch() {
        assert_eq!(plan_switch(PlanMode::Estimated), "SHOWPLAN_XML");
        assert_eq!(plan_switch(PlanMode::Actual), "STATISTICS XML");
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
    fn the_snapshot_statement_reads_the_columns_and_the_relation_type() {
        let text = snapshot_query("[Sales]", 10);
        assert!(text.contains("FROM [Sales].INFORMATION_SCHEMA.COLUMNS AS c"));
        assert!(text.contains("JOIN [Sales].INFORMATION_SCHEMA.TABLES AS t"));
        assert!(text.ends_with("ORDER BY w.sch, w.rel, w.ord"));
    }

    #[test]
    fn the_snapshot_statement_sends_one_column_past_the_bound() {
        let text = snapshot_query("[Sales]", 10);
        assert!(text.contains("PARTITION BY CASE WHEN u.col IS NULL THEN 0 ELSE 1 END"));
        assert!(text.contains("WHERE w.col IS NULL OR w.n <= 11 "));
        assert!(snapshot_query("[Sales]", usize::MAX).contains(&format!("w.n <= {} ", i64::MAX)));
    }

    #[test]
    fn the_snapshot_statement_gives_a_synonym_the_columns_of_its_target() {
        let text = snapshot_query("[Sales]", 10);
        assert!(text.contains("UNION ALL SELECT s.name, sy.name, N'SYNONYM', bc.COLUMN_NAME"));
        assert!(text.contains("FROM [Sales].sys.synonyms AS sy"));
        // The target resolves in the database of the snapshot, and an
        // absent schema stays empty.
        assert!(text.contains("OBJECT_ID(QUOTENAME(@P1) + N'.'"));
        assert!(text.contains("ISNULL(QUOTENAME(PARSENAME(sy.base_object_name, 2)), N'')"));
        // A target on another server or in another database gets no column.
        assert!(text.contains("WHERE PARSENAME(sy.base_object_name, 4) IS NULL"));
        assert!(text.contains("AND ISNULL(PARSENAME(sy.base_object_name, 3), @P1) = @P1"));
        // Only a table or a view gives columns, and a synonym with none
        // keeps its row.
        assert!(text.contains("LEFT JOIN ([Sales].INFORMATION_SCHEMA.COLUMNS AS bc"));
        assert!(text.contains("JOIN [Sales].INFORMATION_SCHEMA.TABLES AS bt"));
        assert!(text.contains("ON bc.TABLE_SCHEMA = OBJECT_SCHEMA_NAME(b.id, DB_ID(@P1))"));
        assert!(text.contains("AND bc.TABLE_NAME = OBJECT_NAME(b.id, DB_ID(@P1))"));
    }

    #[test]
    fn a_snapshot_row_with_no_column_name_gives_no_column() {
        assert_eq!(snapshot_column(None, None), None);
        assert_eq!(
            snapshot_column(Some("id"), Some("int")),
            Some(SnapshotColumn {
                name: "id".into(),
                data_type: "int".into(),
            })
        );
        assert_eq!(snapshot_column(Some("id"), None).unwrap().data_type, "");
    }

    #[test]
    fn the_word_of_a_synonym_names_its_relation_type() {
        assert_eq!(relation_type_of("SYNONYM"), RelationType::Synonym);
        assert_eq!(relation_type_of("VIEW"), RelationType::View);
        assert_eq!(relation_type_of("BASE TABLE"), RelationType::Table);
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
        // The relation of a foreign key gets its schema.
        assert!(constraints.contains("rs.name + N'.' + ro.name"));
        assert!(constraints.contains(
            "JOIN [Sales].sys.schemas AS rs ON rs.schema_id = ro.schema_id) \
             ON ro.object_id = fk.referenced_object_id"
        ));
        assert!(constraints.contains("WHERE tc.TABLE_SCHEMA = @P1 AND tc.TABLE_NAME = @P2"));
        // A `DEFAULT` constraint comes from the `sys` views, with its
        // expression as the detail.
        assert!(constraints
            .contains("UNION ALL SELECT dc.name, N'DEFAULT', c.name, NULL, dc.definition, 1"));
        assert!(constraints.contains("FROM [Sales].sys.default_constraints AS dc"));
        assert!(constraints.contains("WHERE s.name = @P1 AND t.name = @P2"));
        assert!(constraints.ends_with("ORDER BY constraint_name, position"));
    }

    #[test]
    fn the_create_statement_covers_a_view_alone() {
        let view = create_query_text(Some("db"), Some("dbo"), "v", RelationType::View).unwrap();
        assert_eq!(
            view.sql,
            "SELECT m.definition FROM [db].sys.sql_modules AS m \
             WHERE m.object_id = OBJECT_ID('[db].[dbo].[v]');"
        );
        assert_eq!(view.column, 0);
        // With no database the statement reads the current one.
        let current = create_query_text(None, Some("dbo"), "v", RelationType::View).unwrap();
        assert_eq!(
            current.sql,
            "SELECT m.definition FROM sys.sql_modules AS m \
             WHERE m.object_id = OBJECT_ID('[dbo].[v]');"
        );
        assert!(create_query_text(Some("db"), Some("dbo"), "t", RelationType::Table).is_none());
    }

    #[test]
    fn the_triggers_come_from_the_catalog_of_the_named_database() {
        let text = trigger_query("[Sales]");
        assert!(text
            .starts_with("SELECT tr.name, tr.is_instead_of_trigger, tr.is_disabled, te.type_desc"));
        assert!(text.contains("FROM [Sales].sys.triggers AS tr"));
        assert!(text.contains("LEFT JOIN [Sales].sys.trigger_events AS te"));
        // A trigger of the database has the parent class 0.
        assert!(text.contains("WHERE tr.parent_class = 1 AND tr.parent_id = OBJECT_ID(@P1)"));
        assert!(text.ends_with("ORDER BY tr.name, te.type"));
        assert_eq!(trigger_timing_of(true), TriggerTiming::InsteadOf);
        assert_eq!(trigger_timing_of(false), TriggerTiming::After);
    }

    #[test]
    fn the_create_statement_of_a_trigger_reads_its_module() {
        let trigger =
            object_query_text(Some("db"), Some("dbo"), "audit", ObjectType::Trigger).unwrap();
        assert_eq!(
            trigger.sql,
            "SELECT m.definition FROM [db].sys.sql_modules AS m \
             WHERE m.object_id = OBJECT_ID('[db].[dbo].[audit]');"
        );
        assert!(object_query_text(Some("db"), Some("dbo"), "e", ObjectType::Event).is_none());
    }

    #[test]
    fn the_create_statement_of_a_synonym_names_its_target() {
        let synonym =
            create_query_text(Some("db"), Some("dbo"), "s", RelationType::Synonym).unwrap();
        assert_eq!(
            synonym.sql,
            "SELECT N'CREATE SYNONYM ' + QUOTENAME(s.name) + N'.' + QUOTENAME(sy.name) + \
             N' FOR ' + sy.base_object_name + N';' \
             FROM [db].sys.synonyms AS sy \
             JOIN [db].sys.schemas AS s ON s.schema_id = sy.schema_id \
             WHERE sy.object_id = OBJECT_ID('[db].[dbo].[s]');"
        );
        assert_eq!(synonym.column, 0);
    }

    #[test]
    fn the_list_of_tables_adds_the_synonyms_with_their_target() {
        let text = tables_query("[Sales]");
        assert!(text.contains("FROM [Sales].INFORMATION_SCHEMA.TABLES WHERE TABLE_SCHEMA = @P1"));
        assert!(text.contains("UNION ALL SELECT sy.name, N'SYNONYM', sy.base_object_name"));
        assert!(text.contains("FROM [Sales].sys.synonyms AS sy"));
        assert!(text.contains("JOIN [Sales].sys.schemas AS s"));
        assert!(text.ends_with("ORDER BY 2, 1"));

        assert_eq!(relation_of("t", "BASE TABLE", None), Table::table("t"));
        assert_eq!(relation_of("v", "VIEW", None), Table::view("v"));
        assert_eq!(
            relation_of("s", "SYNONYM", Some("[Other].[dbo].[t]")),
            Table::synonym("s", "[Other].[dbo].[t]")
        );
        assert_eq!(relation_of("s", "SYNONYM", None), Table::synonym("s", ""));
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
        // A string without a credential takes the method of the form.
        assert!(matches!(
            config.get_authentication(),
            AuthMethod::Integrated
        ));

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
        assert_eq!(encryption_level(TlsMode::Prefer), EncryptionLevel::Required);
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
    fn the_keys_of_a_connection_string_are_read_past_quoted_values() {
        assert_eq!(
            string_keys("Server=a;Password={x;encrypt=1};User Id = sa; Encrypt=\"a;b\";App='q;r'"),
            ["server", "password", "user id", "encrypt", "app"]
        );
        assert_eq!(
            string_keys("jdbc:sqlserver://h:1433;databaseName=d;trustServerCertificate=true"),
            ["databasename", "trustservercertificate"]
        );
        assert!(string_keys("jdbc:sqlserver://h").is_empty());
        assert_eq!(string_keys("Server=a;;"), ["server"]);
    }

    #[tokio::test]
    async fn a_connection_string_takes_the_settings_of_the_form_that_it_does_not_give() {
        let mut input = connection();
        input.options.tls_mode = TlsMode::Prefer;
        input.options.application_name = Some("Explorer".into());
        input.options.connection_url = Some("Server=tcp:h,1433;User Id=sa".into());
        let config = build_config(&input).await.unwrap();
        let debug = format!("{config:?}");
        assert!(debug.contains("encryption: Required"), "{debug}");
        assert!(debug.contains("TrustAll"), "{debug}");
        assert!(debug.contains("Explorer"), "{debug}");

        input.options.connection_url =
            Some("Server=tcp:h,1433;User Id=sa;Encrypt=false;TrustServerCertificate=false;Application Name=Mine".into());
        let config = build_config(&input).await.unwrap();
        let debug = format!("{config:?}");
        assert!(!debug.contains("encryption: Required"), "{debug}");
        assert!(!debug.contains("TrustAll"), "{debug}");
        assert!(
            debug.contains("Mine") && !debug.contains("Explorer"),
            "{debug}"
        );

        input.options.tls_mode = TlsMode::VerifyFull;
        input.options.ca_cert_path = Some("/tmp/ca.pem".into());
        input.options.connection_url = Some("Server=tcp:h,1433;User Id=sa".into());
        let debug = format!("{:?}", build_config(&input).await.unwrap());
        assert!(debug.contains("ca.pem"), "{debug}");

        input.options.mssql_auth = MssqlAuth::EntraAccessToken;
        input.password = Some("token".into());
        input.options.connection_url = Some("Server=tcp:h,1433".into());
        let config = build_config(&input).await.unwrap();
        assert!(matches!(
            config.get_authentication(),
            AuthMethod::AADToken(_)
        ));
        input.options.connection_url = Some("Server=tcp:h,1433;User Id=sa;Password=p".into());
        let config = build_config(&input).await.unwrap();
        assert!(matches!(
            config.get_authentication(),
            AuthMethod::SqlServer(_)
        ));
    }

    #[test]
    fn a_server_error_names_its_line_in_the_whole_text() {
        let server = |line| {
            Error::Tiberius(tiberius::error::Error::Server(
                tiberius::error::TokenError::new(207, 1, 16, "Invalid column name 'x'.", "", line),
            ))
        };
        let query = "SELECT 1\nGO\nSELECT 2\nSELECT x";
        let start = query.find("SELECT 2");
        let payload = locate_error(server(2), query, start).to_payload();
        assert_eq!((payload.line, payload.column), (Some(4), Some(1)));

        let payload = locate_error(server(0), query, start).to_payload();
        assert_eq!(payload.line, None);
        let payload = locate_error(server(2), query, None).to_payload();
        assert_eq!(payload.line, None);
        let payload = locate_error(Error::Timeout(1), query, start).to_payload();
        assert_eq!(payload.line, None);
    }

    #[test]
    fn a_later_error_shows_the_text_of_sql_server_management_studio() {
        let token =
            tiberius::error::TokenError::new(208, 1, 16, "Invalid object name 't'.", "p", 3);
        let message = later_error(&token);
        assert_eq!(
            message.detail.as_deref(),
            Some(mssql_error_detail(&token).as_str())
        );
        assert!(message.detail.unwrap().contains("Procedure p"));
    }

    #[tokio::test]
    async fn a_named_instance_asks_the_sql_browser_and_not_the_port_of_the_record() {
        let mut input = connection();
        input.options.instance_name = Some("SQLEXPRESS".into());
        input.port = Some(1433);
        let config = build_config(&input).await.unwrap();
        assert_eq!(config.get_instance_name(), Some("SQLEXPRESS"));
        assert!(config.get_addr().ends_with(":1434"));

        input.options.instance_name = Some("  ".into());
        input.port = Some(1500);
        let config = build_config(&input).await.unwrap();
        assert_eq!(config.get_instance_name(), None);
        assert!(config.get_addr().ends_with(":1500"));
    }

    #[tokio::test]
    async fn a_connection_string_names_its_instance() {
        let mut input = connection();
        input.options.connection_url =
            Some("Server=tcp:db.example.com\\SQLEXPRESS;User Id=sa;Password=x".into());
        let config = build_config(&input).await.unwrap();
        assert_eq!(config.get_instance_name(), Some("SQLEXPRESS"));
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
            bind_params(Some(&params)).err().unwrap().category(),
            crate::error::ErrorCategory::Configuration
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
            bind_params(Some(&params)).err().unwrap().category(),
            crate::error::ErrorCategory::Configuration
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
        assert_eq!(
            format_type("datetime2", None, None, Some(3)),
            "datetime2(3)"
        );
        assert_eq!(format_type("time", None, None, Some(0)), "time(0)");
        assert_eq!(
            format_type("datetimeoffset", None, None, Some(7)),
            "datetimeoffset(7)"
        );
        assert_eq!(format_type("datetime2", None, None, None), "datetime2");
        assert_eq!(format_type("float", None, Some(24), None), "real");
        assert_eq!(format_type("float", None, Some(53), None), "float");
        assert_eq!(format_type("float", None, None, None), "float");
    }

    #[test]
    fn the_token_is_read_from_the_answer_of_the_cli() {
        let good = r#"{"accessToken":"abc","expiresOn":"2026-01-01"}"#;
        assert_eq!(token_from_cli_output(good).unwrap(), "abc");

        for bad in [r#"{"accessToken":""}"#, r#"{"other":1}"#, "not json"] {
            assert_eq!(
                token_from_cli_output(bad).err().unwrap().category(),
                crate::error::ErrorCategory::Authentication
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
            auth_method(&input).await.err().unwrap().category(),
            crate::error::ErrorCategory::Authentication
        );
    }

    #[tokio::test]
    async fn a_token_with_a_date_in_the_past_is_refused_before_the_socket() {
        let mut input = connection();
        input.options.mssql_auth = MssqlAuth::EntraAccessToken;
        input.password = Some(token_with_claims(r#"{"exp":1000000000}"#));

        let error = auth_method(&input).await.err().unwrap();
        assert_eq!(
            error.category(),
            crate::error::ErrorCategory::Authentication
        );
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
        assert_eq!(
            error.category(),
            crate::error::ErrorCategory::Authentication
        );
        assert!(error
            .to_string()
            .contains("Couldn't run the Azure CLI at /nowhere/az. Check the Azure CLI path."));
    }

    #[test]
    fn the_advice_to_sign_in_follows_the_cli() {
        assert!(cli_refusal("Please run 'az login' to setup account.").contains("Run `az login`"));
        let other = cli_refusal("Subscription not found.");
        assert!(!other.contains("Run `az login`"));
        assert!(other.ends_with("Subscription not found."));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_token_of_the_azure_cli_serves_the_next_connection_while_it_is_valid() {
        use std::os::unix::fs::PermissionsExt;
        let token = token_with_claims(r#"{"exp":4000000000}"#);
        let dir = std::env::temp_dir().join(format!("az-cli-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("az");
        std::fs::write(
            &script,
            format!("#!/bin/sh\necho '{{\"accessToken\": \"{token}\"}}'\n"),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = Some(script.to_string_lossy().into_owned());

        assert_eq!(azure_cli_token(&path).await.unwrap(), token);
        // The second call takes the cached token, so the CLI need not run.
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(azure_cli_token(&path).await.unwrap(), token);

        // A token that ends inside the margin is not used again.
        let short = token_with_claims(r#"{"exp":1}"#);
        cache_cli_token("short", &short);
        assert_eq!(cached_cli_token("short", SystemTime::now()), None);
    }

    /// Writes an executable script named `az` into a new folder for one test.
    #[cfg(unix)]
    fn fake_cli(folder: &str, body: &str) -> (std::path::PathBuf, Option<String>) {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("{folder}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("az");
        std::fs::write(&script, format!("#!/bin/sh\n{body}")).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = Some(script.to_string_lossy().into_owned());
        (dir, path)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn connections_that_open_together_share_one_run_of_the_cli() {
        let token = token_with_claims(r#"{"exp":4000000000}"#);
        let runs = std::env::temp_dir().join(format!("az-shared-runs-{}", std::process::id()));
        let (dir, path) = fake_cli(
            "az-shared",
            &format!(
                "echo run >> '{}'\nsleep 0.3\necho '{{\"accessToken\": \"{token}\"}}'\n",
                runs.display()
            ),
        );

        let (first, second) = tokio::join!(azure_cli_token(&path), azure_cli_token(&path));
        let count = std::fs::read_to_string(&runs).unwrap().lines().count();
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_file(&runs).unwrap();
        assert_eq!(first.unwrap(), token);
        assert_eq!(second.unwrap(), token);
        assert_eq!(count, 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_time_limit_of_the_connection_covers_the_azure_cli() {
        let (dir, path) = fake_cli("az-slow", "sleep 5\n");
        let mut input = connection();
        input.options.mssql_auth = MssqlAuth::EntraAzureCli;
        input.options.azure_cli_path = path;
        input.options.connect_timeout_secs = 1;

        let started = Instant::now();
        let error = MssqlDriver::connect(&input).await.err().unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(started.elapsed() < Duration::from_secs(4));
        assert_eq!(error.category(), crate::error::ErrorCategory::Connection);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_cli_that_refuses_reports_its_reason() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("az-refuse-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("az");
        std::fs::write(
            &script,
            "#!/bin/sh\necho 'Please run az login' >&2\nexit 1\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = Some(script.to_string_lossy().into_owned());
        let error = azure_cli_token(&path).await.unwrap_err();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(error.to_string().contains("Run `az login`"));
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
            describe_login(TiberiusError::Utf8, MssqlAuth::Integrated).category(),
            crate::error::ErrorCategory::Database
        );

        // A fault of the ticket becomes an error about the credentials.
        let error = describe_login(
            TiberiusError::Protocol("no credentials were supplied".into()),
            MssqlAuth::Integrated,
        );
        assert_eq!(
            error.category(),
            crate::error::ErrorCategory::Authentication
        );
        assert!(error.to_string().contains("kinit"));

        // Another method keeps the error of the driver as it is.
        assert_eq!(
            describe_login(
                TiberiusError::Protocol("no credentials were supplied".into()),
                MssqlAuth::SqlLogin
            )
            .category(),
            crate::error::ErrorCategory::Database
        );
    }

    #[test]
    fn a_time_limit_during_a_kerberos_call_names_the_kerberos_server() {
        let late = || -> Result<()> { Err(Error::Connection("within 5 seconds".into())) };
        let named = name_kerberos_wait(late(), true).unwrap_err();
        assert!(matches!(&named, Error::KerberosUnreachable(inner)
            if matches!(inner.as_ref(), Error::Connection(text) if text == "within 5 seconds")));

        // A limit that passed in another step keeps its own message.
        assert!(matches!(
            name_kerberos_wait(late(), false),
            Err(Error::Connection(_))
        ));
        // Only the time limit gets the hint. A login error that came back
        // while the flag was up keeps its own error.
        let refused: Result<()> = Err(Error::Authentication("refused".into()));
        assert!(matches!(
            name_kerberos_wait(refused, true),
            Err(Error::Authentication(_))
        ));
        assert_eq!(name_kerberos_wait(Ok(7), true).unwrap(), 7);
    }

    #[test]
    fn a_gssapi_error_that_names_no_reachable_kdc_gets_the_hint() {
        use tiberius::error::Error as TiberiusError;

        for text in [
            "Cannot contact any KDC for realm 'CORP.EXAMPLE.COM'",
            "unable to reach any KDC in realm CORP.EXAMPLE.COM",
            "Cannot find KDC for realm \"CORP.EXAMPLE.COM\"",
            "Cannot resolve network address for KDC in requested realm",
        ] {
            let error = describe_login(TiberiusError::Gssapi(text.into()), MssqlAuth::Integrated);
            assert!(matches!(&error, Error::KerberosUnreachable(_)), "{text}");
            let payload = error.to_payload();
            assert!(payload
                .message
                .starts_with("Couldn't reach the Kerberos server."));
            assert!(payload.detail.unwrap().contains(text));
        }

        // A missing ticket keeps the advice to run kinit.
        let error = describe_login(
            TiberiusError::Gssapi("No Kerberos credentials available".into()),
            MssqlAuth::Integrated,
        );
        assert!(error.to_string().contains("kinit"));
        // The same words outside GSSAPI do not match.
        assert!(!names_an_unreachable_kdc(&TiberiusError::Protocol(
            "Cannot contact any KDC".into()
        )));
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
        assert_eq!(
            error.category(),
            crate::error::ErrorCategory::Authentication
        );
        assert!(error.to_string().contains("has expired"));

        // Another fault keeps the error of the driver, because a token that
        // is old is not its cause.
        assert_eq!(
            describe_login(TiberiusError::Utf8, MssqlAuth::EntraAccessToken).category(),
            crate::error::ErrorCategory::Database
        );
    }
}
