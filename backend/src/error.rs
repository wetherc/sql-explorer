// backend/src/error.rs
use serde::Serialize;

pub type Result<T> = std::result::Result<T, Error>;

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

    #[error("Database not connected")]
    NotConnected,
}

// we must manually implement serde::Serialize
impl Serialize for Error {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        serializer.serialize_str(self.to_string().as_ref())
    }
}
