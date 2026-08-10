//! The PostgreSQL driver.
//!
//! A script without parameters goes through the simple protocol. That
//! protocol accepts more than one statement in one call, it reports the
//! command tag of each statement, and it returns every value as text, so no
//! type mapping can fail.

use crate::db::drivers::{
    add_constraint_column, add_index_column, add_snapshot_column, bytes_to_json, constraint_kind,
    f64_to_json, number_out_of_range, number_value, prefixed_plan, routine_kind,
    rows_affected_message, rows_returned_message, size_text, table_kind, CancelHandle,
    DatabaseDriver, NumberValue,
};
use crate::db::{
    AppColumn, ColumnInfo, Constraint, CreateQuery, Database, DriverCapabilities, ExecOptions,
    IndexInfo, Message, MessageLevel, PlanKind, QueryParams, QueryResponse, ResultSet, Routine,
    Schema, SchemaSnapshot, SnapshotColumn, Table, TableFact, TableKind,
};
use crate::error::{Error, Result};
use crate::sql::Dialect;
use crate::storage::{SavedConnection, TlsMode};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use futures_util::{stream, StreamExt};
use postgres_types::Type;
use rust_decimal::Decimal;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use serde_json::Value as JsonValue;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_postgres::error::DbError;
use tokio_postgres::{AsyncMessage, Client, Config as PgConfig, Row, SimpleQueryMessage};

pub struct PostgresDriver {
    client: Client,
    /// The notices that the server sent since the last answer.
    notices: NoticeBuffer,
}

