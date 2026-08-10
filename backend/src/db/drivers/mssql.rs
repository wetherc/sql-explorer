//! The MS SQL Server driver.
//!
//! The configuration is built with the builder of `tiberius` and not from a
//! connection string. A connection string loses the port, because the
//! parser reads the port only from inside the `server` value, and it also
//! loses any password that holds a semicolon or a brace.

use crate::db::drivers::{
    bytes_to_json, f64_to_json, number_out_of_range, number_value, parameter_type_refused,
    rows_affected_message, rows_returned_message, DatabaseDriver, NumberValue,
};
use crate::db::{
    AppColumn, ColumnInfo, Database, DriverCapabilities, ExecOptions, QueryParams, QueryResponse,
    ResultSet, Schema, Table,
};
use crate::error::{Error, Result};
use crate::sql::{split_statements, Dialect};
use crate::storage::{MssqlAuth, SavedConnection, TlsMode};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use futures_util::TryStreamExt;
use serde_json::Value as JsonValue;
use std::time::{Duration, Instant};
use tiberius::numeric::Numeric;
use tiberius::{AuthMethod, Client, ColumnType, Config, EncryptionLevel, QueryItem, Row};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

type MssqlClient = Client<Compat<TcpStream>>;

pub struct MssqlDriver {
    client: MssqlClient,
}

/// Builds the `tiberius` configuration from a saved connection.
pub async fn build_config(connection: &SavedConnection) -> Result<Config> {
    let mut config = if let Some(url) = connection.options.connection_url.as_deref() {
        let trimmed = url.trim();
        if trimmed.starts_with("jdbc:") {
            Config::from_jdbc_string(trimmed)?
        } else {
            Config::from_ado_string(trimmed)?
        }
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
        if !connection.options.tls_mode.verifies_certificate() {
            config.trust_cert();
        }
        if let Some(path) = non_empty(connection.options.ca_cert_path.as_deref()) {
            config.trust_cert_ca(path);
        }
        config.readonly(connection.options.read_only);
    }

    Ok(config)
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
    match connection.effective_auth() {
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
            .map_err(|error| describe_login(error, connection.effective_auth()))?;
        Ok(Box::new(MssqlDriver { client }))
    }
}

