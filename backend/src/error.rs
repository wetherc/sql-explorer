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
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("No open connection has the identifier '{0}'. Connect first.")]
    NotConnected(String),

    #[error("{0}")]
    Connection(String),

    #[error("The operation did not finish inside {0} seconds.")]
    Timeout(u64),

    #[error("The operation was cancelled.")]
    Cancelled,

    #[error("{0}")]
    Configuration(String),

    #[error("{0}")]
    Authentication(String),

    #[error("{0}")]
    Unsupported(String),

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

    #[error(transparent)]
    Store(#[from] tauri_plugin_store::Error),

    #[error(transparent)]
    Tauri(#[from] tauri::Error),

    #[error(transparent)]
    Keyring(#[from] keyring::Error),

    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),
}

/// The number MySQL reports when `KILL QUERY` ended a statement.
const MYSQL_QUERY_INTERRUPTED: u16 = 1317;

/// True when the failure is the one Postgres sends after a stop.
fn is_postgres_stop(error: &tokio_postgres::Error) -> bool {
    error.code() == Some(&tokio_postgres::error::SqlState::QUERY_CANCELED)
}

/// True when the failure is the one MySQL sends after a stop.
pub(crate) fn is_mysql_stop(error: &mysql_async::Error) -> bool {
    matches!(
        error,
        mysql_async::Error::Server(server) if server.code == MYSQL_QUERY_INTERRUPTED
    )
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
            // A stop reaches the server on a channel of its own, and the
            // server then ends the statement and reports that through the
            // connection. That report is the answer to the Stop button of the
            // user, not a fault of the database.
            Error::Postgres(error) if is_postgres_stop(error) => ErrorCategory::Cancelled,
            Error::MySql(error) if is_mysql_stop(error) => ErrorCategory::Cancelled,
            Error::Tiberius(tiberius::error::Error::Canceled) => ErrorCategory::Cancelled,
            Error::Tiberius(_)
            | Error::MySql(_)
            | Error::Postgres(_)
            | Error::Sqlite(_)
            | Error::Athena(_) => ErrorCategory::Database,
            Error::Io(_) => ErrorCategory::Io,
            Error::Store(_) | Error::SerdeJson(_) => ErrorCategory::Storage,
            Error::Keyring(_) => ErrorCategory::Secret,
            Error::Tauri(_) | Error::Anyhow(_) => ErrorCategory::Internal,
        }
    }

    /// Builds the payload that the user interface receives.
    pub fn to_payload(&self) -> ErrorPayload {
        ErrorPayload {
            category: self.category().as_str(),
            message: self.to_string(),
            detail: self.server_detail().or_else(|| source_chain(self)),
        }
    }

    /// What the server said about an error of its own, beside the text.
    ///
    /// MS SQL Server carries the number, the severity, the state, the line and
    /// the procedure of every error it reports. A reader needs the number to
    /// look the error up and the line to find the place in a long script.
    fn server_detail(&self) -> Option<String> {
        let Error::Tiberius(tiberius::error::Error::Server(token)) = self else {
            return None;
        };
        Some(server_error_detail(
            token.code(),
            token.class(),
            token.state(),
            token.line(),
            token.procedure(),
        ))
    }
}

/// Writes the fields that MS SQL Server sends with an error of its own. A
/// line of zero and a procedure with no name are left out, because a
/// statement that the user sent carries neither.
pub fn server_error_detail(
    code: u32,
    severity: u8,
    state: u8,
    line: u32,
    procedure: &str,
) -> String {
    let mut parts = vec![
        format!("Number {code}"),
        format!("severity {severity}"),
        format!("state {state}"),
    ];
    if line > 0 {
        parts.push(format!("line {line}"));
    }
    if !procedure.is_empty() {
        parts.push(format!("procedure {procedure}"));
    }
    parts.join(", ")
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
    fn the_detail_of_a_server_error_names_the_number_and_the_place() {
        assert_eq!(
            server_error_detail(208, 16, 1, 3, "usp_load"),
            "Number 208, severity 16, state 1, line 3, procedure usp_load"
        );
        // A statement of the user carries no procedure, and a line of zero
        // means the server named none.
        assert_eq!(
            server_error_detail(4060, 11, 1, 0, ""),
            "Number 4060, severity 11, state 1"
        );
    }

    #[test]
    fn an_error_that_is_not_of_the_server_keeps_the_chain_of_causes() {
        let error: Error = tiberius::error::Error::Tls("handshake".into()).into();
        assert_eq!(error.to_payload().detail, None);
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
        let tiberius: Error = tiberius::error::Error::Tls("handshake".into()).into();
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
    fn a_postgres_error_maps_to_the_database_category() {
        // `tokio_postgres::Error` has no public constructor, so build one
        // through a parse failure of a connection string.
        let error = "host=".parse::<tokio_postgres::Config>().unwrap_err();
        let mapped: Error = error.into();
        assert_eq!(mapped.category(), ErrorCategory::Database);
    }

    #[test]
    fn a_failure_that_mysql_sends_after_a_stop_is_a_stop() {
        let stopped = mysql_async::Error::Server(mysql_async::ServerError {
            code: MYSQL_QUERY_INTERRUPTED,
            state: "70100".to_string(),
            message: "Query execution was interrupted".to_string(),
        });
        assert_eq!(Error::MySql(stopped).category(), ErrorCategory::Cancelled);
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
}