/// Builds the connection configuration from a saved connection.
pub fn build_config(connection: &SavedConnection) -> Result<PgConfig> {
    if let Some(url) = connection.options.connection_url.as_deref() {
        return url
            .trim()
            .parse::<PgConfig>()
            .map_err(|error| Error::Configuration(error.to_string()));
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
    if connection.options.read_only {
        config.options("-c default_transaction_read_only=on");
    }
    Ok(config)
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
        let (client, mut io) = tokio::time::timeout(limit, config.connect(tls))
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

        Ok(Box::new(PostgresDriver { client, notices }))
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

    async fn execute_query(
        &mut self,
        query: &str,
        params: Option<&QueryParams>,
        options: &ExecOptions,
    ) -> Result<QueryResponse> {
        let started = Instant::now();
        // A notice that arrived before this run belongs to no answer, so the
        // buffer starts empty.
        let _ = self.take_notices();
        let outcome = match params {
            Some(params) => self.run_with_params(query, params, options).await,
            None => self.run_simple(query, options).await,
        };

        let notices = self.take_notices();
        let mut response = match outcome {
            Ok(response) => response,
            Err(error) => {
                // A run that failed carries the error and no answer, so the
                // notices of that run reach the log alone.
                for notice in &notices {
                    log::info!("The PostgreSQL server said: {}", notice.text);
                }
                return Err(error);
            }
        };
        response.messages.splice(0..0, notices);
        response.elapsed_ms = started.elapsed().as_millis() as u64;
        Ok(response)
    }

    async fn explain(
        &mut self,
        query: &str,
        kind: PlanKind,
        options: &ExecOptions,
    ) -> Result<QueryResponse> {
        let statement = prefixed_plan(query, Dialect::Postgres, plan_prefix(kind))?;
        self.execute_query(&statement, None, options).await
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
        Some(Arc::new(PostgresCancel(self.client.cancel_token())))
    }
}

/// A token that asks the server to stop the statement that runs on this
/// connection. It opens its own socket, so it works while the connection
/// is busy.
struct PostgresCancel(tokio_postgres::CancelToken);

#[async_trait]
impl CancelHandle for PostgresCancel {
    async fn cancel(&self) -> Result<()> {
        self.0.cancel_query(tokio_postgres::NoTls).await?;
        Ok(())
    }
}

impl PostgresDriver {
    /// Runs a script through the simple protocol.
    async fn run_simple(&mut self, query: &str, options: &ExecOptions) -> Result<QueryResponse> {
        let messages = self.client.simple_query(query).await?;
        let mut response = QueryResponse::default();
        let mut current: Option<ResultSet> = None;

        for message in messages {
            match message {
                SimpleQueryMessage::RowDescription(columns) => {
                    if let Some(set) = current.take() {
                        push_set(&mut response, set);
                    }
                    current = Some(ResultSet::new(
                        columns
                            .iter()
                            .map(|column| ColumnInfo::new(column.name(), "text"))
                            .collect(),
                    ));
                }
                SimpleQueryMessage::Row(row) => {
                    let Some(set) = current.as_mut() else {
                        continue;
                    };
                    if set.rows.len() >= options.max_rows {
                        set.truncated = true;
                        continue;
                    }
                    set.rows.push(
                        (0..row.len())
                            .map(|index| match row.get(index) {
                                Some(value) => JsonValue::String(value.to_string()),
                                None => JsonValue::Null,
                            })
                            .collect(),
                    );
                }
                SimpleQueryMessage::CommandComplete(affected) => {
                    if let Some(set) = current.take() {
                        push_set(&mut response, set);
                    } else {
                        response.rows_affected =
                            Some(response.rows_affected.unwrap_or(0) + affected);
                        response.messages.push(rows_affected_message(affected));
                    }
                }
                _ => {}
            }
        }
        if let Some(set) = current.take() {
            push_set(&mut response, set);
        }
        Ok(response)
    }

    /// Runs one statement with bound parameters through the extended
    /// protocol.
    async fn run_with_params(
        &mut self,
        query: &str,
        params: &QueryParams,
        options: &ExecOptions,
    ) -> Result<QueryResponse> {
        let bound = bind_params(params)?;
        let borrowed: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = bound
            .iter()
            .map(|value| value.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let rows = self.client.query(query, borrowed.as_slice()).await?;
        let mut response = QueryResponse::default();
        if rows.is_empty() {
            response.messages.push(rows_affected_message(0));
            return Ok(response);
        }

        let mut set = ResultSet::new(
            rows[0]
                .columns()
                .iter()
                .map(|column| ColumnInfo::new(column.name(), column.type_().name()))
                .collect(),
        );
        for row in &rows {
            if set.rows.len() >= options.max_rows {
                set.truncated = true;
                break;
            }
            set.rows.push(row_to_json(row));
        }
        push_set(&mut response, set);
        Ok(response)
    }
}

/// Adds a result set and the message that belongs to it.
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

fn push_set(response: &mut QueryResponse, set: ResultSet) {
    response
        .messages
        .push(rows_returned_message(set.rows.len(), set.truncated));
    response.results.push(set);
}

/// Turns the JSON parameters into values the driver can bind.
pub fn bind_params(
    params: &QueryParams,
) -> Result<Vec<Box<dyn tokio_postgres::types::ToSql + Send + Sync>>> {
    let mut bound: Vec<Box<dyn tokio_postgres::types::ToSql + Send + Sync>> = Vec::new();
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
            other => bound.push(Box::new(other.clone())),
        }
    }
    Ok(bound)
}

/// Converts one row into an array of JSON values.
pub fn row_to_json(row: &Row) -> Vec<JsonValue> {
    (0..row.columns().len())
        .map(|index| cell_to_json(row, index))
        .collect()
}

