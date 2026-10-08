//! Application error type and the payload that reaches the user interface.
//!
//! The user interface must show the reason a connection or a query failed.
//! Each error therefore serialises to an object with a machine-readable
//! `category`, a short `message` and an optional `detail` that holds the full
//! chain of causes.

use serde::Serialize;
use std::error::Error as StdError;

/// The category of an error. The user interface selects an icon and a
/// recovery action from this value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCategory {
    /// No open connection has the given identifier.
    NotConnected,
    /// The driver could not open or keep a connection.
    Connection,
    /// The operation did not finish inside the configured time.
    Timeout,
    /// The user stopped the operation.
    Cancelled,
    /// The server refused or failed the statement.
    Database,
    /// The connection details or the options are not valid.
    Configuration,
    /// The credentials were refused, or none could be read.
    Authentication,
    /// A local file or socket operation failed.
    Io,
    /// The stored data could not be read or written.
    Storage,
    /// The OS keychain refused the operation.
    Secret,
    /// The driver does not support the operation.
    Unsupported,
    /// The request is not valid, for a reason that is not about a connection.
    Invalid,
    /// Any other failure.
    Internal,
}

impl ErrorCategory {
    /// Returns the identifier used in the serialised payload.
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorCategory::NotConnected => "notConnected",
            ErrorCategory::Connection => "connection",
            ErrorCategory::Timeout => "timeout",
            ErrorCategory::Cancelled => "cancelled",
            ErrorCategory::Database => "database",
            ErrorCategory::Configuration => "configuration",
            ErrorCategory::Authentication => "authentication",
            ErrorCategory::Io => "io",
            ErrorCategory::Storage => "storage",
            ErrorCategory::Secret => "secret",
            ErrorCategory::Unsupported => "unsupported",
            ErrorCategory::Invalid => "invalid",
            ErrorCategory::Internal => "internal",
        }
    }
}

