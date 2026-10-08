//! The PostgreSQL driver.
//!
//! A script without parameters goes through the simple protocol. That
//! protocol accepts more than one statement in one call, it reports the
//! command tag of each statement, and it returns every value as text, so no
//! type mapping can fail.

use crate::db::drivers::{
    add_constraint_column, add_included_column, add_index_column, add_snapshot_column,
    bytes_to_json, connect_within, constraint_type, f32_to_json, f64_to_json, hex_text,
    number_out_of_range, number_value, prefixed_plan, routine_type, rows_affected_message,
    rows_returned_message, size_text, system_roots, CancelHandle, DatabaseDriver, NumberValue,
    KEEPALIVE_IDLE, KEEPALIVE_INTERVAL,
};
use crate::db::sink::{RowSink, RunSummary, SinkControl};
use crate::db::{
    AppColumn, ColumnInfo, Constraint, CreateQuery, Database, DriverCapabilities, ExecOptions,
    IndexInfo, Message, MessageLevel, ObjectType, Partition, PartitionList, PlanMode, QueryParams,
    QueryResponse, RelationType, Routine, Schema, SchemaSnapshot, SnapshotColumn, Table, TableFact,
    Trigger, TriggerEvent, TriggerTiming,
};
use crate::error::{offset_place, place_of_byte_offset, place_of_char_position, Error, Result};
use crate::sql::{leading_keyword, locks_rows, only_reads, split_statements, Dialect};
use crate::storage::{SavedConnection, TlsMode};
use async_trait::async_trait;
use bytes::BytesMut;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use futures_util::{pin_mut, stream, StreamExt, TryStreamExt};
use jiff::tz::TimeZone;
use postgres_types::{to_sql_checked, Field, Format, FromSql, IsNull, Kind, ToSql, Type};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use serde_json::Value as JsonValue;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_postgres::error::{DbError, ErrorPosition, SqlState};
use tokio_postgres::{AsyncMessage, Client, Config as PgConfig, Row, SimpleQueryMessage};

pub struct PostgresDriver {
    client: Client,
    /// The notices that the server sent since the last answer.
    notices: NoticeBuffer,
    /// The token that stops the statement that runs on this connection. The
    /// driver holds it, so a read that reaches the row limit can end the
    /// statement without a second handle from the caller.
    stop: Arc<dyn CancelHandle>,
}

/// Builds the connection configuration from a saved connection.
pub fn build_config(connection: &SavedConnection) -> Result<PgConfig> {
    if let Some(url) = connection.options.connection_url.as_deref() {
        let mut config = parse_string(url)?;
        add_fields_of_record(&mut config, url, connection);
        add_read_only_option(&mut config, connection);
        return Ok(config);
    }

    let mut config = PgConfig::new();
    config.host(connection.effective_host());
    if let Some(port) = connection.effective_port() {
        config.port(port);
    }
    if let Some(user) = connection.user.as_deref().filter(|v| !v.is_empty()) {
        config.user(user);
    }
    if let Some(password) = connection.password.as_deref().filter(|v| !v.is_empty()) {
        config.password(password);
    }
    if let Some(database) = connection.database.as_deref().filter(|v| !v.is_empty()) {
        config.dbname(database);
    }
    if let Some(name) = connection
        .options
        .application_name
        .as_deref()
        .filter(|v| !v.trim().is_empty())
    {
        config.application_name(name);
    }
    config.ssl_mode(ssl_mode(connection.options.tls_mode));
    config.connect_timeout(Duration::from_secs(
        connection.options.connect_timeout_secs.max(1),
    ));
    keep_alive(&mut config);
    add_read_only_option(&mut config, connection);
    Ok(config)
}

/// Sets the TCP keepalive times of the application. The default of the
/// driver sends the first probe after two hours, and a firewall can drop an
/// idle connection long before that.
fn keep_alive(config: &mut PgConfig) {
    config.keepalives(true);
    config.keepalives_idle(KEEPALIVE_IDLE);
    config.keepalives_interval(KEEPALIVE_INTERVAL);
}

/// Reads a connection string, in the URL form or in the key and value form.
fn parse_string(url: &str) -> Result<PgConfig> {
    url.trim()
        .parse::<PgConfig>()
        .map_err(|error| Error::Configuration(error.to_string()))
}

/// True when a connection string gives a password.
pub fn string_has_password(url: &str) -> Result<bool> {
    Ok(parse_string(url)?
        .get_password()
        .is_some_and(|password| !password.is_empty()))
}

/// Adds the fields of the record that a connection string does not give.
/// The keychain keeps the password, so the string does not give one. The
/// time limit of the connection and the transport mode of the form also
/// apply when the string names no value for them.
fn add_fields_of_record(config: &mut PgConfig, url: &str, connection: &SavedConnection) {
    if config.get_user().is_none() {
        if let Some(user) = connection.user.as_deref().filter(|v| !v.is_empty()) {
            config.user(user);
        }
    }
    if config.get_password().is_none() {
        if let Some(password) = connection.password.as_deref().filter(|v| !v.is_empty()) {
            config.password(password);
        }
    }
    if config.get_connect_timeout().is_none() {
        config.connect_timeout(Duration::from_secs(
            connection.options.connect_timeout_secs.max(1),
        ));
    }
    // The parser gives `Prefer` also when the string names no mode, so the
    // text of the string decides.
    if !url.contains("sslmode") {
        config.ssl_mode(ssl_mode(connection.options.tls_mode));
    }
    // The parser gives the default keepalive values also when the string
    // names none, so the text of the string decides here too.
    if !url.contains("keepalives") {
        keep_alive(config);
    }
}

/// The server option that makes each transaction of the session read-only.
const READ_ONLY_OPTION: &str = "-c default_transaction_read_only=on";

/// Adds the read-only option of a read-only connection to the options that
/// the configuration has. A connection string can give options of its own,
/// such as a search path, and the read-only option goes after them.
fn add_read_only_option(config: &mut PgConfig, connection: &SavedConnection) {
    if !connection.options.read_only {
        return;
    }
    let options = match config.get_options() {
        Some(existing) if !existing.trim().is_empty() => format!("{existing} {READ_ONLY_OPTION}"),
        _ => READ_ONLY_OPTION.to_string(),
    };
    config.options(options);
}

/// Maps the transport setting of the application onto the mode of the
/// driver.
pub fn ssl_mode(mode: TlsMode) -> tokio_postgres::config::SslMode {
    match mode {
        TlsMode::Disable => tokio_postgres::config::SslMode::Disable,
        TlsMode::Prefer => tokio_postgres::config::SslMode::Prefer,
        TlsMode::Require | TlsMode::VerifyFull => tokio_postgres::config::SslMode::Require,
    }
}

/// A verifier that accepts every certificate. It serves the modes that ask
/// for encryption without a check of the identity of the server.
#[derive(Debug)]
struct AcceptAnyCertificate(Arc<rustls::crypto::CryptoProvider>);

impl ServerCertVerifier for AcceptAnyCertificate {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// Builds the TLS settings. A mode that verifies uses the trusted roots of
/// the system and any extra authority the user named.
pub fn build_tls_config(connection: &SavedConnection) -> Result<ClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());

    if !connection.options.tls_mode.verifies_certificate() {
        let config = ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .map_err(|error| Error::Configuration(error.to_string()))?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAnyCertificate(provider)))
            .with_no_client_auth();
        return Ok(config);
    }

    let mut roots = roots_from(system_roots());
    if let Some(path) = connection
        .options
        .ca_cert_path
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        let bytes = std::fs::read(path)?;
        let certificates = rustls_pemfile_certs(&bytes);
        if certificates.is_empty() {
            return Err(Error::Configuration(format!(
                "The CA certificate file {path} has no certificate that can be read. Use a file \
                 with PEM or DER certificates."
            )));
        }
        for certificate in certificates {
            roots
                .add(certificate)
                .map_err(|error| Error::Configuration(error.to_string()))?;
        }
    }

    ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| Error::Configuration(error.to_string()))?
        .with_root_certificates(roots)
        .with_no_client_auth()
        .pipe(Ok)
}

/// Builds the store of trusted roots from the roots of the operating system.
/// When the system gives no usable root, the store takes the Mozilla roots
/// of `webpki-roots`.
fn roots_from(system: &[CertificateDer<'static>]) -> RootCertStore {
    let mut roots = RootCertStore::empty();
    if system.is_empty() {
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    } else {
        roots.add_parsable_certificates(system.iter().cloned());
    }
    roots
}

/// A small helper that lets a value flow into a function at the end of a
/// chain.
trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl<T> Pipe for T {}

/// Reads every certificate out of a file that holds PEM blocks or one DER
/// block.
fn rustls_pemfile_certs(bytes: &[u8]) -> Vec<CertificateDer<'static>> {
    let text = String::from_utf8_lossy(bytes);
    if !text.contains("-----BEGIN CERTIFICATE-----") {
        return vec![CertificateDer::from(bytes.to_vec())];
    }
    text.split("-----BEGIN CERTIFICATE-----")
        .skip(1)
        .filter_map(|block| block.split("-----END CERTIFICATE-----").next())
        .filter_map(|body| {
            let cleaned: String = body.chars().filter(|c| !c.is_whitespace()).collect();
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD
                .decode(cleaned)
                .ok()
        })
        .map(CertificateDer::from)
        .collect()
}

impl PostgresDriver {
    pub async fn connect(connection: &SavedConnection) -> Result<Box<dyn DatabaseDriver>> {
        let config = build_config(connection)?;
        let limit = Duration::from_secs(connection.options.connect_timeout_secs.max(1));

        let tls = tokio_postgres_rustls::MakeRustlsConnect::new(build_tls_config(connection)?);
        let (client, io) = connect_within(limit.as_secs(), config.connect(tls.clone())).await??;

        let notices = drive_connection(io);

        let stop: Arc<dyn CancelHandle> = Arc::new(PostgresCancel {
            token: client.cancel_token(),
            tls,
        });
        Ok(Box::new(PostgresDriver {
            client,
            notices,
            stop,
        }))
    }

    /// Sends the notices that arrived since the last call to the sink.
    fn pass_notices(&self, sink: &mut dyn RowSink) {
        for notice in self.take_notices() {
            sink.message(notice);
        }
    }

    /// Takes the notices that arrived since the last run.
    fn take_notices(&self) -> Vec<Message> {
        match self.notices.lock() {
            Ok(mut buffer) => buffer.take(),
            // The lock breaks only when a holder panicked, and a lost notice
            // must not stop the run itself.
            Err(_) => Vec::new(),
        }
    }
}

/// Drives the socket of a connection on a task of its own and gives the
/// buffer of its notices.
///
/// The notices of the server arrive on the connection object and not with
/// the result of a statement, so the task that drives the socket is a stream
/// of messages and not a future. Each notice goes into a buffer that the
/// driver drains into the sink after each statement. See [`Notices`] for the
/// bound of the buffer.
fn drive_connection<S, T>(mut io: tokio_postgres::Connection<S, T>) -> NoticeBuffer
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let notices: NoticeBuffer = Arc::new(Mutex::new(Notices::default()));
    let held = notices.clone();
    let mut stream = stream::poll_fn(move |context| io.poll_message(context));
    tokio::spawn(async move {
        while let Some(message) = stream.next().await {
            match message {
                Ok(AsyncMessage::Notice(notice)) => {
                    if let Ok(mut buffer) = held.lock() {
                        buffer.push(notice_message(&notice));
                    }
                }
                // A notification comes from LISTEN, which this
                // application does not use.
                Ok(_) => {}
                Err(error) => {
                    log::warn!("The PostgreSQL connection closed: {error}");
                    break;
                }
            }
        }
    });
    notices
}

/// The notices that wait for the next answer.
type NoticeBuffer = Arc<Mutex<Notices>>;

/// The most notices that the buffer keeps between two drains. A statement
/// that raises a notice in a loop of millions of rounds would otherwise fill
/// the memory of the application.
const NOTICE_LIMIT: usize = 1000;

/// The notices that arrived since the last drain, up to [`NOTICE_LIMIT`],
/// and the count of the notices past that limit.
#[derive(Debug, Default)]
struct Notices {
    kept: Vec<Message>,
    dropped: u64,
}

impl Notices {
    /// Keeps a notice, or only counts it when the buffer is full.
    fn push(&mut self, notice: Message) {
        if self.kept.len() < NOTICE_LIMIT {
            self.kept.push(notice);
        } else {
            self.dropped += 1;
        }
    }

    /// Gives the kept notices and empties the buffer. When the buffer
    /// dropped notices, a warning with their count follows the kept ones.
    fn take(&mut self) -> Vec<Message> {
        let mut notices = std::mem::take(&mut self.kept);
        let dropped = std::mem::take(&mut self.dropped);
        if dropped > 0 {
            notices.push(dropped_notices_message(dropped));
        }
        notices
    }
}

/// The warning that counts the notices past [`NOTICE_LIMIT`].
fn dropped_notices_message(dropped: u64) -> Message {
    let counted = if dropped == 1 {
        "1 more notice was".to_string()
    } else {
        format!("{dropped} more notices were")
    };
    Message::warning(format!(
        "{counted} dropped. Only the first {NOTICE_LIMIT} notices of a statement are shown."
    ))
}

/// Reads the fields of a notice of the server and builds one message. The
/// severity decides the level, and the fields beside the text become the
/// detail. The fields arrive as text, so a test needs no notice of its own.
fn notice_message_from(
    severity: &str,
    code: &str,
    text: &str,
    detail: Option<&str>,
    hint: Option<&str>,
) -> Message {
    let level = match severity.to_uppercase().as_str() {
        "WARNING" | "EXCEPTION" => MessageLevel::Warning,
        "ERROR" | "FATAL" | "PANIC" => MessageLevel::Error,
        _ => MessageLevel::Info,
    };
    let parts: Vec<&str> = [Some(severity), Some(code), detail, hint]
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect();
    Message {
        level,
        text: text.to_string(),
        detail: Some(parts.join(" · ")),
    }
}

/// Builds one message from a notice of the server.
fn notice_message(notice: &DbError) -> Message {
    notice_message_from(
        notice.severity(),
        notice.code().code(),
        notice.message(),
        notice.detail(),
        notice.hint(),
    )
}

#[async_trait]
impl DatabaseDriver for PostgresDriver {
    fn capabilities(&self) -> DriverCapabilities {
        DriverCapabilities {
            supports_schemas: true,
            // One connection is attached to one database, so the explorer
            // shows only that database.
            supports_multiple_databases: false,
            supports_cancel: true,
            supports_transactions: true,
            supports_routines: true,
            supports_indexes: true,
            supports_constraints: true,
            supports_partitions: true,
            supports_explain: true,
            supports_materialized_views: true,
            supports_foreign_tables: true,
            supports_synonyms: false,
            supports_triggers: true,
            supports_view_triggers: true,
            supports_events: false,
        }
    }

    fn dialect(&self) -> Dialect {
        Dialect::Postgres
    }

    fn create_query(
        &self,
        _database: Option<&str>,
        schema: Option<&str>,
        table: &str,
        relation_type: RelationType,
    ) -> Option<CreateQuery> {
        create_query_text(schema, table, relation_type)
    }

    fn object_create_query(
        &self,
        _database: Option<&str>,
        schema: Option<&str>,
        parent: Option<&str>,
        name: &str,
        object_type: ObjectType,
    ) -> Option<CreateQuery> {
        object_query_text(schema, parent?, name, object_type)
    }

    async fn ping(&mut self) -> Result<()> {
        self.client.simple_query("SELECT 1").await?;
        Ok(())
    }

    async fn limit_lock_waits(&mut self, limit: Duration) -> Result<()> {
        self.client
            .simple_query(&lock_timeout_statement(limit))
            .await?;
        Ok(())
    }

    /// A block that an error aborted still waits for a `ROLLBACK`, so the
    /// error 25P02 of the probe counts as an open block.
    async fn holds_open_transaction(&mut self) -> Result<bool> {
        match self.outside_a_block().await {
            Ok(outside) => Ok(!outside),
            Err(error) if error.code() == Some(&SqlState::IN_FAILED_SQL_TRANSACTION) => Ok(true),
            Err(error) => Err(error.into()),
        }
    }

    /// Runs a script and feeds the sink. The path with parameters streams
    /// the rows through `query_raw`. The path without parameters goes
    /// through the simple protocol, which carries the whole script in one
    /// exchange and streams the messages of that exchange one at a time.
    ///
    /// The notices of the server reach the sink after each statement, in
    /// their arrival order, and also when the run fails.
    async fn execute_stream(
        &mut self,
        query: &str,
        params: Option<&QueryParams>,
        options: &ExecOptions,
        sink: &mut dyn RowSink,
    ) -> Result<RunSummary> {
        let started = Instant::now();
        // A notice that arrived before this run belongs to no answer, so the
        // buffer starts empty.
        let _ = self.take_notices();
        let outcome = match params {
            Some(params) => self.stream_with_params(query, params, options, sink).await,
            None => self.stream_simple(query, options, sink).await,
        };

        // The notices of a failed statement go to the sink before the error,
        // because a RAISE NOTICE often tells why the statement failed.
        self.pass_notices(sink);
        let rows_affected = outcome?;
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
        let statement = prefixed_plan(query, Dialect::Postgres, plan_prefix(mode))?;
        self.execute_query(&statement, params, options).await
    }

    async fn list_databases(&mut self) -> Result<Vec<Database>> {
        let rows = self
            .client
            .query(
                "SELECT datname FROM pg_database \
                 WHERE datistemplate = false AND has_database_privilege(datname, 'CONNECT') \
                 ORDER BY datname",
                &[],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|row| Database { name: row.get(0) })
            .collect())
    }

    async fn list_schemas(&mut self, _database: &str) -> Result<Vec<Schema>> {
        // The name of a temporary schema starts with `pg_temp_`. An
        // underscore is a wildcard in `LIKE`, so it is escaped.
        let rows = self
            .client
            .query(
                "SELECT nspname FROM pg_catalog.pg_namespace \
                 WHERE nspname NOT IN ('pg_toast', 'pg_catalog', 'information_schema') \
                   AND nspname NOT LIKE 'pg\\_temp\\_%' \
                   AND nspname NOT LIKE 'pg\\_toast\\_temp\\_%' \
                 ORDER BY nspname",
                &[],
            )
            .await?;
        Ok(rows.iter().map(|row| Schema { name: row.get(0) }).collect())
    }

    async fn list_tables(&mut self, _database: &str, schema: Option<&str>) -> Result<Vec<Table>> {
        let schema = schema.unwrap_or("public");
        let rows = self.client.query(TABLES_QUERY, &[&schema]).await?;
        Ok(rows
            .iter()
            .map(|row| relation_of(row.get(0), row.get(1)))
            .collect())
    }

    async fn list_columns(
        &mut self,
        _database: &str,
        schema: Option<&str>,
        table: &str,
    ) -> Result<Vec<AppColumn>> {
        let schema = schema.unwrap_or("public");
        let rows = self
            .client
            .query(
                "SELECT a.attname, \
                        format_type(a.atttypid, a.atttypmod), \
                        NOT a.attnotnull, \
                        COALESCE(i.indisprimary, false), \
                        a.attidentity <> '' OR a.attgenerated <> '' \
                 FROM pg_catalog.pg_attribute AS a \
                 JOIN pg_catalog.pg_class AS c ON c.oid = a.attrelid \
                 JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace \
                 LEFT JOIN pg_catalog.pg_index AS i \
                        ON i.indrelid = c.oid AND a.attnum = ANY(i.indkey) AND i.indisprimary \
                 WHERE n.nspname = $1 AND c.relname = $2 \
                   AND a.attnum > 0 AND NOT a.attisdropped \
                 ORDER BY a.attnum",
                &[&schema, &table],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|row| AppColumn {
                name: row.get(0),
                data_type: row.get(1),
                nullable: row.get(2),
                is_primary_key: row.get(3),
                is_generated: row.get(4),
            })
            .collect())
    }

    /// Reads the facts of one relation. The number of rows is the estimate
    /// that the catalog holds, which the planner keeps and which ANALYZE
    /// refreshes.
    async fn table_facts(
        &mut self,
        _database: &str,
        schema: Option<&str>,
        table: &str,
    ) -> Result<Vec<TableFact>> {
        let schema = schema.unwrap_or("public");
        let rows = self
            .client
            .query(
                "SELECT c.reltuples::bigint, \
                        pg_catalog.pg_total_relation_size(c.oid)::bigint, \
                        pg_catalog.pg_get_userbyid(c.relowner) \
                 FROM pg_catalog.pg_class AS c \
                 JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace \
                 WHERE n.nspname = $1 AND c.relname = $2",
                &[&schema, &table],
            )
            .await?;
        let Some(row) = rows.first() else {
            return Ok(Vec::new());
        };

        let mut facts = Vec::new();
        let estimate: i64 = row.get(0);
        if estimate >= 0 {
            facts.push(TableFact::new("Rows", format!("about {estimate}")));
        }
        let bytes: i64 = row.get(1);
        facts.push(TableFact::new("Size", size_text(bytes.max(0) as u64)));
        facts.push(TableFact::new("Owner", row.get::<_, String>(2)));
        Ok(facts)
    }

    /// Reads every relation and every column of the database of the
    /// connection in one statement. One PostgreSQL connection reaches one
    /// database, so the name of the database is not part of the statement.
    async fn schema_snapshot(
        &mut self,
        database: &str,
        max_columns: usize,
    ) -> Result<SchemaSnapshot> {
        let rows = self.client.query(&snapshot_query(max_columns), &[]).await?;
        let mut snapshot = SchemaSnapshot {
            database: database.to_string(),
            complete: true,
            ..SchemaSnapshot::default()
        };
        for row in &rows {
            if !add_snapshot_column(
                &mut snapshot,
                max_columns,
                row.get(0),
                row.get(1),
                relation_type_of(row.get(2)),
                SnapshotColumn {
                    name: row.get(3),
                    data_type: row.get(4),
                },
            ) {
                break;
            }
        }
        Ok(snapshot)
    }

    async fn list_routines(
        &mut self,
        _database: &str,
        schema: Option<&str>,
    ) -> Result<Vec<Routine>> {
        let schema = schema.unwrap_or("public");
        let rows = self.client.query(ROUTINES_QUERY, &[&schema]).await?;
        Ok(rows
            .iter()
            .map(|row| Routine {
                name: row.get(0),
                routine_type: routine_type(row.get(1)),
            })
            .collect())
    }

    /// Reads the indexes from the catalog with [`INDEXES_QUERY`].
    async fn list_indexes(
        &mut self,
        _database: &str,
        schema: Option<&str>,
        table: &str,
    ) -> Result<Vec<IndexInfo>> {
        let schema = schema.unwrap_or("public");
        let rows = self.client.query(INDEXES_QUERY, &[&schema, &table]).await?;
        let mut indexes = Vec::new();
        for row in &rows {
            add_postgres_index_column(
                &mut indexes,
                row.get(0),
                row.get(1),
                row.get(2),
                row.get(3),
                row.get(4),
            );
        }
        Ok(indexes)
    }

    /// Reads the partitions of a partitioned table with
    /// [`PARTITIONS_QUERY`]. Any other relation has no partition and gives
    /// an empty list.
    async fn list_partitions(
        &mut self,
        _database: &str,
        schema: Option<&str>,
        table: &str,
    ) -> Result<PartitionList> {
        let schema = schema.unwrap_or("public");
        let limit = PARTITION_LIMIT as i64 + 1;
        let rows = self
            .client
            .query(PARTITIONS_QUERY, &[&schema, &table, &limit])
            .await?;
        Ok(partition_list(
            rows.iter().map(|row| (row.get(0), row.get(1))).collect(),
        ))
    }

    async fn list_constraints(
        &mut self,
        _database: &str,
        schema: Option<&str>,
        table: &str,
    ) -> Result<Vec<Constraint>> {
        let schema = schema.unwrap_or("public");
        let rows = self
            .client
            .query(
                "SELECT c.conname, \
                        c.contype::text, \
                        a.attname, \
                        pg_catalog.pg_get_constraintdef(c.oid) \
                 FROM pg_catalog.pg_constraint AS c \
                 JOIN pg_catalog.pg_class AS t ON t.oid = c.conrelid \
                 JOIN pg_catalog.pg_namespace AS n ON n.oid = t.relnamespace \
                 LEFT JOIN LATERAL unnest(c.conkey) WITH ORDINALITY AS k(attnum, ord) ON true \
                 LEFT JOIN pg_catalog.pg_attribute AS a \
                        ON a.attrelid = t.oid AND a.attnum = k.attnum \
                 WHERE n.nspname = $1 AND t.relname = $2 \
                 ORDER BY c.conname, k.ord",
                &[&schema, &table],
            )
            .await?;
        let mut constraints = Vec::new();
        for row in &rows {
            add_constraint_column(
                &mut constraints,
                row.get(0),
                constraint_type(row.get(1)),
                row.get(2),
                row.get(3),
            );
        }
        Ok(constraints)
    }

    /// Reads the triggers with [`TRIGGERS_QUERY`].
    async fn list_triggers(
        &mut self,
        _database: &str,
        schema: Option<&str>,
        table: &str,
    ) -> Result<Vec<Trigger>> {
        let schema = schema.unwrap_or("public");
        let rows = self
            .client
            .query(TRIGGERS_QUERY, &[&schema, &table])
            .await?;
        Ok(rows
            .iter()
            .map(|row| trigger_of(row.get(0), row.get(1), row.get(2), row.get(3)))
            .collect())
    }

    fn cancel_handle(&self) -> Option<Arc<dyn CancelHandle>> {
        Some(self.stop.clone())
    }
}

/// Asks the server to stop the statement that runs now. It gives back true
/// when the request reached the server. A request that does not reach the
/// server leaves the statement running, and the caller then reads the rest
/// of the result.
async fn request_stop(stop: &Arc<dyn CancelHandle>) -> bool {
    match stop.cancel().await {
        Ok(()) => true,
        Err(error) => {
            log::warn!("The stop of the PostgreSQL statement failed: {error}");
            false
        }
    }
}

/// True for the error that a cancel raises on the statement it stopped.
fn is_query_cancelled(error: &tokio_postgres::Error) -> bool {
    error.code() == Some(&SqlState::QUERY_CANCELED)
}

/// Reads the rest of a statement after the driver sent a cancel for it.
/// Gives true when the statement ended without an error, which tells that
/// the cancel did not stop it. Any error ends the read, because a closed
/// connection gives the same error each time the stream is read.
async fn ended_before_cancel<S, T>(mut rest: std::pin::Pin<&mut S>) -> bool
where
    S: stream::Stream<Item = std::result::Result<T, tokio_postgres::Error>>,
{
    loop {
        match rest.try_next().await {
            Ok(Some(_)) => {}
            Ok(None) => return true,
            Err(_) => return false,
        }
    }
}

/// The statement that gives a late cancel a statement to stop. See
/// [`PostgresDriver::absorb_late_cancel`].
const CANCEL_PROBE: &str = "SELECT 1";

/// True for a `COPY ... FROM STDIN` or a `COPY ... TO STDOUT` statement.
/// The server answers such a statement with a copy exchange, and this
/// client does not take part in that exchange. The words are compared one
/// pair at a time, so a line break between the two words does not hide the
/// statement.
fn copies_through_the_client(statement: &str) -> bool {
    if leading_keyword(statement, Dialect::Postgres) != "copy" {
        return false;
    }
    let lower = statement.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|word| !word.is_empty())
        .collect();
    words
        .windows(2)
        .any(|pair| matches!(pair, ["from", "stdin"] | ["to", "stdout"]))
}

/// Refuses a statement that [`copies_through_the_client`] accepts, before
/// the statement goes to the server. A copy exchange that the client does
/// not answer leaves the connection waiting for copy data.
fn refuse_client_copy(statement: &str) -> Result<()> {
    if copies_through_the_client(statement) {
        return Err(Error::Unsupported(
            "This client doesn't support COPY FROM STDIN or COPY TO STDOUT. Use COPY with a \
             server-side file, or use \\copy in psql."
                .to_string(),
        ));
    }
    Ok(())
}

/// A token that asks the server to stop the statement that runs on this
/// connection. It opens its own socket, so it works while the connection
/// is busy.
///
/// The socket of the cancel asks for TLS in the same mode as the
/// connection. A server that requires TLS refuses a cancel without it, and
/// the statement then runs on.
struct PostgresCancel {
    token: tokio_postgres::CancelToken,
    tls: tokio_postgres_rustls::MakeRustlsConnect,
}

#[async_trait]
impl CancelHandle for PostgresCancel {
    async fn cancel(&self) -> Result<()> {
        wait_for_cancel(self.token.cancel_query(self.tls.clone()), CANCEL_WAIT).await
    }
}

/// The statement that ends each later statement of the session that waits
/// for a lock longer than `limit`. The server then sends the error 55P03. The
/// text names the unit, so the value does not depend on the default unit of
/// the setting.
fn lock_timeout_statement(limit: Duration) -> String {
    format!("SET lock_timeout = '{}ms'", limit.as_millis())
}

/// The longest time that the driver waits for the server to close the socket
/// of a cancel.
const CANCEL_WAIT: Duration = Duration::from_secs(5);

/// Waits for a cancel request until the server closes its socket, or until
/// the limit runs out. A network that drops the socket without a close would
/// otherwise stop the read of the statement for good. After the limit the
/// request counts as sent, and [`PostgresDriver::absorb_late_cancel`] runs
/// when the statement then ends without the error of the cancel.
async fn wait_for_cancel<F>(request: F, limit: Duration) -> Result<()>
where
    F: std::future::Future<Output = std::result::Result<(), tokio_postgres::Error>>,
{
    match tokio::time::timeout(limit, request).await {
        Ok(result) => Ok(result?),
        Err(_) => {
            log::warn!("The server did not close the socket of the cancel in time.");
            Ok(())
        }
    }
}

/// The statement that tells whether the session is outside a transaction
/// block. Outside a block each statement is a transaction of its own, so
/// the start of the transaction and the start of the statement are the same
/// moment. Inside a block the transaction started with an earlier statement.
const OUTSIDE_A_BLOCK: &str = "SELECT now() = statement_timestamp()";

/// The type names of the columns of a prepared statement.
fn type_names(statement: &tokio_postgres::Statement) -> Vec<String> {
    statement
        .columns()
        .iter()
        .map(|column| column.type_().name().to_string())
        .collect()
}

/// Names the columns of a set of the simple protocol from the pairs of a
/// name and a type OID. The type names of the prepared statement apply
/// when their count is the count of the columns. Otherwise each column
/// takes the name of its OID, see [`oid_type_name`].
fn simple_columns(prepared: Option<&[String]>, columns: &[(&str, u32)]) -> Vec<ColumnInfo> {
    match prepared {
        Some(names) if names.len() == columns.len() => columns
            .iter()
            .zip(names)
            .map(|((name, _), type_name)| ColumnInfo::new(*name, type_name))
            .collect(),
        _ => columns
            .iter()
            .map(|(name, oid)| ColumnInfo::new(*name, oid_type_name(*oid)))
            .collect(),
    }
}

/// The name of a built-in type, or the number of the OID for any other
/// type. The driver cannot read the catalog while the stream of a statement
/// is open. The response channel of the client has a fixed size, so a second
/// query inside the walk would wait for ever.
fn oid_type_name(oid: u32) -> String {
    Type::from_oid(oid).map_or_else(|| oid.to_string(), |type_| type_.name().to_string())
}

impl PostgresDriver {
    /// Runs the probe [`OUTSIDE_A_BLOCK`] for a statement that only reads.
    /// Gives `Some(true)` outside a transaction block and `Some(false)`
    /// inside a block. A statement that writes gives `None` and sends no
    /// probe. A probe that fails, for example in a block that is already
    /// aborted, also gives `None`.
    ///
    /// A cancel at the row limit loses no work only for a read outside a
    /// block. A cancel rolls back the statement it ends, so a cancelled
    /// `INSERT ... RETURNING` writes no row. Inside a block the cancel also
    /// aborts the block, and the `COMMIT` that follows then rolls back every
    /// change of the block.
    async fn probe_read(&self, statement: &str) -> Option<bool> {
        if !reads_rows(statement) {
            return None;
        }
        self.outside_a_block().await.ok()
    }

