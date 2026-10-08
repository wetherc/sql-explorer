//! The helpers of the tests that run against a live server.
//!
//! Each live test has `#[ignore]`, so the unit tests and the coverage gate
//! reach no server. `pnpm test:live` runs them against the servers of
//! `backend/live/compose.yaml`. A test reads the address of its server from
//! an environment variable. When the variable is not set, the test writes a
//! line that says so and passes.
//!
//! A test opens its driver through `open_driver`, as the application does.
//! It makes a database with a unique name, loads a fixture of
//! `backend/live/fixtures` into it, and removes the database at the end. A
//! panic of the test body also removes the database, so the tests can run
//! in parallel and again.

use crate::commands::{create_text_of, open_driver};
use crate::db::drivers::DatabaseDriver;
use crate::db::sink::{BufferSink, PausePoint, RunSummary};
use crate::db::{CreateQuery, ExecOptions, QueryResponse};
use crate::pause::{spawn_read, PausedRead, PausingSink, SessionSlot, SharedSink};
use crate::session::{Session, SessionPool};
use crate::storage::{ConnectionOptions, DbType, SavedConnection, TlsMode};
use futures_util::FutureExt;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Once};
use std::time::Duration;

/// The address and the login of one test server.
#[derive(Debug, Clone)]
pub struct Server {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
}

/// Reads the server of the variable. The value is a URL such as
/// `postgres://postgres:secret@127.0.0.1:15416`. A mark in the password,
/// such as `#`, goes in as its percent code, such as `%23`. A variable that
/// is not set gives `None`.
pub fn server(variable: &str) -> Option<Server> {
    let Ok(url) = std::env::var(variable) else {
        eprintln!("skipped: set {variable} to the URL of a test server to run this test");
        return None;
    };
    Some(parse_url(&url).unwrap_or_else(|| panic!("{variable} is not a URL of a server: {url}")))
}

/// Reads `scheme://user:password@host:port`.
fn parse_url(url: &str) -> Option<Server> {
    let rest = url.split_once("://")?.1.trim_end_matches('/');
    let (login, address) = rest.rsplit_once('@')?;
    let (user, password) = login.split_once(':').unwrap_or((login, ""));
    let (host, port) = address.rsplit_once(':')?;
    Some(Server {
        host: host.to_string(),
        port: port.parse().ok()?,
        user: percent_decode(user),
        password: percent_decode(password),
    })
}

