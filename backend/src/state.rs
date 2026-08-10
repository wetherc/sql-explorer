//! The state the application holds while it runs.
//!
//! Each connection carries its own lock. A command takes a copy of that
//! lock out of the map and releases the map at once, so a statement that
//! runs for a long time on one connection leaves the other connections
//! free.

use crate::db::drivers::{CancelHandle, DatabaseDriver};
use crate::db::DriverCapabilities;
use crate::error::{Error, Result};
use crate::secrets::SecretStore;
use crate::sql::Dialect;
use crate::storage::SavedConnection;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

/// The time after which the application confirms that a connection that
/// stood idle still answers.
pub const HEALTH_CHECK_AFTER: Duration = Duration::from_secs(30);

/// What the user interface learns about one connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionHealth {
    Connected,
    Reconnecting,
    Disconnected,
}

/// The event the backend sends when the state of a connection changes.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionStatusEvent {
    pub connection_id: String,
    pub health: ConnectionHealth,
    pub message: Option<String>,
}

/// The name of the event that carries a change of connection state.
pub const CONNECTION_STATUS_EVENT: &str = "connection-status";

/// One open connection.
#[derive(Clone)]
pub struct OpenConnection {
    /// The record the connection came from, without the password.
    pub descriptor: SavedConnection,
    pub driver: Arc<Mutex<Box<dyn DatabaseDriver>>>,
    /// Stops a statement while the driver above is busy with it.
    pub cancel_handle: Option<Arc<dyn CancelHandle>>,
    pub capabilities: DriverCapabilities,
    pub dialect: Dialect,
    /// The moment the connection last answered.
    pub last_ok: Arc<Mutex<Instant>>,
}

impl OpenConnection {
    pub fn new(descriptor: SavedConnection, driver: Box<dyn DatabaseDriver>) -> Self {
        let capabilities = driver.capabilities();
        let dialect = driver.dialect();
        let cancel_handle = driver.cancel_handle();
        Self {
            descriptor: descriptor.without_password(),
            driver: Arc::new(Mutex::new(driver)),
            cancel_handle,
            capabilities,
            dialect,
            last_ok: Arc::new(Mutex::new(Instant::now())),
        }
    }

    /// Records that the connection answered.
    pub async fn mark_ok(&self) {
        *self.last_ok.lock().await = Instant::now();
    }

    /// True when the connection stood idle long enough that it should be
    /// checked before it is used again.
    pub async fn needs_check(&self) -> bool {
        self.last_ok.lock().await.elapsed() >= HEALTH_CHECK_AFTER
    }
}

/// What the user interface receives after a connection opens.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionInfo {
    pub connection_id: String,
    pub capabilities: DriverCapabilities,
    pub dialect: Dialect,
}

/// The state that every command shares.
pub struct AppState {
    pub connections: Mutex<HashMap<String, OpenConnection>>,
    /// One token for each statement that runs, keyed by the identifier the
    /// user interface gave it.
    pub running: Mutex<HashMap<String, CancellationToken>>,
    pub secrets: Box<dyn SecretStore>,
}

impl AppState {
    pub fn new(secrets: Box<dyn SecretStore>) -> Self {
        Self {
            connections: Mutex::new(HashMap::new()),
            running: Mutex::new(HashMap::new()),
            secrets,
        }
    }

    /// Returns the open connection with the given identifier.
    pub async fn connection(&self, connection_id: &str) -> Result<OpenConnection> {
        self.connections
            .lock()
            .await
            .get(connection_id)
            .cloned()
            .ok_or_else(|| Error::NotConnected(connection_id.to_string()))
    }

    /// Adds an open connection and returns what the user interface needs.
    pub async fn insert(&self, connection_id: &str, open: OpenConnection) -> ConnectionInfo {
        let info = ConnectionInfo {
            connection_id: connection_id.to_string(),
            capabilities: open.capabilities,
            dialect: open.dialect,
        };
        self.connections
            .lock()
            .await
            .insert(connection_id.to_string(), open);
        info
    }

    /// Removes an open connection. Returns true when one was present.
    pub async fn remove(&self, connection_id: &str) -> bool {
        self.connections
            .lock()
            .await
            .remove(connection_id)
            .is_some()
    }

    /// Registers a statement that runs and returns its token.
    pub async fn start_request(&self, request_id: &str) -> CancellationToken {
        let token = CancellationToken::new();
        self.running
            .lock()
            .await
            .insert(request_id.to_string(), token.clone());
        token
    }

    /// Removes the token of a statement that ended.
    pub async fn end_request(&self, request_id: &str) {
        self.running.lock().await.remove(request_id);
    }