/// Reads one cell. Every target type is an option, because a column that
/// holds no value would otherwise make the driver panic.
fn cell_to_json(row: &Row, index: usize) -> JsonValue {
    let column_type = row.columns()[index].type_().clone();
    match column_type {
        Type::BOOL => get(row, index).map_or(JsonValue::Null, JsonValue::Bool),
        Type::INT2 => get::<i16>(row, index).map_or(JsonValue::Null, Into::into),
        Type::INT4 => get::<i32>(row, index).map_or(JsonValue::Null, Into::into),
        Type::INT8 | Type::OID => get::<i64>(row, index).map_or(JsonValue::Null, Into::into),
        Type::FLOAT4 => get::<f32>(row, index).map_or(JsonValue::Null, |v| f64_to_json(v as f64)),
        Type::FLOAT8 => get::<f64>(row, index).map_or(JsonValue::Null, f64_to_json),
        Type::NUMERIC => get::<Decimal>(row, index)
            .map(|value| JsonValue::String(value.to_string()))
            .unwrap_or(JsonValue::Null),
        Type::TEXT | Type::VARCHAR | Type::NAME | Type::BPCHAR | Type::UNKNOWN => {
            get::<String>(row, index).map_or(JsonValue::Null, JsonValue::String)
        }
        Type::UUID => get::<uuid::Uuid>(row, index)
            .map(|value| JsonValue::String(value.to_string()))
            .unwrap_or(JsonValue::Null),
        Type::JSON | Type::JSONB => get::<JsonValue>(row, index).unwrap_or(JsonValue::Null),
        Type::BYTEA => get::<Vec<u8>>(row, index).map_or(JsonValue::Null, |v| bytes_to_json(&v)),
        Type::DATE => get::<NaiveDate>(row, index)
            .map(|value| JsonValue::String(value.to_string()))
            .unwrap_or(JsonValue::Null),
        Type::TIME => get::<NaiveTime>(row, index)
            .map(|value| JsonValue::String(value.to_string()))
            .unwrap_or(JsonValue::Null),
        Type::TIMESTAMP => get::<NaiveDateTime>(row, index)
            .map(|value| JsonValue::String(value.to_string()))
            .unwrap_or(JsonValue::Null),
        Type::TIMESTAMPTZ => get::<DateTime<Utc>>(row, index)
            .map(|value| JsonValue::String(value.to_rfc3339()))
            .unwrap_or(JsonValue::Null),
        Type::BOOL_ARRAY => array_to_json(get::<Vec<Option<bool>>>(row, index)),
        Type::INT2_ARRAY => array_to_json(get::<Vec<Option<i16>>>(row, index)),
        Type::INT4_ARRAY => array_to_json(get::<Vec<Option<i32>>>(row, index)),
        Type::INT8_ARRAY => array_to_json(get::<Vec<Option<i64>>>(row, index)),
        Type::FLOAT4_ARRAY => array_to_json(get::<Vec<Option<f32>>>(row, index)),
        Type::FLOAT8_ARRAY => array_to_json(get::<Vec<Option<f64>>>(row, index)),
        Type::TEXT_ARRAY | Type::VARCHAR_ARRAY | Type::NAME_ARRAY => {
            array_to_json(get::<Vec<Option<String>>>(row, index))
        }
        // Every other type is read through its text form, which the server
        // can produce for any type.
        _ => get::<String>(row, index).map_or(JsonValue::Null, JsonValue::String),
    }
}

/// Turns a list of optional values into a JSON array.
fn array_to_json<T: Into<JsonValue>>(values: Option<Vec<Option<T>>>) -> JsonValue {
    match values {
        None => JsonValue::Null,
        Some(values) => JsonValue::Array(
            values
                .into_iter()
                .map(|value| value.map_or(JsonValue::Null, Into::into))
                .collect(),
        ),
    }
}

/// Reads one value and treats a failure as an absent value.
fn get<'a, T: tokio_postgres::types::FromSql<'a>>(row: &'a Row, index: usize) -> Option<T> {
    match row.try_get::<_, Option<T>>(index) {
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
        assert_eq!(bind_params(&params).unwrap().len(), 6);
        assert!(bind_params(&Vec::new()).unwrap().is_empty());
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

    #[test]
    fn an_array_of_values_becomes_a_json_array() {
        assert_eq!(
            array_to_json(Some(vec![Some(1i32), None, Some(3i32)])),
            serde_json::json!([1, null, 3])
        );
        assert_eq!(array_to_json::<i32>(None), JsonValue::Null);
        assert_eq!(
            array_to_json(Some(Vec::<Option<i32>>::new())),
            serde_json::json!([])
        );
    }

    #[test]
    fn a_result_set_is_added_with_its_message() {
        let mut response = QueryResponse::default();
        let mut set = ResultSet::new(vec![ColumnInfo::new("a", "text")]);
        set.rows.push(vec![JsonValue::Null]);
        push_set(&mut response, set);
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.messages, vec![Message::info("1 row returned.")]);
    }
}
