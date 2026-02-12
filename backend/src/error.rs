// backend/src/error.rs
use serde::Serialize;
use tiberius;
use mysql_async;
use tokio_postgres;
use tauri_plugin_store;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Tiberius(#[from] tiberius::error::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),

    #[error(transparent)]
    SerdeJson(#[from] serde_json::Error),

    #[error(transparent)]
    Keyring(#[from] keyring::Error),

    #[error(transparent)]
    MySqlUrl(#[from] mysql_async::UrlError),

    #[error(transparent)]
    MySql(#[from] mysql_async::Error),

    #[error(transparent)]
    Postgres(#[from] tokio_postgres::Error),

    #[error(transparent)]
    Store(#[from] tauri_plugin_store::Error),

    #[error("Database not connected")]
    NotConnected,
}

// we must manually implement serde::Serialize
impl Serialize for Error {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        let error_message = match self {
            Error::Tiberius(_) | Error::MySql(_) | Error::Postgres(_) => "Database error occurred.".to_string(),
            Error::Io(_) | Error::MySqlUrl(_) => "Connection error occurred.".to_string(),
            Error::NotConnected => self.to_string(), // Keep specific message for NotConnected
            _ => "An unexpected error occurred.".to_string(),
        };
        
        // Log the full error for backend debugging
        log::error!("Backend Error: {}", self);

        serializer.serialize_str(&error_message)
    }
}