    /// Prepares a statement that only reads, and gives the type names of
    /// its columns together with the answer of [`Self::probe_read`]. The
    /// prepare and the probe [`OUTSIDE_A_BLOCK`] go to the server in one
    /// round trip. The prepare also finds the names of types that a user
    /// or an extension defines. A statement that writes is not prepared
    /// and gives no names.
    ///
    /// Outside a transaction block a failed prepare gives no names, and the
    /// simple query then reports the error. Inside a block the failed
    /// prepare aborts the block, and the simple query would only report
    /// the aborted block. The error of the prepare then ends the run.
    async fn describe_read(&self, statement: &str) -> Result<(Option<Vec<String>>, Option<bool>)> {
        if !reads_rows(statement) {
            return Ok((None, None));
        }
        let (prepared, outside) =
            futures_util::future::join(self.client.prepare(statement), self.outside_a_block())
                .await;
        let outside = outside.ok();
        match prepared {
            Ok(prepared) => Ok((Some(type_names(&prepared)), outside)),
            Err(error) if outside != Some(true) => Err(error.into()),
            Err(error) => {
                log::debug!("The statement could not be prepared: {error}");
                Ok((None, outside))
            }
        }
    }

    /// Reads the time zone of the session. The binary form of a `timestamptz`
    /// value holds the moment in UTC, and the text form of the simple
    /// protocol shows it in the zone of the session, so the reader of the
    /// binary form needs that zone. A zone that the name does not give, or a
    /// probe that fails, gives UTC.
    async fn session_zone(&self) -> TimeZone {
        let name = match self.client.simple_query("SHOW TimeZone").await {
            Ok(messages) => messages.iter().find_map(|message| match message {
                SimpleQueryMessage::Row(row) => row.get(0).map(str::to_string),
                _ => None,
            }),
            Err(error) => {
                log::warn!("The time zone of the session could not be read: {error}");
                None
            }
        };
        name.map_or(TimeZone::UTC, |name| zone_of(&name))
    }

    /// Runs [`CANCEL_PROBE`] after a statement that ended before its cancel
    /// reached the server.
    ///
    /// The cancel request goes on a second socket. The future of the request
    /// ends when the server closes that socket, and the server sends the
    /// signal of the cancel to the session before the close. A signal that
    /// arrives while the session waits for a statement has no effect. A
    /// signal that arrives while a later statement runs stops that statement
    /// with the error 57014.
    ///
    /// The probe runs for a request whose close did not arrive within
    /// [`CANCEL_WAIT`], and for a pooler that closes the socket before the
    /// server applies the cancel. In these cases the signal can still come
    /// late. The probe runs first, so such a signal stops the probe and the
    /// probe drops the error. A signal that comes later than the end of the
    /// probe still stops the next statement.
    async fn absorb_late_cancel(&self) {
        if let Err(error) = self.client.simple_query(CANCEL_PROBE).await {
            if !is_query_cancelled(&error) {
                log::warn!("The probe after a cancel failed: {error}");
            }
        }
    }

    /// Reads the count of the digits of the fraction of a `money` value. The
    /// binary form holds the amount in the smallest unit of the currency, and
    /// the `lc_monetary` setting of the session gives the count. The cast to
    /// numeric applies that count as the scale. An answer that is not a count
    /// from 0 to 10, or a probe that fails, gives 2, as the server does.
    async fn money_digits(&self) -> u32 {
        match self.client.simple_query(MONEY_DIGITS).await {
            Ok(messages) => messages
                .iter()
                .find_map(|message| match message {
                    SimpleQueryMessage::Row(row) => row.get(0)?.parse::<u32>().ok(),
                    _ => None,
                })
                .filter(|digits| *digits <= 10)
                .unwrap_or(2),
            Err(error) => {
                log::warn!("The money format of the session could not be read: {error}");
                2
            }
        }
    }

    /// Runs the probe [`OUTSIDE_A_BLOCK`] and reads its answer.
    async fn outside_a_block(&self) -> std::result::Result<bool, tokio_postgres::Error> {
        let messages = self.client.simple_query(OUTSIDE_A_BLOCK).await?;
        Ok(messages.iter().any(
            |message| matches!(message, SimpleQueryMessage::Row(row) if row.get(0) == Some("t")),
        ))
    }

    /// Runs a script through the simple protocol, one statement at a time.
    ///
    /// The simple protocol runs every statement of one text in one implicit
    /// transaction. An error in the third statement of a text then rolls back
    /// the first two after their counts reached the window, and a statement
    /// such as `VACUUM` refuses to run at all. Each statement therefore goes
    /// to the server in a text of its own and commits on its own, unless the
    /// script opened a transaction block. The first error ends the script,
    /// and the statements after it do not run.
    ///
    /// A run that sets `one_statement` prepares the text first. The server
    /// refuses to prepare a text of more than one statement, so the run then
    /// stops before the simple protocol runs any part of it. The prepared
    /// statement closes at once, and the rows keep the text form of the
    /// simple protocol. Such a text goes to the server whole, and its
    /// columns take the type names of the prepared statement.
    async fn stream_simple(
        &mut self,
        query: &str,
        options: &ExecOptions,
        sink: &mut dyn RowSink,
    ) -> Result<Option<u64>> {
        let (statements, mut prepared) = if options.one_statement {
            let statement = self
                .client
                .prepare(query)
                .await
                .map_err(|error| locate_error(error, query, query, 0, 0))?;
            (vec![query.to_string()], Some(type_names(&statement)))
        } else {
            (split_statements(query, Dialect::Postgres), None)
        };
        let mut rows_affected: Option<u64> = None;
        // The end of the last statement found in the text, so that a
        // statement that stands twice is found at its own place.
        let mut cursor = 0;
        for statement in &statements {
            let start = query[cursor..]
                .find(statement.as_str())
                .map_or(cursor, |at| cursor + at);
            cursor = (start + statement.len()).min(query.len());
            let outcome = self
                .stream_statement(statement, prepared.take(), options, sink)
                .await;
            self.pass_notices(sink);
            let (affected, stopped) =
                outcome.map_err(|failure| failure.locate(query, statement, start))?;
            if let Some(affected) = affected {
                rows_affected = Some(rows_affected.unwrap_or(0) + affected);
            }
            // The sink takes no more rows, so the statements after this one
            // do not run.
            if stopped {
                break;
            }
        }
        Ok(rows_affected)
    }

    /// Runs one statement through the simple protocol and feeds the sink one
    /// message at a time. Gives the count of the changed rows, and true when
    /// the sink took no more rows.
    ///
    /// A statement goes through [`Self::stream_cursor`] when [`read_path`]
    /// gives a path other than the walk.
    ///
    /// Any other read outside a transaction block, such as `SHOW`, ends at
    /// the row limit:
    /// the driver sends a cancel request on a second socket, and the server
    /// stops the statement instead of sending the rest of the result. The
    /// server answers the cancel with the error 57014, which the walk reads
    /// as the end of the set.
    ///
    /// A statement that ends before its cancel reaches the server is followed
    /// by [`Self::absorb_late_cancel`].
    ///
    /// Every other statement keeps the walk and drops the rows past the
    /// limit. The driver holds one message while it walks, so the memory cost
    /// does not grow with the size of the answer.
    ///
    /// The columns take their type names from `prepared`. Without these
    /// names the driver prepares a statement that only reads, as
    /// [`Self::describe_read`] does. See [`simple_columns`].
    ///
    /// A `COPY` through the client fails before it goes to the server.
    async fn stream_statement(
        &mut self,
        statement: &str,
        prepared: Option<Vec<String>>,
        options: &ExecOptions,
        sink: &mut dyn RowSink,
    ) -> std::result::Result<(Option<u64>, bool), Failure> {
        refuse_client_copy(statement)?;
        let (prepared, outside) = match prepared {
            Some(names) => (Some(names), self.probe_read(statement).await),
            None => self.describe_read(statement).await?,
        };
        let path = read_path(statement, outside);
        if path != ReadPath::Walk {
            let in_block = path == ReadPath::UserBlock;
            return self
                .stream_cursor(statement, prepared.as_deref(), in_block, options, sink)
                .await;
        }
        let alone = outside == Some(true);
        let stop = self.stop.clone();
        let messages = self.client.simple_query_raw(statement).await?;
        pin_mut!(messages);
        let mut rows_affected: Option<u64> = None;
        let mut open = false;
        let mut count = 0usize;
        let mut truncated = false;
        let mut stopped = false;
        // True after the cancel reached the server. The error that the
        // cancel raises then ends the walk and is not a fault of the run.
        let mut cancelled = false;
        // True when the error of the cancel ended the statement.
        let mut stopped_by_cancel = false;

        loop {
            let message = match messages.try_next().await {
                Ok(Some(message)) => message,
                Ok(None) => break,
                Err(error) => {
                    if cancelled && is_query_cancelled(&error) {
                        stopped_by_cancel = true;
                        break;
                    }
                    return Err(error.into());
                }
            };
            match message {
                SimpleQueryMessage::RowDescription(columns) => {
                    if open {
                        sink.message(rows_returned_message(count, truncated));
                        sink.end_set(truncated)?;
                    }
                    let columns: Vec<(&str, u32)> = columns
                        .iter()
                        .map(|column| (column.name(), column.type_oid()))
                        .collect();
                    sink.begin_set(simple_columns(prepared.as_deref(), &columns))?;
                    open = true;
                    count = 0;
                    truncated = false;
                }
                SimpleQueryMessage::Row(row) => {
                    if !open || stopped {
                        continue;
                    }
                    if count >= options.max_rows {
                        truncated = true;
                        if alone && !cancelled {
                            cancelled = request_stop(&stop).await;
                        }
                        continue;
                    }
                    let values = (0..row.len())
                        .map(|index| match row.get(index) {
                            Some(value) => JsonValue::String(value.to_string()),
                            None => JsonValue::Null,
                        })
                        .collect();
                    if sink.row(values)? == SinkControl::Stop {
                        truncated = true;
                        stopped = true;
                        if alone && !cancelled {
                            cancelled = request_stop(&stop).await;
                        }
                        continue;
                    }
                    count += 1;
                }
                SimpleQueryMessage::CommandTag { tag, rows } => {
                    if open {
                        sink.message(rows_returned_message(count, truncated));
                        sink.end_set(truncated)?;
                        open = false;
                    } else if tag_counts_rows(&tag) {
                        rows_affected = Some(rows_affected.unwrap_or(0) + rows);
                        sink.message(rows_affected_message(rows));
                    } else {
                        sink.message(Message::info(tag));
                    }
                }
                _ => {}
            }
        }
        if cancelled && !stopped_by_cancel {
            self.absorb_late_cancel().await;
        }
        if open {
            sink.message(rows_returned_message(count, truncated));
            sink.end_set(truncated)?;
        }
        Ok((rows_affected, stopped))
    }

    /// Reads a statement through a cursor, in a transaction of its own. One
    /// text opens the transaction, declares the cursor and fetches the first
    /// rows. Further fetches follow while the set needs more rows, and a
    /// `COMMIT` ends the read and closes the cursor.
    ///
    /// The fetches stop one row past the row limit, so the server computes
    /// no row that the window does not show. A cancel at the row limit would
    /// roll back the work of the statement, for example the rows that a
    /// function of the `SELECT` inserts. The `COMMIT` keeps that work. A
    /// `SELECT` that calls such a function once per row keeps the work of
    /// the rows that the fetches read.
    ///
    /// The simple protocol keeps the text form of the values. Any error,
    /// and a stop of the user, rolls the transaction back. A read that ends
    /// before the end of its transaction, for example on a time limit, rolls
    /// it back through [`OpenBlock`].
    ///
    /// With `in_block`, the session is inside a transaction block of the
    /// user, and the cursor lives in that block. The first text then has no
    /// `BEGIN`, and a `CLOSE` of the cursor ends the read in place of the
    /// `COMMIT`. An error aborts the block of the user, as the statement
    /// alone would abort it, and the driver sends no `ROLLBACK`. The user
    /// ends the block. A read that ends before the `CLOSE`, for example on a
    /// time limit, leaves the cursor open until the block ends.
    async fn stream_cursor(
        &self,
        statement: &str,
        prepared: Option<&[String]>,
        in_block: bool,
        options: &ExecOptions,
        sink: &mut dyn RowSink,
    ) -> std::result::Result<(Option<u64>, bool), Failure> {
        let name = format!(
            "sql_explorer_read_{}",
            NEXT_CURSOR.fetch_add(1, Ordering::Relaxed)
        );
        let begin = if in_block { "" } else { "BEGIN; " };
        let prefix = format!("{begin}DECLARE {name} NO SCROLL CURSOR FOR ");
        let need = options.max_rows.saturating_add(1);
        let mut block = OpenBlock {
            client: &self.client,
            done: in_block,
        };
        let mut read = CursorRead::default();
        let mut asked = need.min(FETCH_BATCH);
        // The line break ends a comment at the end of the statement.
        let mut text = format!("{prefix}{statement}\n; FETCH {asked} FROM {name}");
        let mut shift = prefix.chars().count() as u32;
        loop {
            let fetched = match read
                .fetch(&self.client, &text, prepared, options, sink)
                .await
            {
                Ok(fetched) => fetched,
                Err(error) => {
                    if !in_block {
                        block.roll_back().await;
                    }
                    return Err(Failure { error, shift });
                }
            };
            read.fetched += fetched;
            if fetched < asked as u64 || read.fetched >= need as u64 || read.stopped {
                break;
            }
            asked = (need - read.fetched as usize).min(FETCH_BATCH);
            text = format!("FETCH {asked} FROM {name}");
            shift = 0;
        }
        block.done = true;
        let end = if in_block {
            format!("CLOSE {name}")
        } else {
            "COMMIT".to_string()
        };
        if let Err(error) = self.client.simple_query(&end).await {
            return Err(error.into());
        }
        if read.open {
            sink.message(rows_returned_message(read.count, read.truncated));
            sink.end_set(read.truncated)?;
        }
        Ok((None, read.stopped))
    }

    /// Runs one statement with bound parameters through the extended
    /// protocol and streams the rows into the sink one at a time. A
    /// statement goes through [`Self::stream_portal`] when [`read_path`]
    /// gives a path other than the walk. For any other read outside a
    /// transaction block, a stop cancels the statement on the
    /// server and drops the stream, so the rows past the stop do not cross
    /// the wire. Any other statement runs to its end, and the walk drops the
    /// rows past the stop. A fault that the server reports after those rows
    /// then still ends the run.
    ///
    /// After a cancel the walk reads the rest of the stream. A stream that
    /// ends without the error of the cancel is followed by
    /// [`Self::absorb_late_cancel`].
    ///
    /// The statement is prepared first, so the columns of the answer are
    /// known before the first row arrives. A `SELECT` that matches no row
    /// then still shows its columns, and a statement that returns no
    /// column reports the count of the rows it changed.
    async fn stream_with_params(
        &mut self,
        query: &str,
        params: &QueryParams,
        options: &ExecOptions,
        sink: &mut dyn RowSink,
    ) -> Result<Option<u64>> {
        refuse_client_copy(query)?;
        let bound = bind_params(params)?;
        let outside = self.probe_read(query).await;
        let may_cancel = outside == Some(true);

        let statement = self
            .client
            .prepare(query)
            .await
            .map_err(|error| locate_error(error, query, query, 0, 0))?;
        let columns: Vec<ColumnInfo> = statement
            .columns()
            .iter()
            .map(|column| ColumnInfo::new(column.name(), column.type_().name()))
            .collect();
        let returns_rows = !columns.is_empty();
        let needs = |wanted: &Type| {
            statement
                .columns()
                .iter()
                .any(|column| holds(column.type_(), wanted))
        };
        let mut settings = Settings::default();
        if needs(&Type::TIMESTAMPTZ) {
            settings.zone = self.session_zone().await;
        }
        if needs(&Type::MONEY) {
            settings.money_digits = self.money_digits().await;
        }

        let path = read_path(query, outside);
        if path != ReadPath::Walk {
            let in_block = path == ReadPath::UserBlock;
            return self
                .stream_portal(
                    &statement, &bound, columns, &settings, in_block, options, sink,
                )
                .await;
        }

        let rows = self.client.query_raw(&statement, &bound).await?;
        pin_mut!(rows);

        if returns_rows {
            sink.begin_set(columns)?;
        }
        let stop = self.stop.clone();
        let mut count = 0usize;
        let mut truncated = false;
        while let Some(row) = rows.try_next().await? {
            if truncated {
                continue;
            }
            if count >= options.max_rows
                || sink.row(row_to_json(&row, &settings))? == SinkControl::Stop
            {
                truncated = true;
                // The statement holds the rest of its result on the server.
                // The cancel ends it there, so those rows never cross the
                // wire.
                if may_cancel && request_stop(&stop).await {
                    if ended_before_cancel(rows.as_mut()).await {
                        self.absorb_late_cancel().await;
                    }
                    break;
                }
                continue;
            }
            count += 1;
        }

        if returns_rows {
            sink.message(rows_returned_message(count, truncated));
            sink.end_set(truncated)?;
            return Ok(None);
        }
        // The count of the rows arrives with the tag that ends the
        // statement, so it can be read after the walk.
        let affected = rows.rows_affected().unwrap_or(0);
        sink.message(rows_affected_message(affected));
        Ok(Some(affected))
    }

    /// Reads a statement with bound parameters through a portal, in a
    /// transaction of its own. Each execute of the portal asks for a count
    /// of rows, and the reads stop one row past the row limit, as in
    /// [`Self::stream_cursor`]. A `COMMIT` then keeps the work of the
    /// statement, where a cancel would roll it back. An error, and a read
    /// that ends early, drop the transaction, and the drop sends a
    /// `ROLLBACK`.
    ///
    /// With `in_block`, the session is inside a transaction block of the
    /// user, and the portal lives in that block. The driver then sends no
    /// `BEGIN` and no `COMMIT`. An error aborts the block of the user, as the
    /// statement alone would abort it, and the user ends the block.
    #[allow(clippy::too_many_arguments)]
    async fn stream_portal(
        &mut self,
        statement: &tokio_postgres::Statement,
        bound: &[TextParam],
        columns: Vec<ColumnInfo>,
        settings: &Settings,
        in_block: bool,
        options: &ExecOptions,
        sink: &mut dyn RowSink,
    ) -> Result<Option<u64>> {
        let returns_rows = !columns.is_empty();
        let (count, truncated) = if in_block {
            read_portal(
                &self.client,
                statement,
                bound,
                columns,
                settings,
                options,
                sink,
            )
            .await?
        } else {
            let transaction = self.client.transaction().await?;
            let read = read_portal(
                transaction.client(),
                statement,
                bound,
                columns,
                settings,
                options,
                sink,
            )
            .await?;
            transaction.commit().await?;
            read
        };
        if returns_rows {
            sink.message(rows_returned_message(count, truncated));
            sink.end_set(truncated)?;
        }
        Ok(None)
    }
}

/// Binds a portal on the client and feeds its rows to the sink, for
/// [`PostgresDriver::stream_portal`]. Each execute of the portal asks for a
/// count of rows, and the reads stop one row past the row limit. The caller
/// opens the transaction of the portal. Gives the count of the rows that the
/// sink took, and true when the set has more rows than the sink took.
async fn read_portal(
    client: &Client,
    statement: &tokio_postgres::Statement,
    bound: &[TextParam],
    columns: Vec<ColumnInfo>,
    settings: &Settings,
    options: &ExecOptions,
    sink: &mut dyn RowSink,
) -> Result<(usize, bool)> {
    let need = options.max_rows.saturating_add(1);
    let portal = client.bind_raw(statement, bound).await?;
    if !columns.is_empty() {
        sink.begin_set(columns)?;
    }
    let mut fetched = 0usize;
    let mut count = 0usize;
    let mut truncated = false;
    loop {
        let asked = (need - fetched).min(FETCH_BATCH);
        let rows = client.query_portal_raw(&portal, asked as i32).await?;
        pin_mut!(rows);
        while let Some(row) = rows.try_next().await? {
            fetched += 1;
            if truncated {
                continue;
            }
            if count >= options.max_rows
                || sink.row(row_to_json(&row, settings))? == SinkControl::Stop
            {
                truncated = true;
                continue;
            }
            count += 1;
        }
        // A portal that gave all its rows ends with its tag. A portal
        // that stops at the count of the execute gives no tag.
        if rows.rows_affected().is_some() || fetched >= need || truncated {
            break;
        }
    }
    drop(portal);
    Ok((count, truncated))
}

/// The way that [`PostgresDriver::stream_statement`] and
/// [`PostgresDriver::stream_with_params`] read the rows of a statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadPath {
    /// The statement runs to its end, and the walk drops the rows past the
    /// row limit. A read outside a transaction block can still end at the
    /// limit through a cancel.
    Walk,
    /// A cursor or a portal reads the rows in a transaction of the driver.
    OwnTransaction,
    /// A cursor or a portal reads the rows in the open transaction block of
    /// the user.
    UserBlock,
}

/// Picks the read path of a statement from the answer of
/// [`PostgresDriver::probe_read`].
///
/// Inside a block of the user, a cursor locks only the rows that its fetches
/// read, and the block keeps its locks until it ends. A statement with
/// `FOR UPDATE` or a similar clause would then lock fewer rows than the user
/// asked for, so it keeps the walk. A probe that failed also keeps the walk.
fn read_path(statement: &str, outside: Option<bool>) -> ReadPath {
    if !reads_through_a_cursor(statement) {
        return ReadPath::Walk;
    }
    match outside {
        Some(true) => ReadPath::OwnTransaction,
        Some(false) if !locks_rows(statement, Dialect::Postgres) => ReadPath::UserBlock,
        _ => ReadPath::Walk,
    }
}

/// The count that names each cursor of [`PostgresDriver::stream_cursor`]. A
/// cursor that the user declared `WITH HOLD` keeps its name past the end of
/// its transaction, so each read takes a name of its own.
static NEXT_CURSOR: AtomicU64 = AtomicU64::new(0);

/// The most rows that one fetch of [`PostgresDriver::stream_cursor`] asks
/// for. A run with a high row limit, such as an export, then reads in
/// steps, and a sink that takes no more rows ends the read at the next step.
const FETCH_BATCH: usize = 100_000;

/// True for a statement that only reads rows. [`only_reads`] accepts
/// `SELECT`, `WITH` and `SHOW`. `VALUES` and `TABLE` also give rows, and a
/// statement that starts with either word cannot change data.
fn reads_rows(statement: &str) -> bool {
    only_reads(statement, Dialect::Postgres)
        || matches!(
            leading_keyword(statement, Dialect::Postgres).as_str(),
            "values" | "table"
        )
}

/// True for a statement that [`reads_rows`] accepts and that a cursor can
/// read. `SHOW` cannot stand in a `DECLARE`.
fn reads_through_a_cursor(statement: &str) -> bool {
    reads_rows(statement) && leading_keyword(statement, Dialect::Postgres) != "show"
}

/// An error of one statement, with the count of the characters that went to
/// the server in front of the statement. See [`locate_error`].
struct Failure {
    error: Error,
    shift: u32,
}

impl Failure {
    /// Marks a server error with its place in the whole text.
    fn locate(self, query: &str, statement: &str, start: usize) -> Error {
        match self.error {
            Error::Postgres(error) => locate_error(error, query, statement, start, self.shift),
            other => other,
        }
    }
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Failure { error, shift: 0 }
    }
}

impl From<tokio_postgres::Error> for Failure {
    fn from(error: tokio_postgres::Error) -> Self {
        Error::from(error).into()
    }
}

/// The transaction of [`PostgresDriver::stream_cursor`]. A read that stops
/// before `done` is set, for example when the caller drops it at a time
/// limit, sends a `ROLLBACK` as it ends. The session then does not stay
/// inside the transaction for the statements that follow.
struct OpenBlock<'a> {
    client: &'a Client,
    done: bool,
}

impl OpenBlock<'_> {
    /// Rolls the transaction back and waits for the answer. A rollback that
    /// fails leaves nothing to do, because the connection is then closed.
    async fn roll_back(&mut self) {
        self.done = true;
        if let Err(error) = self.client.simple_query("ROLLBACK").await {
            log::warn!("The rollback of a read failed: {error}");
        }
    }
}

impl Drop for OpenBlock<'_> {
    fn drop(&mut self) {
        if !self.done {
            self.client.__private_api_rollback(None);
        }
    }
}

/// The state of one set that [`PostgresDriver::stream_cursor`] reads.
#[derive(Default)]
struct CursorRead {
    /// True after the first fetch named the columns.
    open: bool,
    /// The rows that the sink took.
    count: usize,
    /// The rows that all fetches gave.
    fetched: u64,
    truncated: bool,
    /// True when the sink took no more rows.
    stopped: bool,
}

impl CursorRead {
    /// Runs one text that fetches rows and feeds them to the sink. Gives the
    /// count of the rows that the fetch gave. The tags of `BEGIN` and
    /// `DECLARE` give no message.
    async fn fetch(
        &mut self,
        client: &Client,
        text: &str,
        prepared: Option<&[String]>,
        options: &ExecOptions,
        sink: &mut dyn RowSink,
    ) -> Result<u64> {
        let messages = client.simple_query_raw(text).await?;
        pin_mut!(messages);
        let mut fetched = 0;
        while let Some(message) = messages.try_next().await? {
            match message {
                SimpleQueryMessage::RowDescription(columns) if !self.open => {
                    let columns: Vec<(&str, u32)> = columns
                        .iter()
                        .map(|column| (column.name(), column.type_oid()))
                        .collect();
                    sink.begin_set(simple_columns(prepared, &columns))?;
                    self.open = true;
                }
                SimpleQueryMessage::Row(row) => {
                    if self.stopped {
                        continue;
                    }
                    if self.count >= options.max_rows {
                        self.truncated = true;
                        continue;
                    }
                    let values = (0..row.len())
                        .map(|index| match row.get(index) {
                            Some(value) => JsonValue::String(value.to_string()),
                            None => JsonValue::Null,
                        })
                        .collect();
                    if sink.row(values)? == SinkControl::Stop {
                        self.truncated = true;
                        self.stopped = true;
                        continue;
                    }
                    self.count += 1;
                }
                SimpleQueryMessage::CommandTag { tag, rows } if tag.starts_with("FETCH") => {
                    fetched = rows;
                }
                _ => {}
            }
        }
        Ok(fetched)
    }
}

/// True for the tag of a statement whose number counts rows. Any other tag,
/// such as `CREATE TABLE` or `SET`, names its statement and counts nothing.
fn tag_counts_rows(tag: &str) -> bool {
    let word = tag.split(' ').next().unwrap_or_default();
    matches!(
        word,
        "INSERT" | "UPDATE" | "DELETE" | "MERGE" | "SELECT" | "COPY" | "FETCH" | "MOVE"
    )
}

/// Marks a server error with its place in the whole text. The server gives
/// the place as a 1-based character position inside `statement`, and
/// `start` is the byte offset of the statement in `query`. An error without
/// a position marks the start of the statement.
///
/// A statement that goes to the server behind a prefix, such as the
/// `DECLARE` of a cursor, gives the length of that prefix in characters as
/// `shift`. A position inside the prefix marks the start of the statement.
fn locate_error(
    error: tokio_postgres::Error,
    query: &str,
    statement: &str,
    start: usize,
    shift: u32,
) -> Error {
    let base = place_of_byte_offset(query, start);
    let position = match error.as_db_error().and_then(DbError::position) {
        Some(ErrorPosition::Original(position)) if *position > shift => Some(*position - shift),
        _ => None,
    };
    let (line, column) = match position {
        Some(position) => offset_place(base, place_of_char_position(statement, position)),
        None => base,
    };
    Error::from(error).at(line, column)
}

/// Lists the relations of one schema. A partition shows below its parent in
/// the partitions folder, so the list leaves out each relation that is a
/// partition. A table with thousands of partitions then gives one entry.
const TABLES_QUERY: &str = "SELECT c.relname, c.relkind \
     FROM pg_catalog.pg_class AS c \
     JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace \
     WHERE n.nspname = $1 AND c.relkind IN ('r', 'p', 'v', 'm', 'f') \
       AND NOT c.relispartition \
     ORDER BY c.relkind, c.relname";

/// Turns the name and the `relkind` letter of a relation into its entry.
fn relation_of(name: String, letter: i8) -> Table {
    Table::new(name, relation_type_of(letter))
}

/// Reads the `relkind` letter of a relation. [`TABLES_QUERY`] and the
/// snapshot read only the letters of the relations that the tree shows, so
/// any other letter gives a plain table.
fn relation_type_of(letter: i8) -> RelationType {
    match letter as u8 {
        b'v' => RelationType::View,
        b'm' => RelationType::MaterializedView,
        b'p' => RelationType::PartitionedTable,
        b'f' => RelationType::ForeignTable,
        _ => RelationType::Table,
    }
}

/// Lists the routines of one schema from `pg_proc`. Each overload of a name
/// is a row of its own, so the name of the entry contains the types of the
/// arguments. The view `information_schema.routines` shows only the
/// routines that the user owns or can run, and gives the same name to every
/// overload.
const ROUTINES_QUERY: &str = "SELECT p.proname || '(' || \
            pg_catalog.pg_get_function_identity_arguments(p.oid) || ')', \
            CASE p.prokind WHEN 'p' THEN 'PROCEDURE' ELSE 'FUNCTION' END \
     FROM pg_catalog.pg_proc AS p \
     JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace \
     WHERE n.nspname = $1 \
     ORDER BY 2, 1";

/// Lists the columns of the indexes of one relation, one column of one index
/// in each row. The array `indkey` is opened with its order kept. Its first
/// `indnkeyatts` entries are the key, and the entries after them are the
/// `INCLUDE` columns. An entry of zero is an expression, which
/// `pg_get_indexdef` gives as text.
const INDEXES_QUERY: &str = "SELECT i.relname, idx.indisunique, idx.indisprimary, \
            COALESCE(a.attname::text, \
                     pg_catalog.pg_get_indexdef(idx.indexrelid, k.ord::int, true)), \
            k.ord > idx.indnkeyatts \
     FROM pg_catalog.pg_index AS idx \
     JOIN pg_catalog.pg_class AS i ON i.oid = idx.indexrelid \
     JOIN pg_catalog.pg_class AS t ON t.oid = idx.indrelid \
     JOIN pg_catalog.pg_namespace AS n ON n.oid = t.relnamespace \
     JOIN LATERAL unnest(idx.indkey) WITH ORDINALITY AS k(attnum, ord) ON true \
     LEFT JOIN pg_catalog.pg_attribute AS a \
            ON a.attrelid = t.oid AND a.attnum = k.attnum AND k.attnum > 0 \
     WHERE n.nspname = $1 AND t.relname = $2 \
     ORDER BY i.relname, k.ord";

/// Adds one row of [`INDEXES_QUERY`] to the record of its index. An
/// `INCLUDE` column goes into the list of the included columns.
fn add_postgres_index_column(
    indexes: &mut Vec<IndexInfo>,
    name: String,
    unique: bool,
    primary: bool,
    column: Option<String>,
    included: bool,
) {
    match column {
        Some(column) if included => add_included_column(indexes, name, unique, primary, column),
        column => add_index_column(indexes, name, unique, primary, column),
    }
}

/// The largest number of partitions that the tree shows for one table.
const PARTITION_LIMIT: usize = 1000;

/// Lists the partitions of one table with the bound of each. The cast to
/// `regclass` gives the name with its schema when the schema is not on the
/// search path. The third parameter is the limit of the rows.
const PARTITIONS_QUERY: &str = "SELECT c.oid::pg_catalog.regclass::text, \
            pg_catalog.pg_get_expr(c.relpartbound, c.oid) \
     FROM pg_catalog.pg_inherits AS h \
     JOIN pg_catalog.pg_class AS c ON c.oid = h.inhrelid \
     JOIN pg_catalog.pg_class AS p ON p.oid = h.inhparent \
     JOIN pg_catalog.pg_namespace AS n ON n.oid = p.relnamespace \
     WHERE n.nspname = $1 AND p.relname = $2 AND c.relispartition \
     ORDER BY c.relname \
     LIMIT $3";

/// Turns the rows of [`PARTITIONS_QUERY`] into the list of the tree. Each
/// entry names the partition and then its bound. The query reads one row
/// more than [`PARTITION_LIMIT`], and that row marks the list as truncated.
fn partition_list(rows: Vec<(String, Option<String>)>) -> PartitionList {
    let truncated = rows.len() > PARTITION_LIMIT;
    let partitions = rows
        .into_iter()
        .take(PARTITION_LIMIT)
        .map(|(name, bound)| Partition {
            values: match bound {
                Some(bound) => format!("{name} {bound}"),
                None => name,
            },
        })
        .collect();
    PartitionList {
        partitions,
        truncated,
    }
}