/// The structure that crosses the bridge to the user interface.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ErrorPayload {
    pub category: &'static str,
    pub message: String,
    pub detail: Option<String>,
    /// The line of the failure, from 1, in the text that the window sent.
    pub line: Option<u32>,
    /// The column of the failure, from 1, on that line.
    pub column: Option<u32>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Connection '{0}' isn't open. Connect to it first.")]
    NotConnected(String),

    #[error("{0}")]
    Connection(String),

    #[error("The operation timed out after {0} seconds.")]
    Timeout(u64),

    #[error("The operation was cancelled.")]
    Cancelled,

    #[error("{0}")]
    Configuration(String),

    #[error("{0}")]
    Authentication(String),

    #[error("{0}")]
    Unsupported(String),

    /// A request that is not valid, such as a parameter with no value or a
    /// file that is too large.
    #[error("{0}")]
    Invalid(String),

    /// An error at a known place in the text that the window sent.
    #[error("{inner}")]
    Located {
        inner: Box<Error>,
        line: u32,
        column: u32,
    },

    /// A read that waited for a lock of another session longer than the
    /// lock limit of its own session. The inner error is the one that the
    /// server sent.
    #[error(
        "Another session has locked this object, so the read stopped waiting. Commit or roll \
         back that session's transaction, then try again."
    )]
    LockWait(Box<Error>),

    /// A Windows Authentication login that could not reach the Kerberos
    /// server (KDC). The inner error is the time limit of the connect or
    /// the GSSAPI error that names the KDC.
    #[error("Couldn't reach the Kerberos server. Check your VPN or network connection.")]
    KerberosUnreachable(Box<Error>),

    #[error(transparent)]
    Tiberius(#[from] tiberius::error::Error),

    #[error(transparent)]
    MySql(#[from] mysql_async::Error),

    #[error(transparent)]
    MySqlUrl(#[from] mysql_async::UrlError),

    #[error(transparent)]
    Postgres(#[from] tokio_postgres::Error),

    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),

    #[error("{0}")]
    Athena(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    SerdeJson(#[from] serde_json::Error),

    /// A file of the settings that could not be read or written.
    #[error("{0}")]
    Storage(String),

    #[error(transparent)]
    Tauri(#[from] tauri::Error),

    #[error(transparent)]
    Keyring(#[from] keyring::Error),

    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),
}

/// The number MySQL reports when `KILL QUERY` ended a statement.
const MYSQL_QUERY_INTERRUPTED: u16 = 1317;

/// The numbers MySQL and MariaDB send when they refuse a login: access
/// denied for the user (1045), access denied to the database (1044), and no
/// password for an account that uses socket login (1698).
const MYSQL_LOGIN_REFUSED: [u16; 3] = [1045, 1044, 1698];

/// The number MS SQL Server sends when it refuses a login.
const MSSQL_LOGIN_FAILED: u32 = 18456;

/// The number MS SQL Server sends when a statement waited for a lock longer
/// than the `LOCK_TIMEOUT` of the session.
const MSSQL_LOCK_TIMEOUT: u32 = 1222;

/// The number MySQL and MariaDB send when a statement waited for a lock
/// longer than `lock_wait_timeout` or `innodb_lock_wait_timeout`.
const MYSQL_LOCK_WAIT_TIMEOUT: u16 = 1205;

/// True when the failure is the one MySQL sends after a stop. The MySQL
/// driver uses it to know that a stop ended the statement.
pub(crate) fn is_mysql_stop(error: &mysql_async::Error) -> bool {
    matches!(
        error,
        mysql_async::Error::Server(server) if server.code == MYSQL_QUERY_INTERRUPTED
    )
}

/// Gives the category of a Postgres error. The driver keeps the reason of a
/// failure private, but its text names the reason and the vendored driver
/// fixes that text, so the match is on the text.
fn postgres_category(error: &tokio_postgres::Error) -> ErrorCategory {
    if let Some(db) = error.as_db_error() {
        // Class 28 is "invalid authorization specification".
        return if db.code().code().starts_with("28") {
            ErrorCategory::Authentication
        } else {
            ErrorCategory::Database
        };
    }
    if error.is_closed() {
        return ErrorCategory::Connection;
    }
    match error.to_string().as_str() {
        "error communicating with the server"
        | "error performing TLS handshake"
        | "error connecting to server"
        | "timeout waiting for server" => ErrorCategory::Connection,
        "authentication error" => ErrorCategory::Authentication,
        "invalid connection string" | "invalid configuration" => ErrorCategory::Configuration,
        _ => ErrorCategory::Database,
    }
}

/// Gives the category of a MySQL error.
fn mysql_category(error: &mysql_async::Error) -> ErrorCategory {
    use mysql_async::{DriverError, Error as MySqlError};
    match error {
        MySqlError::Io(_)
        | MySqlError::Driver(DriverError::ConnectionClosed | DriverError::PoolDisconnected) => {
            ErrorCategory::Connection
        }
        MySqlError::Server(server) if MYSQL_LOGIN_REFUSED.contains(&server.code) => {
            ErrorCategory::Authentication
        }
        MySqlError::Url(_) => ErrorCategory::Configuration,
        _ => ErrorCategory::Database,
    }
}

/// Gives the category of an MS SQL Server error.
fn mssql_category(error: &tiberius::error::Error) -> ErrorCategory {
    use tiberius::error::Error as MsError;
    match error {
        // The Stop button sends an attention signal, and the server answers
        // that with this error.
        MsError::Canceled => ErrorCategory::Cancelled,
        MsError::Io { .. } | MsError::Tls(_) | MsError::Routing { .. } => ErrorCategory::Connection,
        MsError::Gssapi(_) => ErrorCategory::Authentication,
        MsError::Server(token) if token.code() == MSSQL_LOGIN_FAILED => {
            ErrorCategory::Authentication
        }
        _ => ErrorCategory::Database,
    }
}

/// Gives the category of a SQLite error. A file that SQLite can't open is a
/// fault of the file, not of a statement.
fn sqlite_category(error: &rusqlite::Error) -> ErrorCategory {
    match error.sqlite_error_code() {
        Some(rusqlite::ErrorCode::CannotOpen) => ErrorCategory::Io,
        _ => ErrorCategory::Database,
    }
}

impl Error {
    /// Returns the category of the error.
    pub fn category(&self) -> ErrorCategory {
        match self {
            Error::NotConnected(_) => ErrorCategory::NotConnected,
            Error::Connection(_) => ErrorCategory::Connection,
            Error::Timeout(_) => ErrorCategory::Timeout,
            Error::Cancelled => ErrorCategory::Cancelled,
            Error::Configuration(_) | Error::MySqlUrl(_) => ErrorCategory::Configuration,
            Error::Authentication(_) => ErrorCategory::Authentication,
            Error::Unsupported(_) => ErrorCategory::Unsupported,
            Error::Invalid(_) => ErrorCategory::Invalid,
            Error::Located { inner, .. } => inner.category(),
            // The advice for a timeout names the timeout of the connection,
            // which does not change the lock limit.
            Error::LockWait(_) => ErrorCategory::Database,
            Error::KerberosUnreachable(_) => ErrorCategory::Connection,
            Error::Tiberius(error) => mssql_category(error),
            Error::MySql(error) => mysql_category(error),
            Error::Postgres(error) => postgres_category(error),
            Error::Sqlite(error) => sqlite_category(error),
            Error::Athena(_) => ErrorCategory::Database,
            Error::Io(_) => ErrorCategory::Io,
            Error::Storage(_) | Error::SerdeJson(_) => ErrorCategory::Storage,
            Error::Keyring(_) => ErrorCategory::Secret,
            Error::Tauri(_) | Error::Anyhow(_) => ErrorCategory::Internal,
        }
    }

    /// True when the error is what an engine sends after a request to stop
    /// the statement, or a loss of the connection that a stop can cause.
    /// Other errors, such as a syntax error or a deadlock, are not.
    pub fn is_stop_reply(&self) -> bool {
        match self {
            Error::Located { inner, .. } => inner.is_stop_reply(),
            Error::Postgres(error) => {
                error
                    .as_db_error()
                    .is_some_and(|db| db.code().code() == "57014")
                    || self.category() == ErrorCategory::Connection
            }
            Error::MySql(error) => {
                is_mysql_stop(error) || self.category() == ErrorCategory::Connection
            }
            Error::Sqlite(error) => {
                error.sqlite_error_code() == Some(rusqlite::ErrorCode::OperationInterrupted)
            }
            _ => matches!(
                self.category(),
                ErrorCategory::Cancelled | ErrorCategory::Connection | ErrorCategory::Io
            ),
        }
    }

    /// True when the server ended the statement because it waited for a lock
    /// longer than the lock limit of the session.
    pub fn is_lock_wait(&self) -> bool {
        match self {
            Error::Tiberius(tiberius::error::Error::Server(token)) => {
                token.code() == MSSQL_LOCK_TIMEOUT
            }
            Error::Postgres(error) => {
                error.code() == Some(&tokio_postgres::error::SqlState::LOCK_NOT_AVAILABLE)
            }
            Error::MySql(mysql_async::Error::Server(server)) => {
                server.code == MYSQL_LOCK_WAIT_TIMEOUT
            }
            _ => false,
        }
    }

    /// Gives an error of a lock wait as [`Error::LockWait`], so the user
    /// reads the cause in plain words. Every other error stays as it is.
    pub fn name_lock_wait(self) -> Error {
        if self.is_lock_wait() {
            Error::LockWait(Box::new(self))
        } else {
            self
        }
    }

    /// Marks the error with the place in the sent text where it happened.
    /// The line and the column count from 1. An error that already has a
    /// place keeps it.
    pub fn at(self, line: u32, column: u32) -> Error {
        match self {
            Error::Located { .. } => self,
            inner => Error::Located {
                inner: Box::new(inner),
                line,
                column,
            },
        }
    }

    /// Builds the payload that the user interface receives.
    pub fn to_payload(&self) -> ErrorPayload {
        if let Error::Located {
            inner,
            line,
            column,
        } = self
        {
            return ErrorPayload {
                line: Some(*line),
                column: Some(*column),
                ..inner.to_payload()
            };
        }
        let (message, detail) = match self {
            Error::Postgres(error) => postgres_text(error),
            Error::MySql(mysql_async::Error::Server(server)) => (
                server.message.clone(),
                Some(format!("Error {}, SQLSTATE {}", server.code, server.state)),
            ),
            Error::Tiberius(tiberius::error::Error::Server(token)) => {
                (self.to_string(), Some(mssql_error_detail(token)))
            }
            // The detail gives the text of the server, so the user can look
            // the error up.
            Error::LockWait(inner) => {
                let (text, detail) = match inner.as_ref() {
                    Error::Tiberius(tiberius::error::Error::Server(token)) => {
                        (token.message().to_string(), Some(mssql_error_detail(token)))
                    }
                    other => {
                        let payload = other.to_payload();
                        (payload.message, payload.detail)
                    }
                };
                (self.to_string(), Some(joined_detail(text, detail)))
            }
            // The detail keeps the text of the driver, so the user can see
            // whether the time limit passed or the GSSAPI library failed.
            Error::KerberosUnreachable(inner) => {
                let payload = inner.to_payload();
                (
                    self.to_string(),
                    Some(joined_detail(payload.message, payload.detail)),
                )
            }
            _ => (self.to_string(), source_chain(self)),
        };
        ErrorPayload {
            category: self.category().as_str(),
            message,
            detail,
            line: None,
            column: None,
        }
    }
}

/// Puts the text of an inner error and its detail on separate lines.
fn joined_detail(text: String, detail: Option<String>) -> String {
    match detail {
        Some(detail) => format!("{text}\n{detail}"),
        None => text,
    }
}

/// Writes the fields that MS SQL Server sends with an error of its own, in
/// the form that SQL Server Management Studio shows. A reader needs the
/// number to look the error up and the line to find the place in a long
/// script. A procedure with no name is left out, because a statement that
/// the user sent has none.
pub fn server_error_detail(
    code: u32,
    severity: u8,
    state: u8,
    line: u32,
    procedure: &str,
) -> String {
    let mut text = format!("Msg {code}, Level {severity}, State {state}, Line {line}");
    if !procedure.is_empty() {
        text.push_str(", Procedure ");
        text.push_str(procedure);
    }
    text
}

/// Writes the detail of an error or a message that MS SQL Server sent.
pub fn mssql_error_detail(token: &tiberius::error::TokenError) -> String {
    server_error_detail(
        token.code(),
        token.class(),
        token.state(),
        token.line(),
        token.procedure(),
    )
}

/// Gives the text and the detail of a Postgres error. A statement error
/// shows the text of the server, and the detail gives the SQLSTATE and every
/// other field that the server sent. Any other error names the reason of the
/// driver and its first cause, because the reason alone, such as "error
/// connecting to server", does not tell the user what to fix.
fn postgres_text(error: &tokio_postgres::Error) -> (String, Option<String>) {
    let Some(db) = error.as_db_error() else {
        let Some(cause) = error.source() else {
            return (error.to_string(), None);
        };
        return (format!("{error}: {cause}"), source_chain(cause));
    };
    let mut lines = vec![format!(
        "SQLSTATE {}, severity {}",
        db.code().code(),
        db.severity()
    )];
    let mut field = |name: &str, value: Option<&str>| {
        if let Some(value) = value {
            lines.push(format!("{name}: {value}"));
        }
    };
    field("Detail", db.detail());
    field("Hint", db.hint());
    let position = match db.position() {
        Some(tokio_postgres::error::ErrorPosition::Original(at)) => Some(at.to_string()),
        Some(tokio_postgres::error::ErrorPosition::Internal { position, query }) => {
            Some(format!("{position} in the internal query: {query}"))
        }
        None => None,
    };
    field("Position", position.as_deref());
    field("Where", db.where_());
    field("Schema", db.schema());
    field("Table", db.table());
    field("Column", db.column());
    field("Data type", db.datatype());
    field("Constraint", db.constraint());
    (db.message().to_string(), Some(lines.join("\n")))
}

/// A place in a text: the line and the column, both from 1. The column
/// counts UTF-16 code units, as the editor of the window does.
pub type Place = (u32, u32);

/// Finds the place that follows the given characters. A line ends at `\n`,
/// and a `\r` takes no column, so a text with Windows line ends gives the
/// same places as one with `\n` only.
fn place_after<I: Iterator<Item = char>>(chars: I) -> Place {
    let (mut line, mut column) = (1u32, 1u32);
    for c in chars {
        match c {
            '\n' => {
                line += 1;
                column = 1;
            }
            '\r' => {}
            other => column += other.len_utf16() as u32,
        }
    }
    (line, column)
}

/// Turns a Postgres error position into a place. Postgres counts the
/// position in characters, from 1, inside the statement text that it got.
/// A position of zero is treated as 1, and a position past the end gives
/// the place after the last character.
pub fn place_of_char_position(text: &str, position: u32) -> Place {
    let before = position.saturating_sub(1) as usize;
    place_after(text.chars().take(before))
}

/// Gives the place of the byte at `offset` in `text`, such as the start of a
/// statement inside a script. An offset that is not on a character boundary
/// counts the character that contains it as before the place.
pub fn place_of_byte_offset(text: &str, offset: usize) -> Place {
    place_after(
        text.char_indices()
            .take_while(|(index, _)| *index < offset)
            .map(|(_, c)| c),
    )
}

/// Moves a place inside a statement to a place inside the whole text, from
/// the place where the statement starts. Only the first line of the
/// statement shares a line with the text before it, so only that line gets
/// the start column added.
pub fn offset_place(start: Place, relative: Place) -> Place {
    let (start_line, start_column) = start;
    let (line, column) = relative;
    if line <= 1 {
        (start_line, start_column + column.max(1) - 1)
    } else {
        (start_line + line - 1, column)
    }
}

/// Joins every cause below the given error into one text block. Returns
/// `None` when the error has no cause.
fn source_chain(error: &dyn StdError) -> Option<String> {
    let mut causes: Vec<String> = Vec::new();
    let mut current = error.source();
    while let Some(cause) = current {
        causes.push(cause.to_string());
        current = cause.source();
    }
    if causes.is_empty() {
        None
    } else {
        Some(causes.join("\n"))
    }
}

impl Serialize for Error {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        log::error!("Command failed: {self:?}");
        self.to_payload().serialize(serializer)
    }
}

/// The result type that every command returns.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_detail_of_a_server_error_has_the_form_of_management_studio() {
        assert_eq!(
            server_error_detail(208, 16, 1, 3, "usp_load"),
            "Msg 208, Level 16, State 1, Line 3, Procedure usp_load"
        );
        // A statement of the user has no procedure.
        assert_eq!(
            server_error_detail(4060, 11, 1, 1, ""),
            "Msg 4060, Level 11, State 1, Line 1"
        );
    }

    #[test]
    fn an_error_that_is_not_of_the_server_keeps_the_chain_of_causes() {
        let error: Error = tiberius::error::Error::Tls("handshake".into()).into();
        let payload = error.to_payload();
        assert_eq!(payload.category, "connection");
        assert_eq!(payload.detail, None);
    }

    #[test]
    fn every_category_has_an_identifier() {
        let categories = [
            (ErrorCategory::NotConnected, "notConnected"),
            (ErrorCategory::Connection, "connection"),
            (ErrorCategory::Timeout, "timeout"),
            (ErrorCategory::Cancelled, "cancelled"),
            (ErrorCategory::Database, "database"),
            (ErrorCategory::Configuration, "configuration"),
            (ErrorCategory::Io, "io"),
            (ErrorCategory::Storage, "storage"),
            (ErrorCategory::Secret, "secret"),
            (ErrorCategory::Unsupported, "unsupported"),
            (ErrorCategory::Authentication, "authentication"),
            (ErrorCategory::Invalid, "invalid"),
            (ErrorCategory::Internal, "internal"),
        ];
        for (category, text) in categories {
            assert_eq!(category.as_str(), text);
            assert_eq!(
                serde_json::to_value(category).unwrap(),
                serde_json::Value::String(text.to_string())
            );
        }
    }

    #[test]
    fn not_connected_names_the_connection() {
        let error = Error::NotConnected("abc".into());
        let payload = error.to_payload();
        assert_eq!(payload.category, "notConnected");
        assert!(payload.message.contains("abc"));
        assert_eq!(payload.detail, None);
    }

    #[test]
    fn timeout_reports_the_limit() {
        let payload = Error::Timeout(30).to_payload();
        assert_eq!(payload.category, "timeout");
        assert!(payload.message.contains("30"));
    }

    #[test]
    fn cancelled_has_its_own_category() {
        assert_eq!(Error::Cancelled.category(), ErrorCategory::Cancelled);
    }

    #[test]
    fn connection_and_configuration_keep_the_text() {
        assert_eq!(
            Error::Connection("host is down".into())
                .to_payload()
                .message,
            "host is down"
        );
        assert_eq!(
            Error::Configuration("port is missing".into()).category(),
            ErrorCategory::Configuration
        );
        assert_eq!(
            Error::Unsupported("no schemas".into()).category(),
            ErrorCategory::Unsupported
        );
        assert_eq!(
            Error::Athena("bad query".into()).category(),
            ErrorCategory::Database
        );
    }

    #[test]
    fn driver_errors_map_to_the_database_category() {
        let tiberius: Error = tiberius::error::Error::Protocol("bad packet".into()).into();
        assert_eq!(tiberius.category(), ErrorCategory::Database);

        let mysql: Error = mysql_async::Error::Other("boom".into()).into();
        assert_eq!(mysql.category(), ErrorCategory::Database);

        let sqlite: Error = rusqlite::Error::InvalidQuery.into();
        assert_eq!(sqlite.category(), ErrorCategory::Database);

        let url: Error = mysql_async::UrlError::InvalidParamValue {
            param: "port".into(),
            value: "no".into(),
        }
        .into();
        assert_eq!(url.category(), ErrorCategory::Configuration);
    }

    #[test]
    fn io_and_storage_errors_keep_their_category() {
        let io: Error = std::io::Error::new(std::io::ErrorKind::NotFound, "gone").into();
        assert_eq!(io.category(), ErrorCategory::Io);

        let json: Error = serde_json::from_str::<i32>("nope").unwrap_err().into();
        assert_eq!(json.category(), ErrorCategory::Storage);

        let anyhow: Error = anyhow::anyhow!("internal").into();
        assert_eq!(anyhow.category(), ErrorCategory::Internal);

        let secret: Error = keyring::Error::NoEntry.into();
        assert_eq!(secret.category(), ErrorCategory::Secret);
    }

    #[test]
    fn a_lock_wait_of_the_server_gets_a_plain_message() {
        let mssql = Error::Tiberius(tiberius::error::Error::Server(
            tiberius::error::TokenError::new(
                1222,
                56,
                16,
                "Lock request time out period exceeded.",
                "",
                1,
            ),
        ));
        assert!(mssql.is_lock_wait());

        let named = mssql.name_lock_wait();
        assert!(matches!(named, Error::LockWait(_)));
        assert_eq!(named.category(), ErrorCategory::Database);
        assert!(!named.is_stop_reply());
        let payload = named.to_payload();
        assert!(payload
            .message
            .starts_with("Another session has locked this object"));
        assert_eq!(
            payload.detail.as_deref(),
            Some("Lock request time out period exceeded.\nMsg 1222, Level 16, State 56, Line 1")
        );

        let mysql = Error::MySql(mysql_async::Error::Server(mysql_async::ServerError {
            code: 1205,
            state: "HY000".to_string(),
            message: "Lock wait timeout exceeded; try restarting transaction".to_string(),
        }));
        assert!(mysql.is_lock_wait());
        assert_eq!(
            mysql.name_lock_wait().to_payload().detail.as_deref(),
            Some("Lock wait timeout exceeded; try restarting transaction\nError 1205, SQLSTATE HY000")
        );
    }

    #[test]
    fn other_errors_are_not_lock_waits() {
        let missing = Error::Tiberius(tiberius::error::Error::Server(
            tiberius::error::TokenError::new(208, 1, 16, "Invalid object name 't'.", "", 1),
        ));
        assert!(!missing.is_lock_wait());
        assert!(matches!(missing.name_lock_wait(), Error::Tiberius(_)));

        let refused = Error::MySql(mysql_async::Error::Server(mysql_async::ServerError {
            code: 1045,
            state: "28000".to_string(),
            message: "Access denied".to_string(),
        }));
        assert!(!refused.is_lock_wait());

        let postgres = "port=nope".parse::<tokio_postgres::Config>().unwrap_err();
        assert!(!Error::Postgres(postgres).is_lock_wait());
        assert!(!Error::Cancelled.is_lock_wait());
    }

    #[test]
    fn an_unreachable_kerberos_server_keeps_the_driver_text_in_the_detail() {
        let late = Error::KerberosUnreachable(Box::new(Error::Connection(
            "The server didn't finish opening the connection within 5 seconds.".into(),
        )));
        assert_eq!(late.category(), ErrorCategory::Connection);
        let payload = late.to_payload();
        assert_eq!(payload.category, "connection");
        assert_eq!(
            payload.message,
            "Couldn't reach the Kerberos server. Check your VPN or network connection."
        );
        assert_eq!(
            payload.detail.as_deref(),
            Some("The server didn't finish opening the connection within 5 seconds.")
        );

        let gssapi = Error::KerberosUnreachable(Box::new(Error::Anyhow(
            anyhow::Error::new(std::io::Error::other("Cannot contact any KDC"))
                .context("GSSAPI Error"),
        )));
        assert_eq!(
            gssapi.to_payload().detail.as_deref(),
            Some("GSSAPI Error\nCannot contact any KDC")
        );
    }

    #[test]
    fn a_lock_wait_without_a_server_detail_gives_the_server_text_alone() {
        let payload = Error::LockWait(Box::new(Error::Connection("gone".into()))).to_payload();
        assert_eq!(payload.detail.as_deref(), Some("gone"));
    }

    #[test]
    fn the_detail_holds_the_chain_of_causes() {
        let inner = std::io::Error::other("socket closed");
        let wrapped = anyhow::Error::new(inner).context("while reading the result");
        let payload = Error::Anyhow(wrapped).to_payload();
        assert_eq!(payload.message, "while reading the result");
        assert_eq!(payload.detail.as_deref(), Some("socket closed"));
    }

    #[test]
    fn serialisation_produces_the_three_fields() {
        let value = serde_json::to_value(Error::Cancelled).unwrap();
        assert_eq!(value["category"], "cancelled");
        assert_eq!(value["message"], "The operation was cancelled.");
        assert!(value["detail"].is_null());
    }

    #[test]
    fn a_bad_postgres_connection_string_is_a_configuration_error() {
        // `tokio_postgres::Error` has no public constructor, so build one
        // through a parse failure of a connection string.
        let error = "port=nope".parse::<tokio_postgres::Config>().unwrap_err();
        let mapped: Error = error.into();
        assert_eq!(mapped.category(), ErrorCategory::Configuration);
        let payload = mapped.to_payload();
        assert!(
            payload.message.starts_with("invalid connection string: "),
            "{}",
            payload.message
        );
        assert_eq!(payload.detail, None);
    }

    #[test]
    fn a_postgres_error_with_no_cause_gives_the_reason_alone() {
        // The parse of a value with no end quote fails with no cause.
        let error = "host='open".parse::<tokio_postgres::Config>().unwrap_err();
        let payload = Error::Postgres(error).to_payload();
        assert!(payload.message.starts_with("invalid connection string"));
    }

    #[tokio::test]
    async fn a_postgres_server_that_does_not_answer_is_a_connection_error() {
        // Port 1 has no server, so the connect fails at once.
        let error = tokio_postgres::connect(
            "host=127.0.0.1 port=1 user=x connect_timeout=5",
            tokio_postgres::NoTls,
        )
        .await
        .err()
        .unwrap();
        let mapped = Error::Postgres(error);
        assert_eq!(mapped.category(), ErrorCategory::Connection);
        let payload = mapped.to_payload();
        assert!(
            payload.message.starts_with("error connecting to server: "),
            "{}",
            payload.message
        );
    }

    #[test]
    fn a_mysql_stop_is_known_but_the_category_stays_database() {
        // The server sends the same error for a KILL from another session,
        // so only the token of the run can tell a stop of the user.
        let stopped = mysql_async::Error::Server(mysql_async::ServerError {
            code: MYSQL_QUERY_INTERRUPTED,
            state: "70100".to_string(),
            message: "Query execution was interrupted".to_string(),
        });
        assert!(is_mysql_stop(&stopped));
        assert_eq!(Error::MySql(stopped).category(), ErrorCategory::Database);
        assert!(!is_mysql_stop(&mysql_async::Error::Other("boom".into())));
    }

    #[test]
    fn a_refused_mysql_login_is_an_authentication_error() {
        for code in [1045, 1044, 1698] {
            let refused = mysql_async::Error::Server(mysql_async::ServerError {
                code,
                state: "28000".to_string(),
                message: "Access denied for user 'x'@'localhost'".to_string(),
            });
            assert_eq!(
                Error::MySql(refused).category(),
                ErrorCategory::Authentication
            );
        }
    }

    #[test]
    fn a_mysql_connection_fault_is_a_connection_error() {
        let io = mysql_async::Error::Io(mysql_async::IoError::Io(std::io::Error::other("reset")));
        assert_eq!(Error::MySql(io).category(), ErrorCategory::Connection);
        for driver in [
            mysql_async::DriverError::ConnectionClosed,
            mysql_async::DriverError::PoolDisconnected,
        ] {
            assert_eq!(
                Error::MySql(mysql_async::Error::Driver(driver)).category(),
                ErrorCategory::Connection
            );
        }
        let url = mysql_async::Error::Url(mysql_async::UrlError::UnsupportedScheme {
            scheme: "http".into(),
        });
        assert_eq!(Error::MySql(url).category(), ErrorCategory::Configuration);
    }

    #[test]
    fn a_mysql_server_error_gives_the_text_and_the_number() {
        let error = Error::MySql(mysql_async::Error::Server(mysql_async::ServerError {
            code: 1146,
            state: "42S02".to_string(),
            message: "Table 'db.nope' doesn't exist".to_string(),
        }));
        let payload = error.to_payload();
        assert_eq!(payload.category, "database");
        assert_eq!(payload.message, "Table 'db.nope' doesn't exist");
        assert_eq!(
            payload.detail.as_deref(),
            Some("Error 1146, SQLSTATE 42S02")
        );
    }

    #[test]
    fn mssql_errors_take_the_category_of_their_cause() {
        use tiberius::error::Error as MsError;
        let connection = [
            MsError::Io {
                kind: std::io::ErrorKind::ConnectionReset,
                message: "reset".into(),
            },
            MsError::Tls("handshake".into()),
            MsError::Routing {
                host: "other".into(),
                port: 1433,
            },
        ];
        for error in connection {
            assert_eq!(Error::Tiberius(error).category(), ErrorCategory::Connection);
        }
        assert_eq!(
            Error::Tiberius(MsError::Gssapi("no ticket".into())).category(),
            ErrorCategory::Authentication
        );
    }

    #[test]
    fn a_sqlite_file_that_cannot_open_is_a_file_error() {
        let missing = std::env::temp_dir()
            .join("sql-explorer-no-such-folder")
            .join("x.db");
        let error = rusqlite::Connection::open_with_flags(
            missing,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
        )
        .err()
        .unwrap();
        assert_eq!(Error::Sqlite(error).category(), ErrorCategory::Io);
    }

    #[test]
    fn only_a_stop_or_a_lost_connection_is_a_stop_reply() {
        let mysql = |code: u16| {
            Error::MySql(mysql_async::Error::Server(mysql_async::ServerError {
                code,
                state: "70100".to_string(),
                message: "stopped".to_string(),
            }))
        };
        let lost = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "reset");
        let stops = [
            Error::Cancelled,
            Error::Tiberius(tiberius::error::Error::Canceled),
            mysql(MYSQL_QUERY_INTERRUPTED),
            Error::MySql(mysql_async::Error::Io(mysql_async::IoError::Io(
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "pipe"),
            ))),
            Error::Postgres(tokio_postgres::Error::__private_api_timeout()),
            Error::Sqlite(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_INTERRUPT),
                None,
            )),
            Error::Io(lost),
            Error::Connection("gone".into()),
            Error::Cancelled.at(2, 3),
        ];
        for error in stops {
            assert!(error.is_stop_reply(), "{error:?}");
        }

        let failures = [
            mysql(1064),
            mysql(1213),
            Error::Postgres("port=nope".parse::<tokio_postgres::Config>().unwrap_err()),
            Error::Sqlite(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
                None,
            )),
            Error::Athena("syntax error".into()),
            Error::Invalid("bad".into()).at(1, 1),
        ];
        for error in failures {
            assert!(!error.is_stop_reply(), "{error:?}");
        }
    }

    #[test]
    fn the_cancel_answer_of_mssql_is_a_stop() {
        let stopped = Error::Tiberius(tiberius::error::Error::Canceled);
        assert_eq!(stopped.category(), ErrorCategory::Cancelled);
    }

    #[test]
    fn another_failure_of_mysql_stays_a_fault_of_the_database() {
        let other = mysql_async::Error::Server(mysql_async::ServerError {
            code: 1064,
            state: "42000".to_string(),
            message: "You have an error in your SQL syntax".to_string(),
        });
        assert_eq!(Error::MySql(other).category(), ErrorCategory::Database);

        let outside: Error = mysql_async::Error::Other("boom".into()).into();
        assert_eq!(outside.category(), ErrorCategory::Database);
    }

    #[test]
    fn an_invalid_request_has_its_own_category() {
        let error = Error::Invalid("Parameter ':x' needs a value.".into());
        let payload = error.to_payload();
        assert_eq!(payload.category, "invalid");
        assert_eq!(payload.message, "Parameter ':x' needs a value.");
        assert_eq!((payload.line, payload.column), (None, None));
    }

    #[test]
    fn a_located_error_gives_its_place_and_the_payload_of_its_cause() {
        let error = Error::Timeout(5).at(3, 7);
        assert_eq!(error.category(), ErrorCategory::Timeout);
        assert_eq!(
            error.to_string(),
            "The operation timed out after 5 seconds."
        );
        let payload = error.to_payload();
        assert_eq!(payload.category, "timeout");
        assert_eq!((payload.line, payload.column), (Some(3), Some(7)));
        let value = serde_json::to_value(&payload).unwrap();
        assert_eq!(value["line"], 3);
        assert_eq!(value["column"], 7);
    }

    #[test]
    fn a_located_error_keeps_its_first_place() {
        let error = Error::Cancelled.at(2, 1).at(9, 9);
        let payload = error.to_payload();
        assert_eq!((payload.line, payload.column), (Some(2), Some(1)));
    }

    #[test]
    fn a_postgres_position_becomes_a_place() {
        let text = "SELECT\n  nope\r\nFROM t";
        assert_eq!(place_of_char_position(text, 1), (1, 1));
        assert_eq!(place_of_char_position(text, 0), (1, 1));
        assert_eq!(place_of_char_position(text, 10), (2, 3));
        // The `\r` of a Windows line end takes no column.
        assert_eq!(place_of_char_position(text, 16), (3, 1));
        assert_eq!(place_of_char_position(text, 99), (3, 7));
        // Postgres counts characters. The editor counts UTF-16 units, so a
        // character outside the basic plane takes two columns.
        assert_eq!(place_of_char_position("SELECT '\u{1F600}' x", 11), (1, 12));
        assert_eq!(place_of_char_position("SELECT 'é' x", 11), (1, 11));
    }

    #[test]
    fn a_byte_offset_becomes_a_place() {
        let text = "SELECT 1;\nSELECT 'é';\n  SELECT 3";
        assert_eq!(place_of_byte_offset(text, 0), (1, 1));
        assert_eq!(place_of_byte_offset(text, 10), (2, 1));
        let third = text.rfind("SELECT").unwrap();
        assert_eq!(place_of_byte_offset(text, third), (3, 3));
        // An offset inside the two bytes of `é` counts that character.
        let inside = text.find('é').unwrap() + 1;
        assert_eq!(place_of_byte_offset(text, inside), (2, 10));
    }

    #[test]
    fn a_place_in_a_statement_moves_by_the_start_of_the_statement() {
        assert_eq!(offset_place((1, 1), (1, 1)), (1, 1));
        assert_eq!(offset_place((4, 5), (1, 3)), (4, 7));
        assert_eq!(offset_place((4, 5), (1, 0)), (4, 5));
        assert_eq!(offset_place((4, 5), (3, 2)), (6, 2));
    }

    /// Opens a client of the live Postgres server, with the password that
    /// the caller gives.
    async fn live_postgres(
        password: &str,
    ) -> Option<std::result::Result<tokio_postgres::Client, tokio_postgres::Error>> {
        let server = crate::db::drivers::live::server("SQLX_LIVE_PG")?;
        let mut config = tokio_postgres::Config::new();
        config
            .host(&server.host)
            .port(server.port)
            .user(&server.user)
            .password(password)
            .dbname("postgres");
        Some(
            config
                .connect(tokio_postgres::NoTls)
                .await
                .map(|(client, connection)| {
                    tokio::spawn(connection);
                    client
                }),
        )
    }

    #[tokio::test]
    #[ignore]
    async fn live_a_postgres_statement_error_gives_the_text_and_the_fields_of_the_server() {
        let Some(server) = crate::db::drivers::live::server("SQLX_LIVE_PG") else {
            return;
        };
        let client = live_postgres(&server.password).await.unwrap().unwrap();
        let error = client.simple_query("SELECT nope").await.err().unwrap();
        let mapped = Error::Postgres(error);
        assert_eq!(mapped.category(), ErrorCategory::Database);
        let payload = mapped.to_payload();
        assert_eq!(payload.message, "column \"nope\" does not exist");
        let detail = payload.detail.unwrap();
        assert!(
            detail.starts_with("SQLSTATE 42703, severity ERROR\n"),
            "{detail}"
        );
        assert!(detail.contains("Position: 8"), "{detail}");

        // A server stop, such as a statement timeout, stays a database error,
        // because only the token of the run knows that the user stopped it.
        client
            .simple_query("SET statement_timeout = 10")
            .await
            .unwrap();
        let error = client
            .simple_query("SELECT pg_sleep(5)")
            .await
            .err()
            .unwrap();
        assert_eq!(Error::Postgres(error).category(), ErrorCategory::Database);
    }

    #[tokio::test]
    #[ignore]
    async fn live_a_refused_postgres_login_is_an_authentication_error() {
        let Some(result) = live_postgres("not the password").await else {
            return;
        };
        let mapped = Error::Postgres(result.err().unwrap());
        assert_eq!(mapped.category(), ErrorCategory::Authentication);
        assert!(mapped
            .to_payload()
            .detail
            .unwrap()
            .starts_with("SQLSTATE 28P01"));
    }

    #[tokio::test]
    #[ignore]
    async fn live_a_refused_mysql_login_is_an_authentication_error() {
        let Some(server) = crate::db::drivers::live::server("SQLX_LIVE_MYSQL") else {
            return;
        };
        let options = mysql_async::OptsBuilder::default()
            .ip_or_hostname(server.host)
            .tcp_port(server.port)
            .user(Some(server.user))
            .pass(Some("not the password"));
        let error = mysql_async::Conn::new(options).await.err().unwrap();
        let mapped = Error::MySql(error);
        assert_eq!(mapped.category(), ErrorCategory::Authentication);
        assert!(mapped
            .to_payload()
            .detail
            .unwrap()
            .starts_with("Error 1045, SQLSTATE 28000"));
    }
}