    /// Cancels a statement that runs. Returns true when the identifier
    /// belonged to a statement.
    pub async fn cancel_request(&self, request_id: &str) -> bool {
        match self.running.lock().await.remove(request_id) {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{AppColumn, Database, ExecOptions, QueryParams, QueryResponse, Schema, Table};
    use crate::secrets::MemoryStore;
    use crate::storage::{ConnectionOptions, DbType};
    use async_trait::async_trait;

    struct StubDriver;

    #[async_trait]
    impl DatabaseDriver for StubDriver {
        fn capabilities(&self) -> DriverCapabilities {
            DriverCapabilities {
                supports_schemas: true,
                supports_multiple_databases: true,
                supports_cancel: true,
                supports_transactions: true,
                ..DriverCapabilities::default()
            }
        }
        fn dialect(&self) -> Dialect {
            Dialect::MsSql
        }
        async fn ping(&mut self) -> Result<()> {
            Ok(())
        }
        async fn execute_query(
            &mut self,
            _query: &str,
            _params: Option<&QueryParams>,
            _options: &ExecOptions,
        ) -> Result<QueryResponse> {
            Ok(QueryResponse::default())
        }
        async fn list_databases(&mut self) -> Result<Vec<Database>> {
            Ok(Vec::new())
        }
        async fn list_schemas(&mut self, _database: &str) -> Result<Vec<Schema>> {
            Ok(Vec::new())
        }
        async fn list_tables(
            &mut self,
            _database: &str,
            _schema: Option<&str>,
        ) -> Result<Vec<Table>> {
            Ok(Vec::new())
        }
        async fn list_columns(
            &mut self,
            _database: &str,
            _schema: Option<&str>,
            _table: &str,
        ) -> Result<Vec<AppColumn>> {
            Ok(Vec::new())
        }
    }

    fn descriptor() -> SavedConnection {
        SavedConnection {
            id: "c1".into(),
            name: "Server".into(),
            db_type: DbType::Mssql,
            host: Some("localhost".into()),
            port: Some(1433),
            user: Some("sa".into()),
            database: None,
            password: Some("secret".into()),
            options: ConnectionOptions::default(),
            color: None,
            group: None,
        }
    }

    fn state() -> AppState {
        AppState::new(Box::new(MemoryStore::default()))
    }

    #[tokio::test]
    async fn an_open_connection_drops_the_password() {
        let open = OpenConnection::new(descriptor(), Box::new(StubDriver));
        assert_eq!(open.descriptor.password, None);
        assert_eq!(open.dialect, Dialect::MsSql);
        assert!(open.capabilities.supports_schemas);
    }

    #[tokio::test]
    async fn a_connection_that_just_answered_needs_no_check() {
        let open = OpenConnection::new(descriptor(), Box::new(StubDriver));
        assert!(!open.needs_check().await);
        *open.last_ok.lock().await = Instant::now() - HEALTH_CHECK_AFTER;
        assert!(open.needs_check().await);
        open.mark_ok().await;
        assert!(!open.needs_check().await);
    }

    #[tokio::test]
    async fn a_connection_can_be_added_read_and_removed() {
        let state = state();
        assert_eq!(
            state.connection("c1").await.err().unwrap().kind(),
            crate::error::ErrorKind::NotConnected
        );

        let info = state
            .insert(
                "c1",
                OpenConnection::new(descriptor(), Box::new(StubDriver)),
            )
            .await;
        assert_eq!(info.connection_id, "c1");
        assert_eq!(info.dialect, Dialect::MsSql);

        assert_eq!(
            state.connection("c1").await.unwrap().descriptor.name,
            "Server"
        );
        assert!(state.remove("c1").await);
        assert!(!state.remove("c1").await);
    }

    #[tokio::test]
    async fn a_statement_can_be_registered_and_cancelled() {
        let state = state();
        let token = state.start_request("r1").await;
        assert!(!token.is_cancelled());

        assert!(state.cancel_request("r1").await);
        assert!(token.is_cancelled());
        assert!(!state.cancel_request("r1").await);
    }

    #[tokio::test]
    async fn a_statement_that_ended_leaves_no_token() {
        let state = state();
        state.start_request("r2").await;
        state.end_request("r2").await;
        assert!(!state.cancel_request("r2").await);
    }

    #[test]
    fn the_status_event_serialises_with_the_expected_names() {
        let event = ConnectionStatusEvent {
            connection_id: "c1".into(),
            health: ConnectionHealth::Reconnecting,
            message: Some("retrying".into()),
        };
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["connectionId"], "c1");
        assert_eq!(value["health"], "reconnecting");
        assert_eq!(value["message"], "retrying");
        assert_eq!(
            serde_json::to_value(ConnectionHealth::Connected).unwrap(),
            serde_json::json!("connected")
        );
        assert_eq!(
            serde_json::to_value(ConnectionHealth::Disconnected).unwrap(),
            serde_json::json!("disconnected")
        );
        assert_eq!(CONNECTION_STATUS_EVENT, "connection-status");
    }
}