/// Builds the statement that reads the columns of every relation for the
/// schema snapshot. The statement reads the catalog directly, because the
/// views of `information_schema` are slow on a large catalog, leave out the
/// materialized views, and name an enumerated type `USER-DEFINED`. The
/// limit is one row past the count of columns, so the walk can tell that
/// the snapshot is not complete.
///
/// The statement leaves out the partitions, as the tree does. The columns
/// of a table with thousands of partitions then do not fill the limit.
fn snapshot_query(max_columns: usize) -> String {
    format!(
        "SELECT n.nspname, c.relname, c.relkind, a.attname, \
                pg_catalog.format_type(a.atttypid, a.atttypmod) \
         FROM pg_catalog.pg_attribute AS a \
         JOIN pg_catalog.pg_class AS c ON c.oid = a.attrelid \
         JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace \
         WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f') AND NOT c.relispartition \
           AND a.attnum > 0 AND NOT a.attisdropped \
           AND n.nspname NOT IN ('pg_toast', 'pg_catalog', 'information_schema') \
           AND n.nspname NOT LIKE 'pg\\_temp\\_%' \
           AND n.nspname NOT LIKE 'pg\\_toast\\_temp\\_%' \
         ORDER BY n.nspname, c.relname, a.attnum \
         LIMIT {}",
        max_columns.saturating_add(1).min(i64::MAX as usize)
    )
}

/// The keyword that asks PostgreSQL for a plan. The analysed form runs the
/// statement, so a statement that writes rows writes them.
pub fn plan_prefix(mode: PlanMode) -> &'static str {
    match mode {
        PlanMode::Estimated => "EXPLAIN (FORMAT TEXT)",
        PlanMode::Actual => "EXPLAIN (ANALYZE, BUFFERS)",
    }
}

/// Builds the statement that reads the CREATE text of one view or one
/// materialized view. PostgreSQL keeps no text for a table, so a table gives no statement and the command
/// layer builds a draft instead.
///
/// `pg_get_viewdef` gives the query of the view alone, so the statement adds
/// the CREATE clause and the name. A materialized view gets the clause of
/// its own relation type, because `CREATE VIEW` makes a plain view. A
/// materialized view that has no rows yet (`relispopulated` is false) ends
/// with `WITH NO DATA`. Without it, the text runs the query of the view and
/// fills the view. A plain view is always marked as populated. The
/// statements of [`INDEX_STATEMENTS`] come after the CREATE statement, in
/// the same row and column.
///
/// The name goes into the statement as a literal that `regclass` reads. A
/// name of another database cannot be read this way, so the name contains
/// the schema and the table alone. The CREATE clause names the view with its
/// schema, and the body of the view is read under [`QUALIFIED_NAMES`].
fn create_query_text(
    schema: Option<&str>,
    table: &str,
    relation_type: RelationType,
) -> Option<CreateQuery> {
    if !relation_type.is_view() {
        return None;
    }
    let name = Dialect::Postgres.qualified_name(None, schema, table);
    Some(CreateQuery::new(
        format!(
            "SELECT {QUALIFIED_NAMES}'CREATE ' || CASE c.relkind \
             WHEN 'm' THEN 'MATERIALIZED VIEW ' ELSE 'OR REPLACE VIEW ' END || \
             pg_catalog.quote_ident(n.nspname) || '.' || pg_catalog.quote_ident(c.relname) || \
             E' AS\\n' || pg_catalog.rtrim(pg_catalog.pg_get_viewdef(c.oid, true), ';') || \
             CASE WHEN c.relispopulated THEN ';' ELSE E'\\nWITH NO DATA;' END || \
             {INDEX_STATEMENTS} END \
             FROM pg_catalog.pg_class AS c \
             JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace \
             WHERE c.oid = {}::regclass;",
            Dialect::Postgres.quote_literal(&name)
        ),
        0,
    ))
}

/// The CREATE INDEX statement of each index of the relation `c`, in the order
/// of the index names. Each statement ends with `;` and starts after a blank
/// line. A relation without an index gives an empty text.
///
/// The text of a materialized view without these statements makes the view
/// again without its indexes. `REFRESH MATERIALIZED VIEW CONCURRENTLY` then
/// fails, because it needs a unique index. A plain view has no index, so its
/// text does not change. The statements run in the `THEN` branch of
/// [`QUALIFIED_NAMES`], so each one names the view with its schema.
const INDEX_STATEMENTS: &str = "COALESCE((SELECT pg_catalog.string_agg(E'\\n\\n' || \
     pg_catalog.pg_get_indexdef(i.indexrelid) || ';', '' ORDER BY x.relname) \
     FROM pg_catalog.pg_index AS i \
     JOIN pg_catalog.pg_class AS x ON x.oid = i.indexrelid \
     WHERE i.indrelid = c.oid), '')";

/// The start of a `CASE` expression that sets the search path to
/// `pg_catalog` alone before its `THEN` branch runs. `pg_get_viewdef` and
/// `pg_get_triggerdef` leave out the schema of a name that the search path
/// of the session finds. A text read under the search path `app` then names
/// `orders` where it means `app.orders`, and fails in a session with another
/// search path. With `pg_catalog` alone, every name of a user schema gets
/// its schema.
///
/// The third argument `true` limits the change to the transaction of the
/// statement, so the search path of the session comes back after it. A
/// `CASE` runs its condition before its branch.
const QUALIFIED_NAMES: &str =
    "CASE WHEN pg_catalog.set_config('search_path', 'pg_catalog', true) IS NOT NULL THEN ";

/// Lists the triggers of one relation. A foreign key makes triggers of its
/// own, and `tgisinternal` marks these, so they stay out of the list.
///
/// A `CREATE CONSTRAINT TRIGGER` statement makes a trigger and a row of
/// `pg_constraint` with the type `t`. The `tgconstraint` field of the trigger
/// points at that row. The **Keys** folder shows the row, so the list leaves
/// out each trigger with a `tgconstraint` value other than 0. Without this
/// condition, the tree shows the trigger in two folders.
///
/// The `tgattr` field keeps the column numbers of an `UPDATE OF` clause in
/// the order of the clause. The subquery gives their names in that order.
const TRIGGERS_QUERY: &str = "SELECT t.tgname, t.tgtype, t.tgenabled, \
            ARRAY(SELECT a.attname::text \
                  FROM unnest(t.tgattr) WITH ORDINALITY AS k(attnum, ord) \
                  JOIN pg_catalog.pg_attribute AS a \
                         ON a.attrelid = t.tgrelid AND a.attnum = k.attnum \
                  ORDER BY k.ord) \
     FROM pg_catalog.pg_trigger AS t \
     JOIN pg_catalog.pg_class AS c ON c.oid = t.tgrelid \
     JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace \
     WHERE n.nspname = $1 AND c.relname = $2 AND NOT t.tgisinternal \
       AND t.tgconstraint = 0 \
     ORDER BY t.tgname";

/// The bits of `tgtype`, as the header `pg_trigger.h` of the server names
/// them. The bit of the row level does not change the time or the events.
const TRIGGER_TYPE_BEFORE: i16 = 1 << 1;
const TRIGGER_TYPE_INSERT: i16 = 1 << 2;
const TRIGGER_TYPE_DELETE: i16 = 1 << 3;
const TRIGGER_TYPE_UPDATE: i16 = 1 << 4;
const TRIGGER_TYPE_TRUNCATE: i16 = 1 << 5;
const TRIGGER_TYPE_INSTEAD: i16 = 1 << 6;

/// Builds the record of one trigger from its name, its `tgtype` bits, its
/// `tgenabled` letter and the columns of its `UPDATE OF` clause. The letter
/// `D` marks a disabled trigger. The letter `O` marks a trigger that runs in
/// a normal session, and `A` marks a trigger that runs in every session. The letter `R` marks a replica
/// trigger, which runs only when `session_replication_role` is `replica`, so
/// the record gives it as not enabled.
fn trigger_of(name: String, bits: i16, enabled: i8, update_columns: Vec<String>) -> Trigger {
    let timing = if bits & TRIGGER_TYPE_INSTEAD != 0 {
        TriggerTiming::InsteadOf
    } else if bits & TRIGGER_TYPE_BEFORE != 0 {
        TriggerTiming::Before
    } else {
        TriggerTiming::After
    };
    let events = [
        (TRIGGER_TYPE_INSERT, TriggerEvent::Insert),
        (TRIGGER_TYPE_UPDATE, TriggerEvent::Update),
        (TRIGGER_TYPE_DELETE, TriggerEvent::Delete),
        (TRIGGER_TYPE_TRUNCATE, TriggerEvent::Truncate),
    ]
    .into_iter()
    .filter(|(bit, _)| bits & bit != 0)
    .map(|(_, event)| event)
    .collect();
    Trigger {
        name,
        timing,
        events,
        enabled: enabled == b'O' as i8 || enabled == b'A' as i8,
        replica: enabled == b'R' as i8,
        update_columns,
    }
}

/// Builds the statement that reads the CREATE text of one trigger. A
/// trigger name is unique within its relation alone, so the statement finds
/// the trigger by the relation and the name. PostgreSQL has no scheduled
/// events. The text is read under [`QUALIFIED_NAMES`], and the form that is
/// not pretty always names the relation with its schema.
///
/// `pg_get_triggerdef` does not write the `tgenabled` letter, and `CREATE
/// TRIGGER` always makes a trigger with the letter `O`. Thus the text of a
/// trigger with the letter `D`, `R` or `A` gets a second statement on a line
/// of its own. This `ALTER TABLE` statement disables the trigger, or enables
/// it as a replica trigger or as an always trigger. A view cannot have
/// these letters, because `ALTER TABLE` does not change the triggers of a
/// view.
fn object_query_text(
    schema: Option<&str>,
    table: &str,
    name: &str,
    object_type: ObjectType,
) -> Option<CreateQuery> {
    if object_type != ObjectType::Trigger {
        return None;
    }
    let relation = Dialect::Postgres.qualified_name(None, schema, table);
    Some(CreateQuery::new(
        format!(
            "SELECT {QUALIFIED_NAMES}pg_catalog.pg_get_triggerdef(t.oid, false) || ';' || \
             CASE t.tgenabled WHEN 'O' THEN '' ELSE \
             E'\\nALTER TABLE ' || t.tgrelid::pg_catalog.regclass::text || \
             CASE t.tgenabled WHEN 'D' THEN ' DISABLE' \
             WHEN 'R' THEN ' ENABLE REPLICA' ELSE ' ENABLE ALWAYS' END || \
             ' TRIGGER ' || pg_catalog.quote_ident(t.tgname) || ';' END END \
             FROM pg_catalog.pg_trigger AS t \
             WHERE t.tgrelid = {}::regclass AND t.tgname = {};",
            Dialect::Postgres.quote_literal(&relation),
            Dialect::Postgres.quote_literal(name)
        ),
        0,
    ))
}

/// One bound parameter, in the text form that the server reads.
///
/// A parameter of this type accepts every column type and writes the value
/// in the text format. The server then converts the text to the type that
/// the statement asks for. A whole number binds against `int2`, `int4`,
/// `int8` and `numeric` alike, a float binds against `real`, and text binds
/// against a type that is not `text`. A binary parameter would have to hold
/// one form for each type, so the driver leaves that work to the server.
pub struct TextParam(Option<String>);

impl ToSql for TextParam {
    fn to_sql(&self, _: &Type, out: &mut BytesMut) -> std::result::Result<IsNull, BoxError> {
        match &self.0 {
            Some(text) => {
                out.extend_from_slice(text.as_bytes());
                Ok(IsNull::No)
            }
            None => Ok(IsNull::Yes),
        }
    }

    fn accepts(_: &Type) -> bool {
        true
    }

    fn encode_format(&self, _: &Type) -> Format {
        Format::Text
    }

    to_sql_checked!();
}

impl std::fmt::Debug for TextParam {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            Some(text) => write!(formatter, "{text:?}"),
            None => formatter.write_str("NULL"),
        }
    }
}

/// Turns the JSON parameters into values the driver can bind.
pub fn bind_params(params: &QueryParams) -> Result<Vec<TextParam>> {
    let mut bound: Vec<TextParam> = Vec::new();
    for param in params {
        let text = match &param.value {
            JsonValue::String(text) => Some(text.clone()),
            JsonValue::Bool(flag) => Some(flag.to_string()),
            JsonValue::Null => None,
            JsonValue::Number(number) => match number_value(number) {
                Some(NumberValue::Integer(value)) => Some(value.to_string()),
                Some(NumberValue::Float(value)) => Some(value.to_string()),
                None => return Err(number_out_of_range(number)),
            },
            other => Some(other.to_string()),
        };
        bound.push(TextParam(text));
    }
    Ok(bound)
}

/// The probe that reads the count of the digits of the fraction of `money`.
const MONEY_DIGITS: &str = "SELECT scale(0::money::numeric)";

/// The settings of the session that the text of some binary values needs.
/// The text form of the simple protocol applies them on the server.
pub struct Settings {
    /// The zone that a `timestamptz` value shows in.
    zone: TimeZone,
    /// The count of the digits of the fraction of a `money` value.
    money_digits: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            zone: TimeZone::UTC,
            money_digits: 2,
        }
    }
}

/// Converts one row into an array of JSON values.
pub fn row_to_json(row: &Row, settings: &Settings) -> Vec<JsonValue> {
    (0..row.columns().len())
        .map(|index| cell_to_json(row, index, settings))
        .collect()
}

/// Reads one cell. The extended protocol sends every value in its binary
/// form, so the bytes of the cell come out of the row and a reader for the
/// type of the column turns them into JSON. A cell shows NULL only when the
/// server sent no value: bytes that no reader understands show as text when
/// they are text, and as base64 when they are not.
fn cell_to_json(row: &Row, index: usize, settings: &Settings) -> JsonValue {
    // The type stays in the row, because a copy of it would cost a count on
    // a shared record for each cell of the answer.
    let column_type = row.columns()[index].type_();
    match row.try_get::<_, Option<Raw>>(index) {
        Ok(None) => JsonValue::Null,
        Ok(Some(Raw(bytes))) => decode_value(column_type, bytes, settings),
        Err(error) => {
            log::debug!("A column gave no value: {error}");
            JsonValue::Null
        }
    }
}

/// The bytes of one cell, as the server sent them.
struct Raw<'a>(&'a [u8]);

impl<'a> FromSql<'a> for Raw<'a> {
    fn from_sql(_: &Type, bytes: &'a [u8]) -> std::result::Result<Self, BoxError> {
        Ok(Raw(bytes))
    }

    fn accepts(_: &Type) -> bool {
        true
    }
}

/// The error that the conversions of `tokio_postgres` give.
type BoxError = Box<dyn std::error::Error + Sync + Send>;

/// Turns the binary form of one value into JSON. The shape of a value that
/// holds other values comes from the `Kind` of its type, so an array, a
/// range, and a composite of any element type read the same way.
fn decode_value(column_type: &Type, bytes: &[u8], settings: &Settings) -> JsonValue {
    match column_type.kind() {
        Kind::Array(element) => decode_array(element, bytes, settings),
        Kind::Range(element) => decode_range(element, bytes, settings),
        Kind::Multirange(element) => decode_multirange(element, bytes, settings),
        // A domain carries the value of the type it is built on.
        Kind::Domain(inner) => decode_value(inner, bytes, settings),
        Kind::Composite(fields) => decode_composite(fields, bytes, settings),
        // The value of an enumerated type is the label itself.
        Kind::Enum(_) => text_or_bytes(bytes),
        _ => decode_scalar(column_type, bytes, settings),
    }
}

/// Reads one value that holds no other value.
fn decode_scalar(column_type: &Type, bytes: &[u8], settings: &Settings) -> JsonValue {
    match *column_type {
        Type::BOOL => scalar(column_type, bytes, JsonValue::Bool),
        Type::INT2 => scalar(column_type, bytes, |value: i16| value.into()),
        Type::INT4 => scalar(column_type, bytes, |value: i32| value.into()),
        Type::INT8 => scalar(column_type, bytes, |value: i64| value.into()),
        // An OID is four bytes without a sign, so the read of a signed
        // eight-byte number refuses it.
        Type::OID => scalar(column_type, bytes, |value: u32| value.into()),
        Type::FLOAT4 => scalar(column_type, bytes, f32_to_json),
        Type::FLOAT8 => scalar(column_type, bytes, f64_to_json),
        Type::NUMERIC => numeric_text(bytes),
        Type::TEXT
        | Type::VARCHAR
        | Type::NAME
        | Type::BPCHAR
        | Type::CHAR
        | Type::XML
        | Type::UNKNOWN => text_or_bytes(bytes),
        Type::UUID => scalar(column_type, bytes, |value: uuid::Uuid| {
            JsonValue::String(value.to_string())
        }),
        // The text of a JSON value stays as the server wrote it, as on the
        // simple protocol. A parse into a JSON tree would lose the order of
        // the keys of `json` and the digits of a long number. The binary
        // form of `jsonb` starts with the byte of its version.
        Type::JSON => text_or_bytes(bytes),
        Type::JSONB => match bytes.split_first() {
            Some((1, text)) => text_or_bytes(text),
            _ => text_or_bytes(bytes),
        },
        Type::BYTEA => bytea_text(bytes),
        Type::DATE => endless(
            Reader::new(bytes).i32().map(i64::from),
            i32::MAX as i64,
            i32::MIN as i64,
        )
        .unwrap_or_else(|| {
            scalar(column_type, bytes, |value: NaiveDate| {
                let (date, before) = date_text(value);
                JsonValue::String(format!("{date}{}", era(before)))
            })
        }),
        // The clock of PostgreSQL reaches 24:00:00, which `chrono` refuses,
        // so the count of microseconds since midnight gives the text.
        Type::TIME => match Reader::new(bytes).i64() {
            Some(micros) if bytes.len() == 8 => JsonValue::String(clock_text(micros)),
            _ => text_or_bytes(bytes),
        },
        Type::TIMESTAMP => {
            endless(Reader::new(bytes).i64(), i64::MAX, i64::MIN).unwrap_or_else(|| {
                scalar(column_type, bytes, |value: NaiveDateTime| {
                    JsonValue::String(date_time_text(value, ""))
                })
            })
        }
        Type::TIMESTAMPTZ => {
            endless(Reader::new(bytes).i64(), i64::MAX, i64::MIN).unwrap_or_else(|| {
                scalar(column_type, bytes, |value: DateTime<Utc>| {
                    JsonValue::String(zoned_text(value, &settings.zone))
                })
            })
        }
        Type::MONEY => money_text(bytes, settings.money_digits),
        Type::INTERVAL => interval_text(bytes),
        Type::INET | Type::CIDR => inet_text(bytes),
        Type::MACADDR => mac_text(bytes, 6),
        Type::MACADDR8 => mac_text(bytes, 8),
        Type::TIMETZ => time_with_zone_text(bytes),
        Type::BIT | Type::VARBIT => bits_text(bytes),
        // The binary form of these types is an OID or a counter of four
        // bytes without a sign. The name that the text form gives needs a
        // read of the catalog, so the grid shows the number.
        Type::REGCLASS
        | Type::REGTYPE
        | Type::REGPROC
        | Type::REGPROCEDURE
        | Type::REGOPER
        | Type::REGOPERATOR
        | Type::REGNAMESPACE
        | Type::REGROLE
        | Type::REGCONFIG
        | Type::REGDICTIONARY
        | Type::XID
        | Type::CID => match Reader::new(bytes).u32() {
            Some(value) if bytes.len() == 4 => value.into(),
            _ => text_or_bytes(bytes),
        },
        // tokio-postgres asks for every column of the extended protocol in
        // the binary form, and it gives no way to ask for the text form of
        // one column. So each type below has a reader of its binary form.
        Type::POINT
        | Type::LSEG
        | Type::BOX
        | Type::PATH
        | Type::POLYGON
        | Type::LINE
        | Type::CIRCLE => geometry_text(column_type, bytes),
        Type::PG_LSN => lsn_text(bytes),
        Type::XID8 => match Reader::new(bytes).u64() {
            Some(value) if bytes.len() == 8 => value.into(),
            _ => text_or_bytes(bytes),
        },
        Type::TID => tid_text(bytes),
        Type::PG_SNAPSHOT | Type::TXID_SNAPSHOT => snapshot_text(bytes),
        // A jsonpath value is a version byte and then the text.
        Type::JSONPATH => match bytes.split_first() {
            Some((1, text)) => text_or_bytes(text),
            _ => text_or_bytes(bytes),
        },
        Type::TS_VECTOR => tsvector_text(bytes),
        Type::TSQUERY => tsquery_text(bytes),
        // The extension gives hstore a new OID in each database, so the
        // name finds the type.
        _ if column_type.name() == "hstore" => hstore_text(bytes),
        // The binary form of the type of another extension is unknown. It is
        // often the text itself, as for citext.
        _ => text_or_bytes(bytes),
    }
}

/// Writes a `timetz` value in the form that PostgreSQL writes, such as
/// `10:00:00+02`. The value holds the microseconds since midnight and the
/// offset of the zone in seconds west of UTC.
fn time_with_zone_text(bytes: &[u8]) -> JsonValue {
    let mut reader = Reader::new(bytes);
    let (Some(micros), Some(west)) = (reader.i64(), reader.i32()) else {
        return text_or_bytes(bytes);
    };
    JsonValue::String(format!("{}{}", clock_text(micros), offset_text(-west)))
}

/// Writes an offset of a zone in seconds east of UTC, in the form that
/// PostgreSQL writes, such as `+05`, `-03:30` or `+00:19:32`.
fn offset_text(east: i32) -> String {
    let sign = if east < 0 { '-' } else { '+' };
    let whole = east.unsigned_abs();
    let mut text = format!("{sign}{:02}", whole / 3600);
    if !whole.is_multiple_of(3600) {
        text.push_str(&format!(":{:02}", whole % 3600 / 60));
    }
    if !whole.is_multiple_of(60) {
        text.push_str(&format!(":{:02}", whole % 60));
    }
    text
}

/// Writes a `bit` or a `varbit` value as its digits, such as `1010`. The
/// value holds the count of the bits and then the bits, first bit high.
fn bits_text(bytes: &[u8]) -> JsonValue {
    let mut reader = Reader::new(bytes);
    let Some(count) = reader.i32().and_then(|count| usize::try_from(count).ok()) else {
        return text_or_bytes(bytes);
    };
    let Some(body) = reader.take(count.div_ceil(8)) else {
        return text_or_bytes(bytes);
    };
    let digits = (0..count)
        .map(|index| {
            if body[index / 8] & (0x80 >> (index % 8)) != 0 {
                '1'
            } else {
                '0'
            }
        })
        .collect();
    JsonValue::String(digits)
}

/// Writes a value of a geometric type in the form that PostgreSQL writes,
/// such as `(1,2)` for a point or `<(0,0),5>` for a circle. Each type sends
/// its coordinates as float8 values. A path also sends a flag for a closed
/// path and the count of its points, and a polygon sends the count of its
/// points.
fn geometry_text(column_type: &Type, bytes: &[u8]) -> JsonValue {
    let mut reader = Reader::new(bytes);
    let text = match *column_type {
        Type::POINT => points(&mut reader, 1),
        Type::LSEG => points(&mut reader, 2).map(|text| format!("[{text}]")),
        Type::BOX => points(&mut reader, 2),
        Type::PATH => reader.u8().and_then(|closed| {
            let count = usize::try_from(reader.i32()?).ok()?;
            let text = points(&mut reader, count)?;
            Some(if closed != 0 {
                format!("({text})")
            } else {
                format!("[{text}]")
            })
        }),
        Type::POLYGON => reader
            .i32()
            .and_then(|count| points(&mut reader, usize::try_from(count).ok()?))
            .map(|text| format!("({text})")),
        Type::LINE => floats(&mut reader, 3).map(|values| format!("{{{}}}", values.join(","))),
        // A circle, the only other geometric type.
        _ => floats(&mut reader, 3)
            .map(|values| format!("<({},{}),{}>", values[0], values[1], values[2])),
    };
    complete(text, &reader, bytes)
}

/// Gives the text when the reader used every byte of the value, and the
/// text rule for the bytes when it did not.
fn complete(text: Option<String>, reader: &Reader<'_>, bytes: &[u8]) -> JsonValue {
    match text {
        Some(text) if reader.is_empty() => JsonValue::String(text),
        _ => text_or_bytes(bytes),
    }
}

/// Runs the read of one value over its bytes. The text rule for the bytes
/// applies when the read fails or leaves bytes unused.
fn read_with<'a>(
    bytes: &'a [u8],
    read: impl FnOnce(&mut Reader<'a>) -> Option<String>,
) -> JsonValue {
    let mut reader = Reader::new(bytes);
    let text = read(&mut reader);
    complete(text, &reader, bytes)
}

/// Reads float8 values and writes each one as PostgreSQL writes it.
fn floats(reader: &mut Reader<'_>, count: usize) -> Option<Vec<String>> {
    (0..count).map(|_| reader.f64().map(float_text)).collect()
}

/// Reads points and writes them as `(x,y)`, separated by commas.
fn points(reader: &mut Reader<'_>, count: usize) -> Option<String> {
    let values = floats(reader, count * 2)?;
    let points: Vec<String> = values
        .chunks(2)
        .map(|pair| format!("({},{})", pair[0], pair[1]))
        .collect();
    Some(points.join(","))
}

/// Writes a float8 value as PostgreSQL 12 and later write it. The text has
/// the fewest digits that read back to the same value. A decimal exponent
/// below -4 or above 14 gives the exponent form, such as `1e+21` or
/// `1.5e-05`.
fn float_text(value: f64) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    let scientific = format!("{value:e}");
    let (mantissa, exponent) = scientific
        .split_once('e')
        .unwrap_or((scientific.as_str(), "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    if (-4..15).contains(&exponent) {
        return value.to_string();
    }
    let sign = if exponent < 0 { '-' } else { '+' };
    format!("{mantissa}e{sign}{:02}", exponent.unsigned_abs())
}

/// Writes a `pg_lsn` value in the form that PostgreSQL writes, such as
/// `16/B374D848`. The value is a count of eight bytes, and the text shows
/// the high and the low four bytes in hexadecimal.
fn lsn_text(bytes: &[u8]) -> JsonValue {
    match Reader::new(bytes).u64() {
        Some(value) if bytes.len() == 8 => {
            JsonValue::String(format!("{:X}/{:X}", value >> 32, value & 0xFFFF_FFFF))
        }
        _ => text_or_bytes(bytes),
    }
}

/// Writes a `tid` value, which holds the number of a block and the place of
/// the row in the block, such as `(0,1)`.
fn tid_text(bytes: &[u8]) -> JsonValue {
    let mut reader = Reader::new(bytes);
    let text = match (reader.u32(), reader.u16()) {
        (Some(block), Some(offset)) => Some(format!("({block},{offset})")),
        _ => None,
    };
    complete(text, &reader, bytes)
}

/// Writes a `pg_snapshot` or a `txid_snapshot` value in the form that
/// PostgreSQL writes, such as `10:20:12,15`. The value holds the count of
/// the transactions in progress, the lowest and the highest transaction,
/// and then the transactions in progress.
fn snapshot_text(bytes: &[u8]) -> JsonValue {
    read_with(bytes, |reader| {
        let count = usize::try_from(reader.i32()?).ok()?;
        let (low, high) = (reader.u64()?, reader.u64()?);
        let running: Option<Vec<String>> = (0..count)
            .map(|_| reader.u64().map(|id| id.to_string()))
            .collect();
        Some(format!("{low}:{high}:{}", running?.join(",")))
    })
}

/// Writes a lexeme of text search between single quotes. A quote and a
/// backslash inside the lexeme are doubled, as in PostgreSQL.
fn lexeme_text(word: &str) -> String {
    let mut text = String::from('\'');
    for character in word.chars() {
        if character == '\'' || character == '\\' {
            text.push(character);
        }
        text.push(character);
    }
    text.push('\'');
    text
}

/// Writes a `tsvector` value in the form that PostgreSQL writes, such as
/// `'cat':3 'fat':2A`. The value holds the count of the lexemes. Each
/// lexeme is text that ends with a zero byte, then the count of its
/// positions, then the positions. The top two bits of a position give its
/// weight.
fn tsvector_text(bytes: &[u8]) -> JsonValue {
    read_with(bytes, |reader| {
        let count = reader.i32()?;
        let mut lexemes = Vec::new();
        for _ in 0..count.max(0) {
            let mut text = lexeme_text(reader.c_text()?);
            let positions: Option<Vec<String>> = (0..reader.u16()?)
                .map(|_| {
                    reader.u16().map(|entry| {
                        let weight = match entry >> 14 {
                            3 => "A",
                            2 => "B",
                            1 => "C",
                            _ => "",
                        };
                        format!("{}{weight}", entry & 0x3FFF)
                    })
                })
                .collect();
            let positions = positions?;
            if !positions.is_empty() {
                text.push(':');
                text.push_str(&positions.join(","));
            }
            lexemes.push(text);
        }
        Some(lexemes.join(" "))
    })
}

/// The types and the operators of the items of a `tsquery` value.
const TSQUERY_VALUE: u8 = 1;
const TSQUERY_OPERATOR: u8 = 2;
const TSQUERY_NOT: u8 = 1;
const TSQUERY_AND: u8 = 2;
const TSQUERY_OR: u8 = 3;
const TSQUERY_PHRASE: u8 = 4;

/// One item of a `tsquery` value, as the binary form sends it.
enum QueryItem<'a> {
    Value {
        word: &'a str,
        weight: u8,
        prefix: bool,
    },
    Operator {
        operator: u8,
        distance: i16,
    },
}

/// One part of a `tsquery` value that is already text. The operator of the
/// part, if it has one, decides whether its parent puts it in parentheses.
struct QueryPart {
    text: String,
    operator: Option<u8>,
}

/// The priority of an operator of a `tsquery` value. An operator of a
/// lower priority than its parent goes in parentheses.
fn query_priority(operator: u8) -> u8 {
    match operator {
        TSQUERY_NOT => 4,
        TSQUERY_PHRASE => 3,
        TSQUERY_AND => 2,
        _ => 1,
    }
}

/// Writes a `tsquery` value in the form that PostgreSQL writes, such as
/// `'fat' & ( 'rat' | !'cat' )`. The value holds the count of the items and
/// then the items in prefix order. An operator comes before its right
/// operand, and the right operand comes before the left operand.
///
/// The walk reads the items from the end, so each operand is text before
/// its operator needs it. A stack of parts keeps the depth of the query off
/// the call stack.
fn tsquery_text(bytes: &[u8]) -> JsonValue {
    read_with(bytes, |reader| {
        let count = reader.i32()?;
        let mut items = Vec::new();
        for _ in 0..count.max(0) {
            items.push(match reader.u8()? {
                TSQUERY_VALUE => {
                    let (weight, prefix) = (reader.u8()?, reader.u8()?);
                    QueryItem::Value {
                        word: reader.c_text()?,
                        weight,
                        prefix: prefix != 0,
                    }
                }
                TSQUERY_OPERATOR => {
                    let operator = reader.u8()?;
                    let distance = if operator == TSQUERY_PHRASE {
                        reader.i16()?
                    } else {
                        0
                    };
                    QueryItem::Operator { operator, distance }
                }
                _ => return None,
            });
        }
        let mut stack: Vec<QueryPart> = Vec::new();
        for item in items.iter().rev() {
            let part = match *item {
                QueryItem::Value {
                    word,
                    weight,
                    prefix,
                } => {
                    let mut text = lexeme_text(word);
                    if weight != 0 || prefix {
                        text.push(':');
                        if prefix {
                            text.push('*');
                        }
                        for (bit, letter) in [(8, 'A'), (4, 'B'), (2, 'C'), (1, 'D')] {
                            if weight & bit != 0 {
                                text.push(letter);
                            }
                        }
                    }
                    QueryPart {
                        text,
                        operator: None,
                    }
                }
                QueryItem::Operator {
                    operator: TSQUERY_NOT,
                    ..
                } => {
                    let operand = stack.pop()?;
                    QueryPart {
                        text: format!("!{}", query_operand(operand, TSQUERY_NOT, false)),
                        operator: Some(TSQUERY_NOT),
                    }
                }
                QueryItem::Operator { operator, distance } => {
                    let right = stack.pop()?;
                    let left = stack.pop()?;
                    let symbol = match operator {
                        TSQUERY_AND => "&".to_string(),
                        TSQUERY_OR => "|".to_string(),
                        TSQUERY_PHRASE if distance == 1 => "<->".to_string(),
                        TSQUERY_PHRASE => format!("<{distance}>"),
                        _ => return None,
                    };
                    QueryPart {
                        text: format!(
                            "{} {symbol} {}",
                            query_operand(left, operator, false),
                            query_operand(right, operator, true)
                        ),
                        operator: Some(operator),
                    }
                }
            };
            stack.push(part);
        }
        match (stack.pop(), stack.is_empty()) {
            (None, true) => Some(String::new()),
            (Some(part), true) => Some(part.text),
            _ => None,
        }
    })
}

