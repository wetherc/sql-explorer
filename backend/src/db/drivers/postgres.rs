//! The PostgreSQL driver.
//!
//! A script without parameters goes through the simple protocol. That
//! protocol accepts more than one statement in one call, it reports the
//! command tag of each statement, and it returns every value as text, so no
//! type mapping can fail.

use crate::db::drivers::{
    add_constraint_column, add_index_column, add_snapshot_column, bytes_to_json, constraint_kind,
    f32_to_json, f64_to_json, number_out_of_range, number_value, prefixed_plan, routine_kind,
    rows_affected_message, rows_returned_message, size_text, table_kind, CancelHandle,
    DatabaseDriver, NumberValue,
};
use crate::db::sink::{RowSink, RunSummary, SinkControl};
use crate::db::{
    AppColumn, ColumnInfo, Constraint, CreateQuery, Database, DriverCapabilities, ExecOptions,
    IndexInfo, Message, MessageLevel, PlanKind, QueryParams, QueryResponse, Routine, Schema,
    SchemaSnapshot, SnapshotColumn, Table, TableFact, TableKind,
};
use crate::error::{Error, Result};
use crate::sql::{only_reads, split_statements, Dialect};
use crate::storage::{SavedConnection, TlsMode};
use async_trait::async_trait;
use bytes::BytesMut;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use futures_util::{pin_mut, stream, StreamExt, TryStreamExt};
use postgres_types::{to_sql_checked, Field, Format, FromSql, IsNull, Kind, ToSql, Type};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use serde_json::Value as JsonValue;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_postgres::error::{DbError, SqlState};
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
    add_read_only_option(&mut config, connection);
    Ok(config)
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

    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    if let Some(path) = connection
        .options
        .ca_cert_path
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        let bytes = std::fs::read(path)?;
        for certificate in rustls_pemfile_certs(&bytes) {
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
        let (client, mut io) = tokio::time::timeout(limit, config.connect(tls.clone()))
            .await
            .map_err(|_| Error::Timeout(limit.as_secs()))??;

        // The notices of the server arrive on the connection object and not
        // with the result of a statement, so the task that drives the socket
        // is a stream of messages and not a future. Each notice goes into a
        // buffer that the driver drains into the answer of the next run.
        let notices: NoticeBuffer = Arc::new(Mutex::new(Vec::new()));
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

    /// Takes the notices that arrived since the last run.
    fn take_notices(&self) -> Vec<Message> {
        match self.notices.lock() {
            Ok(mut buffer) => std::mem::take(&mut *buffer),
            // The lock breaks only when a holder panicked, and a lost notice
            // must not stop the run itself.
            Err(_) => Vec::new(),
        }
    }
}