/// Names the reason a login failed. Kerberos reports a missing ticket in
/// words that mean nothing to a user of a database, so the message says what
/// to do instead.
fn describe_login(error: tiberius::error::Error, auth: MssqlAuth) -> Error {
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

/// True when the first keyword of the statement introduces a statement that
/// gives rows back. A statement that does not is sent through `execute`, so
/// that the number of changed rows reaches the user.
pub fn returns_rows(statement: &str) -> bool {
    let keyword = first_keyword(statement);
    !matches!(
        keyword.as_str(),
        "insert"
            | "update"
            | "delete"
            | "merge"
            | "create"
            | "alter"
            | "drop"
            | "truncate"
            | "grant"
            | "revoke"
            | "deny"
            | "use"
            | "set"
            | "begin"
            | "commit"
            | "rollback"
            | "backup"
            | "restore"
    )
}

/// Reads the first word of a statement, and steps over the comments and the
/// opening brackets that stand before it.
fn first_keyword(statement: &str) -> String {
    let mut rest = statement.trim_start();
    loop {
        if let Some(tail) = rest.strip_prefix("--") {
            rest = tail
                .find('\n')
                .map_or("", |index| &tail[index + 1..])
                .trim_start();
            continue;
        }
        if let Some(tail) = rest.strip_prefix("/*") {
            rest = tail
                .find("*/")
                .map_or("", |index| &tail[index + 2..])
                .trim_start();
            continue;
        }
        if let Some(tail) = rest.strip_prefix('(') {
            rest = tail.trim_start();
            continue;
        }
        break;
    }
    rest.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .next()
        .unwrap_or("")
        .to_lowercase()
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
        }
    }

    fn dialect(&self) -> Dialect {
        Dialect::MsSql
    }

    async fn ping(&mut self) -> Result<()> {
        let mut stream = self.client.simple_query("SELECT 1").await?;
        while stream.try_next().await?.is_some() {}
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

        // A script with parameters is sent whole, because the parameter
        // positions belong to the script and not to one statement.
        let statements: Vec<String> = if params.is_some() {
            vec![query.to_string()]
        } else {
            split_statements(query, Dialect::MsSql)
        };

        for statement in statements {
            let bound = bind_params(params)?;
            let borrowed: Vec<&dyn tiberius::ToSql> =
                bound.iter().map(|value| value.as_ref()).collect();

            if returns_rows(&statement) {
                let mut stream = self.client.query(&statement, borrowed.as_slice()).await?;
                let mut sets: Vec<ResultSet> = Vec::new();
                let mut current: Option<ResultSet> = None;

                while let Some(item) = stream.try_next().await? {
                    match item {
                        QueryItem::Metadata(metadata) => {
                            if let Some(set) = current.take() {
                                sets.push(set);
                            }
                            current = Some(ResultSet::new(
                                metadata
                                    .columns()
                                    .iter()
                                    .map(|column| {
                                        ColumnInfo::new(
                                            column.name(),
                                            type_name(column.column_type()),
                                        )
                                    })
                                    .collect(),
                            ));
                        }
                        QueryItem::Row(row) => {
                            let Some(set) = current.as_mut() else {
                                continue;
                            };
                            if set.rows.len() >= options.max_rows {
                                set.truncated = true;
                                continue;
                            }
                            set.rows.push(row_to_json(&row));
                        }
                    }
                }
                if let Some(set) = current.take() {
                    sets.push(set);
                }
                for set in &sets {
                    response
                        .messages
                        .push(rows_returned_message(set.rows.len(), set.truncated));
                }
                response.results.extend(sets);
            } else {
                let result = self.client.execute(&statement, borrowed.as_slice()).await?;
                let affected: u64 = result.rows_affected().iter().sum();
                response.rows_affected = Some(response.rows_affected.unwrap_or(0) + affected);
                response.messages.push(rows_affected_message(affected));
            }
        }

        response.elapsed_ms = started.elapsed().as_millis() as u64;
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
        // The catalog view is scoped to the database that the connection is
        // attached to, so the name of the database goes in front of it.
        let query = format!(
            "SELECT s.name FROM {}.sys.schemas AS s \
             JOIN {}.sys.database_principals AS p ON s.principal_id = p.principal_id \
             WHERE s.name NOT IN ('sys', 'INFORMATION_SCHEMA') \
               AND s.name NOT LIKE 'db\\_%' ESCAPE '\\' \
             ORDER BY s.name",
            Dialect::MsSql.quote_identifier(database),
            Dialect::MsSql.quote_identifier(database)
        );
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
    let types: Vec<ColumnType> = row
        .columns()
        .iter()
        .map(|column| column.column_type())
        .collect();
    types
        .iter()
        .enumerate()
        .map(|(index, column_type)| cell_to_json(row, index, *column_type))
        .collect()
}

