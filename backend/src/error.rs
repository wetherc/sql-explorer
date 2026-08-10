//! Application error type and the payload that reaches the user interface.
//!
//! The user interface must show the reason a connection or a query failed.
//! Each error therefore serialises to an object with a machine-readable
//! `kind`, a short `message` and an optional `detail` that holds the full
//! chain of causes.

use serde::Serialize;
use std::error::Error as StdError;

/// The category of an error. The user interface selects an icon and a
/// recovery action from this value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorKind {
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

impl ErrorKind {
    /// Returns the identifier used in the serialised payload.
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorKind::NotConnected => "notConnected",
            ErrorKind::Connection => "connection",
            ErrorKind::Timeout => "timeout",
            ErrorKind::Cancelled => "cancelled",
            ErrorKind::Database => "database",
            ErrorKind::Configuration => "configuration",
            ErrorKind::Authentication => "authentication",
            ErrorKind::Io => "io",
            ErrorKind::Storage => "storage",
            ErrorKind::Secret => "secret",
            ErrorKind::Unsupported => "unsupported",
            ErrorKind::Internal => "internal",
        }
    }
}

/// The structure that crosses the bridge to the user interface.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ErrorPayload {
    pub kind: &'static str,
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

impl Error {
    /// Returns the category of the error.
    pub fn kind(&self) -> ErrorKind {
        match self {
            Error::NotConnected(_) => ErrorKind::NotConnected,
            Error::Connection(_) => ErrorKind::Connection,
            Error::Timeout(_) => ErrorKind::Timeout,
            Error::Cancelled => ErrorKind::Cancelled,
            Error::Configuration(_) | Error::MySqlUrl(_) => ErrorKind::Configuration,
            Error::Authentication(_) => ErrorKind::Authentication,
            Error::Unsupported(_) => ErrorKind::Unsupported,
            Error::Tiberius(_)
            | Error::MySql(_)
            | Error::Postgres(_)
            | Error::Sqlite(_)
            | Error::Athena(_) => ErrorKind::Database,
            Error::Io(_) => ErrorKind::Io,
            Error::Store(_) | Error::SerdeJson(_) => ErrorKind::Storage,
            Error::Keyring(_) => ErrorKind::Secret,
            Error::Tauri(_) | Error::Anyhow(_) => ErrorKind::Internal,
        }
    }

    /// Builds the payload that the user interface receives.
    pub fn to_payload(&self) -> ErrorPayload {
        ErrorPayload {
            kind: self.kind().as_str(),
            message: self.to_string(),
            detail: source_chain(self),
        }
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
    fn every_kind_has_an_identifier() {
        let kinds = [
            (ErrorKind::NotConnected, "notConnected"),
            (ErrorKind::Connection, "connection"),
            (ErrorKind::Timeout, "timeout"),
            (ErrorKind::Cancelled, "cancelled"),
            (ErrorKind::Database, "database"),
            (ErrorKind::Configuration, "configuration"),
            (ErrorKind::Io, "io"),
            (ErrorKind::Storage, "storage"),
            (ErrorKind::Secret, "secret"),
            (ErrorKind::Unsupported, "unsupported"),
            (ErrorKind::Internal, "internal"),
        ];
        for (kind, text) in kinds {
            assert_eq!(kind.as_str(), text);
            assert_eq!(
                serde_json::to_value(kind).unwrap(),
                serde_json::Value::String(text.to_string())
            );
        }
    }

    #[test]
    fn not_connected_names_the_connection() {
        let error = Error::NotConnected("abc".into());
        let payload = error.to_payload();
        assert_eq!(payload.kind, "notConnected");
        assert!(payload.message.contains("abc"));
        assert_eq!(payload.detail, None);
    }

    #[test]
    fn timeout_reports_the_limit() {
        let payload = Error::Timeout(30).to_payload();
        assert_eq!(payload.kind, "timeout");
        assert!(payload.message.contains("30"));
    }

    #[test]
    fn cancelled_has_its_own_kind() {
        assert_eq!(Error::Cancelled.kind(), ErrorKind::Cancelled);
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
            Error::Configuration("port is missing".into()).kind(),
            ErrorKind::Configuration
        );
        assert_eq!(
            Error::Unsupported("no schemas".into()).kind(),
            ErrorKind::Unsupported
        );
        assert_eq!(
            Error::Athena("bad query".into()).kind(),
            ErrorKind::Database
        );
    }

    #[test]
    fn driver_errors_map_to_the_database_kind() {
        let tiberius: Error = tiberius::error::Error::Tls("handshake".into()).into();
        assert_eq!(tiberius.kind(), ErrorKind::Database);

        let mysql: Error = mysql_async::Error::Other("boom".into()).into();
        assert_eq!(mysql.kind(), ErrorKind::Database);

        let sqlite: Error = rusqlite::Error::InvalidQuery.into();
        assert_eq!(sqlite.kind(), ErrorKind::Database);

        let url: Error = mysql_async::UrlError::InvalidParamValue {
            param: "port".into(),
            value: "no".into(),
        }
        .into();
        assert_eq!(url.kind(), ErrorKind::Configuration);
    }

    #[test]
    fn io_and_storage_errors_keep_their_kind() {
        let io: Error = std::io::Error::new(std::io::ErrorKind::NotFound, "gone").into();
        assert_eq!(io.kind(), ErrorKind::Io);

        let json: Error = serde_json::from_str::<i32>("nope").unwrap_err().into();
        assert_eq!(json.kind(), ErrorKind::Storage);

        let anyhow: Error = anyhow::anyhow!("internal").into();
        assert_eq!(anyhow.kind(), ErrorKind::Internal);

        let secret: Error = keyring::Error::NoEntry.into();
        assert_eq!(secret.kind(), ErrorKind::Secret);
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
        assert_eq!(value["kind"], "cancelled");
        assert_eq!(value["message"], "The operation was cancelled.");
        assert!(value["detail"].is_null());
    }

    #[test]
    fn a_postgres_error_maps_to_the_database_kind() {
        // `tokio_postgres::Error` has no public constructor, so build one
        // through a parse failure of a connection string.
        let error = "host=".parse::<tokio_postgres::Config>().unwrap_err();
        let mapped: Error = error.into();
        assert_eq!(mapped.kind(), ErrorKind::Database);
    }
}
