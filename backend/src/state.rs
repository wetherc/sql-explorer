//! The state the application holds while it runs.
//!
//! Each connection carries a pool of sessions. A command takes one session
//! out of the pool and releases the maps at once, so a statement that runs
//! for a long time on one session leaves the other sessions and the other
//! connections free.

use crate::db::drivers::{CancelHandle, DatabaseDriver};
use crate::db::DriverCapabilities;
use crate::error::{Error, Result};
use crate::secrets::SecretStore;
use crate::session::{Session, SessionPool, DEFAULT_SESSION};
use crate::sql::Dialect;
use crate::storage::SavedConnection;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
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
    /// The sessions of the connection, keyed by the tab that holds each one.
    pub sessions: Arc<SessionPool>,
    pub capabilities: DriverCapabilities,
    pub dialect: Dialect,
}

impl OpenConnection {
    pub fn new(descriptor: SavedConnection, driver: Box<dyn DatabaseDriver>) -> Self {
        let capabilities = driver.capabilities();
        let dialect = driver.dialect();
        let cap = descriptor.options.max_sessions.max(1);
        let sessions = Arc::new(SessionPool::with_session(
            cap,
            DEFAULT_SESSION,
            Session::new(driver),
        ));
        Self {
            descriptor: descriptor.without_password(),
            sessions,
            capabilities,
            dialect,
        }
    }

    /// Returns the session that a request without a tab uses.
    pub async fn default_session(&self) -> Result<Arc<Session>> {
        self.sessions
            .get(DEFAULT_SESSION)
            .await
            .ok_or_else(|| Error::NotConnected(self.descriptor.id.clone()))
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

/// A driver that only background work uses.
pub type BackgroundDriver = Arc<Mutex<Box<dyn DatabaseDriver>>>;

/// One statement that runs, with the means to stop it.
pub struct RunningRequest {
    /// Ends the wait for the statement.
    pub token: CancellationToken,
    /// Asks the server to stop the statement, on the session that runs it.
    pub cancel_handle: Option<Arc<dyn CancelHandle>>,
}

/// The state that every command shares.
pub struct AppState {
    pub connections: Mutex<HashMap<String, OpenConnection>>,
    /// A second driver for each connection that has asked for one. Background
    /// work runs there, so that it never waits behind a statement of the user
    /// and no statement of the user waits behind it.
    pub background: Mutex<HashMap<String, BackgroundDriver>>,
    /// One record for each statement that runs, keyed by the identifier the
    /// user interface gave it.
    pub running: Mutex<HashMap<String, RunningRequest>>,
    pub secrets: Box<dyn SecretStore>,
}

impl AppState {
    pub fn new(secrets: Box<dyn SecretStore>) -> Self {
        Self {
            connections: Mutex::new(HashMap::new()),
            background: Mutex::new(HashMap::new()),
            running: Mutex::new(HashMap::new()),
            secrets,
        }
    }

    /// Drops the background driver of a connection, so that the next
    /// metadata read opens a fresh one.
    pub async fn clear_background(&self, connection_id: &str) {
        self.background.lock().await.remove(connection_id);
    }

    /// Returns the background driver of a connection, when one is open.
    pub async fn background_driver(&self, connection_id: &str) -> Option<BackgroundDriver> {
        self.background.lock().await.get(connection_id).cloned()
    }

    /// Keeps a background driver for a connection and returns it.
    pub async fn set_background_driver(
        &self,
        connection_id: &str,
        driver: Box<dyn DatabaseDriver>,
    ) -> BackgroundDriver {
        let held: BackgroundDriver = Arc::new(Mutex::new(driver));
        self.background
            .lock()
            .await
            .insert(connection_id.to_string(), held.clone());
        held
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

    /// Removes an open connection, together with its sessions and its
    /// background driver. Returns true when one was present.
    pub async fn remove(&self, connection_id: &str) -> bool {
        self.background.lock().await.remove(connection_id);
        self.connections
            .lock()
            .await
            .remove(connection_id)
            .is_some()
    }

    /// Registers a statement that runs, together with the handle that stops
    /// it on its own session, and returns its token.
    pub async fn start_request(
        &self,
        request_id: &str,
        cancel_handle: Option<Arc<dyn CancelHandle>>,
    ) -> CancellationToken {
        let token = CancellationToken::new();
        self.running.lock().await.insert(
            request_id.to_string(),
            RunningRequest {
                token: token.clone(),
                cancel_handle,
            },
        );
        token
    }

    /// Removes the record of a statement that ended.
    pub async fn end_request(&self, request_id: &str) {
        self.running.lock().await.remove(request_id);
    }

    /// Takes the record of a statement that runs, so that the caller can
    /// stop it. Returns `None` when the identifier belongs to no statement.
    pub async fn take_request(&self, request_id: &str) -> Option<RunningRequest> {
        self.running.lock().await.remove(request_id)
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

        let session = open.default_session().await.unwrap();
        assert!(session.needs_ping);
        assert!(!session.keeps_connection_after_stop);
        assert_eq!(
            open.sessions.cap(),
            descriptor().options.max_sessions.max(1)
        );
    }

    #[tokio::test]
    async fn a_connection_without_its_default_session_reports_the_loss() {
        let open = OpenConnection::new(descriptor(), Box::new(StubDriver));
        open.sessions.release(DEFAULT_SESSION).await;
        let error = open.default_session().await.err().unwrap();
        assert_eq!(error.kind(), crate::error::ErrorKind::NotConnected);
    }

    #[tokio::test]
    async fn a_session_cap_below_one_becomes_one() {
        let mut record = descriptor();
        record.options.max_sessions = 0;
        let open = OpenConnection::new(record, Box::new(StubDriver));
        assert_eq!(open.sessions.cap(), 1);
    }

    #[tokio::test]
    async fn a_driver_that_reads_no_plan_refuses_the_request() {
        let mut driver = StubDriver;
        let error = driver
            .explain(
                "SELECT 1",
                None,
                crate::db::PlanKind::Estimated,
                &ExecOptions::default(),
            )
            .await
            .unwrap_err();
        assert_eq!(error.kind(), crate::error::ErrorKind::Unsupported);
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
    async fn a_statement_can_be_registered_and_taken() {
        let state = state();
        let token = state.start_request("r1", None).await;
        assert!(!token.is_cancelled());

        let request = state.take_request("r1").await.expect("the statement runs");
        assert!(request.cancel_handle.is_none());
        request.token.cancel();
        assert!(token.is_cancelled());
        assert!(state.take_request("r1").await.is_none());
    }

    #[tokio::test]
    async fn a_statement_keeps_the_handle_of_its_session() {
        struct NoopCancel;
        #[async_trait]
        impl crate::db::drivers::CancelHandle for NoopCancel {
            async fn cancel(&self) -> Result<()> {
                Ok(())
            }
        }

        let state = state();
        state.start_request("r1", Some(Arc::new(NoopCancel))).await;
        let request = state.take_request("r1").await.expect("the statement runs");
        assert!(request.cancel_handle.is_some());
    }

    #[tokio::test]
    async fn a_statement_that_ended_leaves_no_record() {
        let state = state();
        state.start_request("r2", None).await;
        state.end_request("r2").await;
        assert!(state.take_request("r2").await.is_none());
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