/// Reads one cell. A read that fails falls back on the next target type,
/// and at the end on text, so that an unknown type shows a value and does
/// not stop the whole result.
fn cell_to_json(row: &Row, index: usize, column_type: ColumnType) -> JsonValue {
    match column_type {
        ColumnType::Bit | ColumnType::Bitn => {
            read(row.try_get::<bool, _>(index)).map_or(JsonValue::Null, JsonValue::Bool)
        }
        ColumnType::Int1 => read(row.try_get::<u8, _>(index)).map_or(JsonValue::Null, Into::into),
        ColumnType::Int2 => read(row.try_get::<i16, _>(index)).map_or(JsonValue::Null, Into::into),
        ColumnType::Int4 => read(row.try_get::<i32, _>(index)).map_or(JsonValue::Null, Into::into),
        ColumnType::Int8 => read(row.try_get::<i64, _>(index)).map_or(JsonValue::Null, Into::into),
        // A nullable integer covers every width from one to eight bytes, so
        // each width is tried in turn.
        ColumnType::Intn => read(row.try_get::<i64, _>(index))
            .map(JsonValue::from)
            .or_else(|| read(row.try_get::<i32, _>(index)).map(JsonValue::from))
            .or_else(|| read(row.try_get::<i16, _>(index)).map(JsonValue::from))
            .or_else(|| read(row.try_get::<u8, _>(index)).map(JsonValue::from))
            .unwrap_or(JsonValue::Null),
        ColumnType::Float4 => read(row.try_get::<f32, _>(index))
            .map(|value| f64_to_json(value as f64))
            .unwrap_or(JsonValue::Null),
        ColumnType::Float8 | ColumnType::Money | ColumnType::Money4 => {
            read(row.try_get::<f64, _>(index)).map_or(JsonValue::Null, f64_to_json)
        }
        ColumnType::Floatn => read(row.try_get::<f64, _>(index))
            .map(f64_to_json)
            .or_else(|| read(row.try_get::<f32, _>(index)).map(|value| f64_to_json(value as f64)))
            .unwrap_or(JsonValue::Null),
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
        ColumnType::DatetimeOffsetn => read(row.try_get::<DateTime<Utc>, _>(index))
            .map(|value| JsonValue::String(value.to_rfc3339()))
            .unwrap_or(JsonValue::Null),
        ColumnType::BigVarBin | ColumnType::BigBinary | ColumnType::Image | ColumnType::Udt => {
            read(row.try_get::<&[u8], _>(index)).map_or(JsonValue::Null, bytes_to_json)
        }
        _ => text_or_bytes(row, index),
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
    async fn the_optional_fields_are_accepted() {
        let mut input = connection();
        input.options.instance_name = Some("SQLEXPRESS".into());
        input.options.ca_cert_path = Some("/etc/ca.pem".into());
        input.options.read_only = true;
        input.database = Some(String::new());
        input.options.application_name = Some("  ".into());
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
        input.options.integrated_security = true;
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
    fn a_statement_that_changes_data_is_not_a_query() {
        for statement in [
            "INSERT INTO t VALUES (1)",
            "update t set a = 1",
            "DELETE FROM t",
            "MERGE t USING s ON 1=1",
            "CREATE TABLE t (a int)",
            "ALTER TABLE t ADD b int",
            "DROP TABLE t",
            "TRUNCATE TABLE t",
            "GRANT SELECT ON t TO r",
            "REVOKE SELECT ON t FROM r",
            "DENY SELECT ON t TO r",
            "USE Sales",
            "SET NOCOUNT ON",
            "BEGIN TRANSACTION",
            "COMMIT",
            "ROLLBACK",
            "BACKUP DATABASE a TO DISK = 'x'",
            "RESTORE DATABASE a FROM DISK = 'x'",
        ] {
            assert!(!returns_rows(statement), "{statement}");
        }
    }

    #[test]
    fn a_statement_that_reads_data_is_a_query() {
        for statement in [
            "SELECT 1",
            "  select 1",
            "WITH x AS (SELECT 1) SELECT * FROM x",
            "EXEC sp_who",
            "DECLARE @a int",
            "(SELECT 1)",
            "-- a comment\nSELECT 1",
            "/* a comment */ SELECT 1",
            "",
        ] {
            assert!(returns_rows(statement), "{statement}");
        }
    }

    #[test]
    fn the_first_keyword_steps_over_comments_that_never_close() {
        assert_eq!(first_keyword("-- only a comment"), "");
        assert_eq!(first_keyword("/* never closed"), "");
        assert_eq!(first_keyword("/* a */ /* b */ select"), "select");
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
    async fn a_missing_azure_cli_is_reported() {
        let mut input = connection();
        input.options.mssql_auth = MssqlAuth::EntraAzureCli;
        input.options.azure_cli_path = Some("/nowhere/az".into());
        let error = auth_method(&input).await.err().unwrap();
        assert_eq!(error.kind(), crate::error::ErrorKind::Authentication);
        assert!(error.to_string().contains("was not found"));
    }

    #[tokio::test]
    async fn the_windows_method_comes_from_the_older_flag_as_well() {
        let mut input = connection();
        input.options.integrated_security = true;
        assert_eq!(input.effective_auth(), MssqlAuth::Integrated);

        input.options.mssql_auth = MssqlAuth::EntraAccessToken;
        // The new field wins once it holds something other than its default.
        assert_eq!(input.effective_auth(), MssqlAuth::EntraAccessToken);
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
}