/// The notices that wait for the next answer.
type NoticeBuffer = Arc<Mutex<Vec<Message>>>;

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
            supports_partitions: false,
            supports_explain: true,
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
        kind: TableKind,
    ) -> Option<CreateQuery> {
        create_query_text(schema, table, kind)
    }

    async fn ping(&mut self) -> Result<()> {
        self.client.simple_query("SELECT 1").await?;
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
    /// The notices of the server reach the sink after the run, in their
    /// arrival order.
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

        let notices = self.take_notices();
        let rows_affected = match outcome {
            Ok(rows_affected) => rows_affected,
            Err(error) => {
                // A run that failed carries the error and no answer, so the
                // notices of that run reach the log alone.
                for notice in &notices {
                    log::info!("The PostgreSQL server said: {}", notice.text);
                }
                return Err(error);
            }
        };
        for notice in notices {
            sink.message(notice);
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
        let statement = prefixed_plan(query, Dialect::Postgres, plan_prefix(kind))?;
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
        let rows = self
            .client
            .query(
                "SELECT c.relname, c.relkind \
                 FROM pg_catalog.pg_class AS c \
                 JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace \
                 WHERE n.nspname = $1 AND c.relkind IN ('r', 'p', 'v', 'm', 'f') \
                 ORDER BY c.relkind, c.relname",
                &[&schema],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|row| {
                let name: String = row.get(0);
                let kind: i8 = row.get(1);
                if kind == b'v' as i8 || kind == b'm' as i8 {
                    Table::view(name)
                } else {
                    Table::table(name)
                }
            })
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
                        COALESCE(i.indisprimary, false) \
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
        let rows = self
            .client
            .query(
                "SELECT c.table_schema, c.table_name, t.table_type, c.column_name, c.data_type \
                 FROM information_schema.columns AS c \
                 JOIN information_schema.tables AS t \
                   ON t.table_schema = c.table_schema AND t.table_name = c.table_name \
                 WHERE c.table_schema NOT IN ('pg_catalog', 'information_schema') \
                 ORDER BY c.table_schema, c.table_name, c.ordinal_position",
                &[],
            )
            .await?;
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
                table_kind(row.get(2)),
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
        let rows = self
            .client
            .query(
                "SELECT routine_name, routine_type FROM information_schema.routines \
                 WHERE specific_schema = $1 ORDER BY routine_type, routine_name",
                &[&schema],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|row| Routine {
                name: row.get(0),
                kind: routine_kind(row.get(1)),
            })
            .collect())
    }

    /// Reads the indexes from the catalog. The list of columns of an index is
    /// an array, so the array is opened with its order kept, which gives one
    /// column of one index in each row.
    async fn list_indexes(
        &mut self,
        _database: &str,
        schema: Option<&str>,
        table: &str,
    ) -> Result<Vec<IndexInfo>> {
        let schema = schema.unwrap_or("public");
        let rows = self
            .client
            .query(
                "SELECT i.relname, idx.indisunique, idx.indisprimary, a.attname \
                 FROM pg_catalog.pg_index AS idx \
                 JOIN pg_catalog.pg_class AS i ON i.oid = idx.indexrelid \
                 JOIN pg_catalog.pg_class AS t ON t.oid = idx.indrelid \
                 JOIN pg_catalog.pg_namespace AS n ON n.oid = t.relnamespace \
                 JOIN LATERAL unnest(idx.indkey) WITH ORDINALITY AS k(attnum, ord) ON true \
                 LEFT JOIN pg_catalog.pg_attribute AS a \
                        ON a.attrelid = t.oid AND a.attnum = k.attnum \
                 WHERE n.nspname = $1 AND t.relname = $2 \
                 ORDER BY i.relname, k.ord",
                &[&schema, &table],
            )
            .await?;
        let mut indexes = Vec::new();
        for row in &rows {
            add_index_column(&mut indexes, row.get(0), row.get(1), row.get(2), row.get(3));
        }
        Ok(indexes)
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
                constraint_kind(row.get(1)),
                row.get(2),
                row.get(3),
            );
        }
        Ok(constraints)
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
        self.token.cancel_query(self.tls.clone()).await?;
        Ok(())
    }
}

/// The statement that tells whether the session is outside a transaction
/// block. Outside a block each statement is a transaction of its own, so
/// the start of the transaction and the start of the statement are the same
/// moment. Inside a block the transaction started with an earlier statement.
const OUTSIDE_A_BLOCK: &str = "SELECT now() = statement_timestamp()";