/// Gives the text of an operand of a `tsquery` operator. An operand with an
/// operator of a lower priority goes in parentheses. A phrase as the right
/// operand of a phrase goes in parentheses too, because the order of a
/// phrase changes its meaning.
fn query_operand(part: QueryPart, parent: u8, right: bool) -> String {
    let wrap = part.operator.is_some_and(|operator| {
        query_priority(operator) < query_priority(parent)
            || (right && parent == TSQUERY_PHRASE && operator == TSQUERY_PHRASE)
    });
    if wrap {
        format!("( {} )", part.text)
    } else {
        part.text
    }
}

/// Writes an `hstore` value in the form that the extension writes, such as
/// `"a"=>"1", "b"=>NULL`. The value holds the count of the pairs. Each key
/// and each value has its length first, and a value of length minus one is
/// NULL.
fn hstore_text(bytes: &[u8]) -> JsonValue {
    read_with(bytes, |reader| {
        let count = reader.i32()?;
        let mut pairs = Vec::new();
        for _ in 0..count.max(0) {
            let key = reader.sized_text()?.map(hstore_quoted)?;
            let value = reader
                .sized_text()?
                .map_or_else(|| "NULL".to_string(), hstore_quoted);
            pairs.push(format!("{key}=>{value}"));
        }
        Some(pairs.join(", "))
    })
}

/// Writes a key or a value of an `hstore` between double quotes, with a
/// backslash before each double quote and each backslash.
fn hstore_quoted(text: &str) -> String {
    let mut quoted = String::from('"');
    for character in text.chars() {
        if character == '"' || character == '\\' {
            quoted.push('\\');
        }
        quoted.push(character);
    }
    quoted.push('"');
    quoted
}

/// Reads one value through the conversion of `tokio_postgres`. Bytes that
/// the target type refuses fall back on the text rule, so a value that the
/// server sent never shows as NULL.
fn scalar<'a, T: FromSql<'a>>(
    column_type: &Type,
    bytes: &'a [u8],
    to_json: impl FnOnce(T) -> JsonValue,
) -> JsonValue {
    match T::from_sql(column_type, bytes) {
        Ok(value) => to_json(value),
        Err(error) => {
            log::debug!("A column did not match the target type: {error}");
            text_or_bytes(bytes)
        }
    }
}

/// Gives the bytes as text when they are text, and as base64 when they are
/// not.
fn text_or_bytes(bytes: &[u8]) -> JsonValue {
    match std::str::from_utf8(bytes) {
        Ok(text) => JsonValue::String(text.to_string()),
        Err(_) => JsonValue::String(base64_text(bytes)),
    }
}

/// Writes a `bytea` value in the hex form that the server gives on the
/// simple protocol, such as `\x6869`.
fn bytea_text(bytes: &[u8]) -> JsonValue {
    let mut text = String::with_capacity(2 + bytes.len() * 2);
    text.push_str("\\x");
    hex_text(&mut text, bytes, false);
    JsonValue::String(text)
}

/// Writes bytes in base64, in the form that the grid shows for a blob.
fn base64_text(bytes: &[u8]) -> String {
    match bytes_to_json(bytes) {
        JsonValue::String(text) => text,
        other => other.to_string(),
    }
}

/// Reads an array of any element type. The value holds the count of the
/// dimensions, the type of the elements, the length of each dimension, and
/// then the elements in row order.
fn decode_array(element: &Type, bytes: &[u8], settings: &Settings) -> JsonValue {
    let mut reader = Reader::new(bytes);
    let Some(dimensions) = reader.i32() else {
        return text_or_bytes(bytes);
    };
    // The flag of the null values and the type of the elements are known
    // from the type of the column already.
    if reader.i32().is_none() || reader.u32().is_none() {
        return text_or_bytes(bytes);
    }
    if dimensions <= 0 {
        return JsonValue::Array(Vec::new());
    }
    let mut lengths = Vec::new();
    for _ in 0..dimensions {
        // The lower bound of a dimension does not reach the grid, which
        // shows the elements in their order.
        match (reader.i32(), reader.i32()) {
            (Some(length), Some(_)) if length >= 0 => lengths.push(length as usize),
            _ => return text_or_bytes(bytes),
        }
    }
    match nested_elements(&mut reader, element, &lengths, settings) {
        Some(value) => value,
        None => text_or_bytes(bytes),
    }
}

/// Builds the elements of one dimension of an array, and the dimensions
/// under it.
fn nested_elements(
    reader: &mut Reader<'_>,
    element: &Type,
    lengths: &[usize],
    settings: &Settings,
) -> Option<JsonValue> {
    let (length, rest) = lengths.split_first()?;
    let mut values = Vec::with_capacity(*length);
    for _ in 0..*length {
        if rest.is_empty() {
            values.push(reader.value(element, settings)?);
        } else {
            values.push(nested_elements(reader, element, rest, settings)?);
        }
    }
    Some(JsonValue::Array(values))
}

/// The flags of a range value.
const RANGE_EMPTY: u8 = 0x01;
const RANGE_LOWER_CLOSED: u8 = 0x02;
const RANGE_UPPER_CLOSED: u8 = 0x04;
const RANGE_LOWER_OPEN_END: u8 = 0x08;
const RANGE_UPPER_OPEN_END: u8 = 0x10;

/// Reads a range of any element type and writes it in the form that
/// PostgreSQL itself writes, such as `[1,10)`.
fn decode_range(element: &Type, bytes: &[u8], settings: &Settings) -> JsonValue {
    match range_text(element, &mut Reader::new(bytes), settings) {
        Some(text) => JsonValue::String(text),
        None => text_or_bytes(bytes),
    }
}

/// Reads one range out of the reader and writes it as text.
fn range_text(element: &Type, reader: &mut Reader<'_>, settings: &Settings) -> Option<String> {
    let flags = reader.u8()?;
    if flags & RANGE_EMPTY != 0 {
        return Some("empty".to_string());
    }
    let lower = if flags & RANGE_LOWER_OPEN_END != 0 {
        String::new()
    } else {
        render(&reader.value(element, settings)?, true)
    };
    let upper = if flags & RANGE_UPPER_OPEN_END != 0 {
        String::new()
    } else {
        render(&reader.value(element, settings)?, true)
    };
    let open = if flags & RANGE_LOWER_CLOSED != 0 {
        '['
    } else {
        '('
    };
    let close = if flags & RANGE_UPPER_CLOSED != 0 {
        ']'
    } else {
        ')'
    };
    Some(format!("{open}{lower},{upper}{close}"))
}

/// Reads a multirange, which holds a count and then the ranges.
fn decode_multirange(element: &Type, bytes: &[u8], settings: &Settings) -> JsonValue {
    let mut reader = Reader::new(bytes);
    let Some(count) = reader.i32() else {
        return text_or_bytes(bytes);
    };
    let mut parts = Vec::new();
    for _ in 0..count.max(0) {
        let Some(part) = reader.i32().and_then(|length| {
            let mut inner = Reader::new(reader.take(length.max(0) as usize)?);
            range_text(element, &mut inner, settings)
        }) else {
            return text_or_bytes(bytes);
        };
        parts.push(part);
    }
    JsonValue::String(format!("{{{}}}", parts.join(",")))
}

/// Reads a composite value and writes it in the form that PostgreSQL itself
/// writes, such as `(1,two)`.
fn decode_composite(fields: &[Field], bytes: &[u8], settings: &Settings) -> JsonValue {
    let mut reader = Reader::new(bytes);
    let Some(count) = reader.i32() else {
        return text_or_bytes(bytes);
    };
    if count < 0 || count as usize != fields.len() {
        return text_or_bytes(bytes);
    }
    let mut parts = Vec::with_capacity(fields.len());
    for field in fields {
        // The type of the field arrives with the value, and the type of the
        // column holds the same one.
        let Some(value) = reader
            .u32()
            .and_then(|_| reader.value(field.type_(), settings))
        else {
            return text_or_bytes(bytes);
        };
        parts.push(render(&value, false));
    }
    JsonValue::String(format!("({})", parts.join(",")))
}

/// Writes one JSON value as the text that a value inside a range, a
/// multirange, or a composite shows. The server puts a value in double
/// quotes when it is empty or contains a quote, a backslash, a parenthesis, a
/// comma or blank space, and a range also quotes a square bracket. Inside
/// the quotes each quote and each backslash is written twice. A NULL field
/// of a composite stays empty and without quotes.
fn render(value: &JsonValue, range: bool) -> String {
    let text = match value {
        JsonValue::Null => return String::new(),
        JsonValue::String(text) => text.clone(),
        other => other.to_string(),
    };
    let needs_quotes = text.is_empty()
        || text.chars().any(|c| {
            matches!(c, '"' | '\\' | '(' | ')' | ',')
                || c.is_whitespace()
                || (range && matches!(c, '[' | ']'))
        });
    if !needs_quotes {
        return text;
    }
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for c in text.chars() {
        if matches!(c, '"' | '\\') {
            quoted.push(c);
        }
        quoted.push(c);
    }
    quoted.push('"');
    quoted
}

/// Writes a money value as a number. The server sends the amount in the
/// smallest unit of the currency, and `digits` is the count of the digits
/// of the fraction that the `lc_monetary` setting of the session gives.
/// The symbol of the currency and the group separators of the text form
/// are not written.
fn money_text(bytes: &[u8], digits: u32) -> JsonValue {
    let Some(amount) = Reader::new(bytes).i64() else {
        return text_or_bytes(bytes);
    };
    let sign = if amount < 0 { "-" } else { "" };
    let units = amount.unsigned_abs();
    if digits == 0 {
        return JsonValue::String(format!("{sign}{units}"));
    }
    let scale = 10u64.pow(digits);
    let width = digits as usize;
    JsonValue::String(format!("{sign}{}.{:0width$}", units / scale, units % scale))
}

/// True when a value of the type holds a value of the `wanted` type, alone
/// or inside an array, a range, a domain or a composite.
fn holds(column_type: &Type, wanted: &Type) -> bool {
    match column_type.kind() {
        Kind::Array(inner) | Kind::Range(inner) | Kind::Multirange(inner) | Kind::Domain(inner) => {
            holds(inner, wanted)
        }
        Kind::Composite(fields) => fields.iter().any(|field| holds(field.type_(), wanted)),
        _ => column_type == wanted,
    }
}

/// The zone that a PostgreSQL name of a time zone gives. The name is a zone
/// of the IANA database, such as `Europe/Paris`, or a POSIX rule, such as
/// `<+05>-05`, which `SET TIME ZONE '+5'` gives.
fn zone_of(name: &str) -> TimeZone {
    TimeZone::get(name)
        .or_else(|_| TimeZone::posix(name))
        .unwrap_or_else(|error| {
            log::warn!("The time zone '{name}' is not known, so UTC applies: {error}");
            TimeZone::UTC
        })
}

/// Writes a `timestamptz` value in the zone of the session, in the form that
/// PostgreSQL writes under the ISO date style, such as
/// `2024-01-01 09:00:00.25-05`. A moment that the zone database cannot
/// place keeps the RFC 3339 form in UTC.
fn zoned_text(value: DateTime<Utc>, zone: &TimeZone) -> String {
    let Ok(moment) = jiff::Timestamp::from_microsecond(value.timestamp_micros()) else {
        return value.to_rfc3339();
    };
    let offset = zone.to_offset(moment).seconds();
    let local = value.naive_utc() + chrono::TimeDelta::seconds(i64::from(offset));
    date_time_text(local, &offset_text(offset))
}

/// Writes a date and a time in the form that PostgreSQL writes under the
/// ISO date style, such as `2024-01-01 09:00:00.25`. The `zone` text, such
/// as `-05`, follows the clock. The era of a date before the year 1 comes
/// last, as in `0044-03-15 12:00:00+00 BC`.
fn date_time_text(value: NaiveDateTime, zone: &str) -> String {
    let (date, before) = date_text(value.date());
    format!("{date} {}{zone}{}", time_of_day(value.time()), era(before))
}

/// Writes a date as `2024-01-31`, with four digits of the year at least.
/// PostgreSQL counts no year zero, so the year 0 of `chrono` is 1 BC and the
/// year -43 is 44 BC. Gives the text and true for a date before the year 1.
fn date_text(date: NaiveDate) -> (String, bool) {
    use chrono::Datelike;
    let year = date.year();
    let shown = if year > 0 { year } else { 1 - year };
    let text = format!("{shown:04}-{:02}-{:02}", date.month(), date.day());
    (text, year <= 0)
}

/// The text that PostgreSQL puts after a date before the year 1.
fn era(before: bool) -> &'static str {
    if before {
        " BC"
    } else {
        ""
    }
}

/// Writes a time of day as a clock. The fraction of a second has no zeros
/// at its end, as in PostgreSQL, so 250 milliseconds give `.25`.
fn time_of_day(time: NaiveTime) -> String {
    use chrono::Timelike;
    let micros = i64::from(time.num_seconds_from_midnight()) * 1_000_000
        + i64::from(time.nanosecond() / 1_000);
    clock_text(micros)
}

/// Gives `infinity` or `-infinity` for a date or a timestamp that holds one
/// of these values. PostgreSQL sends them as the largest and the smallest
/// value of the binary form, which no calendar type can read.
fn endless(value: Option<i64>, largest: i64, smallest: i64) -> Option<JsonValue> {
    match value? {
        value if value == largest => Some(JsonValue::String("infinity".into())),
        value if value == smallest => Some(JsonValue::String("-infinity".into())),
        _ => None,
    }
}

/// The sign words of the binary form of a NUMERIC value.
const NUMERIC_NEGATIVE: u16 = 0x4000;
const NUMERIC_NAN: u16 = 0xC000;
const NUMERIC_INFINITY: u16 = 0xD000;
const NUMERIC_NEGATIVE_INFINITY: u16 = 0xF000;

/// Writes a NUMERIC value in the form that PostgreSQL itself writes, with
/// every digit. The binary form holds digits of base ten thousand, the
/// weight of the first digit, the sign, and the count of the decimal digits
/// of the fraction. A reader of a fixed width, such as `rust_decimal`,
/// rounds a value of more than 28 digits and refuses NaN and Infinity.
fn numeric_text(bytes: &[u8]) -> JsonValue {
    let mut reader = Reader::new(bytes);
    let (Some(count), Some(weight), Some(sign), Some(scale)) =
        (reader.i16(), reader.i16(), reader.i16(), reader.i16())
    else {
        return text_or_bytes(bytes);
    };
    let digits: Option<Vec<i16>> = (0..count.max(0)).map(|_| reader.i16()).collect();
    let Some(digits) = digits else {
        return text_or_bytes(bytes);
    };
    let text = match sign as u16 {
        NUMERIC_NAN => "NaN".to_string(),
        NUMERIC_INFINITY => "Infinity".to_string(),
        NUMERIC_NEGATIVE_INFINITY => "-Infinity".to_string(),
        sign => {
            let weight = i32::from(weight);
            // The digit of base ten thousand at the given power.
            let digit = |power: i32| {
                usize::try_from(weight - power)
                    .ok()
                    .and_then(|index| digits.get(index))
                    .copied()
                    .unwrap_or(0)
            };
            let mut out = String::new();
            if sign == NUMERIC_NEGATIVE {
                out.push('-');
            }
            if weight < 0 {
                out.push('0');
            } else {
                out.push_str(&digit(weight).to_string());
                for power in (0..weight).rev() {
                    out.push_str(&format!("{:04}", digit(power)));
                }
            }
            let scale = usize::from(scale as u16);
            if scale > 0 {
                let mut fraction = String::new();
                let mut power = -1;
                while fraction.len() < scale {
                    fraction.push_str(&format!("{:04}", digit(power)));
                    power -= 1;
                }
                fraction.truncate(scale);
                out.push('.');
                out.push_str(&fraction);
            }
            out
        }
    };
    JsonValue::String(text)
}

/// Writes an interval in the form that PostgreSQL writes under the default
/// `postgres` interval style, such as `1 year 2 mons 3 days 04:05:06` or
/// `-1 days +02:00:00`. Only a count of exactly 1 takes the singular word,
/// so `-1` gives `-1 days`. A positive part that comes after a negative part
/// takes a plus sign, as in PostgreSQL.
fn interval_text(bytes: &[u8]) -> JsonValue {
    let mut reader = Reader::new(bytes);
    let (Some(micros), Some(days), Some(months)) = (reader.i64(), reader.i32(), reader.i32())
    else {
        return text_or_bytes(bytes);
    };
    // PostgreSQL 17 sends an infinite interval as the largest or the smallest
    // value of each part.
    if (micros, days, months) == (i64::MAX, i32::MAX, i32::MAX) {
        return JsonValue::String("infinity".into());
    }
    if (micros, days, months) == (i64::MIN, i32::MIN, i32::MIN) {
        return JsonValue::String("-infinity".into());
    }
    let mut text = String::new();
    // True when the last part in the text is negative.
    let mut after_negative = false;
    for (count, unit) in [(months / 12, "year"), (months % 12, "mon"), (days, "day")] {
        if count == 0 {
            continue;
        }
        if !text.is_empty() {
            text.push(' ');
        }
        let plus = if after_negative && count > 0 { "+" } else { "" };
        let plural = if count == 1 { "" } else { "s" };
        text.push_str(&format!("{plus}{count} {unit}{plural}"));
        after_negative = count < 0;
    }
    if micros != 0 || text.is_empty() {
        if !text.is_empty() {
            text.push(' ');
        }
        if after_negative && micros > 0 {
            text.push('+');
        }
        text.push_str(&clock_text(micros));
    }
    JsonValue::String(text)
}

/// Writes a count of microseconds as a clock, with the fraction only when
/// the count holds one.
fn clock_text(micros: i64) -> String {
    let sign = if micros < 0 { "-" } else { "" };
    let total = micros.unsigned_abs();
    let seconds = total / 1_000_000;
    let fraction = total % 1_000_000;
    let clock = format!(
        "{sign}{:02}:{:02}:{:02}",
        seconds / 3600,
        (seconds / 60) % 60,
        seconds % 60
    );
    if fraction == 0 {
        clock
    } else {
        format!("{clock}.{:06}", fraction)
            .trim_end_matches('0')
            .into()
    }
}

/// Writes an address of a network. The value holds the family, the count of
/// the bits of the mask, a flag for a network, and the bytes of the address.
fn inet_text(bytes: &[u8]) -> JsonValue {
    let mut reader = Reader::new(bytes);
    let (Some(_family), Some(mask_bits), Some(is_network), Some(length)) =
        (reader.u8(), reader.u8(), reader.u8(), reader.u8())
    else {
        return text_or_bytes(bytes);
    };
    let Some(address) = reader.take(length as usize) else {
        return text_or_bytes(bytes);
    };
    // The count of the bytes names the family, so a server that numbers the
    // families in its own way still reads.
    let text = match address.len() {
        4 => {
            let mut octets = [0u8; 4];
            octets.copy_from_slice(address);
            Ipv4Addr::from(octets).to_string()
        }
        16 => {
            let mut octets = [0u8; 16];
            octets.copy_from_slice(address);
            Ipv6Addr::from(octets).to_string()
        }
        _ => return text_or_bytes(bytes),
    };
    let full_mask = (address.len() * 8) as u8;
    if is_network == 1 || mask_bits != full_mask {
        return JsonValue::String(format!("{text}/{mask_bits}"));
    }
    JsonValue::String(text)
}

/// Writes the bytes of a hardware address, in groups of one byte.
fn mac_text(bytes: &[u8], length: usize) -> JsonValue {
    if bytes.len() != length {
        return text_or_bytes(bytes);
    }
    let groups: Vec<String> = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    JsonValue::String(groups.join(":"))
}

/// A walk over the bytes of one value. A read that runs past the end gives
/// nothing, so a value of a form the reader does not expect falls back on
/// the text rule instead of stopping the run.
struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Reader { bytes }
    }

    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        if self.bytes.len() < count {
            return None;
        }
        let (head, rest) = self.bytes.split_at(count);
        self.bytes = rest;
        Some(head)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|bytes| bytes[0])
    }

    fn i16(&mut self) -> Option<i16> {
        self.take(2)
            .map(|bytes| i16::from_be_bytes(bytes.try_into().unwrap()))
    }

    fn i32(&mut self) -> Option<i32> {
        self.take(4)
            .map(|bytes| i32::from_be_bytes(bytes.try_into().unwrap()))
    }

    fn u32(&mut self) -> Option<u32> {
        self.i32().map(|value| value as u32)
    }

    fn u16(&mut self) -> Option<u16> {
        self.i16().map(|value| value as u16)
    }

    fn u64(&mut self) -> Option<u64> {
        self.i64().map(|value| value as u64)
    }

    fn f64(&mut self) -> Option<f64> {
        self.u64().map(f64::from_bits)
    }

    /// True when the reader used every byte.
    fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Reads text that ends with a zero byte, and the zero byte.
    fn c_text(&mut self) -> Option<&'a str> {
        let end = self.bytes.iter().position(|&byte| byte == 0)?;
        let text = std::str::from_utf8(self.take(end)?).ok()?;
        self.take(1)?;
        Some(text)
    }

    /// Reads text that has its length first. A length of minus one gives
    /// `Some(None)`, which is NULL.
    fn sized_text(&mut self) -> Option<Option<&'a str>> {
        let length = self.i32()?;
        if length < 0 {
            return Some(None);
        }
        let bytes = self.take(length as usize)?;
        std::str::from_utf8(bytes).ok().map(Some)
    }

    fn i64(&mut self) -> Option<i64> {
        self.take(8)
            .map(|bytes| i64::from_be_bytes(bytes.try_into().unwrap()))
    }

    /// Reads one value that carries its own length, as the elements of an
    /// array and the bounds of a range do. A length of minus one means a
    /// value that is null.
    fn value(&mut self, column_type: &Type, settings: &Settings) -> Option<JsonValue> {
        let length = self.i32()?;
        if length < 0 {
            return Some(JsonValue::Null);
        }
        let bytes = self.take(length as usize)?;
        Some(decode_value(column_type, bytes, settings))
    }
}