/// Replaces each `%` and its two hexadecimal digits with the byte they code.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let code = (bytes[index] == b'%')
            .then(|| text.get(index + 1..index + 3))
            .flatten()
            .and_then(|digits| u8::from_str_radix(digits, 16).ok());
        match code {
            Some(byte) => {
                out.push(byte);
                index += 3;
            }
            None => {
                out.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl Server {
    /// The record of a connection to this server, with the given database.
    /// The test servers have no certificate of a trusted root, so the
    /// connection accepts any certificate.
    pub fn connection(&self, db_type: DbType, database: Option<&str>) -> SavedConnection {
        SavedConnection {
            id: "live".to_string(),
            name: "live".to_string(),
            db_type,
            host: Some(self.host.clone()),
            port: Some(self.port),
            user: Some(self.user.clone()),
            database: database.map(str::to_string),
            password: Some(self.password.clone()),
            aws_secret_access_key: None,
            aws_session_token: None,
            options: ConnectionOptions {
                tls_mode: TlsMode::Prefer,
                ..ConnectionOptions::default()
            },
            color: None,
            group: None,
        }
    }

    /// Opens a driver on this server through the open path of the
    /// application.
    ///
    /// The test binary does not run `main`, which installs the crypto
    /// provider of rustls. Without a provider the TLS step of the MySQL
    /// driver panics, so the first open installs the provider of `main`.
    pub async fn open(&self, db_type: DbType, database: Option<&str>) -> Box<dyn DatabaseDriver> {
        static PROVIDER: Once = Once::new();
        PROVIDER.call_once(|| {
            // A provider that another test installed first is also good.
            let _ = rustls::crypto::ring::default_provider().install_default();
        });
        open_driver(&self.connection(db_type, database))
            .await
            .unwrap_or_else(|error| panic!("the connection to {} failed: {error}", self.host))
    }
}

/// A name that no other run of a test uses, such as `live_trig_3f2a9c1d`.
/// It contains lower-case letters, digits and low lines alone, so every engine
/// takes it without quotes.
pub fn unique_name(tag: &str) -> String {
    let id = uuid::Uuid::new_v4().simple().to_string();
    format!("live_{tag}_{}", &id[..8])
}

/// Runs the body, and then the cleanup, also when the body panics. The panic
/// of the body then goes on after the cleanup.
pub async fn with_cleanup<B, C>(body: B, cleanup: C)
where
    B: Future<Output = ()>,
    C: Future<Output = ()>,
{
    let outcome = AssertUnwindSafe(body).catch_unwind().await;
    cleanup.await;
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

/// Runs a script with the default limits, and panics with the text of the
/// script when the run fails.
pub async fn run(driver: &mut dyn DatabaseDriver, sql: &str) -> QueryResponse {
    run_with(driver, sql, &ExecOptions::default()).await
}

/// Runs a script with the given limits, and panics with the text of the
/// script when the run fails.
pub async fn run_with(
    driver: &mut dyn DatabaseDriver,
    sql: &str,
    options: &ExecOptions,
) -> QueryResponse {
    driver
        .execute_query(sql, None, options)
        .await
        .unwrap_or_else(|error| panic!("the run failed: {error}\n{sql}"))
}

/// Runs a fixture and gives the fault of the run. A test removes its
/// database before it panics on the fault, so a fixture that fails leaves
/// no database behind.
pub async fn load_fixture(driver: &mut dyn DatabaseDriver, sql: &str) -> crate::error::Result<()> {
    driver
        .execute_query(sql, None, &ExecOptions::default())
        .await
        .map(|_| ())
}

/// The value of one cell of the first result set as text. A NULL gives
/// `None`.
pub fn cell(response: &QueryResponse, row: usize, column: usize) -> Option<String> {
    match &response.results[0].rows[row][column] {
        serde_json::Value::Null => None,
        serde_json::Value::String(text) => Some(text.clone()),
        other => Some(other.to_string()),
    }
}

/// Runs the statement of a CREATE text and reads the text, as the script
/// command of the application does.
pub async fn create_text(driver: &mut dyn DatabaseDriver, query: Option<CreateQuery>) -> String {
    let query = query.expect("the engine gives a statement for the CREATE text");
    let response = run(driver, &query.sql).await;
    create_text_of(&response, &query)
        .unwrap_or_else(|| panic!("the statement gave no CREATE text: {}", query.sql))
}

/// A read of a live server that paused at the row limit, with the rows of
/// its grid and its session.
pub struct Paused {
    pub read: PausedRead<BufferSink>,
    pub grid: Vec<Vec<serde_json::Value>>,
    pub session: Arc<Session>,
}

/// Runs the query in a task that pauses its read after `rows` rows, as a run
/// of a tab does, and waits for the pause.
pub async fn pause_read(
    driver: Box<dyn DatabaseDriver>,
    query: &str,
    rows: usize,
    limit: Duration,
) -> Paused {
    let sessions = Arc::new(SessionPool::new(4));
    let session = sessions.insert("tab", Session::new(driver)).await;
    let slot = SessionSlot {
        sessions,
        key: "tab".to_string(),
        session: session.clone(),
    };
    let guard = session.driver.clone().lock_owned().await;
    let point = PausePoint { rows, limit };
    let (sink, control) = PausingSink::new(BufferSink::new(rows), point);
    let options = ExecOptions {
        max_rows: usize::MAX,
        ..ExecOptions::default()
    };
    let task = spawn_read(guard, query.to_string(), None, options, sink, slot.clone());
    let Ok(handoff) = control.handoff.await else {
        let (result, _) = task.await.expect("the read ends");
        panic!("the read ended without a pause: {:?}", result.err());
    };
    let mut response = handoff.grid.into_response(RunSummary::default());
    Paused {
        read: PausedRead::new(control.commands, task, slot, limit),
        grid: response.results.remove(0).rows,
        session,
    }
}

/// Continues a paused read into a buffer, and gives the rows of the first
/// set of the export.
pub async fn export_paused(
    read: &PausedRead<BufferSink>,
    max_rows: usize,
) -> crate::error::Result<Vec<Vec<serde_json::Value>>> {
    let shared = Arc::new(std::sync::Mutex::new(BufferSink::new(usize::MAX)));
    read.continue_into(Box::new(SharedSink(shared.clone())), max_rows)
        .await?;
    let buffer = Arc::into_inner(shared)
        .expect("the read dropped the sink")
        .into_inner()
        .unwrap();
    let mut response = buffer.into_response(RunSummary::default());
    Ok(response.results.remove(0).rows)
}

/// The first column of each row as a number.
pub fn numbers(rows: &[Vec<serde_json::Value>]) -> Vec<i64> {
    rows.iter()
        .map(|row| match &row[0] {
            serde_json::Value::Number(number) => number.as_i64().unwrap(),
            serde_json::Value::String(text) => text.parse().unwrap(),
            other => panic!("not a number: {other}"),
        })
        .collect()
}

/// Waits for the driver of the session, and runs a query on it. A session
/// that a paused read used must be fit for the next run.
pub async fn run_after(session: &Session, sql: &str) -> QueryResponse {
    let mut driver = session.driver.lock().await;
    assert!(!session.is_broken(), "the session closed");
    run(driver.as_mut(), sql).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_gives_the_address_and_the_login() {
        let server = parse_url("mssql://sa:Live%23Pass@127.0.0.1:11433/").unwrap();
        assert_eq!(server.host, "127.0.0.1");
        assert_eq!(server.port, 11433);
        assert_eq!(server.user, "sa");
        assert_eq!(server.password, "Live#Pass");
        assert_eq!(parse_url("pg://u@h:1").unwrap().password, "");
        assert!(parse_url("h:1").is_none());
        assert!(parse_url("pg://u@h").is_none());
        assert_eq!(percent_decode("a%2"), "a%2");
        assert_eq!(percent_decode("a%zz"), "a%zz");
    }

    #[test]
    fn a_unique_name_needs_no_quotes() {
        let name = unique_name("x");
        assert!(name.starts_with("live_x_"));
        assert_eq!(name.len(), "live_x_".len() + 8);
        assert_ne!(name, unique_name("x"));
    }
}