impl PostgresDriver {
    /// True when a cancel at the row limit loses no work. The statement must
    /// only read, and the session must be outside a transaction block.
    ///
    /// A cancel rolls back the statement it ends, so a cancelled
    /// `INSERT ... RETURNING` writes no row. Inside a block the cancel also
    /// aborts the block, and the `COMMIT` that follows then rolls back every
    /// change of the block. A probe that fails, for example in a block that
    /// is already aborted, gives false.
    async fn may_cancel(&self, statement: &str) -> bool {
        if !only_reads(statement, Dialect::Postgres) {
            return false;
        }
        self.outside_a_block().await.unwrap_or(false)
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
    /// simple protocol. Such a text goes to the server whole.
    async fn stream_simple(
        &mut self,
        query: &str,
        options: &ExecOptions,
        sink: &mut dyn RowSink,
    ) -> Result<Option<u64>> {
        let statements = if options.one_statement {
            self.client.prepare(query).await?;
            vec![query.to_string()]
        } else {
            split_statements(query, Dialect::Postgres)
        };
        let mut rows_affected: Option<u64> = None;
        for statement in &statements {
            let (affected, stopped) = self.stream_statement(statement, options, sink).await?;
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
    /// A statement that [`Self::may_cancel`] accepts ends at the row limit:
    /// the driver sends a cancel request on a second socket, and the server
    /// stops the statement instead of sending the rest of the result. The
    /// server answers the cancel with the error 57014, which the walk reads
    /// as the end of the set.
    ///
    /// Every other statement keeps the walk and drops the rows past the
    /// limit. The driver holds one message while it walks, so the memory cost
    /// does not grow with the size of the answer.
    async fn stream_statement(
        &mut self,
        statement: &str,
        options: &ExecOptions,
        sink: &mut dyn RowSink,
    ) -> Result<(Option<u64>, bool)> {
        let alone = self.may_cancel(statement).await;
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

        loop {
            let message = match messages.try_next().await {
                Ok(Some(message)) => message,
                Ok(None) => break,
                Err(error) => {
                    if cancelled && is_query_cancelled(&error) {
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
                    sink.begin_set(
                        columns
                            .iter()
                            .map(|column| ColumnInfo::new(column.name(), "text"))
                            .collect(),
                    )?;
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
                SimpleQueryMessage::CommandComplete(affected) => {
                    if open {
                        sink.message(rows_returned_message(count, truncated));
                        sink.end_set(truncated)?;
                        open = false;
                    } else {
                        rows_affected = Some(rows_affected.unwrap_or(0) + affected);
                        sink.message(rows_affected_message(affected));
                    }
                }
                _ => {}
            }
        }
        if open {
            sink.message(rows_returned_message(count, truncated));
            sink.end_set(truncated)?;
        }
        Ok((rows_affected, stopped))
    }

    /// Runs one statement with bound parameters through the extended
    /// protocol and streams the rows into the sink one at a time. When
    /// [`Self::may_cancel`] accepts the statement, a stop cancels it on the
    /// server and drops the stream, so the rows past the stop do not cross
    /// the wire. Any other statement runs to its end, and the walk drops the
    /// rows past the stop. A fault that the server reports after those rows
    /// then still ends the run.
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
        let bound = bind_params(params)?;
        let may_cancel = self.may_cancel(query).await;

        let statement = self.client.prepare(query).await?;
        let columns: Vec<ColumnInfo> = statement
            .columns()
            .iter()
            .map(|column| ColumnInfo::new(column.name(), column.type_().name()))
            .collect();
        let returns_rows = !columns.is_empty();

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
            if count >= options.max_rows || sink.row(row_to_json(&row))? == SinkControl::Stop {
                truncated = true;
                // The statement holds the rest of its result on the server.
                // The cancel ends it there, so those rows never cross the
                // wire.
                if may_cancel && request_stop(&stop).await {
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
}

/// The keyword that asks PostgreSQL for a plan. The analysed form runs the
/// statement, so a statement that writes rows writes them.
pub fn plan_prefix(kind: PlanKind) -> &'static str {
    match kind {
        PlanKind::Estimated => "EXPLAIN (FORMAT TEXT)",
        PlanKind::Actual => "EXPLAIN (ANALYZE, BUFFERS)",
    }
}

/// Builds the statement that reads the CREATE text of one view. PostgreSQL
/// keeps no text for a table, so a table gives no statement and the command
/// layer builds a draft instead.
///
/// The name goes into the statement as a literal that `regclass` reads. A
/// name of another database cannot be read this way, so the name holds the
/// schema and the table alone.
fn create_query_text(schema: Option<&str>, table: &str, kind: TableKind) -> Option<CreateQuery> {
    if kind != TableKind::View {
        return None;
    }
    let name = Dialect::Postgres.qualified_name(None, schema, table);
    Some(CreateQuery::new(
        format!(
            "SELECT pg_get_viewdef({}::regclass, true);",
            Dialect::Postgres.quote_literal(&name)
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

/// Converts one row into an array of JSON values.
pub fn row_to_json(row: &Row) -> Vec<JsonValue> {
    (0..row.columns().len())
        .map(|index| cell_to_json(row, index))
        .collect()
}

/// Reads one cell. The extended protocol sends every value in its binary
/// form, so the bytes of the cell come out of the row and a reader for the
/// type of the column turns them into JSON. A cell shows NULL only when the
/// server sent no value: bytes that no reader understands show as text when
/// they are text, and as base64 when they are not.
fn cell_to_json(row: &Row, index: usize) -> JsonValue {
    // The type stays in the row, because a copy of it would cost a count on
    // a shared record for each cell of the answer.
    let column_type = row.columns()[index].type_();
    match row.try_get::<_, Option<Raw>>(index) {
        Ok(None) => JsonValue::Null,
        Ok(Some(Raw(bytes))) => decode_value(column_type, bytes),
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
/// holds other values comes from the kind of its type, so an array, a
/// range, and a composite of any element type read the same way.
fn decode_value(column_type: &Type, bytes: &[u8]) -> JsonValue {
    match column_type.kind() {
        Kind::Array(element) => decode_array(element, bytes),
        Kind::Range(element) => decode_range(element, bytes),
        Kind::Multirange(element) => decode_multirange(element, bytes),
        // A domain carries the value of the type it is built on.
        Kind::Domain(inner) => decode_value(inner, bytes),
        Kind::Composite(fields) => decode_composite(fields, bytes),
        // The value of an enumerated type is the label itself.
        Kind::Enum(_) => text_or_bytes(bytes),
        _ => decode_scalar(column_type, bytes),
    }
}

/// Reads one value that holds no other value.
fn decode_scalar(column_type: &Type, bytes: &[u8]) -> JsonValue {
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
        Type::JSON | Type::JSONB => scalar(column_type, bytes, |value: JsonValue| value),
        Type::BYTEA => JsonValue::String(base64_text(bytes)),
        Type::DATE => endless(
            Reader::new(bytes).i32().map(i64::from),
            i32::MAX as i64,
            i32::MIN as i64,
        )
        .unwrap_or_else(|| {
            scalar(column_type, bytes, |value: NaiveDate| {
                JsonValue::String(value.to_string())
            })
        }),
        Type::TIME => scalar(column_type, bytes, |value: NaiveTime| {
            JsonValue::String(value.to_string())
        }),
        Type::TIMESTAMP => {
            endless(Reader::new(bytes).i64(), i64::MAX, i64::MIN).unwrap_or_else(|| {
                scalar(column_type, bytes, |value: NaiveDateTime| {
                    JsonValue::String(value.to_string())
                })
            })
        }
        Type::TIMESTAMPTZ => {
            endless(Reader::new(bytes).i64(), i64::MAX, i64::MIN).unwrap_or_else(|| {
                scalar(column_type, bytes, |value: DateTime<Utc>| {
                    JsonValue::String(value.to_rfc3339())
                })
            })
        }
        Type::MONEY => money_text(bytes),
        Type::INTERVAL => interval_text(bytes),
        Type::INET | Type::CIDR => inet_text(bytes),
        Type::MACADDR => mac_text(bytes, 6),
        Type::MACADDR8 => mac_text(bytes, 8),
        _ => text_or_bytes(bytes),
    }
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
fn decode_array(element: &Type, bytes: &[u8]) -> JsonValue {
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
    match nested_elements(&mut reader, element, &lengths) {
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
) -> Option<JsonValue> {
    let (length, rest) = lengths.split_first()?;
    let mut values = Vec::with_capacity(*length);
    for _ in 0..*length {
        if rest.is_empty() {
            values.push(reader.value(element)?);
        } else {
            values.push(nested_elements(reader, element, rest)?);
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
fn decode_range(element: &Type, bytes: &[u8]) -> JsonValue {
    match range_text(element, &mut Reader::new(bytes)) {
        Some(text) => JsonValue::String(text),
        None => text_or_bytes(bytes),
    }
}

/// Reads one range out of the reader and writes it as text.
fn range_text(element: &Type, reader: &mut Reader<'_>) -> Option<String> {
    let flags = reader.u8()?;
    if flags & RANGE_EMPTY != 0 {
        return Some("empty".to_string());
    }
    let lower = if flags & RANGE_LOWER_OPEN_END != 0 {
        String::new()
    } else {
        render(&reader.value(element)?)
    };
    let upper = if flags & RANGE_UPPER_OPEN_END != 0 {
        String::new()
    } else {
        render(&reader.value(element)?)
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
fn decode_multirange(element: &Type, bytes: &[u8]) -> JsonValue {
    let mut reader = Reader::new(bytes);
    let Some(count) = reader.i32() else {
        return text_or_bytes(bytes);
    };
    let mut parts = Vec::new();
    for _ in 0..count.max(0) {
        let Some(part) = reader.i32().and_then(|length| {
            let mut inner = Reader::new(reader.take(length.max(0) as usize)?);
            range_text(element, &mut inner)
        }) else {
            return text_or_bytes(bytes);
        };
        parts.push(part);
    }
    JsonValue::String(format!("{{{}}}", parts.join(",")))
}

/// Reads a composite value and writes it in the form that PostgreSQL itself
/// writes, such as `(1,two)`.
fn decode_composite(fields: &[Field], bytes: &[u8]) -> JsonValue {
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
        let Some(value) = reader.u32().and_then(|_| reader.value(field.type_())) else {
            return text_or_bytes(bytes);
        };
        parts.push(render(&value));
    }
    JsonValue::String(format!("({})", parts.join(",")))
}

/// Writes one JSON value as the text that a value inside a range, a
/// multirange, or a composite shows.
fn render(value: &JsonValue) -> String {
    match value {
        JsonValue::Null => String::new(),
        JsonValue::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Writes a money value. The server sends the amount in the smallest unit
/// of the currency, and the count of the digits of the fraction comes from
/// the `lc_monetary` setting of the server. Two digits hold for every
/// currency that PostgreSQL ships a locale for.
fn money_text(bytes: &[u8]) -> JsonValue {
    let Some(amount) = Reader::new(bytes).i64() else {
        return text_or_bytes(bytes);
    };
    let sign = if amount < 0 { "-" } else { "" };
    let units = amount.unsigned_abs();
    JsonValue::String(format!("{sign}{}.{:02}", units / 100, units % 100))
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

/// Writes an interval in the form that PostgreSQL itself writes, such as
/// `1 year 2 mons 3 days 04:05:06`.
fn interval_text(bytes: &[u8]) -> JsonValue {
    let mut reader = Reader::new(bytes);
    let (Some(micros), Some(days), Some(months)) = (reader.i64(), reader.i32(), reader.i32())
    else {
        return text_or_bytes(bytes);
    };
    let mut parts = Vec::new();
    let years = months / 12;
    let rest_months = months % 12;
    if years != 0 {
        parts.push(format!("{years} {}", plural(years, "year", "years")));
    }
    if rest_months != 0 {
        parts.push(format!(
            "{rest_months} {}",
            plural(rest_months, "mon", "mons")
        ));
    }
    if days != 0 {
        parts.push(format!("{days} {}", plural(days, "day", "days")));
    }
    if micros != 0 || parts.is_empty() {
        parts.push(clock_text(micros));
    }
    JsonValue::String(parts.join(" "))
}

/// Gives the singular word for a count of one, and the plural for every
/// other count.
fn plural<'a>(count: i32, one: &'a str, many: &'a str) -> &'a str {
    if count.abs() == 1 {
        one
    } else {
        many
    }
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

    fn i64(&mut self) -> Option<i64> {
        self.take(8)
            .map(|bytes| i64::from_be_bytes(bytes.try_into().unwrap()))
    }

    /// Reads one value that carries its own length, as the elements of an
    /// array and the bounds of a range do. A length of minus one means a
    /// value that is null.
    fn value(&mut self, column_type: &Type) -> Option<JsonValue> {
        let length = self.i32()?;
        if length < 0 {
            return Some(JsonValue::Null);
        }
        let bytes = self.take(length as usize)?;
        Some(decode_value(column_type, bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::sink::BufferSink;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
    use tokio::sync::Notify;

    /// Wraps a body of a message with its kind and its length. The length
    /// counts itself and the body, and never the byte of the kind.
    fn message(kind: u8, body: &[u8]) -> Vec<u8> {
        let mut out = vec![kind];
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
    /// text value, which the simple protocol always sends.
    fn row_description(names: &[&str]) -> Vec<u8> {
        let mut body = (names.len() as i16).to_be_bytes().to_vec();
        for (index, name) in names.iter().enumerate() {
            body.extend_from_slice(name.as_bytes());
            body.push(0);
            body.extend_from_slice(&0i32.to_be_bytes());
            body.extend_from_slice(&(index as i16 + 1).to_be_bytes());
            body.extend_from_slice(&25i32.to_be_bytes());
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
        // The startup message carries a length and no byte of a kind.
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
        let mut kind = [0u8; 1];
        server.read_exact(&mut kind).await.unwrap();
        assert_eq!(kind[0], b'Q');
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

    /// The answer to a statement that the client prepares. The kinds of the
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

    /// Reads the messages of the client up to the one that asks the server
    /// to answer.
    async fn read_until_sync(server: &mut DuplexStream) {
        loop {
            let mut kind = [0u8; 1];
            server.read_exact(&mut kind).await.unwrap();
            let mut length = [0u8; 4];
            server.read_exact(&mut length).await.unwrap();
            let mut body = vec![0u8; i32::from_be_bytes(length) as usize - 4];
            server.read_exact(&mut body).await.unwrap();
            if kind[0] == b'S' {
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
        tokio::spawn(connection);
        PostgresDriver {
            client,
            notices: Arc::new(Mutex::new(Vec::new())),
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
            answer_probe(&mut server, true).await;
            answer_query(
                &mut server,
                "SELECT 1",
                &[
                    row_description(&["id", "name"]),
                    data_row(&[Some("1"), Some("Ada")]),
                    data_row(&[Some("2"), None]),
                    command_complete("SELECT 2"),
                ],
            )
            .await;
            // A statement that writes gets no probe.
            answer_query(
                &mut server,
                "UPDATE t SET a = 1",
                &[command_complete("UPDATE 3")],
            )
            .await;
            answer_probe(&mut server, true).await;
            answer_query(
                &mut server,
                "SELECT 7",
                &[
                    row_description(&["n"]),
                    data_row(&[Some("7")]),
                    command_complete("SELECT 1"),
                ],
            )
            .await;
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
    async fn one_statement_that_gives_two_sets_keeps_both() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_probe(&mut server, true).await;
            answer_query(
                &mut server,
                "SELECT 1",
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
            .stream_simple("SELECT 1", &options, &mut sink)
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

    #[tokio::test]
    async fn a_set_that_passes_the_row_limit_inside_a_block_is_cut_and_the_walk_goes_on() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_probe(&mut server, false).await;
            answer_query(
                &mut server,
                "SELECT id FROM t",
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
            .stream_simple("SELECT id FROM t; DELETE FROM t", &options, &mut sink)
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
            .any(|message| message.text.contains("The row limit stopped the read")));
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
            answer_probe(&mut server, true).await;
            assert_eq!(read_query(&mut server).await, "SELECT id FROM t");
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
            .stream_simple("SELECT id FROM t; DELETE FROM t", &options, &mut sink)
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
            answer_probe(&mut server, false).await;
            answer_query(
                &mut server,
                "SELECT id FROM t",
                &[
                    row_description(&["id"]),
                    data_row(&[Some("1")]),
                    data_row(&[Some("2")]),
                    data_row(&[Some("3")]),
                    command_complete("SELECT 3"),
                ],
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
            answer_probe(&mut server, true).await;
            read_query(&mut server).await;

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
            .stream_simple("SELECT id FROM t", &options, &mut sink)
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
            assert_eq!(read_query(&mut server).await, "SELECT id FROM t");
            let mut answer = row_description(&["id"]);
            answer.extend_from_slice(&data_row(&[Some("1")]));
            answer.extend_from_slice(&command_complete("SELECT 1"));
            answer.extend_from_slice(&ready_for_query());
            server.write_all(&answer).await.unwrap();
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
            Error::Postgres(ref error) if error.code() == Some(&SqlState::SYNTAX_ERROR)
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
            answer_probe(&mut server, true).await;
            read_query(&mut server).await;

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
            .stream_simple("SELECT id FROM t", &options, &mut sink)
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
            answer_probe(&mut server, true).await;
            read_query(&mut server).await;

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
        let outcome = driver
            .stream_simple("SELECT id FROM t", &options, &mut sink)
            .await;

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
            answer_probe(&mut server, true).await;
            read_query(&mut server).await;

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
            .stream_simple("SELECT id FROM t", &options, &mut sink)
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

            read_until_sync(&mut server).await;
            let mut answer = message(b'2', &[]);
            answer.extend_from_slice(&binary_data_row(&[Some(&7i32.to_be_bytes()), Some(b"Ada")]));
            answer.extend_from_slice(&binary_data_row(&[None, None]));
            answer.extend_from_slice(&command_complete("SELECT 2"));
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

            read_until_sync(&mut server).await;
            let mut answer = message(b'2', &[]);
            answer.extend_from_slice(&command_complete("SELECT 0"));
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
            .stream_with_params(
                "SELECT id FROM t WHERE id > $1",
                &one_param(),
                &options,
                &mut sink,
            )
            .await
            .unwrap();
        let response = sink.into_response(RunSummary::default());

        // The rest of the result stays on the server.
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(response.results[0].rows.len(), 1);
        assert!(response.results[0].truncated);

        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_script_of_one_statement_inside_a_block_walks_past_the_row_limit() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let (stop, calls, _signal) = test_stop(true);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            answer_probe(&mut server, false).await;
            read_query(&mut server).await;

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
            .stream_simple("SELECT id FROM t", &options, &mut sink)
            .await
            .unwrap();
        let response = sink.into_response(RunSummary::default());

        // A cancel would abort the open block, so no cancel went out.
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(response.results[0].rows.len(), 1);
        assert!(response.results[0].truncated);

        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_probe_that_fails_keeps_the_walk() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let (stop, calls, _signal) = test_stop(true);
        let task = tokio::spawn(async move {
            accept_startup(&mut server).await;
            assert_eq!(read_query(&mut server).await, OUTSIDE_A_BLOCK);
            let mut refusal = error_response("25P02", "current transaction is aborted");
            refusal.extend_from_slice(&ready_for_query());
            server.write_all(&refusal).await.unwrap();
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

        task.await.unwrap();
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
            .stream_with_params(
                "SELECT id FROM t WHERE id > $1",
                &one_param(),
                &options,
                &mut sink,
            )
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
        decode_value(column_type, bytes)
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
            decoded(&Type::BYTEA, b"hi"),
            JsonValue::String("aGk=".into())
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
            JsonValue::String("2000-01-01T00:00:00+00:00".into())
        );
    }

    #[test]
    fn a_uuid_and_a_json_value_are_read() {
        assert_eq!(
            decoded(&Type::UUID, &[0x11; 16]),
            JsonValue::String("11111111-1111-1111-1111-111111111111".into())
        );
        assert_eq!(decoded(&Type::JSON, b"[1]"), serde_json::json!([1]));
        let mut jsonb = vec![1u8];
        jsonb.extend_from_slice(b"[1]");
        assert_eq!(decoded(&Type::JSONB, &jsonb), serde_json::json!([1]));
    }

    #[test]
    fn a_money_value_holds_two_digits_of_the_fraction() {
        assert_eq!(
            decoded(&Type::MONEY, &123456i64.to_be_bytes()),
            JsonValue::String("1234.56".into())
        );
        assert_eq!(
            decoded(&Type::MONEY, &(-5i64).to_be_bytes()),
            JsonValue::String("-0.05".into())
        );
        // A value of the wrong length falls back on the text rule.
        assert_eq!(decoded(&Type::MONEY, b"12"), JsonValue::String("12".into()));
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
        let kind = range_type(Type::INT4);
        let one = 1i32.to_be_bytes();
        let ten = 10i32.to_be_bytes();
        assert_eq!(
            decoded(&kind, &range_body(RANGE_LOWER_CLOSED, &[&one, &ten])),
            JsonValue::String("[1,10)".into())
        );
        assert_eq!(
            decoded(
                &kind,
                &range_body(RANGE_LOWER_CLOSED | RANGE_UPPER_CLOSED, &[&one, &ten])
            ),
            JsonValue::String("[1,10]".into())
        );
        assert_eq!(
            decoded(&kind, &[RANGE_EMPTY]),
            JsonValue::String("empty".into())
        );
        assert_eq!(
            decoded(
                &kind,
                &range_body(RANGE_LOWER_OPEN_END | RANGE_UPPER_OPEN_END, &[])
            ),
            JsonValue::String("(,)".into())
        );
        // A range without its bounds falls back on the text rule.
        assert_eq!(
            decoded(&kind, &[RANGE_LOWER_CLOSED]),
            JsonValue::String("\u{2}".into())
        );
        assert_eq!(decoded(&kind, &[]), JsonValue::String(String::new()));
    }

    #[test]
    fn a_multirange_holds_its_ranges_in_braces() {
        let element = range_type(Type::INT4);
        let kind = Type::new(
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
        assert_eq!(decoded(&kind, &body), JsonValue::String("{[1,10)}".into()));
        assert_eq!(
            decoded(&kind, &0i32.to_be_bytes()),
            JsonValue::String("{}".into())
        );
        // A count that names a range the value does not hold.
        assert_eq!(
            decoded(&kind, &1i32.to_be_bytes()),
            JsonValue::String("\u{0}\u{0}\u{0}\u{1}".into())
        );
        assert_eq!(decoded(&kind, b"ab"), JsonValue::String("ab".into()));
        // The element type of the multirange reads on its own as well.
        assert_eq!(
            decoded(&element, &[RANGE_EMPTY]),
            JsonValue::String("empty".into())
        );
    }

    #[test]
    fn a_composite_shows_its_fields_in_order() {
        let kind = Type::new(
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
        assert_eq!(decoded(&kind, &body), JsonValue::String("(1,two)".into()));

        // A count that does not match the fields of the type, and a value
        // that ends too early, both fall back on the text rule.
        assert_eq!(
            decoded(&kind, &1i32.to_be_bytes()),
            JsonValue::String("\u{0}\u{0}\u{0}\u{1}".into())
        );
        assert_eq!(
            decoded(&kind, &2i32.to_be_bytes()),
            JsonValue::String("\u{0}\u{0}\u{0}\u{2}".into())
        );
        assert_eq!(decoded(&kind, b"ab"), JsonValue::String("ab".into()));
    }

    #[test]
    fn a_field_of_a_composite_that_holds_no_value_shows_as_empty() {
        let kind = Type::new(
            "one".to_string(),
            17001,
            Kind::Composite(vec![Field::new("name".to_string(), Type::TEXT)]),
            "public".to_string(),
        );
        let mut body = 1i32.to_be_bytes().to_vec();
        body.extend_from_slice(&Type::TEXT.oid().to_be_bytes());
        body.extend_from_slice(&element_body(None));
        assert_eq!(decoded(&kind, &body), JsonValue::String("()".into()));
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
        assert_eq!(plan_prefix(PlanKind::Estimated), "EXPLAIN (FORMAT TEXT)");
        assert_eq!(plan_prefix(PlanKind::Actual), "EXPLAIN (ANALYZE, BUFFERS)");
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
        let view = create_query_text(Some("public"), "v", TableKind::View).unwrap();
        assert_eq!(
            view.sql,
            "SELECT pg_get_viewdef('\"public\".\"v\"'::regclass, true);"
        );
        assert_eq!(view.column, 0);
        assert!(create_query_text(Some("public"), "t", TableKind::Table).is_none());
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
            build_config(&input).unwrap_err().kind(),
            crate::error::ErrorKind::Configuration
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
    fn a_certificate_authority_file_that_is_missing_gives_an_error() {
        let mut input = connection();
        input.options.ca_cert_path = Some("/does/not/exist.pem".into());
        assert_eq!(
            build_tls_config(&input).unwrap_err().kind(),
            crate::error::ErrorKind::Io
        );

        input.options.ca_cert_path = Some("   ".into());
        assert!(build_tls_config(&input).is_ok());
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
            bind_params(&params).unwrap_err().kind(),
            crate::error::ErrorKind::Configuration
        );
    }
}