#[cfg(test)]
mod live;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::sink::BufferSink;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
    use tokio::sync::Notify;

    /// Wraps a body of a message with its type and its length. The length
    /// counts itself and the body, and never the byte of the type.
    fn message(message_type: u8, body: &[u8]) -> Vec<u8> {
        let mut out = vec![message_type];
        out.extend_from_slice(&((body.len() + 4) as i32).to_be_bytes());
        out.extend_from_slice(body);
        out
    }

    /// The answer of a server that asks for no password.
    fn authentication_ok() -> Vec<u8> {
        message(b'R', &0i32.to_be_bytes())
    }

    /// The message that says the server waits for a statement.
    fn ready_for_query() -> Vec<u8> {
        message(b'Z', b"I")
    }

    /// Names the columns of a result set. Each column takes the type of a
    /// text value.
    fn row_description(names: &[&str]) -> Vec<u8> {
        let columns: Vec<(&str, u32)> = names.iter().map(|name| (*name, 25)).collect();
        simple_row_description(&columns)
    }

    /// Names the columns of a result set of the simple protocol, with the
    /// type OID of each column. The values come in their text form.
    fn simple_row_description(columns: &[(&str, u32)]) -> Vec<u8> {
        let mut body = (columns.len() as i16).to_be_bytes().to_vec();
        for (index, (name, oid)) in columns.iter().enumerate() {
            body.extend_from_slice(name.as_bytes());
            body.push(0);
            body.extend_from_slice(&0i32.to_be_bytes());
            body.extend_from_slice(&(index as i16 + 1).to_be_bytes());
            body.extend_from_slice(&oid.to_be_bytes());
            body.extend_from_slice(&(-1i16).to_be_bytes());
            body.extend_from_slice(&(-1i32).to_be_bytes());
            body.extend_from_slice(&0i16.to_be_bytes());
        }
        message(b'T', &body)
    }

    /// One row of a result set. A column without a value carries the length
    /// of minus one.
    fn data_row(values: &[Option<&str>]) -> Vec<u8> {
        let mut body = (values.len() as i16).to_be_bytes().to_vec();
        for value in values {
            match value {
                Some(text) => {
                    body.extend_from_slice(&(text.len() as i32).to_be_bytes());
                    body.extend_from_slice(text.as_bytes());
                }
                None => body.extend_from_slice(&(-1i32).to_be_bytes()),
            }
        }
        message(b'D', &body)
    }

    /// An error of the server, with the state that names its cause.
    fn error_response(state: &str, text: &str) -> Vec<u8> {
        let mut body = Vec::new();
        for (field, value) in [(b'S', "ERROR"), (b'C', state), (b'M', text)] {
            body.push(field);
            body.extend_from_slice(value.as_bytes());
            body.push(0);
        }
        body.push(0);
        message(b'E', &body)
    }

    /// The tag that ends one statement of the script.
    fn command_complete(tag: &str) -> Vec<u8> {
        let mut body = tag.as_bytes().to_vec();
        body.push(0);
        message(b'C', &body)
    }

    /// Answers the startup message of a client that connects.
    async fn accept_startup(server: &mut DuplexStream) {
        // The startup message carries a length and no byte of a type.
        let mut length = [0u8; 4];
        server.read_exact(&mut length).await.unwrap();
        let rest = i32::from_be_bytes(length) as usize - 4;
        let mut body = vec![0u8; rest];
        server.read_exact(&mut body).await.unwrap();
        server.write_all(&authentication_ok()).await.unwrap();
        server.write_all(&ready_for_query()).await.unwrap();
    }

    /// Reads one simple query of the client and gives its text back.
    async fn read_query(server: &mut DuplexStream) -> String {
        let mut message_type = [0u8; 1];
        server.read_exact(&mut message_type).await.unwrap();
        assert_eq!(message_type[0], b'Q');
        let mut length = [0u8; 4];
        server.read_exact(&mut length).await.unwrap();
        let mut body = vec![0u8; i32::from_be_bytes(length) as usize - 4];
        server.read_exact(&mut body).await.unwrap();
        body.pop();
        String::from_utf8(body).unwrap()
    }

    /// Names the columns of a result set of the extended protocol, with the
    /// type of each column and the binary form of the values.
    fn typed_row_description(columns: &[(&str, u32)]) -> Vec<u8> {
        let mut body = (columns.len() as i16).to_be_bytes().to_vec();
        for (index, (name, oid)) in columns.iter().enumerate() {
            body.extend_from_slice(name.as_bytes());
            body.push(0);
            body.extend_from_slice(&0i32.to_be_bytes());
            body.extend_from_slice(&(index as i16 + 1).to_be_bytes());
            body.extend_from_slice(&oid.to_be_bytes());
            body.extend_from_slice(&(-1i16).to_be_bytes());
            body.extend_from_slice(&(-1i32).to_be_bytes());
            body.extend_from_slice(&1i16.to_be_bytes());
        }
        message(b'T', &body)
    }

    /// One row of the extended protocol. Each value carries its length and
    /// its bytes, and a value that is null carries the length of minus one.
    fn binary_data_row(values: &[Option<&[u8]>]) -> Vec<u8> {
        let mut body = (values.len() as i16).to_be_bytes().to_vec();
        for value in values {
            body.extend_from_slice(&element_body(*value));
        }
        message(b'D', &body)
    }

    /// The answer to a statement that the client prepares. The types of the
    /// parameters go back as the client asked for them.
    fn prepared(columns: Option<&[(&str, u32)]>) -> Vec<u8> {
        let mut out = message(b'1', &[]);
        let mut body = 1i16.to_be_bytes().to_vec();
        body.extend_from_slice(&20u32.to_be_bytes());
        out.extend_from_slice(&message(b't', &body));
        match columns {
            Some(columns) => out.extend_from_slice(&typed_row_description(columns)),
            None => out.extend_from_slice(&message(b'n', &[])),
        }
        out.extend_from_slice(&ready_for_query());
        out
    }

    /// Reads the probe that asks whether the session is outside a
    /// transaction block, and answers it.
    async fn answer_probe(server: &mut DuplexStream, outside: bool) {
        assert_eq!(read_query(server).await, OUTSIDE_A_BLOCK);
        let mut answer = row_description(&["?column?"]);
        answer.extend_from_slice(&data_row(&[Some(if outside { "t" } else { "f" })]));
        answer.extend_from_slice(&command_complete("SELECT 1"));
        answer.extend_from_slice(&ready_for_query());
        server.write_all(&answer).await.unwrap();
    }

    /// Answers the prepare of a statement that only reads with the columns
    /// given, then the probe that comes in the same round trip, and then
    /// the close of the prepared statement.
    async fn answer_described(server: &mut DuplexStream, columns: &[(&str, u32)], outside: bool) {
        read_until_sync(server).await;
        let mut answer = message(b'1', &[]);
        answer.extend_from_slice(&message(b't', &0i16.to_be_bytes()));
        answer.extend_from_slice(&typed_row_description(columns));
        answer.extend_from_slice(&ready_for_query());
        server.write_all(&answer).await.unwrap();
        answer_probe(server, outside).await;
        read_until_sync(server).await;
        let mut closed = message(b'3', &[]);
        closed.extend_from_slice(&ready_for_query());
        server.write_all(&closed).await.unwrap();
    }

    /// Answers the prepare of a statement with an error, and then the probe
    /// that comes in the same round trip.
    async fn refuse_described(server: &mut DuplexStream, outside: bool) {
        read_until_sync(server).await;
        let mut answer = error_response("42P01", "relation \"t\" does not exist");
        answer.extend_from_slice(&ready_for_query());
        server.write_all(&answer).await.unwrap();
        answer_probe(server, outside).await;
    }

    /// Reads the messages of the client up to the one that asks the server
    /// to answer.
    async fn read_until_sync(server: &mut DuplexStream) {
        loop {
            let mut message_type = [0u8; 1];
            server.read_exact(&mut message_type).await.unwrap();
            let mut length = [0u8; 4];
            server.read_exact(&mut length).await.unwrap();
            let mut body = vec![0u8; i32::from_be_bytes(length) as usize - 4];
            server.read_exact(&mut body).await.unwrap();
            if message_type[0] == b'S' {
                return;
            }
        }
    }

    /// One parameter of a statement.
    fn one_param() -> QueryParams {
        vec![crate::db::QueryParam {
            value: JsonValue::from(1),
        }]
    }

    /// Builds a driver that speaks to a fake server on a pipe.
    /// A stop that counts its calls. The test server waits on the same
    /// signal, so it can answer the cancel the way a server answers it.
    struct TestStop {
        calls: Arc<AtomicUsize>,
        signal: Arc<Notify>,
        works: bool,
    }

    #[async_trait]
    impl CancelHandle for TestStop {
        async fn cancel(&self) -> Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.signal.notify_one();
            if self.works {
                Ok(())
            } else {
                Err(Error::Connection("no socket for the stop".into()))
            }
        }
    }

    fn test_stop(works: bool) -> (Arc<TestStop>, Arc<AtomicUsize>, Arc<Notify>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let signal = Arc::new(Notify::new());
        let stop = Arc::new(TestStop {
            calls: calls.clone(),
            signal: signal.clone(),
            works,
        });
        (stop, calls, signal)
    }

    async fn driver_on(client_end: DuplexStream) -> PostgresDriver {
        driver_with_stop(client_end, test_stop(false).0).await
    }

    async fn driver_with_stop(client_end: DuplexStream, stop: Arc<TestStop>) -> PostgresDriver {
        let mut config = PgConfig::new();
        config.user("tester").dbname("test");
        let (client, connection) = config
            .connect_raw(client_end, tokio_postgres::NoTls)
            .await
            .unwrap();
        PostgresDriver {
            client,
            notices: drive_connection(connection),
            stop,
        }
    }

    /// Reads one statement of the simple protocol and answers it with the
    /// messages given, and then with the end of the exchange.
    async fn answer_query(server: &mut DuplexStream, expected: &str, messages: &[Vec<u8>]) {
        assert_eq!(read_query(server).await, expected);
        let mut answer: Vec<u8> = messages.concat();
        answer.extend_from_slice(&ready_for_query());
        server.write_all(&answer).await.unwrap();
    }

    /// Reads the text that opens a read through a cursor, checks the
    /// statement and the count of the first fetch, and gives the name of
    /// the cursor back.
    async fn read_cursor_open(server: &mut DuplexStream, statement: &str, asked: usize) -> String {
        let text = read_query(server).await;
        let rest = text.strip_prefix("BEGIN; ").unwrap();
        cursor_name(rest, statement, asked)
    }

    /// Reads the text that opens a read through a cursor inside a block of
    /// the user, which has no `BEGIN`, and gives the name of the cursor.
    async fn read_block_cursor_open(
        server: &mut DuplexStream,
        statement: &str,
        asked: usize,
    ) -> String {
        let text = read_query(server).await;
        cursor_name(&text, statement, asked)
    }

    /// Checks a text that declares a cursor and fetches from it, and gives
    /// the name of the cursor.
    fn cursor_name(text: &str, statement: &str, asked: usize) -> String {
        let rest = text.strip_prefix("DECLARE ").unwrap();
        let (name, rest) = rest.split_once(" NO SCROLL CURSOR FOR ").unwrap();
        assert_eq!(rest, format!("{statement}\n; FETCH {asked} FROM {name}"));
        name.to_string()
    }

    /// The answer of the server to a fetch, with one row for each list of
    /// values. The tag counts the rows.
    fn fetched(columns: &[&str], rows: &[&[Option<&str>]]) -> Vec<u8> {
        let mut out = row_description(columns);
        for row in rows {
            out.extend_from_slice(&data_row(row));
        }
        out.extend_from_slice(&command_complete(&format!("FETCH {}", rows.len())));
        out
    }

    /// The tags that open the transaction and declare the cursor.
    fn opened() -> Vec<u8> {
        let mut out = command_complete("BEGIN");
        out.extend_from_slice(&command_complete("DECLARE CURSOR"));
        out
    }

    /// Answers a read through a cursor whose first fetch gives all its rows,
    /// and then the `COMMIT` that ends it.
    async fn answer_cursor(
        server: &mut DuplexStream,
        statement: &str,
        asked: usize,
        columns: &[&str],
        rows: &[&[Option<&str>]],
    ) {
        read_cursor_open(server, statement, asked).await;
        let mut answer = opened();
        answer.extend_from_slice(&fetched(columns, rows));
        answer.extend_from_slice(&ready_for_query());
        server.write_all(&answer).await.unwrap();
        answer_query(server, "COMMIT", &[command_complete("COMMIT")]).await;
    }

    /// Answers a read through a portal: the `BEGIN`, the bind, one execute
    /// for each answer given, the close of the portal and the `COMMIT`.
    async fn answer_portal(server: &mut DuplexStream, executes: &[Vec<u8>]) {
        answer_query(server, "START TRANSACTION", &[command_complete("BEGIN")]).await;
        read_until_sync(server).await;
        let mut bound = message(b'2', &[]);
        bound.extend_from_slice(&ready_for_query());
        server.write_all(&bound).await.unwrap();
        for answer in executes {
            read_until_sync(server).await;
            let mut answer = answer.clone();
            answer.extend_from_slice(&ready_for_query());
            server.write_all(&answer).await.unwrap();
        }
        read_until_sync(server).await;
        let mut closed = message(b'3', &[]);
        closed.extend_from_slice(&ready_for_query());
        server.write_all(&closed).await.unwrap();
        answer_query(server, "COMMIT", &[command_complete("COMMIT")]).await;
    }

    fn no_limit() -> ExecOptions {
        ExecOptions {
            max_rows: 100,
            timeout_secs: 30,
            one_statement: false,
        }
    }

    #[tokio::test]
    async fn a_script_of_several_statements_sends_each_one_alone() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_described(&mut server, &[("id", 23)], true).await;
            answer_cursor(
                &mut server,
                "SELECT 1",
                101,
                &["id", "name"],
                &[&[Some("1"), Some("Ada")], &[Some("2"), None]],
            )
            .await;
            // A statement that writes gets no probe.
            answer_query(
                &mut server,
                "UPDATE t SET a = 1",
                &[command_complete("UPDATE 3")],
            )
            .await;
            answer_described(&mut server, &[("id", 23)], true).await;
            answer_cursor(&mut server, "SELECT 7", 101, &["n"], &[&[Some("7")]]).await;
        });

        let mut driver = driver_on(client_end).await;
        let options = no_limit();
        let mut sink = BufferSink::new(options.max_rows);
        let rows_affected = driver
            .stream_simple(
                "SELECT 1; UPDATE t SET a = 1; SELECT 7",
                &options,
                &mut sink,
            )
            .await
            .unwrap();
        let response = sink.into_response(RunSummary::default());

        assert_eq!(rows_affected, Some(3));
        assert_eq!(response.results.len(), 2);
        assert_eq!(response.results[0].rows.len(), 2);
        assert_eq!(response.results[0].rows[1][1], JsonValue::Null);
        assert_eq!(response.results[1].rows.len(), 1);
        assert!(!response.results[0].truncated);

        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_lock_limit_ends_a_later_wait_for_a_lock_with_a_lock_error() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_query(
                &mut server,
                "SET lock_timeout = '5000ms'",
                &[command_complete("SET")],
            )
            .await;
            answer_query(
                &mut server,
                "SELECT 1",
                &[error_response(
                    "55P03",
                    "canceling statement due to lock timeout",
                )],
            )
            .await;
        });

        let mut driver = driver_on(client_end).await;
        driver
            .limit_lock_waits(Duration::from_secs(5))
            .await
            .unwrap();
        let error = driver.ping().await.unwrap_err();

        assert!(error.is_lock_wait());
        task.await.unwrap();
    }

    #[tokio::test]
    async fn one_statement_that_gives_two_sets_keeps_both() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_described(&mut server, &[("id", 23)], true).await;
            answer_query(
                &mut server,
                "SHOW ALL",
                &[
                    row_description(&["a"]),
                    data_row(&[Some("1")]),
                    row_description(&["b"]),
                    data_row(&[Some("2")]),
                ],
            )
            .await;
        });

        let mut driver = driver_on(client_end).await;
        let options = no_limit();
        let mut sink = BufferSink::new(options.max_rows);
        driver
            .stream_simple("SHOW ALL", &options, &mut sink)
            .await
            .unwrap();
        let response = sink.into_response(RunSummary::default());

        // A set with no end tag closes at the next set and at the end.
        assert_eq!(response.results.len(), 2);

        task.await.unwrap();
    }

    #[tokio::test]
    async fn the_first_error_of_a_script_ends_it() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_query(
                &mut server,
                "DELETE FROM a",
                &[command_complete("DELETE 2")],
            )
            .await;
            answer_query(
                &mut server,
                "DELETE FROM b",
                &[error_response("42P01", "relation \"b\" does not exist")],
            )
            .await;
            // The client sends no third statement.
            let mut rest = Vec::new();
            server.read_to_end(&mut rest).await.unwrap();
            assert!(!rest.contains(&b'Q'));
        });

        let mut driver = driver_on(client_end).await;
        let options = no_limit();
        let mut sink = BufferSink::new(options.max_rows);
        let outcome = driver
            .stream_simple(
                "DELETE FROM a; DELETE FROM b; DELETE FROM c",
                &options,
                &mut sink,
            )
            .await;

        assert!(outcome.is_err());
        drop(driver);
        task.await.unwrap();
    }

    /// A notice of the server, with the text it gives.
    fn notice_response(text: &str) -> Vec<u8> {
        let mut body = Vec::new();
        for (field, value) in [(b'S', "NOTICE"), (b'C', "00000"), (b'M', text)] {
            body.push(field);
            body.extend_from_slice(value.as_bytes());
            body.push(0);
        }
        body.push(0);
        message(b'N', &body)
    }

    /// An error of the server that names the character where it happened.
    fn positioned_error(text: &str, position: u32) -> Vec<u8> {
        let mut body = Vec::new();
        let position = position.to_string();
        for (field, value) in [
            (b'S', "ERROR"),
            (b'C', "42703"),
            (b'M', text),
            (b'P', position.as_str()),
        ] {
            body.push(field);
            body.extend_from_slice(value.as_bytes());
            body.push(0);
        }
        body.push(0);
        message(b'E', &body)
    }

    #[tokio::test]
    async fn the_notices_of_each_statement_and_of_a_failed_one_reach_the_sink() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_query(
                &mut server,
                "DELETE FROM a",
                &[notice_response("first"), command_complete("DELETE 2")],
            )
            .await;
            answer_query(
                &mut server,
                "DELETE FROM b\nWHERE nope = 1",
                &[
                    notice_response("second"),
                    positioned_error("column \"nope\" does not exist", 21),
                ],
            )
            .await;
        });

        let mut driver = driver_on(client_end).await;
        let options = no_limit();
        let mut sink = BufferSink::new(options.max_rows);
        let error = driver
            .execute_stream(
                "DELETE FROM a;\n  DELETE FROM b\nWHERE nope = 1",
                None,
                &options,
                &mut sink,
            )
            .await
            .unwrap_err();

        let texts: Vec<String> = sink
            .into_response(RunSummary {
                rows_affected: None,
                elapsed_ms: 0,
                stats: None,
            })
            .messages
            .into_iter()
            .map(|message| message.text)
            .collect();
        assert_eq!(texts, ["2 rows affected.", "first", "second"]);
        let payload = error.to_payload();
        assert_eq!((payload.line, payload.column), (Some(3), Some(7)));
        drop(driver);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_statement_whose_tag_counts_no_rows_reports_its_tag() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_query(
                &mut server,
                "CREATE TABLE t (a int)",
                &[command_complete("CREATE TABLE")],
            )
            .await;
            answer_query(
                &mut server,
                "INSERT INTO t VALUES (1)",
                &[command_complete("INSERT 0 1")],
            )
            .await;
        });

        let mut driver = driver_on(client_end).await;
        let options = no_limit();
        let mut sink = BufferSink::new(options.max_rows);
        let rows_affected = driver
            .stream_simple(
                "CREATE TABLE t (a int); INSERT INTO t VALUES (1); -- nothing",
                &options,
                &mut sink,
            )
            .await
            .unwrap();

        assert_eq!(rows_affected, Some(1));
        let texts: Vec<String> = sink
            .into_response(RunSummary {
                rows_affected: None,
                elapsed_ms: 0,
                stats: None,
            })
            .messages
            .into_iter()
            .map(|message| message.text)
            .collect();
        assert_eq!(texts, ["CREATE TABLE", "1 row affected."]);
        drop(driver);
        task.await.unwrap();
    }

    #[test]
    fn only_the_tags_of_statements_that_touch_rows_count_rows() {
        for tag in [
            "INSERT 0 2",
            "UPDATE 1",
            "DELETE 0",
            "MERGE 3",
            "SELECT 4",
            "COPY 5",
            "FETCH 6",
            "MOVE 7",
        ] {
            assert!(tag_counts_rows(tag), "{tag}");
        }
        for tag in ["CREATE TABLE", "SET", "VACUUM", "BEGIN", ""] {
            assert!(!tag_counts_rows(tag), "{tag}");
        }
    }

    #[test]
    fn an_error_without_a_position_marks_the_start_of_its_statement() {
        let error = tokio_postgres::Error::__private_api_timeout();
        let payload = locate_error(error, "SELECT 1;\n  SELECT 2", "SELECT 2", 12, 0).to_payload();
        assert_eq!((payload.line, payload.column), (Some(2), Some(3)));
    }

    #[tokio::test]
    async fn a_locking_read_inside_a_block_keeps_the_walk_past_the_row_limit() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_described(&mut server, &[("id", 23)], false).await;
            // A cursor would lock only the rows it fetches, so the statement
            // runs whole and locks every row.
            answer_query(
                &mut server,
                "SELECT id FROM t FOR UPDATE",
                &[
                    row_description(&["id"]),
                    data_row(&[Some("1")]),
                    data_row(&[Some("2")]),
                    data_row(&[Some("3")]),
                    command_complete("SELECT 3"),
                ],
            )
            .await;
            answer_query(
                &mut server,
                "DELETE FROM t",
                &[command_complete("DELETE 5")],
            )
            .await;
        });

        let (stop, calls, _signal) = test_stop(true);
        let mut driver = driver_with_stop(client_end, stop).await;
        let options = ExecOptions {
            max_rows: 1,
            ..no_limit()
        };
        let mut sink = BufferSink::new(options.max_rows);
        let rows_affected = driver
            .stream_simple(
                "SELECT id FROM t FOR UPDATE; DELETE FROM t",
                &options,
                &mut sink,
            )
            .await
            .unwrap();
        let response = sink.into_response(RunSummary::default());

        // The session is inside a transaction block, so no cancel goes out.
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].rows.len(), 1);
        assert!(response.results[0].truncated);
        assert!(response
            .messages
            .iter()
            .any(|message| message.text.contains("Stopped at the row limit")));
        assert_eq!(rows_affected, Some(5));

        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_read_of_a_longer_script_stops_the_server_at_the_row_limit() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let (stop, calls, signal) = test_stop(true);
        let waiter = signal.clone();
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_described(&mut server, &[("id", 23)], true).await;
            assert_eq!(read_query(&mut server).await, "SHOW ALL");
            let mut answer = row_description(&["id"]);
            answer.extend_from_slice(&data_row(&[Some("1")]));
            answer.extend_from_slice(&data_row(&[Some("2")]));
            server.write_all(&answer).await.unwrap();
            waiter.notified().await;
            let mut end = error_response("57014", "canceling statement due to user request");
            end.extend_from_slice(&ready_for_query());
            server.write_all(&end).await.unwrap();

            answer_query(
                &mut server,
                "DELETE FROM t",
                &[command_complete("DELETE 5")],
            )
            .await;
        });

        let mut driver = driver_with_stop(client_end, stop).await;
        let options = ExecOptions {
            max_rows: 1,
            ..no_limit()
        };
        let mut sink = BufferSink::new(options.max_rows);
        let rows_affected = driver
            .stream_simple("SHOW ALL; DELETE FROM t", &options, &mut sink)
            .await
            .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(rows_affected, Some(5));

        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_sink_that_takes_no_more_rows_ends_the_script() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_described(&mut server, &[("id", 23)], false).await;
            let name = read_block_cursor_open(&mut server, "SELECT id FROM t", 101).await;
            let mut answer = command_complete("DECLARE CURSOR");
            answer.extend_from_slice(&fetched(
                &["id"],
                &[&[Some("1")], &[Some("2")], &[Some("3")]],
            ));
            answer.extend_from_slice(&ready_for_query());
            server.write_all(&answer).await.unwrap();
            // The stop of the sink ends the read inside the block of the
            // user, so the cursor closes and no COMMIT follows.
            answer_query(
                &mut server,
                &format!("CLOSE {name}"),
                &[command_complete("CLOSE CURSOR")],
            )
            .await;
            let mut rest = Vec::new();
            server.read_to_end(&mut rest).await.unwrap();
            assert!(!rest.contains(&b'Q'));
        });

        let mut driver = driver_on(client_end).await;
        let options = no_limit();
        // The sink stops before the row limit of the run.
        let mut sink = BufferSink::new(1);
        driver
            .stream_simple("SELECT id FROM t; DELETE FROM t", &options, &mut sink)
            .await
            .unwrap();
        let response = sink.into_response(RunSummary::default());

        assert!(response.results[0].truncated);
        drop(driver);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_script_of_one_statement_stops_the_server_at_the_row_limit() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let (stop, calls, signal) = test_stop(true);
        let waiter = signal.clone();
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_described(&mut server, &[("id", 23)], true).await;
            assert_eq!(read_query(&mut server).await, "SHOW ALL");

            let mut answer = row_description(&["id"]);
            for value in ["1", "2", "3"] {
                answer.extend_from_slice(&data_row(&[Some(value)]));
            }
            server.write_all(&answer).await.unwrap();

            // The server answers the cancel the way PostgreSQL answers it.
            waiter.notified().await;
            let mut end = error_response("57014", "canceling statement due to user request");
            end.extend_from_slice(&ready_for_query());
            server.write_all(&end).await.unwrap();
        });

        let mut driver = driver_with_stop(client_end, stop).await;
        let options = ExecOptions {
            max_rows: 1,
            timeout_secs: 30,
            one_statement: false,
        };
        let mut sink = BufferSink::new(options.max_rows);
        driver
            .stream_simple("SHOW ALL", &options, &mut sink)
            .await
            .unwrap();
        let response = sink.into_response(RunSummary::default());

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].rows.len(), 1);
        assert!(response.results[0].truncated);

        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_run_of_one_statement_prepares_the_text_before_it_runs_it() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            read_until_sync(&mut server).await;
            let mut answer = message(b'1', &[]);
            answer.extend_from_slice(&message(b't', &0i16.to_be_bytes()));
            answer.extend_from_slice(&typed_row_description(&[("id", 25)]));
            answer.extend_from_slice(&ready_for_query());
            server.write_all(&answer).await.unwrap();

            // The client closes the prepared statement at once.
            read_until_sync(&mut server).await;
            let mut closed = message(b'3', &[]);
            closed.extend_from_slice(&ready_for_query());
            server.write_all(&closed).await.unwrap();

            answer_probe(&mut server, true).await;
            answer_cursor(
                &mut server,
                "SELECT id FROM t",
                101,
                &["id"],
                &[&[Some("1")]],
            )
            .await;
        });

        let mut driver = driver_on(client_end).await;
        let options = ExecOptions {
            max_rows: 100,
            timeout_secs: 30,
            one_statement: true,
        };
        let mut sink = BufferSink::new(options.max_rows);
        driver
            .stream_simple("SELECT id FROM t", &options, &mut sink)
            .await
            .unwrap();
        let response = sink.into_response(RunSummary::default());

        // The rows keep the text form of the simple protocol.
        assert_eq!(
            response.results[0].rows[0][0],
            JsonValue::String("1".into())
        );

        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_run_of_one_statement_refuses_a_text_that_the_server_cannot_prepare() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            read_until_sync(&mut server).await;
            let mut answer = error_response(
                "42601",
                "cannot insert multiple commands into a prepared statement",
            );
            answer.extend_from_slice(&ready_for_query());
            server.write_all(&answer).await.unwrap();

            // The client sends nothing more, so the read finds the end of
            // the pipe.
            let mut rest = Vec::new();
            server.read_to_end(&mut rest).await.unwrap();
            assert!(!rest.contains(&b'Q'));
        });

        let mut driver = driver_on(client_end).await;
        let options = ExecOptions {
            max_rows: 100,
            timeout_secs: 30,
            one_statement: true,
        };
        let mut sink = BufferSink::new(options.max_rows);
        let error = driver
            .stream_simple("SELECT 1; DELETE FROM t", &options, &mut sink)
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            Error::Located { ref inner, line: 1, column: 1 }
                if matches!(**inner, Error::Postgres(ref error)
                    if error.code() == Some(&SqlState::SYNTAX_ERROR))
        ));
        drop(driver);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_stop_that_does_not_reach_the_server_keeps_the_walk() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let (stop, calls, _signal) = test_stop(false);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_described(&mut server, &[("id", 23)], true).await;
            assert_eq!(read_query(&mut server).await, "SHOW ALL");

            let mut answer = row_description(&["id"]);
            for value in ["1", "2", "3"] {
                answer.extend_from_slice(&data_row(&[Some(value)]));
            }
            answer.extend_from_slice(&command_complete("SELECT 3"));
            answer.extend_from_slice(&ready_for_query());
            server.write_all(&answer).await.unwrap();
        });

        let mut driver = driver_with_stop(client_end, stop).await;
        let options = ExecOptions {
            max_rows: 1,
            timeout_secs: 30,
            one_statement: false,
        };
        let mut sink = BufferSink::new(options.max_rows);
        driver
            .stream_simple("SHOW ALL", &options, &mut sink)
            .await
            .unwrap();
        let response = sink.into_response(RunSummary::default());

        // The stop was asked for at each row past the limit, because no
        // request reached the server.
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(response.results[0].rows.len(), 1);
        assert!(response.results[0].truncated);

        task.await.unwrap();
    }

    #[tokio::test]
    async fn an_error_that_is_not_the_stop_ends_the_run() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let (stop, _calls, signal) = test_stop(true);
        let waiter = signal.clone();
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_described(&mut server, &[("id", 23)], true).await;
            assert_eq!(read_query(&mut server).await, "SHOW ALL");

            let mut answer = row_description(&["id"]);
            for value in ["1", "2"] {
                answer.extend_from_slice(&data_row(&[Some(value)]));
            }
            server.write_all(&answer).await.unwrap();

            waiter.notified().await;
            let mut end = error_response("42P01", "relation \"t\" does not exist");
            end.extend_from_slice(&ready_for_query());
            server.write_all(&end).await.unwrap();
        });

        let mut driver = driver_with_stop(client_end, stop).await;
        let options = ExecOptions {
            max_rows: 1,
            timeout_secs: 30,
            one_statement: false,
        };
        let mut sink = BufferSink::new(options.max_rows);
        let outcome = driver.stream_simple("SHOW ALL", &options, &mut sink).await;

        assert!(outcome.is_err());

        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_sink_that_stops_a_script_of_one_statement_stops_the_server() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let (stop, calls, signal) = test_stop(true);
        let waiter = signal.clone();
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_described(&mut server, &[("id", 23)], true).await;
            assert_eq!(read_query(&mut server).await, "SHOW ALL");

            let mut answer = row_description(&["id"]);
            for value in ["1", "2", "3"] {
                answer.extend_from_slice(&data_row(&[Some(value)]));
            }
            server.write_all(&answer).await.unwrap();

            waiter.notified().await;
            let mut end = error_response("57014", "canceling statement due to user request");
            end.extend_from_slice(&ready_for_query());
            server.write_all(&end).await.unwrap();
        });

        let mut driver = driver_with_stop(client_end, stop).await;
        let options = ExecOptions {
            max_rows: 100,
            timeout_secs: 30,
            one_statement: false,
        };
        // The sink holds one row and asks the walk to stop after it.
        let mut sink = BufferSink::new(1);
        driver
            .stream_simple("SHOW ALL", &options, &mut sink)
            .await
            .unwrap();
        let response = sink.into_response(RunSummary::default());

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(response.results[0].rows.len(), 1);
        assert!(response.results[0].truncated);

        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_parameterised_select_reads_its_rows_in_their_binary_form() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_probe(&mut server, true).await;
            read_until_sync(&mut server).await;
            server
                .write_all(&prepared(Some(&[("id", 23), ("name", 25)])))
                .await
                .unwrap();

            let mut answer = Vec::new();
            answer.extend_from_slice(&binary_data_row(&[Some(&7i32.to_be_bytes()), Some(b"Ada")]));
            answer.extend_from_slice(&binary_data_row(&[None, None]));
            answer.extend_from_slice(&command_complete("SELECT 2"));
            answer_portal(&mut server, &[answer]).await;
        });

        let mut driver = driver_on(client_end).await;
        let options = ExecOptions {
            max_rows: 100,
            timeout_secs: 30,
            one_statement: false,
        };
        let mut sink = BufferSink::new(options.max_rows);
        let rows_affected = driver
            .stream_with_params(
                "SELECT id, name FROM t WHERE id = $1",
                &one_param(),
                &options,
                &mut sink,
            )
            .await
            .unwrap();
        let response = sink.into_response(RunSummary::default());

        assert_eq!(rows_affected, None);
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].columns[0].name, "id");
        assert_eq!(response.results[0].columns[0].type_name, "int4");
        assert_eq!(response.results[0].rows[0][0], JsonValue::from(7));
        assert_eq!(
            response.results[0].rows[0][1],
            JsonValue::String("Ada".into())
        );
        // A column without a value shows as NULL.
        assert_eq!(response.results[0].rows[1][0], JsonValue::Null);

        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_parameterised_read_inside_a_block_stops_its_portal_past_the_row_limit() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_probe(&mut server, false).await;
            read_until_sync(&mut server).await;
            server
                .write_all(&prepared(Some(&[("id", 23)])))
                .await
                .unwrap();
            // The portal binds in the block of the user, with no BEGIN.
            read_until_sync(&mut server).await;
            let mut bound = message(b'2', &[]);
            bound.extend_from_slice(&ready_for_query());
            server.write_all(&bound).await.unwrap();
            // The execute asks for one row past the limit and ends at that
            // count, so the portal gives no tag.
            read_until_sync(&mut server).await;
            let mut answer = Vec::new();
            for value in [1i32, 2] {
                answer.extend_from_slice(&binary_data_row(&[Some(&value.to_be_bytes())]));
            }
            answer.extend_from_slice(&message(b's', &[]));
            answer.extend_from_slice(&ready_for_query());
            server.write_all(&answer).await.unwrap();
            read_until_sync(&mut server).await;
            let mut closed = message(b'3', &[]);
            closed.extend_from_slice(&ready_for_query());
            server.write_all(&closed).await.unwrap();
            // No COMMIT follows. The next message closes the prepared
            // statement.
            let mut message_type = [0u8; 1];
            server.read_exact(&mut message_type).await.unwrap();
            assert_eq!(message_type[0], b'C');
            let mut length = [0u8; 4];
            server.read_exact(&mut length).await.unwrap();
            let mut body = vec![0u8; i32::from_be_bytes(length) as usize - 4];
            server.read_exact(&mut body).await.unwrap();
            assert_eq!(body[0], b'S');
            read_until_sync(&mut server).await;
            let mut closed = message(b'3', &[]);
            closed.extend_from_slice(&ready_for_query());
            server.write_all(&closed).await.unwrap();
        });

        let mut driver = driver_on(client_end).await;
        let options = ExecOptions {
            max_rows: 1,
            ..no_limit()
        };
        let mut sink = BufferSink::new(options.max_rows);
        driver
            .stream_with_params(
                "SELECT id FROM t WHERE $1 = 1",
                &one_param(),
                &options,
                &mut sink,
            )
            .await
            .unwrap();
        let response = sink.into_response(RunSummary::default());

        assert_eq!(response.results[0].rows.len(), 1);
        assert!(response.results[0].truncated);
        drop(driver);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_parameterised_select_of_a_timestamptz_reads_the_zone_of_the_session() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_probe(&mut server, true).await;
            read_until_sync(&mut server).await;
            server
                .write_all(&prepared(Some(&[("at", 1184)])))
                .await
                .unwrap();

            let mut zone = row_description(&["TimeZone"]);
            zone.extend_from_slice(&data_row(&[Some("<-05>+05")]));
            zone.extend_from_slice(&command_complete("SHOW"));
            answer_query(&mut server, "SHOW TimeZone", &[zone]).await;

            let mut answer = Vec::new();
            answer.extend_from_slice(&binary_data_row(&[Some(&0i64.to_be_bytes())]));
            answer.extend_from_slice(&command_complete("SELECT 1"));
            answer_portal(&mut server, &[answer]).await;
        });

        let mut driver = driver_on(client_end).await;
        let mut sink = BufferSink::new(100);
        driver
            .stream_with_params(
                "SELECT $1::timestamptz",
                &one_param(),
                &no_limit(),
                &mut sink,
            )
            .await
            .unwrap();
        let response = sink.into_response(RunSummary::default());

        assert_eq!(
            response.results[0].rows[0][0],
            JsonValue::String("1999-12-31 19:00:00-05".into())
        );

        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_parameterised_select_of_money_reads_the_digits_of_the_session() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_probe(&mut server, true).await;
            read_until_sync(&mut server).await;
            server
                .write_all(&prepared(Some(&[("price", 790)])))
                .await
                .unwrap();

            let mut digits = row_description(&["scale"]);
            digits.extend_from_slice(&data_row(&[Some("3")]));
            digits.extend_from_slice(&command_complete("SELECT 1"));
            answer_query(&mut server, MONEY_DIGITS, &[digits]).await;

            let mut answer = Vec::new();
            answer.extend_from_slice(&binary_data_row(&[Some(&123456i64.to_be_bytes())]));
            answer.extend_from_slice(&command_complete("SELECT 1"));
            answer_portal(&mut server, &[answer]).await;
        });

        let mut driver = driver_on(client_end).await;
        let mut sink = BufferSink::new(100);
        driver
            .stream_with_params("SELECT $1::money", &one_param(), &no_limit(), &mut sink)
            .await
            .unwrap();
        let response = sink.into_response(RunSummary::default());

        assert_eq!(
            response.results[0].rows[0][0],
            JsonValue::String("123.456".into())
        );

        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_money_probe_that_fails_or_gives_no_count_gives_two_digits() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_query(
                &mut server,
                MONEY_DIGITS,
                &[error_response("25P02", "transaction is aborted")],
            )
            .await;
            answer_query(&mut server, MONEY_DIGITS, &[command_complete("SELECT 0")]).await;
            let mut large = row_description(&["scale"]);
            large.extend_from_slice(&data_row(&[Some("11")]));
            large.extend_from_slice(&command_complete("SELECT 1"));
            answer_query(&mut server, MONEY_DIGITS, &[large]).await;
        });

        let driver = driver_on(client_end).await;
        assert_eq!(driver.money_digits().await, 2);
        // An answer with no row, and a count past 10, give 2 as well.
        assert_eq!(driver.money_digits().await, 2);
        assert_eq!(driver.money_digits().await, 2);

        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_zone_probe_that_fails_gives_utc() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_query(
                &mut server,
                "SHOW TimeZone",
                &[error_response("42501", "permission denied")],
            )
            .await;
            answer_query(&mut server, "SHOW TimeZone", &[command_complete("SHOW")]).await;
        });

        let driver = driver_on(client_end).await;
        assert_eq!(driver.session_zone().await, TimeZone::UTC);
        // An answer with no row gives UTC as well.
        assert_eq!(driver.session_zone().await, TimeZone::UTC);

        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_parameterised_select_that_matches_no_row_still_shows_its_columns() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_probe(&mut server, true).await;
            read_until_sync(&mut server).await;
            server
                .write_all(&prepared(Some(&[("id", 23)])))
                .await
                .unwrap();

            let mut answer = Vec::new();
            answer.extend_from_slice(&command_complete("SELECT 0"));
            answer_portal(&mut server, &[answer]).await;
        });

        let mut driver = driver_on(client_end).await;
        let options = ExecOptions {
            max_rows: 100,
            timeout_secs: 30,
            one_statement: false,
        };
        let mut sink = BufferSink::new(options.max_rows);
        driver
            .stream_with_params(
                "SELECT id FROM t WHERE id = $1",
                &one_param(),
                &options,
                &mut sink,
            )
            .await
            .unwrap();
        let response = sink.into_response(RunSummary::default());

        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].columns.len(), 1);
        assert!(response.results[0].rows.is_empty());

        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_parameterised_write_reports_the_rows_it_changed() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            read_until_sync(&mut server).await;
            server.write_all(&prepared(None)).await.unwrap();

            read_until_sync(&mut server).await;
            let mut answer = message(b'2', &[]);
            answer.extend_from_slice(&command_complete("UPDATE 3"));
            answer.extend_from_slice(&ready_for_query());
            server.write_all(&answer).await.unwrap();
        });

        let mut driver = driver_on(client_end).await;
        let options = ExecOptions {
            max_rows: 100,
            timeout_secs: 30,
            one_statement: false,
        };
        let mut sink = BufferSink::new(options.max_rows);
        let rows_affected = driver
            .stream_with_params(
                "UPDATE t SET a = 1 WHERE b = $1",
                &one_param(),
                &options,
                &mut sink,
            )
            .await
            .unwrap();
        let response = sink.into_response(RunSummary::default());

        assert_eq!(rows_affected, Some(3));
        assert!(response.results.is_empty());
        assert!(response
            .messages
            .iter()
            .any(|message| message.text.contains('3')));

        task.await.unwrap();
    }

    #[tokio::test]
    async fn the_row_limit_cuts_a_parameterised_select() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let (stop, calls, _signal) = test_stop(true);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_probe(&mut server, true).await;
            read_until_sync(&mut server).await;
            server
                .write_all(&prepared(Some(&[("id", 23)])))
                .await
                .unwrap();

            // Each execute stops at its count, so no tag ends it. The second
            // execute asks only for the rows that the first one left.
            let row = |value: i32| binary_data_row(&[Some(&value.to_be_bytes())]);
            let mut first = row(0);
            first.extend_from_slice(&message(b's', &[]));
            let mut second = row(1);
            second.extend_from_slice(&row(2));
            second.extend_from_slice(&message(b's', &[]));
            answer_portal(&mut server, &[first, second]).await;
        });

        let mut driver = driver_with_stop(client_end, stop).await;
        let options = ExecOptions {
            max_rows: 2,
            timeout_secs: 30,
            one_statement: false,
        };
        let mut sink = BufferSink::new(options.max_rows);
        driver
            .stream_with_params(
                "SELECT id FROM t WHERE id > $1",
                &one_param(),
                &options,
                &mut sink,
            )
            .await
            .unwrap();
        let response = sink.into_response(RunSummary::default());

        // The portal ends the read, and the COMMIT keeps the work of the
        // statement, so no cancel went out.
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(response.results[0].rows.len(), 2);
        assert!(response.results[0].truncated);

        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_parameterised_select_that_the_cancel_stops_needs_no_probe() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let (stop, calls, signal) = test_stop(true);
        let waiter = signal.clone();
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_probe(&mut server, true).await;
            read_until_sync(&mut server).await;
            server
                .write_all(&prepared(Some(&[("id", 23)])))
                .await
                .unwrap();

            read_until_sync(&mut server).await;
            let mut answer = message(b'2', &[]);
            for value in 0..2i32 {
                answer.extend_from_slice(&binary_data_row(&[Some(&value.to_be_bytes())]));
            }
            server.write_all(&answer).await.unwrap();

            waiter.notified().await;
            let mut end = error_response("57014", "canceling statement due to user request");
            end.extend_from_slice(&ready_for_query());
            server.write_all(&end).await.unwrap();

            // The drop of the prepared statement sends a close and a sync,
            // and the connection ends only after the server answers them.
            // No simple query comes, so no probe ran.
            let mut message_type = [0u8; 1];
            while server.read_exact(&mut message_type).await.is_ok() {
                assert_ne!(message_type[0], b'Q');
                let mut length = [0u8; 4];
                server.read_exact(&mut length).await.unwrap();
                let mut body = vec![0u8; i32::from_be_bytes(length) as usize - 4];
                server.read_exact(&mut body).await.unwrap();
                if message_type[0] == b'S' {
                    let mut answer = message(b'3', &[]);
                    answer.extend_from_slice(&ready_for_query());
                    server.write_all(&answer).await.unwrap();
                }
            }
        });

        let mut driver = driver_with_stop(client_end, stop).await;
        let options = ExecOptions {
            max_rows: 1,
            timeout_secs: 30,
            one_statement: false,
        };
        let mut sink = BufferSink::new(options.max_rows);
        driver
            .stream_with_params("SHOW ALL", &one_param(), &options, &mut sink)
            .await
            .unwrap();
        let response = sink.into_response(RunSummary::default());

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(response.results[0].rows.len(), 1);
        assert!(response.results[0].truncated);

        drop(driver);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_cancel_that_the_server_closes_gives_its_own_result() {
        assert!(wait_for_cancel(async { Ok(()) }, CANCEL_WAIT).await.is_ok());
        let refused = wait_for_cancel(
            async { Err(tokio_postgres::Error::__private_api_timeout()) },
            CANCEL_WAIT,
        )
        .await;
        assert!(refused.is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn a_cancel_without_a_close_counts_as_sent_after_the_limit() {
        let never = std::future::pending::<std::result::Result<(), tokio_postgres::Error>>();
        assert!(wait_for_cancel(never, CANCEL_WAIT).await.is_ok());
    }

    #[tokio::test]
    async fn a_cancel_that_comes_after_the_end_of_its_statement_stops_the_probe() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let (stop, calls, signal) = test_stop(true);
        let waiter = signal.clone();
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_described(&mut server, &[("id", 23)], true).await;
            assert_eq!(read_query(&mut server).await, "SHOW ALL");
            let mut answer = row_description(&["id"]);
            answer.extend_from_slice(&data_row(&[Some("1")]));
            answer.extend_from_slice(&data_row(&[Some("2")]));
            server.write_all(&answer).await.unwrap();

            // The statement ends before the cancel reaches it.
            waiter.notified().await;
            let mut end = command_complete("SELECT 2");
            end.extend_from_slice(&ready_for_query());
            server.write_all(&end).await.unwrap();

            // The cancel then stops the probe and not the next statement.
            answer_query(
                &mut server,
                CANCEL_PROBE,
                &[error_response(
                    "57014",
                    "canceling statement due to user request",
                )],
            )
            .await;
            answer_query(
                &mut server,
                "DELETE FROM t",
                &[command_complete("DELETE 5")],
            )
            .await;
        });

        let mut driver = driver_with_stop(client_end, stop).await;
        let options = ExecOptions {
            max_rows: 1,
            ..no_limit()
        };
        let mut sink = BufferSink::new(options.max_rows);
        let rows_affected = driver
            .stream_simple("SHOW ALL; DELETE FROM t", &options, &mut sink)
            .await
            .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(rows_affected, Some(5));

        task.await.unwrap();
    }

    #[test]
    fn a_copy_through_the_client_is_found() {
        assert!(copies_through_the_client("COPY t FROM STDIN"));
        assert!(copies_through_the_client(
            "-- load\ncopy t (a, b) from\n  stdin with (format csv)"
        ));
        assert!(copies_through_the_client(
            "COPY (SELECT * FROM t) TO STDOUT WITH CSV HEADER"
        ));
        assert!(!copies_through_the_client("COPY t FROM '/tmp/t.csv'"));
        assert!(!copies_through_the_client("COPY t TO '/tmp/t.csv'"));
        assert!(!copies_through_the_client("SELECT 'from stdin'"));
        assert!(!copies_through_the_client("COPY"));
    }

    #[tokio::test]
    async fn a_copy_through_the_client_fails_before_it_reaches_the_server() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            let mut rest = Vec::new();
            server.read_to_end(&mut rest).await.unwrap();
            assert!(!rest.contains(&b'Q'));
            assert!(!rest.contains(&b'P'));
        });

        let mut driver = driver_on(client_end).await;
        let mut sink = BufferSink::new(10);
        let error = driver
            .stream_simple("COPY t FROM STDIN", &no_limit(), &mut sink)
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Unsupported(_)));
        assert!(error.to_string().contains("\\copy"));

        let error = driver
            .stream_with_params("COPY t TO STDOUT", &one_param(), &no_limit(), &mut sink)
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Unsupported(_)));

        drop(driver);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_read_inside_a_block_fetches_through_a_cursor_in_that_block() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let (stop, calls, _signal) = test_stop(true);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_described(&mut server, &[("id", 23)], false).await;
            // The fetch asks for one row past the limit, and the server
            // computes no other row.
            let name = read_block_cursor_open(&mut server, "SELECT id FROM t", 2).await;
            let mut answer = command_complete("DECLARE CURSOR");
            answer.extend_from_slice(&fetched(&["id"], &[&[Some("1")], &[Some("2")]]));
            answer.extend_from_slice(&ready_for_query());
            server.write_all(&answer).await.unwrap();
            answer_query(
                &mut server,
                &format!("CLOSE {name}"),
                &[command_complete("CLOSE CURSOR")],
            )
            .await;
            answer_query(
                &mut server,
                "DELETE FROM t",
                &[command_complete("DELETE 5")],
            )
            .await;
        });

        let mut driver = driver_with_stop(client_end, stop).await;
        let options = ExecOptions {
            max_rows: 1,
            timeout_secs: 30,
            one_statement: false,
        };
        let mut sink = BufferSink::new(options.max_rows);
        let rows_affected = driver
            .stream_simple("SELECT id FROM t; DELETE FROM t", &options, &mut sink)
            .await
            .unwrap();
        let response = sink.into_response(RunSummary::default());

        // A cancel would abort the open block, so no cancel went out.
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(response.results[0].rows.len(), 1);
        assert!(response.results[0].truncated);
        assert_eq!(rows_affected, Some(5));

        task.await.unwrap();
    }

    #[tokio::test]
    async fn an_error_of_a_read_inside_a_block_leaves_the_block_to_the_user() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_described(&mut server, &[("id", 23)], false).await;
            read_block_cursor_open(&mut server, "SELECT 1 / 0", 101).await;
            let mut answer = error_response("22012", "division by zero");
            answer.extend_from_slice(&ready_for_query());
            server.write_all(&answer).await.unwrap();
            // The driver sends no ROLLBACK and no later statement.
            let mut rest = Vec::new();
            server.read_to_end(&mut rest).await.unwrap();
            assert!(!rest.contains(&b'Q'));
        });

        let mut driver = driver_on(client_end).await;
        let mut sink = BufferSink::new(100);
        let error = driver
            .stream_simple("SELECT 1 / 0; SELECT 2", &no_limit(), &mut sink)
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            Error::Located { ref inner, .. }
                if matches!(**inner, Error::Postgres(ref error)
                    if error.code() == Some(&SqlState::DIVISION_BY_ZERO))
        ));
        drop(driver);
        task.await.unwrap();
    }

    #[test]
    fn the_read_path_follows_the_statement_and_the_probe() {
        let read = "SELECT * FROM t";
        assert_eq!(read_path(read, Some(true)), ReadPath::OwnTransaction);
        assert_eq!(read_path(read, Some(false)), ReadPath::UserBlock);
        assert_eq!(read_path(read, None), ReadPath::Walk);

        // A lock in a block of the user keeps the walk. Outside a block the
        // driver's own COMMIT ends every lock at once.
        let locks = "SELECT * FROM t FOR SHARE";
        assert_eq!(read_path(locks, Some(false)), ReadPath::Walk);
        assert_eq!(read_path(locks, Some(true)), ReadPath::OwnTransaction);

        for statement in ["SHOW ALL", "DELETE FROM t RETURNING *"] {
            for outside in [Some(true), Some(false), None] {
                assert_eq!(read_path(statement, outside), ReadPath::Walk);
            }
        }
    }

    #[tokio::test]
    async fn a_probe_that_fails_keeps_the_walk() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let (stop, calls, _signal) = test_stop(true);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            read_until_sync(&mut server).await;
            let mut answer = message(b'1', &[]);
            answer.extend_from_slice(&message(b't', &0i16.to_be_bytes()));
            answer.extend_from_slice(&typed_row_description(&[("id", 23)]));
            answer.extend_from_slice(&ready_for_query());
            server.write_all(&answer).await.unwrap();
            assert_eq!(read_query(&mut server).await, OUTSIDE_A_BLOCK);
            let mut refusal = error_response("25P02", "current transaction is aborted");
            refusal.extend_from_slice(&ready_for_query());
            server.write_all(&refusal).await.unwrap();
            read_until_sync(&mut server).await;
            let mut closed = message(b'3', &[]);
            closed.extend_from_slice(&ready_for_query());
            server.write_all(&closed).await.unwrap();
            read_query(&mut server).await;

            let mut answer = row_description(&["id"]);
            for value in ["1", "2"] {
                answer.extend_from_slice(&data_row(&[Some(value)]));
            }
            answer.extend_from_slice(&command_complete("SELECT 2"));
            answer.extend_from_slice(&ready_for_query());
            server.write_all(&answer).await.unwrap();
        });

        let mut driver = driver_with_stop(client_end, stop).await;
        let options = ExecOptions {
            max_rows: 1,
            timeout_secs: 30,
            one_statement: false,
        };
        let mut sink = BufferSink::new(options.max_rows);
        driver
            .stream_simple("SELECT id FROM t", &options, &mut sink)
            .await
            .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 0);

        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_write_that_returns_rows_runs_to_its_end() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let (stop, calls, _signal) = test_stop(true);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            // A write needs no probe, so the statement is the first query.
            assert!(read_query(&mut server).await.starts_with("INSERT"));

            let mut answer = row_description(&["id"]);
            for value in ["1", "2", "3"] {
                answer.extend_from_slice(&data_row(&[Some(value)]));
            }
            answer.extend_from_slice(&command_complete("INSERT 0 3"));
            answer.extend_from_slice(&ready_for_query());
            server.write_all(&answer).await.unwrap();
        });

        let mut driver = driver_with_stop(client_end, stop).await;
        let options = ExecOptions {
            max_rows: 1,
            timeout_secs: 30,
            one_statement: false,
        };
        let mut sink = BufferSink::new(options.max_rows);
        driver
            .stream_simple(
                "INSERT INTO t SELECT generate_series(1, 3) RETURNING id",
                &options,
                &mut sink,
            )
            .await
            .unwrap();
        let response = sink.into_response(RunSummary::default());

        // A cancel would roll back the whole INSERT.
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(response.results[0].rows.len(), 1);
        assert!(response.results[0].truncated);
        // A write is not prepared, so the column takes the name of its OID.
        assert_eq!(response.results[0].columns[0].type_name, "text");

        task.await.unwrap();
    }

    /// The type names of the columns of the first set of a response.
    fn type_names_of(response: &crate::db::QueryResponse) -> Vec<&str> {
        response.results[0]
            .columns
            .iter()
            .map(|column| column.type_name.as_str())
            .collect()
    }

    #[tokio::test]
    async fn a_read_takes_the_type_names_of_its_prepared_statement() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_described(&mut server, &[("id", 23), ("at", 1184)], true).await;
            answer_cursor(
                &mut server,
                "SELECT id, at FROM t",
                101,
                &["id", "at"],
                &[&[Some("1"), None]],
            )
            .await;
        });

        let mut driver = driver_on(client_end).await;
        let mut sink = BufferSink::new(10);
        driver
            .stream_simple("SELECT id, at FROM t", &no_limit(), &mut sink)
            .await
            .unwrap();
        let response = sink.into_response(RunSummary::default());

        assert_eq!(type_names_of(&response), ["int4", "timestamptz"]);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_read_that_cannot_be_prepared_outside_a_block_names_its_types_by_oid() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            refuse_described(&mut server, true).await;
            read_cursor_open(&mut server, "SELECT id, mood FROM t", 101).await;
            let mut answer = opened();
            answer.extend_from_slice(&simple_row_description(&[("id", 23), ("mood", 16423)]));
            answer.extend_from_slice(&data_row(&[Some("1"), Some("happy")]));
            answer.extend_from_slice(&command_complete("FETCH 1"));
            answer.extend_from_slice(&ready_for_query());
            server.write_all(&answer).await.unwrap();
            answer_query(&mut server, "COMMIT", &[command_complete("COMMIT")]).await;
        });

        let mut driver = driver_on(client_end).await;
        let mut sink = BufferSink::new(10);
        driver
            .stream_simple("SELECT id, mood FROM t", &no_limit(), &mut sink)
            .await
            .unwrap();
        let response = sink.into_response(RunSummary::default());

        assert_eq!(type_names_of(&response), ["int4", "16423"]);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_read_that_cannot_be_prepared_inside_a_block_ends_with_the_error_of_the_prepare() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            refuse_described(&mut server, false).await;
            // The client sends no simple query after the prepare.
            let mut rest = Vec::new();
            server.read_to_end(&mut rest).await.unwrap();
            assert!(!rest.contains(&b'Q'));
        });

        let mut driver = driver_on(client_end).await;
        let mut sink = BufferSink::new(10);
        let error = driver
            .stream_simple("SELECT id FROM t", &no_limit(), &mut sink)
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            Error::Located { ref inner, line: 1, column: 1 }
                if matches!(**inner, Error::Postgres(ref error)
                    if error.code() == Some(&SqlState::UNDEFINED_TABLE))
        ));
        drop(driver);
        task.await.unwrap();
    }

    #[test]
    fn the_columns_of_a_simple_set_take_the_prepared_names_or_the_names_of_their_oids() {
        let names = |columns: Vec<ColumnInfo>| -> Vec<(String, String)> {
            columns
                .into_iter()
                .map(|column| (column.name, column.type_name))
                .collect()
        };
        let columns = [("id", 23), ("mood", 16423)];
        let prepared = vec!["int4".to_string(), "mood".to_string()];

        assert_eq!(
            names(simple_columns(Some(&prepared), &columns)),
            [("id".into(), "int4".into()), ("mood".into(), "mood".into())]
        );
        // A count that differs, or no prepared names, falls back on the OIDs.
        let fallback = [
            ("id".into(), "int4".into()),
            ("mood".into(), "16423".into()),
        ];
        assert_eq!(
            names(simple_columns(Some(&prepared[..1]), &columns)),
            fallback
        );
        assert_eq!(names(simple_columns(None, &columns)), fallback);
    }

    #[tokio::test]
    async fn a_parameterised_write_that_returns_rows_reports_a_late_fault() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let (stop, calls, _signal) = test_stop(true);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            read_until_sync(&mut server).await;
            server
                .write_all(&prepared(Some(&[("id", 23)])))
                .await
                .unwrap();

            read_until_sync(&mut server).await;
            let mut answer = message(b'2', &[]);
            for value in 0..3i32 {
                answer.extend_from_slice(&binary_data_row(&[Some(&value.to_be_bytes())]));
            }
            answer.extend_from_slice(&error_response("23505", "duplicate key value"));
            answer.extend_from_slice(&ready_for_query());
            server.write_all(&answer).await.unwrap();
        });

        let mut driver = driver_with_stop(client_end, stop).await;
        let options = ExecOptions {
            max_rows: 1,
            timeout_secs: 30,
            one_statement: false,
        };
        let mut sink = BufferSink::new(options.max_rows);
        let outcome = driver
            .stream_with_params(
                "INSERT INTO t (id) SELECT $1 RETURNING id",
                &one_param(),
                &options,
                &mut sink,
            )
            .await;

        // The walk went past the limit and read the fault that rolled back
        // the INSERT.
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(outcome.is_err());

        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_parameterised_select_whose_stop_fails_walks_to_the_end() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let (stop, calls, _signal) = test_stop(false);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_probe(&mut server, true).await;
            read_until_sync(&mut server).await;
            server
                .write_all(&prepared(Some(&[("id", 23)])))
                .await
                .unwrap();

            read_until_sync(&mut server).await;
            let mut answer = message(b'2', &[]);
            for value in 0..3i32 {
                answer.extend_from_slice(&binary_data_row(&[Some(&value.to_be_bytes())]));
            }
            answer.extend_from_slice(&command_complete("SELECT 3"));
            answer.extend_from_slice(&ready_for_query());
            server.write_all(&answer).await.unwrap();
        });

        let mut driver = driver_with_stop(client_end, stop).await;
        let options = ExecOptions {
            max_rows: 1,
            timeout_secs: 30,
            one_statement: false,
        };
        let mut sink = BufferSink::new(options.max_rows);
        driver
            .stream_with_params("SHOW ALL", &one_param(), &options, &mut sink)
            .await
            .unwrap();
        let response = sink.into_response(RunSummary::default());

        // The first stop failed, so the walk read the rest and asked no
        // second time.
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(response.results[0].rows.len(), 1);
        assert!(response.results[0].truncated);

        task.await.unwrap();
    }

    #[test]
    fn the_reads_of_rows_and_the_reads_through_a_cursor_are_found() {
        for statement in [
            "SELECT 1",
            "WITH a AS (SELECT 1) TABLE a",
            "VALUES (1)",
            "TABLE t",
        ] {
            assert!(reads_rows(statement), "{statement}");
            assert!(reads_through_a_cursor(statement), "{statement}");
        }
        assert!(reads_rows("SHOW ALL"));
        assert!(!reads_through_a_cursor("SHOW ALL"));
        for statement in ["DELETE FROM t", "SELECT 1 INTO t", "EXPLAIN SELECT 1"] {
            assert!(!reads_rows(statement), "{statement}");
            assert!(!reads_through_a_cursor(statement), "{statement}");
        }
    }

    #[test]
    fn an_error_that_is_not_one_of_the_server_keeps_its_own_form() {
        let failure = Failure::from(Error::Invalid("bad".into()));
        assert!(matches!(
            failure.locate("q", "q", 0),
            Error::Invalid(ref text) if text == "bad"
        ));
    }

    #[tokio::test]
    async fn a_read_through_a_cursor_fetches_in_steps_up_to_the_row_limit() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let (stop, calls, _signal) = test_stop(true);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_described(&mut server, &[("n", 23)], true).await;
            let name = read_cursor_open(&mut server, "VALUES (1)", FETCH_BATCH).await;
            // The tag says the fetch filled its count, so a second fetch
            // asks for the rest of the rows up to one past the limit.
            let mut answer = opened();
            answer.extend_from_slice(&row_description(&["n"]));
            answer.extend_from_slice(&data_row(&[Some("1")]));
            answer.extend_from_slice(&command_complete(&format!("FETCH {FETCH_BATCH}")));
            answer.extend_from_slice(&ready_for_query());
            server.write_all(&answer).await.unwrap();
            answer_query(
                &mut server,
                &format!("FETCH 50001 FROM {name}"),
                &[fetched(&["n"], &[&[Some("2")]])],
            )
            .await;
            answer_query(&mut server, "COMMIT", &[command_complete("COMMIT")]).await;
        });

        let mut driver = driver_with_stop(client_end, stop).await;
        let options = ExecOptions {
            max_rows: 150_000,
            ..no_limit()
        };
        let mut sink = BufferSink::new(options.max_rows);
        let rows_affected = driver
            .stream_simple("VALUES (1)", &options, &mut sink)
            .await
            .unwrap();
        let response = sink.into_response(RunSummary::default());

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(rows_affected, None);
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].columns[0].type_name, "int4");
        assert_eq!(response.results[0].rows.len(), 2);
        assert!(!response.results[0].truncated);
        // The tags of BEGIN, DECLARE and FETCH give no message.
        let texts: Vec<&str> = response.messages.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(texts.len(), 1, "{texts:?}");
        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_read_through_a_cursor_cuts_the_set_at_the_row_limit_and_commits() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let (stop, calls, _signal) = test_stop(true);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_described(&mut server, &[("id", 23)], true).await;
            answer_cursor(
                &mut server,
                "SELECT f()",
                3,
                &["id"],
                &[&[Some("1")], &[Some("2")], &[Some("3")]],
            )
            .await;
        });

        let mut driver = driver_with_stop(client_end, stop).await;
        let options = ExecOptions {
            max_rows: 2,
            ..no_limit()
        };
        let mut sink = BufferSink::new(options.max_rows);
        driver
            .stream_simple("SELECT f()", &options, &mut sink)
            .await
            .unwrap();
        let response = sink.into_response(RunSummary::default());

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(response.results[0].rows.len(), 2);
        assert!(response.results[0].truncated);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_sink_that_takes_no_more_rows_ends_a_read_through_a_cursor() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_described(&mut server, &[("id", 23)], true).await;
            read_cursor_open(&mut server, "TABLE t", FETCH_BATCH).await;
            let mut answer = opened();
            answer.extend_from_slice(&row_description(&["id"]));
            answer.extend_from_slice(&data_row(&[Some("1")]));
            answer.extend_from_slice(&data_row(&[Some("2")]));
            answer.extend_from_slice(&command_complete(&format!("FETCH {FETCH_BATCH}")));
            answer.extend_from_slice(&ready_for_query());
            server.write_all(&answer).await.unwrap();
            // No second fetch comes.
            answer_query(&mut server, "COMMIT", &[command_complete("COMMIT")]).await;
        });

        let mut driver = driver_on(client_end).await;
        let options = ExecOptions {
            max_rows: 150_000,
            ..no_limit()
        };
        let mut sink = BufferSink::new(1);
        driver
            .stream_simple("TABLE t", &options, &mut sink)
            .await
            .unwrap();
        let response = sink.into_response(RunSummary::default());

        assert_eq!(response.results[0].rows.len(), 1);
        assert!(response.results[0].truncated);
        task.await.unwrap();
    }

    /// Runs a read whose `DECLARE` fails at the position that `at` gives
    /// from the text of the open, and gives the line and the column of the
    /// error. The test server checks the ROLLBACK that follows.
    async fn declare_error_place(at: fn(&str) -> u32) -> (Option<u32>, Option<u32>) {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_described(&mut server, &[("id", 23)], true).await;
            let text = read_query(&mut server).await;
            let mut answer = command_complete("BEGIN");
            answer.extend_from_slice(&positioned_error("no column", at(&text)));
            answer.extend_from_slice(&ready_for_query());
            server.write_all(&answer).await.unwrap();
            answer_query(&mut server, "ROLLBACK", &[command_complete("ROLLBACK")]).await;
        });
        let mut driver = driver_on(client_end).await;
        let mut sink = BufferSink::new(10);
        let error = driver
            .stream_simple("\n  SELECT\n nope", &no_limit(), &mut sink)
            .await
            .unwrap_err();
        task.await.unwrap();
        let payload = error.to_payload();
        (payload.line, payload.column)
    }

    #[tokio::test]
    async fn an_error_of_the_declare_rolls_back_and_marks_its_place_in_the_statement() {
        // The server counts the position in the whole text of the open.
        let place = declare_error_place(|text| text.find("nope").unwrap() as u32 + 1).await;
        assert_eq!(place, (Some(3), Some(2)));
        // A position inside the prefix marks the start of the statement.
        assert_eq!(declare_error_place(|_| 1).await, (Some(2), Some(3)));
    }

    #[tokio::test]
    async fn a_stop_during_a_read_through_a_cursor_rolls_it_back() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_described(&mut server, &[("id", 23)], true).await;
            read_cursor_open(&mut server, "SELECT pg_sleep(9)", 101).await;
            let mut answer = opened();
            answer.extend_from_slice(&error_response(
                "57014",
                "canceling statement due to user request",
            ));
            answer.extend_from_slice(&ready_for_query());
            server.write_all(&answer).await.unwrap();
            answer_query(&mut server, "ROLLBACK", &[command_complete("ROLLBACK")]).await;
        });

        let mut driver = driver_on(client_end).await;
        let mut sink = BufferSink::new(10);
        let error = driver
            .stream_simple("SELECT pg_sleep(9)", &no_limit(), &mut sink)
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            Error::Located { ref inner, .. }
                if matches!(**inner, Error::Postgres(ref error) if is_query_cancelled(error))
        ));
        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_read_through_a_cursor_that_is_dropped_rolls_back() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let (opened_text, wait_open) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_described(&mut server, &[("id", 23)], true).await;
            read_cursor_open(&mut server, "SELECT pg_sleep(9)", 101).await;
            opened_text.send(()).unwrap();
            // The server answers the open only after the drop.
            assert_eq!(read_query(&mut server).await, "ROLLBACK");
        });

        let mut driver = driver_on(client_end).await;
        let mut sink = BufferSink::new(10);
        let options = no_limit();
        tokio::select! {
            _ = driver.stream_simple("SELECT pg_sleep(9)", &options, &mut sink) => {
                panic!("the read ended")
            }
            _ = wait_open => {}
        }
        task.await.unwrap();
    }

    #[tokio::test]
    async fn an_error_of_a_parameterised_read_through_a_portal_rolls_back() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_probe(&mut server, true).await;
            read_until_sync(&mut server).await;
            server
                .write_all(&prepared(Some(&[("id", 23)])))
                .await
                .unwrap();
            answer_query(
                &mut server,
                "START TRANSACTION",
                &[command_complete("BEGIN")],
            )
            .await;
            read_until_sync(&mut server).await;
            let mut bound = message(b'2', &[]);
            bound.extend_from_slice(&ready_for_query());
            server.write_all(&bound).await.unwrap();
            read_until_sync(&mut server).await;
            let mut answer = error_response("22012", "division by zero");
            answer.extend_from_slice(&ready_for_query());
            server.write_all(&answer).await.unwrap();
            // The portal closes, and the drop of the transaction sends a
            // ROLLBACK.
            read_until_sync(&mut server).await;
            let mut closed = message(b'3', &[]);
            closed.extend_from_slice(&ready_for_query());
            server.write_all(&closed).await.unwrap();
            assert_eq!(read_query(&mut server).await, "ROLLBACK");
        });

        let mut driver = driver_on(client_end).await;
        let mut sink = BufferSink::new(10);
        let outcome = driver
            .stream_with_params("SELECT 1 / $1", &one_param(), &no_limit(), &mut sink)
            .await;

        assert!(outcome.is_err());
        task.await.unwrap();
    }

    /// Reads the eight bytes of an SSLRequest or the sixteen of a
    /// CancelRequest and gives back the code that follows the length.
    async fn read_request_code(socket: &mut tokio::net::TcpStream) -> (i32, Vec<u8>) {
        let mut length = [0u8; 4];
        socket.read_exact(&mut length).await.unwrap();
        let mut body = vec![0u8; i32::from_be_bytes(length) as usize - 4];
        socket.read_exact(&mut body).await.unwrap();
        let code = i32::from_be_bytes([body[0], body[1], body[2], body[3]]);
        (code, body)
    }

    #[tokio::test]
    async fn the_cancel_asks_for_tls_in_the_mode_of_the_connection() {
        const SSL_REQUEST: i32 = 80877103;
        const CANCEL_REQUEST: i32 = 80877102;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            // The login asks for TLS first, and the server declines it.
            let (mut login, _) = listener.accept().await.unwrap();
            assert_eq!(read_request_code(&mut login).await.0, SSL_REQUEST);
            login.write_all(b"N").await.unwrap();
            read_request_code(&mut login).await;
            login.write_all(&authentication_ok()).await.unwrap();
            let mut key = 42i32.to_be_bytes().to_vec();
            key.extend_from_slice(&7i32.to_be_bytes());
            login.write_all(&message(b'K', &key)).await.unwrap();
            login.write_all(&ready_for_query()).await.unwrap();

            // The cancel asks for TLS on its own socket in the same way.
            let (mut cancel, _) = listener.accept().await.unwrap();
            assert_eq!(read_request_code(&mut cancel).await.0, SSL_REQUEST);
            cancel.write_all(b"N").await.unwrap();
            let (code, body) = read_request_code(&mut cancel).await;
            assert_eq!(code, CANCEL_REQUEST);
            assert_eq!(&body[4..8], &42i32.to_be_bytes());
            login
        });

        let mut input = connection();
        input.host = Some("127.0.0.1".into());
        input.port = Some(port);
        input.options.tls_mode = TlsMode::Prefer;
        let driver = PostgresDriver::connect(&input).await.unwrap();
        driver.cancel_handle().unwrap().cancel().await.unwrap();

        drop(server.await.unwrap());
    }

    /// Builds the binary form of an array. The lengths name the dimensions,
    /// and the values follow in row order.
    fn array_body(element: &Type, lengths: &[i32], values: &[Option<&[u8]>]) -> Vec<u8> {
        let mut body = (lengths.len() as i32).to_be_bytes().to_vec();
        body.extend_from_slice(&0i32.to_be_bytes());
        body.extend_from_slice(&element.oid().to_be_bytes());
        for length in lengths {
            body.extend_from_slice(&length.to_be_bytes());
            body.extend_from_slice(&1i32.to_be_bytes());
        }
        for value in values {
            body.extend_from_slice(&element_body(*value));
        }
        body
    }

    /// Builds one value that carries its own length.
    fn element_body(value: Option<&[u8]>) -> Vec<u8> {
        match value {
            None => (-1i32).to_be_bytes().to_vec(),
            Some(bytes) => {
                let mut out = (bytes.len() as i32).to_be_bytes().to_vec();
                out.extend_from_slice(bytes);
                out
            }
        }
    }

    /// A type of a range over the given element type.
    fn range_type(element: Type) -> Type {
        Type::new(
            "int4range".to_string(),
            3904,
            Kind::Range(element),
            "pg_catalog".to_string(),
        )
    }

    /// Builds the binary form of a range.
    fn range_body(flags: u8, bounds: &[&[u8]]) -> Vec<u8> {
        let mut body = vec![flags];
        for bound in bounds {
            body.extend_from_slice(&element_body(Some(bound)));
        }
        body
    }

    fn decoded(column_type: &Type, bytes: &[u8]) -> JsonValue {
        decode_value(column_type, bytes, &Settings::default())
    }

    #[test]
    fn the_numbers_and_the_text_of_a_binary_value_are_read() {
        assert_eq!(decoded(&Type::BOOL, &[1]), JsonValue::Bool(true));
        assert_eq!(
            decoded(&Type::INT2, &7i16.to_be_bytes()),
            JsonValue::from(7)
        );
        assert_eq!(
            decoded(&Type::INT4, &7i32.to_be_bytes()),
            JsonValue::from(7)
        );
        assert_eq!(
            decoded(&Type::INT8, &7i64.to_be_bytes()),
            JsonValue::from(7)
        );
        assert_eq!(
            decoded(&Type::FLOAT4, &1.5f32.to_be_bytes()),
            JsonValue::from(1.5)
        );
        assert_eq!(
            decoded(&Type::FLOAT4, &0.1f32.to_be_bytes()),
            JsonValue::from(0.1)
        );
        assert_eq!(
            decoded(&Type::FLOAT8, &1.5f64.to_be_bytes()),
            JsonValue::from(1.5)
        );
        assert_eq!(
            decoded(&Type::TEXT, b"hello"),
            JsonValue::String("hello".into())
        );
        assert_eq!(
            decoded(&Type::XML, b"<a/>"),
            JsonValue::String("<a/>".into())
        );
        assert_eq!(
            decoded(&Type::BYTEA, b"hi\xff"),
            JsonValue::String("\\x6869ff".into())
        );
    }

    #[test]
    fn an_oid_is_read_as_a_number_without_a_sign() {
        assert_eq!(
            decoded(&Type::OID, &4294967295u32.to_be_bytes()),
            JsonValue::from(4294967295u32)
        );
    }

    #[test]
    fn a_numeric_keeps_its_digits() {
        // One digit of the base of ten thousand, the weight of the first
        // digit, the sign, and the count of the digits of the fraction.
        let mut body = 1i16.to_be_bytes().to_vec();
        body.extend_from_slice(&0i16.to_be_bytes());
        body.extend_from_slice(&0i16.to_be_bytes());
        body.extend_from_slice(&0i16.to_be_bytes());
        body.extend_from_slice(&1i16.to_be_bytes());
        assert_eq!(
            decoded(&Type::NUMERIC, &body),
            JsonValue::String("1".into())
        );
    }

    /// Builds the binary form of a NUMERIC value.
    fn numeric(weight: i16, sign: u16, scale: u16, digits: &[i16]) -> Vec<u8> {
        let mut body = (digits.len() as i16).to_be_bytes().to_vec();
        body.extend_from_slice(&weight.to_be_bytes());
        body.extend_from_slice(&sign.to_be_bytes());
        body.extend_from_slice(&scale.to_be_bytes());
        for digit in digits {
            body.extend_from_slice(&digit.to_be_bytes());
        }
        body
    }

    #[test]
    fn a_numeric_keeps_every_digit_and_its_special_values() {
        let text = |body: Vec<u8>| decoded(&Type::NUMERIC, &body);
        let string = |value: &str| JsonValue::String(value.into());
        // 123456789012345678901234567890.12, more digits than 28.
        assert_eq!(
            text(numeric(
                7,
                0,
                2,
                &[12, 3456, 7890, 1234, 5678, 9012, 3456, 7890, 1200]
            )),
            string("123456789012345678901234567890.12")
        );
        assert_eq!(text(numeric(0, 0x4000, 3, &[5, 1000])), string("-5.100"));
        assert_eq!(text(numeric(-2, 0, 8, &[1])), string("0.00000001"));
        assert_eq!(text(numeric(0, 0, 2, &[])), string("0.00"));
        assert_eq!(text(numeric(1, 0, 0, &[1])), string("10000"));
        assert_eq!(text(numeric(0, 0xC000, 0, &[])), string("NaN"));
        assert_eq!(text(numeric(0, 0xD000, 0, &[])), string("Infinity"));
        assert_eq!(text(numeric(0, 0xF000, 0, &[])), string("-Infinity"));
        // A body that ends early falls back on the text rule.
        assert_eq!(text(vec![0, 1]), text_or_bytes(&[0, 1]));
        let mut short = numeric(0, 0, 0, &[1, 2]);
        short.truncate(10);
        assert_eq!(text(short.clone()), text_or_bytes(&short));
    }

    #[test]
    fn an_endless_date_or_timestamp_shows_its_word() {
        let string = |value: &str| JsonValue::String(value.into());
        assert_eq!(
            decoded(&Type::DATE, &i32::MAX.to_be_bytes()),
            string("infinity")
        );
        assert_eq!(
            decoded(&Type::DATE, &i32::MIN.to_be_bytes()),
            string("-infinity")
        );
        for column_type in [Type::TIMESTAMP, Type::TIMESTAMPTZ] {
            assert_eq!(
                decoded(&column_type, &i64::MAX.to_be_bytes()),
                string("infinity")
            );
            assert_eq!(
                decoded(&column_type, &i64::MIN.to_be_bytes()),
                string("-infinity")
            );
        }
        // A body of the wrong length falls back on the text rule.
        assert_eq!(decoded(&Type::DATE, &[1]), text_or_bytes(&[1]));
    }

    #[test]
    fn the_dates_and_the_times_carry_the_epoch_of_the_server() {
        assert_eq!(
            decoded(&Type::DATE, &0i32.to_be_bytes()),
            JsonValue::String("2000-01-01".into())
        );
        assert_eq!(
            decoded(&Type::TIME, &0i64.to_be_bytes()),
            JsonValue::String("00:00:00".into())
        );
        assert_eq!(
            decoded(&Type::TIMESTAMP, &0i64.to_be_bytes()),
            JsonValue::String("2000-01-01 00:00:00".into())
        );
        assert_eq!(
            decoded(&Type::TIMESTAMPTZ, &0i64.to_be_bytes()),
            JsonValue::String("2000-01-01 00:00:00+00".into())
        );
    }

    #[test]
    fn a_date_and_a_time_show_as_postgresql_writes_them() {
        let string = |value: &str| JsonValue::String(value.into());
        let epoch = NaiveDate::from_ymd_opt(2000, 1, 1).unwrap();
        // The days and the microseconds from the epoch of the server.
        let days = |year: i32, month: u32, day: u32| {
            let date = NaiveDate::from_ymd_opt(year, month, day).unwrap();
            (date - epoch).num_days() as i32
        };
        let micros = |year: i32| i64::from(days(year, 1, 1)) * 86_400_000_000;

        // The fraction of a second has no zeros at its end.
        let quarter = 9 * 3_600_000_000i64 + 250_000;
        assert_eq!(
            decoded(&Type::TIME, &quarter.to_be_bytes()),
            string("09:00:00.25")
        );
        assert_eq!(
            decoded(&Type::TIME, &86_400_000_000i64.to_be_bytes()),
            string("24:00:00")
        );
        assert_eq!(decoded(&Type::TIME, b"abc"), string("abc"));
        assert_eq!(
            decoded(&Type::TIMESTAMP, &250_000i64.to_be_bytes()),
            string("2000-01-01 00:00:00.25")
        );
        // A date before the year 1 shows its era, and the year 0 is 1 BC.
        assert_eq!(
            decoded(&Type::DATE, &days(0, 1, 1).to_be_bytes()),
            string("0001-01-01 BC")
        );
        assert_eq!(
            decoded(&Type::DATE, &days(-43, 3, 15).to_be_bytes()),
            string("0044-03-15 BC")
        );
        assert_eq!(
            decoded(&Type::TIMESTAMP, &micros(0).to_be_bytes()),
            string("0001-01-01 00:00:00 BC")
        );
        assert_eq!(
            decoded(&Type::TIMESTAMPTZ, &micros(0).to_be_bytes()),
            string("0001-01-01 00:00:00+00 BC")
        );
        // A year past 9999 has no sign.
        assert_eq!(
            decoded(&Type::DATE, &days(10_000, 1, 1).to_be_bytes()),
            string("10000-01-01")
        );
        assert_eq!(
            decoded(&Type::DATE, &days(5, 1, 1).to_be_bytes()),
            string("0005-01-01")
        );
    }

    #[test]
    fn a_timestamptz_value_shows_in_the_zone_of_the_session() {
        let moment = |micros: i64| DateTime::<Utc>::from_timestamp_micros(micros).unwrap();
        // 2024-01-01 14:00:00 UTC.
        let base = 1_704_117_600_000_000;
        let five_west = zone_of("<-05>+05");
        assert_eq!(
            zoned_text(moment(base), &five_west),
            "2024-01-01 09:00:00-05"
        );
        assert_eq!(
            zoned_text(moment(base + 250_000), &five_west),
            "2024-01-01 09:00:00.25-05"
        );
        assert_eq!(
            zoned_text(moment(base + 1), &TimeZone::UTC),
            "2024-01-01 14:00:00.000001+00"
        );
        // An offset with minutes and one with seconds show them.
        let india = TimeZone::fixed(jiff::tz::Offset::from_seconds(19_800).unwrap());
        assert_eq!(
            zoned_text(moment(base), &india),
            "2024-01-01 19:30:00+05:30"
        );
        let odd = TimeZone::fixed(jiff::tz::Offset::from_seconds(-3_661).unwrap());
        assert_eq!(
            zoned_text(moment(base), &odd),
            "2024-01-01 12:58:59-01:01:01"
        );
        // A zone of the IANA database follows its summer time.
        let paris = zone_of("Europe/Paris");
        assert_eq!(zoned_text(moment(base), &paris), "2024-01-01 15:00:00+01");
        // A name that is no zone gives UTC.
        assert_eq!(
            zoned_text(moment(base), &zone_of("Nowhere/At all")),
            "2024-01-01 14:00:00+00"
        );
        // A moment past the range of the zone database keeps the RFC 3339 form.
        let far = DateTime::<Utc>::MAX_UTC;
        assert_eq!(zoned_text(far, &TimeZone::UTC), far.to_rfc3339());
    }

    #[test]
    fn a_type_that_holds_a_timestamptz_is_found() {
        let tz = &Type::TIMESTAMPTZ;
        assert!(holds(&Type::TIMESTAMPTZ, tz));
        assert!(holds(&Type::TIMESTAMPTZ_ARRAY, tz));
        assert!(holds(&Type::TSTZ_RANGE, tz));
        assert!(!holds(&Type::TIMESTAMP, tz));
        assert!(!holds(&Type::INT4_ARRAY, tz));
        let composite = Type::new(
            "pair".into(),
            0,
            Kind::Composite(vec![Field::new("at".into(), Type::TIMESTAMPTZ)]),
            "public".into(),
        );
        assert!(holds(&composite, tz));
        assert!(holds(&Type::MONEY_ARRAY, &Type::MONEY));
    }

    #[test]
    fn a_part_of_a_composite_or_a_range_takes_quotes_as_postgresql_writes_them() {
        assert_eq!(render(&JsonValue::Null, false), "");
        assert_eq!(render(&serde_json::json!(5), false), "5");
        assert_eq!(render(&JsonValue::String("plain".into()), false), "plain");
        assert_eq!(render(&JsonValue::String(String::new()), false), "\"\"");
        assert_eq!(
            render(&JsonValue::String("a \"b\" c\\d".into()), false),
            "\"a \"\"b\"\" c\\\\d\""
        );
        assert_eq!(
            render(&JsonValue::String("(1,2)".into()), false),
            "\"(1,2)\""
        );
        assert_eq!(render(&JsonValue::String("[x]".into()), false), "[x]");
        assert_eq!(render(&JsonValue::String("[x]".into()), true), "\"[x]\"");
    }

    #[test]
    fn a_uuid_and_a_json_value_are_read() {
        assert_eq!(
            decoded(&Type::UUID, &[0x11; 16]),
            JsonValue::String("11111111-1111-1111-1111-111111111111".into())
        );
        assert_eq!(
            decoded(&Type::JSON, br#"{"b": 1, "a": 12345678901234567890}"#),
            JsonValue::String(r#"{"b": 1, "a": 12345678901234567890}"#.into())
        );
        let mut jsonb = vec![1u8];
        jsonb.extend_from_slice(b"[1]");
        assert_eq!(
            decoded(&Type::JSONB, &jsonb),
            JsonValue::String("[1]".into())
        );
        assert_eq!(decoded(&Type::JSONB, b""), JsonValue::String(String::new()));
    }

    #[test]
    fn a_money_value_holds_the_digits_of_the_fraction_of_the_session() {
        let string = |value: &str| JsonValue::String(value.into());
        assert_eq!(
            decoded(&Type::MONEY, &123456i64.to_be_bytes()),
            string("1234.56")
        );
        assert_eq!(
            decoded(&Type::MONEY, &(-5i64).to_be_bytes()),
            string("-0.05")
        );
        // A currency of three digits, and one of none.
        assert_eq!(money_text(&123456i64.to_be_bytes(), 3), string("123.456"));
        assert_eq!(money_text(&(-1234i64).to_be_bytes(), 0), string("-1234"));
        // A value of the wrong length falls back on the text rule.
        assert_eq!(decoded(&Type::MONEY, b"12"), string("12"));
    }

    #[test]
    fn an_interval_names_every_part_that_it_holds() {
        fn interval(micros: i64, days: i32, months: i32) -> JsonValue {
            let mut body = micros.to_be_bytes().to_vec();
            body.extend_from_slice(&days.to_be_bytes());
            body.extend_from_slice(&months.to_be_bytes());
            decoded(&Type::INTERVAL, &body)
        }

        assert_eq!(interval(0, 0, 0), JsonValue::String("00:00:00".into()));
        assert_eq!(
            interval(14_706_500_000, 3, 14),
            JsonValue::String("1 year 2 mons 3 days 04:05:06.5".into())
        );
        assert_eq!(
            interval(0, 1, 25),
            JsonValue::String("2 years 1 mon 1 day".into())
        );
        assert_eq!(
            interval(-3_600_000_000, 0, 0),
            JsonValue::String("-01:00:00".into())
        );
        assert_eq!(
            decoded(&Type::INTERVAL, b"short"),
            JsonValue::String("short".into())
        );
    }

    #[test]
    fn an_interval_writes_its_signs_as_postgresql_does() {
        fn interval(micros: i64, days: i32, months: i32) -> JsonValue {
            let mut body = micros.to_be_bytes().to_vec();
            body.extend_from_slice(&days.to_be_bytes());
            body.extend_from_slice(&months.to_be_bytes());
            decoded(&Type::INTERVAL, &body)
        }
        let string = |value: &str| JsonValue::String(value.into());

        // Only a count of exactly 1 takes the singular word.
        assert_eq!(interval(0, -1, 0), string("-1 days"));
        assert_eq!(interval(0, 0, -13), string("-1 years -1 mons"));
        // A positive part after a negative part takes a plus sign.
        assert_eq!(interval(7_200_000_000, -1, 0), string("-1 days +02:00:00"));
        assert_eq!(
            interval(3_600_000_000, -1, 1),
            string("1 mon -1 days +01:00:00")
        );
        assert_eq!(interval(0, 1, -1), string("-1 mons +1 day"));
        // A negative part after a negative part keeps its own sign alone.
        assert_eq!(interval(-60_000_000, -2, 0), string("-2 days -00:01:00"));
        // A positive part after a positive part takes no sign.
        assert_eq!(interval(1_000_000, 2, 0), string("2 days 00:00:01"));
        assert_eq!(interval(i64::MAX, i32::MAX, i32::MAX), string("infinity"));
        assert_eq!(interval(i64::MIN, i32::MIN, i32::MIN), string("-infinity"));
    }

    /// Joins the big-endian bytes of float8 values.
    fn float_body(values: &[f64]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_be_bytes())
            .collect()
    }

    #[test]
    fn a_geometric_value_shows_in_the_form_of_postgresql() {
        let string = |value: &str| JsonValue::String(value.into());
        assert_eq!(
            decoded(&Type::POINT, &float_body(&[1.0, -2.5])),
            string("(1,-2.5)")
        );
        assert_eq!(
            decoded(&Type::LSEG, &float_body(&[0.0, 0.0, 1.0, 1.0])),
            string("[(0,0),(1,1)]")
        );
        assert_eq!(
            decoded(&Type::BOX, &float_body(&[1.0, 1.0, 0.0, 0.0])),
            string("(1,1),(0,0)")
        );
        let mut closed = vec![1u8];
        closed.extend_from_slice(&2i32.to_be_bytes());
        closed.extend_from_slice(&float_body(&[0.0, 0.0, 1.0, 1.0]));
        assert_eq!(decoded(&Type::PATH, &closed), string("((0,0),(1,1))"));
        closed[0] = 0;
        assert_eq!(decoded(&Type::PATH, &closed), string("[(0,0),(1,1)]"));
        let mut polygon = 3i32.to_be_bytes().to_vec();
        polygon.extend_from_slice(&float_body(&[0.0, 0.0, 1.0, 0.0, 0.0, 1.0]));
        assert_eq!(
            decoded(&Type::POLYGON, &polygon),
            string("((0,0),(1,0),(0,1))")
        );
        assert_eq!(
            decoded(&Type::LINE, &float_body(&[1.0, -1.0, 0.0])),
            string("{1,-1,0}")
        );
        assert_eq!(
            decoded(&Type::CIRCLE, &float_body(&[0.0, 0.0, 5.0])),
            string("<(0,0),5>")
        );
        // A value that ends early, or that has bytes past its end, falls
        // back on the text rule.
        let short = float_body(&[1.0]);
        assert_eq!(decoded(&Type::POINT, &short), text_or_bytes(&short));
        let long = float_body(&[1.0, 2.0, 3.0]);
        assert_eq!(decoded(&Type::POINT, &long), text_or_bytes(&long));
        let negative = (-1i32).to_be_bytes();
        assert_eq!(decoded(&Type::POLYGON, &negative), text_or_bytes(&negative));
        assert_eq!(
            decoded(&Type::PATH, &[1, 0xFF, 0xFF, 0xFF, 0xFF]),
            text_or_bytes(&[1, 0xFF, 0xFF, 0xFF, 0xFF])
        );
        assert_eq!(decoded(&Type::PATH, &[]), text_or_bytes(&[]));
        // An array of points reads each element.
        assert_eq!(
            decoded(
                &Type::POINT_ARRAY,
                &array_body(&Type::POINT, &[1], &[Some(&float_body(&[1.0, 2.0]))])
            ),
            serde_json::json!(["(1,2)"])
        );
    }

    #[test]
    fn a_float_of_a_geometric_value_follows_the_rules_of_postgresql() {
        assert_eq!(float_text(1.5), "1.5");
        assert_eq!(float_text(0.1), "0.1");
        assert_eq!(float_text(-0.0), "-0");
        assert_eq!(float_text(0.0001), "0.0001");
        assert_eq!(float_text(0.000015), "1.5e-05");
        assert_eq!(float_text(1e14), "100000000000000");
        assert_eq!(float_text(1e15), "1e+15");
        assert_eq!(float_text(1.25e21), "1.25e+21");
        assert_eq!(float_text(1e-300), "1e-300");
        assert_eq!(float_text(f64::NAN), "NaN");
        assert_eq!(float_text(f64::INFINITY), "Infinity");
        assert_eq!(float_text(f64::NEG_INFINITY), "-Infinity");
    }

    #[test]
    fn a_log_place_a_row_place_and_a_snapshot_are_read() {
        let string = |value: &str| JsonValue::String(value.into());
        assert_eq!(
            decoded(&Type::PG_LSN, &0x16_B374_D848u64.to_be_bytes()),
            string("16/B374D848")
        );
        assert_eq!(decoded(&Type::PG_LSN, b"abc"), string("abc"));
        assert_eq!(
            decoded(&Type::XID8, &u64::MAX.to_be_bytes()),
            JsonValue::from(u64::MAX)
        );
        assert_eq!(decoded(&Type::XID8, b"abc"), string("abc"));
        let mut tid = 7u32.to_be_bytes().to_vec();
        tid.extend_from_slice(&3u16.to_be_bytes());
        assert_eq!(decoded(&Type::TID, &tid), string("(7,3)"));
        assert_eq!(decoded(&Type::TID, b"abc"), string("abc"));

        let snapshot = |running: &[u64]| {
            let mut body = (running.len() as i32).to_be_bytes().to_vec();
            body.extend_from_slice(&10u64.to_be_bytes());
            body.extend_from_slice(&20u64.to_be_bytes());
            for id in running {
                body.extend_from_slice(&id.to_be_bytes());
            }
            body
        };
        assert_eq!(
            decoded(&Type::PG_SNAPSHOT, &snapshot(&[])),
            string("10:20:")
        );
        assert_eq!(
            decoded(&Type::TXID_SNAPSHOT, &snapshot(&[12, 15])),
            string("10:20:12,15")
        );
        let negative = (-1i32).to_be_bytes();
        assert_eq!(
            decoded(&Type::PG_SNAPSHOT, &negative),
            text_or_bytes(&negative)
        );
        let mut short = snapshot(&[12]);
        short.truncate(20);
        assert_eq!(decoded(&Type::PG_SNAPSHOT, &short), text_or_bytes(&short));

        // A jsonpath value is a version byte and then the text.
        assert_eq!(decoded(&Type::JSONPATH, b"\x01$.a"), string("$.a"));
        assert_eq!(decoded(&Type::JSONPATH, b"$.a"), string("$.a"));
    }

    #[test]
    fn a_tsvector_shows_its_lexemes_positions_and_weights() {
        let mut body = 3i32.to_be_bytes().to_vec();
        body.extend_from_slice(b"cat\0");
        body.extend_from_slice(&0u16.to_be_bytes());
        body.extend_from_slice(b"it's\0");
        body.extend_from_slice(&1u16.to_be_bytes());
        body.extend_from_slice(&3u16.to_be_bytes());
        body.extend_from_slice(b"a\\b\0");
        body.extend_from_slice(&4u16.to_be_bytes());
        for entry in [0xC001u16, 0x8002, 0x4003, 0x0004] {
            body.extend_from_slice(&entry.to_be_bytes());
        }
        assert_eq!(
            decoded(&Type::TS_VECTOR, &body),
            JsonValue::String("'cat' 'it''s':3 'a\\\\b':1A,2B,3C,4".into())
        );
        assert_eq!(
            decoded(&Type::TS_VECTOR, &0i32.to_be_bytes()),
            JsonValue::String(String::new())
        );
        // A lexeme without its zero byte, or a position that is missing,
        // falls back on the text rule.
        let mut open = 1i32.to_be_bytes().to_vec();
        open.extend_from_slice(b"cat");
        assert_eq!(decoded(&Type::TS_VECTOR, &open), text_or_bytes(&open));
        let mut missing = 1i32.to_be_bytes().to_vec();
        missing.extend_from_slice(b"cat\0");
        missing.extend_from_slice(&1u16.to_be_bytes());
        assert_eq!(decoded(&Type::TS_VECTOR, &missing), text_or_bytes(&missing));
        let mut no_count = 1i32.to_be_bytes().to_vec();
        no_count.extend_from_slice(b"cat\0");
        assert_eq!(
            decoded(&Type::TS_VECTOR, &no_count),
            text_or_bytes(&no_count)
        );
        let mut not_text = 1i32.to_be_bytes().to_vec();
        not_text.extend_from_slice(&[0xFF, 0]);
        not_text.extend_from_slice(&0u16.to_be_bytes());
        assert_eq!(
            decoded(&Type::TS_VECTOR, &not_text),
            text_or_bytes(&not_text)
        );
        assert_eq!(decoded(&Type::TS_VECTOR, &[1]), text_or_bytes(&[1]));
    }

    /// The binary form of an operand of a `tsquery` value.
    fn query_value(word: &str, weight: u8, prefix: u8) -> Vec<u8> {
        let mut body = vec![TSQUERY_VALUE, weight, prefix];
        body.extend_from_slice(word.as_bytes());
        body.push(0);
        body
    }

    /// The binary form of a `tsquery` value of the given items.
    fn query_body(items: &[Vec<u8>]) -> Vec<u8> {
        let mut body = (items.len() as i32).to_be_bytes().to_vec();
        for item in items {
            body.extend_from_slice(item);
        }
        body
    }

    #[test]
    fn a_tsquery_shows_its_operators_and_parentheses_as_postgresql_does() {
        let text = |items: &[Vec<u8>]| decoded(&Type::TSQUERY, &query_body(items));
        let string = |value: &str| JsonValue::String(value.into());
        let and = vec![TSQUERY_OPERATOR, TSQUERY_AND];
        let or = vec![TSQUERY_OPERATOR, TSQUERY_OR];
        let not = vec![TSQUERY_OPERATOR, TSQUERY_NOT];
        let phrase = |distance: i16| {
            let mut item = vec![TSQUERY_OPERATOR, TSQUERY_PHRASE];
            item.extend_from_slice(&distance.to_be_bytes());
            item
        };
        let (fat, rat, cat) = (
            query_value("fat", 0, 0),
            query_value("rat", 0, 0),
            query_value("cat", 0, 0),
        );

        assert_eq!(text(&[]), string(""));
        assert_eq!(text(std::slice::from_ref(&fat)), string("'fat'"));
        // An operator comes first, then its right operand, then its left.
        assert_eq!(
            text(&[and.clone(), rat.clone(), fat.clone()]),
            string("'fat' & 'rat'")
        );
        // 'fat' & ( 'rat' | !'cat' )
        assert_eq!(
            text(&[
                and.clone(),
                or.clone(),
                not.clone(),
                cat.clone(),
                rat.clone(),
                fat.clone()
            ]),
            string("'fat' & ( 'rat' | !'cat' )")
        );
        // ( 'fat' | 'rat' ) & 'cat' and 'fat' | 'rat' & 'cat'.
        assert_eq!(
            text(&[
                and.clone(),
                cat.clone(),
                or.clone(),
                rat.clone(),
                fat.clone()
            ]),
            string("( 'fat' | 'rat' ) & 'cat'")
        );
        assert_eq!(
            text(&[
                or.clone(),
                and.clone(),
                cat.clone(),
                rat.clone(),
                fat.clone()
            ]),
            string("'fat' | 'rat' & 'cat'")
        );
        // A NOT of an operator puts the operator in parentheses.
        assert_eq!(
            text(&[not.clone(), and.clone(), rat.clone(), fat.clone()]),
            string("!( 'fat' & 'rat' )")
        );
        // A phrase on the right of a phrase goes in parentheses, and one on
        // the left does not.
        assert_eq!(
            text(&[phrase(1), phrase(2), cat.clone(), rat.clone(), fat.clone()]),
            string("'fat' <-> ( 'rat' <2> 'cat' )")
        );
        assert_eq!(
            text(&[phrase(1), cat.clone(), phrase(1), rat.clone(), fat.clone()]),
            string("'fat' <-> 'rat' <-> 'cat'")
        );
        // The weights and the prefix flag follow a colon, and a quote is
        // doubled.
        assert_eq!(
            text(&[query_value("it's", 0b1010, 1)]),
            string("'it''s':*AC")
        );
        assert_eq!(text(&[query_value("a", 0b0101, 0)]), string("'a':BD"));

        // An item of an unknown type, an unknown operator, an operator
        // without its operands, and a value with two roots fall back on the
        // text rule.
        let fallback = |items: &[Vec<u8>]| {
            let body = query_body(items);
            assert_eq!(decoded(&Type::TSQUERY, &body), text_or_bytes(&body));
        };
        fallback(&[vec![9]]);
        fallback(&[vec![TSQUERY_OPERATOR, 9], rat.clone(), fat.clone()]);
        fallback(&[and.clone(), fat.clone()]);
        fallback(std::slice::from_ref(&not));
        fallback(&[fat.clone(), rat.clone()]);
        fallback(&[vec![TSQUERY_VALUE, 0]]);
        fallback(&[vec![TSQUERY_OPERATOR, TSQUERY_PHRASE, 0]]);
        assert_eq!(decoded(&Type::TSQUERY, &[1]), text_or_bytes(&[1]));
    }

    #[test]
    fn an_hstore_value_shows_its_pairs() {
        let hstore = Type::new("hstore".into(), 16_400, Kind::Simple, "public".into());
        let pair = |key: &[u8], value: Option<&[u8]>| {
            let mut body = (key.len() as i32).to_be_bytes().to_vec();
            body.extend_from_slice(key);
            match value {
                Some(value) => {
                    body.extend_from_slice(&(value.len() as i32).to_be_bytes());
                    body.extend_from_slice(value);
                }
                None => body.extend_from_slice(&(-1i32).to_be_bytes()),
            }
            body
        };
        let mut body = 2i32.to_be_bytes().to_vec();
        body.extend_from_slice(&pair(b"a", Some(b"say \"hi\" \\")));
        body.extend_from_slice(&pair(b"b", None));
        assert_eq!(
            decoded(&hstore, &body),
            JsonValue::String("\"a\"=>\"say \\\"hi\\\" \\\\\", \"b\"=>NULL".into())
        );
        // A key that is NULL, and a key that is not text, fall back on the
        // text rule.
        let mut null_key = 1i32.to_be_bytes().to_vec();
        null_key.extend_from_slice(&(-1i32).to_be_bytes());
        assert_eq!(decoded(&hstore, &null_key), text_or_bytes(&null_key));
        let mut not_text = 1i32.to_be_bytes().to_vec();
        not_text.extend_from_slice(&pair(&[0xFF], None));
        assert_eq!(decoded(&hstore, &not_text), text_or_bytes(&not_text));
        let mut short = 1i32.to_be_bytes().to_vec();
        short.extend_from_slice(&5i32.to_be_bytes());
        assert_eq!(decoded(&hstore, &short), text_or_bytes(&short));
        assert_eq!(decoded(&hstore, &[1]), text_or_bytes(&[1]));
        // A type of another name keeps the text rule.
        let other = Type::new("citext".into(), 16_401, Kind::Simple, "public".into());
        assert_eq!(decoded(&other, b"Hi"), JsonValue::String("Hi".into()));
    }

    #[test]
    fn an_address_of_a_network_carries_its_mask() {
        let host = [2u8, 32, 0, 4, 10, 0, 0, 1];
        assert_eq!(
            decoded(&Type::INET, &host),
            JsonValue::String("10.0.0.1".into())
        );
        let network = [2u8, 24, 1, 4, 10, 0, 0, 0];
        assert_eq!(
            decoded(&Type::CIDR, &network),
            JsonValue::String("10.0.0.0/24".into())
        );
        let partial = [2u8, 24, 0, 4, 10, 0, 0, 1];
        assert_eq!(
            decoded(&Type::INET, &partial),
            JsonValue::String("10.0.0.1/24".into())
        );
        let mut six = vec![3u8, 128, 0, 16];
        six.extend_from_slice(&[0u8; 15]);
        six.push(1);
        assert_eq!(decoded(&Type::INET, &six), JsonValue::String("::1".into()));
        // A length that names no family, and a value that ends too early,
        // both fall back on the text rule.
        assert_eq!(
            decoded(&Type::INET, &[2u8, 8, 0, 1, 10]),
            JsonValue::String("\u{2}\u{8}\u{0}\u{1}\n".into())
        );
        assert_eq!(
            decoded(&Type::INET, &[2u8, 32, 0, 4]),
            JsonValue::String("\u{2} \u{0}\u{4}".into())
        );
        assert_eq!(
            decoded(&Type::INET, &[2u8]),
            JsonValue::String("\u{2}".into())
        );
    }

    #[test]
    fn a_hardware_address_shows_its_bytes_in_groups() {
        assert_eq!(
            decoded(&Type::MACADDR, &[0x08, 0x00, 0x2b, 0x01, 0x02, 0x03]),
            JsonValue::String("08:00:2b:01:02:03".into())
        );
        assert_eq!(
            decoded(&Type::MACADDR8, &[0x08, 0, 0x2b, 1, 2, 3, 4, 5]),
            JsonValue::String("08:00:2b:01:02:03:04:05".into())
        );
        assert_eq!(
            decoded(&Type::MACADDR, b"ab"),
            JsonValue::String("ab".into())
        );
    }

    #[test]
    fn the_list_of_tables_leaves_out_the_partitions() {
        assert!(TABLES_QUERY.contains("NOT c.relispartition"));
        assert!(TABLES_QUERY.contains("'p'"));
    }

    #[test]
    fn the_bits_of_a_trigger_name_its_time_and_its_events() {
        // A row trigger BEFORE INSERT OR UPDATE OF total, id.
        let before = trigger_of(
            "audit".into(),
            1 | 2 | 4 | 16,
            b'O' as i8,
            vec!["total".into(), "id".into()],
        );
        assert_eq!(before.name, "audit");
        assert_eq!(before.timing, TriggerTiming::Before);
        assert_eq!(
            before.events,
            vec![TriggerEvent::Insert, TriggerEvent::Update]
        );
        assert!(before.enabled);
        assert!(!before.replica);
        assert_eq!(before.update_columns, ["total", "id"]);
        // A statement trigger AFTER DELETE OR TRUNCATE, which is disabled.
        let after = trigger_of("purge".into(), 8 | 32, b'D' as i8, Vec::new());
        assert_eq!(after.timing, TriggerTiming::After);
        assert_eq!(
            after.events,
            vec![TriggerEvent::Delete, TriggerEvent::Truncate]
        );
        assert!(!after.enabled);
        assert!(!after.replica);
        // An INSTEAD OF trigger of a view.
        let instead = trigger_of("write".into(), 1 | 64 | 16, b'A' as i8, Vec::new());
        assert_eq!(instead.timing, TriggerTiming::InsteadOf);
        assert_eq!(instead.events, vec![TriggerEvent::Update]);
        assert!(instead.enabled);
        assert!(!instead.replica);
    }

    #[test]
    fn a_replica_trigger_is_not_enabled_and_is_marked_as_a_replica_trigger() {
        let replica = trigger_of("copy".into(), 4, b'R' as i8, Vec::new());
        assert!(!replica.enabled);
        assert!(replica.replica);
    }

    #[test]
    fn the_list_of_triggers_leaves_out_the_internal_ones_and_the_constraint_triggers() {
        assert!(TRIGGERS_QUERY.contains("NOT t.tgisinternal"));
        assert!(TRIGGERS_QUERY.contains("AND t.tgconstraint = 0"));
        assert!(TRIGGERS_QUERY.contains("unnest(t.tgattr) WITH ORDINALITY"));
        assert!(TRIGGERS_QUERY.contains("ORDER BY k.ord)"));
        assert!(TRIGGERS_QUERY.contains("WHERE n.nspname = $1 AND c.relname = $2"));
        assert!(TRIGGERS_QUERY.ends_with("ORDER BY t.tgname"));
    }

    #[test]
    fn the_create_statement_of_a_trigger_names_its_relation() {
        let trigger =
            object_query_text(Some("public"), "orders", "it's", ObjectType::Trigger).unwrap();
        assert_eq!(
            trigger.sql,
            "SELECT CASE WHEN pg_catalog.set_config('search_path', 'pg_catalog', true) \
             IS NOT NULL THEN pg_catalog.pg_get_triggerdef(t.oid, false) || ';' || \
             CASE t.tgenabled WHEN 'O' THEN '' ELSE \
             E'\\nALTER TABLE ' || t.tgrelid::pg_catalog.regclass::text || \
             CASE t.tgenabled WHEN 'D' THEN ' DISABLE' \
             WHEN 'R' THEN ' ENABLE REPLICA' ELSE ' ENABLE ALWAYS' END || \
             ' TRIGGER ' || pg_catalog.quote_ident(t.tgname) || ';' END END \
             FROM pg_catalog.pg_trigger AS t \
             WHERE t.tgrelid = '\"public\".\"orders\"'::regclass AND t.tgname = 'it''s';"
        );
        assert_eq!(trigger.column, 0);
        assert!(object_query_text(Some("public"), "orders", "e", ObjectType::Event).is_none());
    }

    #[test]
    fn the_create_statement_of_a_trigger_adds_the_statement_of_its_state() {
        let sql = object_query_text(Some("app"), "orders", "t", ObjectType::Trigger)
            .unwrap()
            .sql;
        assert!(sql.contains("CASE t.tgenabled WHEN 'O' THEN '' ELSE"));
        assert!(sql.contains("WHEN 'D' THEN ' DISABLE'"));
        assert!(sql.contains("WHEN 'R' THEN ' ENABLE REPLICA'"));
        assert!(sql.contains("ELSE ' ENABLE ALWAYS' END"));
        // The statement of the state follows the CREATE statement on a line
        // of its own, and each statement ends with one semicolon.
        assert!(sql.contains("|| ';' || CASE"));
        assert!(sql.contains("E'\\nALTER TABLE ' || t.tgrelid::pg_catalog.regclass::text"));
        assert!(sql.contains("|| pg_catalog.quote_ident(t.tgname) || ';' END END"));
    }

    #[test]
    fn the_letter_of_a_relation_names_its_type() {
        for (letter, relation_type) in [
            (b'r', RelationType::Table),
            (b'v', RelationType::View),
            (b'm', RelationType::MaterializedView),
            (b'p', RelationType::PartitionedTable),
            (b'f', RelationType::ForeignTable),
            (b'S', RelationType::Table),
        ] {
            assert_eq!(
                relation_of("r".into(), letter as i8).relation_type,
                relation_type
            );
        }
        assert_eq!(relation_of("orders".into(), b'r' as i8).name, "orders");
    }

    #[test]
    fn the_list_of_routines_names_each_overload_with_its_arguments() {
        assert!(ROUTINES_QUERY.contains("pg_catalog.pg_proc"));
        assert!(ROUTINES_QUERY.contains("pg_get_function_identity_arguments(p.oid)"));
        assert!(ROUTINES_QUERY.contains("WHEN 'p' THEN 'PROCEDURE'"));
        assert!(!ROUTINES_QUERY.contains("information_schema"));
    }

    #[test]
    fn the_list_of_indexes_reads_the_included_columns_and_the_expressions() {
        assert!(INDEXES_QUERY.contains("k.ord > idx.indnkeyatts"));
        assert!(INDEXES_QUERY.contains("pg_get_indexdef(idx.indexrelid, k.ord::int, true)"));
    }

    #[test]
    fn a_row_of_an_index_goes_to_the_key_or_to_the_included_columns() {
        let mut indexes = Vec::new();
        add_postgres_index_column(
            &mut indexes,
            "ix".into(),
            true,
            false,
            Some("a".into()),
            false,
        );
        add_postgres_index_column(
            &mut indexes,
            "ix".into(),
            true,
            false,
            Some("lower(b)".into()),
            false,
        );
        add_postgres_index_column(
            &mut indexes,
            "ix".into(),
            true,
            false,
            Some("c".into()),
            true,
        );
        add_postgres_index_column(&mut indexes, "ix".into(), true, false, None, true);
        assert_eq!(indexes.len(), 1);
        assert_eq!(
            indexes[0].columns,
            vec!["a".to_string(), "lower(b)".to_string()]
        );
        assert_eq!(indexes[0].included, vec!["c".to_string()]);
        assert!(indexes[0].unique);
    }

    #[test]
    fn the_partitions_show_their_bounds_up_to_the_limit() {
        assert!(PARTITIONS_QUERY.contains("c.relispartition"));
        assert!(PARTITIONS_QUERY.ends_with("LIMIT $3"));

        let list = partition_list(vec![
            (
                "orders_2024".to_string(),
                Some("FOR VALUES FROM ('2024-01-01') TO ('2025-01-01')".to_string()),
            ),
            ("orders_rest".to_string(), None),
        ]);
        assert!(!list.truncated);
        assert_eq!(
            list.partitions[0].values,
            "orders_2024 FOR VALUES FROM ('2024-01-01') TO ('2025-01-01')"
        );
        assert_eq!(list.partitions[1].values, "orders_rest");

        let rows = (0..=PARTITION_LIMIT)
            .map(|index| (format!("p{index}"), Some("DEFAULT".to_string())))
            .collect();
        let list = partition_list(rows);
        assert!(list.truncated);
        assert_eq!(list.partitions.len(), PARTITION_LIMIT);
    }

    #[test]
    fn the_snapshot_reads_one_column_past_its_limit() {
        let text = snapshot_query(10);
        assert!(text.ends_with("LIMIT 11"));
        assert!(text.contains("'m'"));
        assert!(text.contains("NOT c.relispartition"));
        // The letter of the relation type goes to the snapshot as the catalog keeps it.
        assert!(text.starts_with("SELECT n.nspname, c.relname, c.relkind, a.attname"));
        assert!(snapshot_query(usize::MAX).ends_with(&format!("LIMIT {}", i64::MAX)));
    }

    #[test]
    fn a_time_with_a_zone_shows_its_offset() {
        let mut body = 36_000_000_000i64.to_be_bytes().to_vec();
        body.extend_from_slice(&(-7_200i32).to_be_bytes());
        assert_eq!(
            decoded(&Type::TIMETZ, &body),
            JsonValue::String("10:00:00+02".into())
        );
        let mut body = 1_500_000i64.to_be_bytes().to_vec();
        body.extend_from_slice(&12_600i32.to_be_bytes());
        assert_eq!(
            decoded(&Type::TIMETZ, &body),
            JsonValue::String("00:00:01.5-03:30".into())
        );
        assert_eq!(
            decoded(&Type::TIMETZ, b"short"),
            JsonValue::String("short".into())
        );
    }

    #[test]
    fn a_bit_string_shows_its_digits() {
        let mut body = 10i32.to_be_bytes().to_vec();
        body.extend_from_slice(&[0b1010_0000, 0b0100_0000]);
        assert_eq!(
            decoded(&Type::VARBIT, &body),
            JsonValue::String("1010000001".into())
        );
        assert_eq!(
            decoded(&Type::BIT, &0i32.to_be_bytes()),
            JsonValue::String(String::new())
        );
        assert_eq!(
            decoded(&Type::BIT, &(-1i32).to_be_bytes()),
            JsonValue::String(base64_text(&(-1i32).to_be_bytes()))
        );
        assert_eq!(
            decoded(&Type::BIT, &9i32.to_be_bytes()),
            text_or_bytes(&9i32.to_be_bytes())
        );
        assert_eq!(decoded(&Type::BIT, b"ab"), JsonValue::String("ab".into()));
    }

    #[test]
    fn a_catalog_reference_shows_its_number() {
        assert_eq!(
            decoded(&Type::REGCLASS, &16_384u32.to_be_bytes()),
            JsonValue::from(16_384u32)
        );
        assert_eq!(
            decoded(&Type::XID, &u32::MAX.to_be_bytes()),
            JsonValue::from(u32::MAX)
        );
        assert_eq!(
            decoded(&Type::REGTYPE, b"abcde"),
            JsonValue::String("abcde".into())
        );
    }

    #[test]
    fn an_array_keeps_its_order_its_nulls_and_its_dimensions() {
        let one = 1i32.to_be_bytes();
        let two = 2i32.to_be_bytes();
        let body = array_body(&Type::INT4, &[3], &[Some(&one), None, Some(&two)]);
        assert_eq!(
            decoded(&Type::INT4_ARRAY, &body),
            serde_json::json!([1, null, 2])
        );

        let nested = array_body(&Type::INT4, &[2, 1], &[Some(&one), Some(&two)]);
        assert_eq!(
            decoded(&Type::INT4_ARRAY, &nested),
            serde_json::json!([[1], [2]])
        );

        let empty = array_body(&Type::INT4, &[], &[]);
        assert_eq!(decoded(&Type::INT4_ARRAY, &empty), serde_json::json!([]));
    }

    #[test]
    fn an_array_of_a_form_the_reader_cannot_use_falls_back_on_the_text_rule() {
        // A count of the dimensions and nothing else.
        assert_eq!(
            decoded(&Type::INT4_ARRAY, &1i32.to_be_bytes()),
            JsonValue::String("\u{0}\u{0}\u{0}\u{1}".into())
        );
        // A dimension of a negative length.
        let mut body = 1i32.to_be_bytes().to_vec();
        body.extend_from_slice(&0i32.to_be_bytes());
        body.extend_from_slice(&Type::INT4.oid().to_be_bytes());
        body.extend_from_slice(&(-1i32).to_be_bytes());
        body.extend_from_slice(&1i32.to_be_bytes());
        assert!(matches!(
            decoded(&Type::INT4_ARRAY, &body),
            JsonValue::String(_)
        ));
        // A dimension that names more elements than the value holds.
        let short = array_body(&Type::INT4, &[2], &[Some(&1i32.to_be_bytes())]);
        assert!(matches!(
            decoded(&Type::INT4_ARRAY, &short),
            JsonValue::String(_)
        ));
        // A value that ends inside the header.
        assert_eq!(
            decoded(&Type::INT4_ARRAY, b"x"),
            JsonValue::String("x".into())
        );
    }

    #[test]
    fn a_range_shows_its_bounds_and_the_form_of_its_ends() {
        let data_type = range_type(Type::INT4);
        let one = 1i32.to_be_bytes();
        let ten = 10i32.to_be_bytes();
        assert_eq!(
            decoded(&data_type, &range_body(RANGE_LOWER_CLOSED, &[&one, &ten])),
            JsonValue::String("[1,10)".into())
        );
        assert_eq!(
            decoded(
                &data_type,
                &range_body(RANGE_LOWER_CLOSED | RANGE_UPPER_CLOSED, &[&one, &ten])
            ),
            JsonValue::String("[1,10]".into())
        );
        assert_eq!(
            decoded(&data_type, &[RANGE_EMPTY]),
            JsonValue::String("empty".into())
        );
        assert_eq!(
            decoded(
                &data_type,
                &range_body(RANGE_LOWER_OPEN_END | RANGE_UPPER_OPEN_END, &[])
            ),
            JsonValue::String("(,)".into())
        );
        // A range without its bounds falls back on the text rule.
        assert_eq!(
            decoded(&data_type, &[RANGE_LOWER_CLOSED]),
            JsonValue::String("\u{2}".into())
        );
        assert_eq!(decoded(&data_type, &[]), JsonValue::String(String::new()));
    }

    #[test]
    fn a_multirange_holds_its_ranges_in_braces() {
        let element = range_type(Type::INT4);
        let data_type = Type::new(
            "int4multirange".to_string(),
            4451,
            Kind::Multirange(Type::INT4),
            "pg_catalog".to_string(),
        );
        let one = 1i32.to_be_bytes();
        let ten = 10i32.to_be_bytes();
        let first = range_body(RANGE_LOWER_CLOSED, &[&one, &ten]);
        let mut body = 1i32.to_be_bytes().to_vec();
        body.extend_from_slice(&element_body(Some(&first)));
        assert_eq!(
            decoded(&data_type, &body),
            JsonValue::String("{[1,10)}".into())
        );
        assert_eq!(
            decoded(&data_type, &0i32.to_be_bytes()),
            JsonValue::String("{}".into())
        );
        // A count that names a range the value does not hold.
        assert_eq!(
            decoded(&data_type, &1i32.to_be_bytes()),
            JsonValue::String("\u{0}\u{0}\u{0}\u{1}".into())
        );
        assert_eq!(decoded(&data_type, b"ab"), JsonValue::String("ab".into()));
        // The element type of the multirange reads on its own as well.
        assert_eq!(
            decoded(&element, &[RANGE_EMPTY]),
            JsonValue::String("empty".into())
        );
    }

    #[test]
    fn a_composite_shows_its_fields_in_order() {
        let data_type = Type::new(
            "pair".to_string(),
            17000,
            Kind::Composite(vec![
                Field::new("id".to_string(), Type::INT4),
                Field::new("name".to_string(), Type::TEXT),
            ]),
            "public".to_string(),
        );
        let mut body = 2i32.to_be_bytes().to_vec();
        body.extend_from_slice(&Type::INT4.oid().to_be_bytes());
        body.extend_from_slice(&element_body(Some(&1i32.to_be_bytes())));
        body.extend_from_slice(&Type::TEXT.oid().to_be_bytes());
        body.extend_from_slice(&element_body(Some(b"two")));
        assert_eq!(
            decoded(&data_type, &body),
            JsonValue::String("(1,two)".into())
        );

        // A count that does not match the fields of the type, and a value
        // that ends too early, both fall back on the text rule.
        assert_eq!(
            decoded(&data_type, &1i32.to_be_bytes()),
            JsonValue::String("\u{0}\u{0}\u{0}\u{1}".into())
        );
        assert_eq!(
            decoded(&data_type, &2i32.to_be_bytes()),
            JsonValue::String("\u{0}\u{0}\u{0}\u{2}".into())
        );
        assert_eq!(decoded(&data_type, b"ab"), JsonValue::String("ab".into()));
    }

    #[test]
    fn a_field_of_a_composite_that_holds_no_value_shows_as_empty() {
        let data_type = Type::new(
            "one".to_string(),
            17001,
            Kind::Composite(vec![Field::new("name".to_string(), Type::TEXT)]),
            "public".to_string(),
        );
        let mut body = 1i32.to_be_bytes().to_vec();
        body.extend_from_slice(&Type::TEXT.oid().to_be_bytes());
        body.extend_from_slice(&element_body(None));
        assert_eq!(decoded(&data_type, &body), JsonValue::String("()".into()));
    }

    #[test]
    fn an_enumerated_value_and_a_domain_carry_the_text_of_the_label() {
        let enumerated = Type::new(
            "mood".to_string(),
            17002,
            Kind::Enum(vec!["happy".to_string()]),
            "public".to_string(),
        );
        assert_eq!(
            decoded(&enumerated, b"happy"),
            JsonValue::String("happy".into())
        );
        let domain = Type::new(
            "positive".to_string(),
            17003,
            Kind::Domain(Type::INT4),
            "public".to_string(),
        );
        assert_eq!(decoded(&domain, &7i32.to_be_bytes()), JsonValue::from(7));
    }

    #[test]
    fn bytes_that_no_reader_understands_show_as_text_or_as_base64() {
        let unknown = Type::new(
            "point".to_string(),
            600,
            Kind::Simple,
            "pg_catalog".to_string(),
        );
        assert_eq!(decoded(&unknown, b"text"), JsonValue::String("text".into()));
        assert_eq!(
            decoded(&unknown, &[0xff, 0xfe]),
            JsonValue::String("//4=".into())
        );
        // A value of a known type that the target type refuses follows the
        // same rule.
        assert_eq!(decoded(&Type::INT4, b"ab"), JsonValue::String("ab".into()));
    }

    #[test]
    fn the_plan_keyword_names_the_form_of_the_answer() {
        assert_eq!(plan_prefix(PlanMode::Estimated), "EXPLAIN (FORMAT TEXT)");
        assert_eq!(plan_prefix(PlanMode::Actual), "EXPLAIN (ANALYZE, BUFFERS)");
    }

    #[test]
    fn the_notice_buffer_keeps_the_first_notices_and_counts_the_rest() {
        let mut notices = Notices::default();
        for index in 0..NOTICE_LIMIT + 2 {
            notices.push(Message::info(format!("n{index}")));
        }
        let taken = notices.take();
        assert_eq!(taken.len(), NOTICE_LIMIT + 1);
        assert_eq!(taken[0].text, "n0");
        assert_eq!(
            taken[NOTICE_LIMIT - 1].text,
            format!("n{}", NOTICE_LIMIT - 1)
        );
        let last = &taken[NOTICE_LIMIT];
        assert_eq!(last.level, MessageLevel::Warning);
        assert!(last.text.starts_with("2 more notices were dropped."));

        // The take empties the buffer and its count.
        notices.push(Message::info("next"));
        let taken = notices.take();
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].text, "next");
        assert!(notices.take().is_empty());

        assert!(dropped_notices_message(1)
            .text
            .starts_with("1 more notice was dropped."));
    }

    #[test]
    fn the_severity_of_a_notice_decides_its_level() {
        for (severity, level) in [
            ("NOTICE", MessageLevel::Info),
            ("INFO", MessageLevel::Info),
            ("DEBUG", MessageLevel::Info),
            ("WARNING", MessageLevel::Warning),
            ("EXCEPTION", MessageLevel::Warning),
            ("ERROR", MessageLevel::Error),
            ("FATAL", MessageLevel::Error),
            ("PANIC", MessageLevel::Error),
        ] {
            let message = notice_message_from(severity, "00000", "text", None, None);
            assert_eq!(message.level, level, "{severity}");
        }
    }

    #[test]
    fn a_notice_carries_its_text_and_what_the_server_added() {
        let message = notice_message_from(
            "WARNING",
            "22001",
            "the value was cut",
            Some("the column holds ten letters"),
            Some("make the column wider"),
        );
        assert_eq!(message.text, "the value was cut");
        let detail = message.detail.unwrap();
        assert_eq!(
            detail,
            "WARNING \u{b7} 22001 \u{b7} the column holds ten letters \u{b7} make the column wider"
        );

        // A notice that carries neither a detail nor a hint keeps the two
        // fields it always holds.
        let short = notice_message_from("NOTICE", "00000", "done", None, Some(""));
        assert_eq!(short.detail.as_deref(), Some("NOTICE \u{b7} 00000"));
    }

    #[test]
    fn the_create_statement_covers_a_view_alone() {
        let view = create_query_text(Some("public"), "v", RelationType::View).unwrap();
        assert_eq!(
            view.sql,
            "SELECT CASE WHEN pg_catalog.set_config('search_path', 'pg_catalog', true) \
             IS NOT NULL THEN 'CREATE ' || CASE c.relkind \
             WHEN 'm' THEN 'MATERIALIZED VIEW ' ELSE 'OR REPLACE VIEW ' END || \
             pg_catalog.quote_ident(n.nspname) || '.' || pg_catalog.quote_ident(c.relname) || \
             E' AS\\n' || pg_catalog.rtrim(pg_catalog.pg_get_viewdef(c.oid, true), ';') || \
             CASE WHEN c.relispopulated THEN ';' ELSE E'\\nWITH NO DATA;' END || \
             COALESCE((SELECT pg_catalog.string_agg(E'\\n\\n' || \
             pg_catalog.pg_get_indexdef(i.indexrelid) || ';', '' ORDER BY x.relname) \
             FROM pg_catalog.pg_index AS i \
             JOIN pg_catalog.pg_class AS x ON x.oid = i.indexrelid \
             WHERE i.indrelid = c.oid), '') END \
             FROM pg_catalog.pg_class AS c \
             JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace \
             WHERE c.oid = '\"public\".\"v\"'::regclass;"
        );
        assert_eq!(view.column, 0);
        // A materialized view reads the same statement, and the catalog
        // gives the clause of its type.
        let materialized =
            create_query_text(Some("public"), "v", RelationType::MaterializedView).unwrap();
        assert_eq!(materialized, view);
        for relation_type in [
            RelationType::Table,
            RelationType::PartitionedTable,
            RelationType::ForeignTable,
        ] {
            assert!(create_query_text(Some("public"), "t", relation_type).is_none());
        }
    }
    use crate::storage::{ConnectionOptions, DbType};

    fn connection() -> SavedConnection {
        SavedConnection {
            id: "id".into(),
            name: "name".into(),
            db_type: DbType::Postgres,
            host: Some("pg.example.com".into()),
            port: Some(5433),
            user: Some("app".into()),
            database: Some("shop".into()),
            password: Some("p@ss word".into()),
            aws_secret_access_key: None,
            aws_session_token: None,
            options: ConnectionOptions::default(),
            color: None,
            group: None,
        }
    }

    #[test]
    fn the_configuration_keeps_the_host_the_port_and_the_credentials() {
        let config = build_config(&connection()).unwrap();
        assert_eq!(config.get_ports(), &[5433]);
        assert_eq!(config.get_user(), Some("app"));
        assert_eq!(config.get_dbname(), Some("shop"));
        assert_eq!(config.get_connect_timeout(), Some(&Duration::from_secs(15)));
    }

    #[test]
    fn the_configuration_falls_back_to_the_default_port() {
        let mut input = connection();
        input.port = None;
        assert_eq!(build_config(&input).unwrap().get_ports(), &[5432]);
    }

    #[test]
    fn empty_credentials_are_left_out() {
        let mut input = connection();
        input.user = Some(String::new());
        input.password = Some(String::new());
        input.database = Some(String::new());
        input.options.application_name = Some("  ".into());
        let config = build_config(&input).unwrap();
        assert_eq!(config.get_user(), None);
        assert_eq!(config.get_dbname(), None);
        assert_eq!(config.get_application_name(), None);
    }

    #[test]
    fn a_read_only_connection_sets_the_server_option() {
        let mut input = connection();
        input.options.read_only = true;
        let config = build_config(&input).unwrap();
        assert_eq!(
            config.get_options(),
            Some("-c default_transaction_read_only=on")
        );
    }

    #[test]
    fn a_connection_string_replaces_the_fields() {
        let mut input = connection();
        input.options.connection_url = Some("postgresql://u:p@other.example.com:5555/other".into());
        let config = build_config(&input).unwrap();
        assert_eq!(config.get_ports(), &[5555]);
        assert_eq!(config.get_dbname(), Some("other"));
    }

    #[test]
    fn a_connection_string_takes_the_fields_that_it_does_not_give() {
        use tokio_postgres::config::SslMode;
        let mut input = connection();
        input.options.connection_url = Some("postgresql://h/d".into());
        let config = build_config(&input).unwrap();
        assert_eq!(config.get_user(), Some("app"));
        assert_eq!(config.get_password(), Some(&b"p@ss word"[..]));
        assert_eq!(config.get_connect_timeout(), Some(&Duration::from_secs(15)));
        assert_eq!(config.get_ssl_mode(), SslMode::Require);

        // The values of the string win over the fields of the record.
        input.options.connection_url =
            Some("postgresql://u:own@h/d?connect_timeout=3&sslmode=disable".into());
        let config = build_config(&input).unwrap();
        assert_eq!(config.get_user(), Some("u"));
        assert_eq!(config.get_password(), Some(&b"own"[..]));
        assert_eq!(config.get_connect_timeout(), Some(&Duration::from_secs(3)));
        assert_eq!(config.get_ssl_mode(), SslMode::Disable);

        // Empty fields of the record add nothing.
        input.options.connection_url = Some("host=h dbname=d".into());
        input.user = Some(String::new());
        input.password = None;
        let config = build_config(&input).unwrap();
        assert_eq!(config.get_user(), None);
        assert_eq!(config.get_password(), None);
    }

    #[test]
    fn a_connection_probes_its_socket_after_a_minute_of_silence() {
        let mut input = connection();
        let config = build_config(&input).unwrap();
        assert!(config.get_keepalives());
        assert_eq!(config.get_keepalives_idle(), KEEPALIVE_IDLE);
        assert_eq!(config.get_keepalives_interval(), Some(KEEPALIVE_INTERVAL));

        // A connection string with no keepalive values takes those of the
        // application.
        input.options.connection_url = Some("postgresql://h/d".into());
        let config = build_config(&input).unwrap();
        assert_eq!(config.get_keepalives_idle(), KEEPALIVE_IDLE);
        assert_eq!(config.get_keepalives_interval(), Some(KEEPALIVE_INTERVAL));

        // The values of the string win.
        input.options.connection_url =
            Some("postgresql://h/d?keepalives=1&keepalives_idle=300".into());
        let config = build_config(&input).unwrap();
        assert_eq!(config.get_keepalives_idle(), Duration::from_secs(300));
        assert_eq!(config.get_keepalives_interval(), None);
    }

    #[test]
    fn a_password_in_a_connection_string_is_found() {
        assert!(string_has_password("postgresql://u:p@h/d").unwrap());
        assert!(string_has_password("host=h password=p").unwrap());
        assert!(!string_has_password("postgresql://u@h/d").unwrap());
        assert!(!string_has_password("host=h password=''").unwrap());
        assert!(string_has_password("host=").is_err());
    }

    #[test]
    fn a_connection_string_keeps_the_read_only_option() {
        let mut input = connection();
        input.options.read_only = true;
        input.options.connection_url = Some("postgresql://u@h/d".into());
        assert_eq!(
            build_config(&input).unwrap().get_options(),
            Some("-c default_transaction_read_only=on")
        );
        input.options.connection_url =
            Some("postgresql://u@h/d?options=-c%20search_path%3Dx".into());
        assert_eq!(
            build_config(&input).unwrap().get_options(),
            Some("-c search_path=x -c default_transaction_read_only=on")
        );
        input.options.read_only = false;
        assert_eq!(
            build_config(&input).unwrap().get_options(),
            Some("-c search_path=x")
        );
    }

    #[test]
    fn a_connection_string_that_is_not_valid_gives_an_error() {
        let mut input = connection();
        input.options.connection_url = Some("host=".into());
        assert_eq!(
            build_config(&input).unwrap_err().category(),
            crate::error::ErrorCategory::Configuration
        );
    }

    #[test]
    fn the_transport_setting_selects_the_mode() {
        use tokio_postgres::config::SslMode;
        assert_eq!(ssl_mode(TlsMode::Disable), SslMode::Disable);
        assert_eq!(ssl_mode(TlsMode::Prefer), SslMode::Prefer);
        assert_eq!(ssl_mode(TlsMode::Require), SslMode::Require);
        assert_eq!(ssl_mode(TlsMode::VerifyFull), SslMode::Require);
    }

    #[test]
    fn the_tls_settings_follow_the_transport_setting() {
        let mut input = connection();
        input.options.tls_mode = TlsMode::Require;
        assert!(build_tls_config(&input).is_ok());

        input.options.tls_mode = TlsMode::VerifyFull;
        assert!(build_tls_config(&input).is_ok());
    }

    #[test]
    fn the_mozilla_roots_stand_in_when_the_system_gives_no_usable_root() {
        let bundled = webpki_roots::TLS_SERVER_ROOTS.len();
        assert_eq!(roots_from(&[]).len(), bundled);
        let system = system_roots();
        if !system.is_empty() {
            assert_eq!(roots_from(system).len(), system.len());
        }
    }

    #[test]
    fn a_certificate_authority_file_that_is_missing_gives_an_error() {
        let mut input = connection();
        input.options.ca_cert_path = Some("/does/not/exist.pem".into());
        assert_eq!(
            build_tls_config(&input).unwrap_err().category(),
            crate::error::ErrorCategory::Io
        );

        input.options.ca_cert_path = Some("   ".into());
        assert!(build_tls_config(&input).is_ok());
    }

    #[test]
    fn a_certificate_authority_file_without_a_readable_certificate_gives_an_error() {
        let path = std::env::temp_dir().join(format!("pg-ca-{}.pem", std::process::id()));
        std::fs::write(
            &path,
            "-----BEGIN CERTIFICATE-----\n!!!\n-----END CERTIFICATE-----\n",
        )
        .unwrap();
        let mut input = connection();
        input.options.ca_cert_path = Some(path.to_string_lossy().into_owned());
        let error = build_tls_config(&input).unwrap_err();
        std::fs::remove_file(&path).unwrap();
        assert!(matches!(error, Error::Configuration(ref text) if text.contains("no certificate")));
    }

    #[test]
    fn a_certificate_file_gives_one_entry_for_each_block() {
        let pem = "-----BEGIN CERTIFICATE-----\nAAEC\n-----END CERTIFICATE-----\n\
                   -----BEGIN CERTIFICATE-----\nAwQF\n-----END CERTIFICATE-----\n";
        let certificates = rustls_pemfile_certs(pem.as_bytes());
        assert_eq!(certificates.len(), 2);
        assert_eq!(certificates[0].as_ref(), &[0x00, 0x01, 0x02]);
        assert_eq!(certificates[1].as_ref(), &[0x03, 0x04, 0x05]);
    }

    #[test]
    fn a_file_without_pem_blocks_counts_as_one_binary_certificate() {
        let certificates = rustls_pemfile_certs(&[1, 2, 3]);
        assert_eq!(certificates.len(), 1);
        assert_eq!(certificates[0].as_ref(), &[1, 2, 3]);
    }

    #[test]
    fn a_block_that_is_not_base64_is_left_out() {
        let pem = "-----BEGIN CERTIFICATE-----\n!!!!\n-----END CERTIFICATE-----\n";
        assert!(rustls_pemfile_certs(pem.as_bytes()).is_empty());
    }

    #[test]
    fn the_parameters_accept_every_json_type() {
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
            crate::db::QueryParam {
                value: serde_json::json!({ "a": 1 }),
            },
        ];
        let bound = bind_params(&params).unwrap();
        let texts: Vec<Option<&str>> = bound.iter().map(|param| param.0.as_deref()).collect();
        assert_eq!(
            texts,
            vec![
                Some("text"),
                Some("7"),
                Some("1.5"),
                Some("true"),
                None,
                Some("{\"a\":1}"),
            ]
        );
        assert!(bind_params(&Vec::new()).unwrap().is_empty());
    }

    /// The width of the column decides the type of the parameter, and the
    /// text form binds against each width. A parameter of the type `int4`
    /// then takes a whole number that the caller sent as JSON.
    #[test]
    fn a_whole_number_binds_against_every_integer_width() {
        let param = TextParam(Some("7".to_string()));
        for column_type in [Type::INT2, Type::INT4, Type::INT8, Type::NUMERIC] {
            let mut out = BytesMut::new();
            assert!(matches!(
                param.to_sql_checked(&column_type, &mut out).unwrap(),
                IsNull::No
            ));
            assert_eq!(&out[..], b"7");
            assert!(matches!(param.encode_format(&column_type), Format::Text));
        }
    }

    #[test]
    fn a_null_parameter_writes_no_bytes() {
        let param = TextParam(None);
        let mut out = BytesMut::new();
        assert!(matches!(
            param.to_sql_checked(&Type::INT4, &mut out).unwrap(),
            IsNull::Yes
        ));
        assert!(out.is_empty());
        assert_eq!(format!("{param:?}"), "NULL");
        assert_eq!(format!("{:?}", TextParam(Some("a".to_string()))), "\"a\"");
    }

    #[test]
    fn a_number_that_is_too_large_is_refused() {
        let params = vec![crate::db::QueryParam {
            value: serde_json::json!(18446744073709551615u64),
        }];
        assert_eq!(
            bind_params(&params).unwrap_err().category(),
            crate::error::ErrorCategory::Configuration
        );
    }
}
