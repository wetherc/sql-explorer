//! Opens one connection to a MS SQL Server and reports where it stops.
//!
//! The example prints the trace of `tiberius` itself, which names each step of
//! the handshake and of the login, so a connection that waits shows the step it
//! waits on. It runs `SELECT 1` and nothing else.
//!
//! ```text
//! cargo run --example mssql_probe -- host.example.com 1433 required
//! ```
//!
//! The settings are `required`, `login-only`, `off` and the default of
//! `required`. The login uses Windows Authentication, which on this system
//! reads the Kerberos ticket of the user.
//!
//! The example holds its own configuration, because the application is a
//! program and not a library, so an example cannot reach inside it.

use std::time::Duration;
use tiberius::{AuthMethod, Client, Config, EncryptionLevel};
use tokio::net::TcpStream;
use tokio_util::compat::TokioAsyncWriteCompatExt;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(host) = args.first().cloned() else {
        eprintln!("Give the host, and then the port and the transport setting.");
        std::process::exit(2);
    };
    let port: u16 = args
        .get(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(1433);
    let level = match args.get(2).map(String::as_str) {
        Some("login-only") => EncryptionLevel::Off,
        Some("off") => EncryptionLevel::NotSupported,
        _ => EncryptionLevel::Required,
    };

    let mut config = Config::new();
    config.host(&host);
    config.port(port);
    config.authentication(AuthMethod::Integrated);
    config.encryption(level);
    config.trust_cert();
    config.application_name("SQL Explorer probe");

    println!("Opening {host}:{port} with {level:?} and Windows Authentication.");

    let limit = Duration::from_secs(30);
    let tcp = match tokio::time::timeout(limit, TcpStream::connect(config.get_addr())).await {
        Ok(Ok(tcp)) => tcp,
        Ok(Err(error)) => return println!("The socket failed: {error}"),
        Err(_) => return println!("The socket did not open inside 30 seconds."),
    };
    tcp.set_nodelay(true).ok();
    println!("The socket is open. Starting the prelogin and the login.");

    match tokio::time::timeout(limit, Client::connect(config, tcp.compat_write())).await {
        Ok(Ok(mut client)) => {
            println!("The login finished.");
            match client.simple_query("SELECT 1").await {
                Ok(_) => println!("The server answered SELECT 1."),
                Err(error) => println!("The statement failed: {error}"),
            }
        }
        Ok(Err(error)) => println!("The login failed: {error}"),
        Err(_) => println!("The login did not finish inside 30 seconds. The last trace line above names the step it waited on."),
    }
}
