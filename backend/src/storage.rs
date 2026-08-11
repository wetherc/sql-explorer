//! The connection record that the application saves, and the options that
//! belong to it.
//!
//! The driver builds its own configuration from these fields. No component
//! joins a connection string by hand, because that is the step where a port
//! or a special character gets lost.

use crate::sql::Dialect;
use serde::{Deserialize, Serialize};

/// The engines the application supports.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DbType {
    Mssql,
    Athena,
    Mysql,
    Postgres,
    Sqlite,
}

impl DbType {
    /// Returns the SQL dialect of the engine.
    pub fn dialect(&self) -> Dialect {
        match self {
            DbType::Mssql => Dialect::MsSql,
            DbType::Athena => Dialect::Athena,
            DbType::Mysql => Dialect::MySql,
            DbType::Postgres => Dialect::Postgres,
            DbType::Sqlite => Dialect::Sqlite,
        }
    }

    /// Returns the port that the engine listens on by default.
    pub fn default_port(&self) -> Option<u16> {
        match self {
            DbType::Mssql => Some(1433),
            DbType::Mysql => Some(3306),
            DbType::Postgres => Some(5432),
            DbType::Athena | DbType::Sqlite => None,
        }
    }
}

/// How much protection the transport must give.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum TlsMode {
    /// Send everything in clear text.
    Disable,
    /// Use TLS when the server offers it, and accept any certificate.
    Prefer,
    /// Demand TLS, and accept any certificate.
    Require,
    /// Demand TLS, and check the certificate against the trusted roots.
    #[default]
    VerifyFull,
}

impl TlsMode {
    /// True when the connection must not continue without TLS.
    pub fn is_required(&self) -> bool {
        matches!(self, TlsMode::Require | TlsMode::VerifyFull)
    }

    /// True when the driver checks the certificate of the server.
    pub fn verifies_certificate(&self) -> bool {
        matches!(self, TlsMode::VerifyFull)
    }
}

/// Where an Athena connection takes its AWS credentials from.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AwsCredentialSource {
    /// The default chain of the AWS tools: the environment, the files of
    /// the user, and the metadata of the instance.
    #[default]
    Chain,
    /// The keys that the user typed into the form.
    Keys,
}

/// How a MS SQL Server connection proves who the user is.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MssqlAuth {
    /// A login and a password that the server holds.
    #[default]
    SqlLogin,
    /// The credentials of the user who runs the application, on Windows.
    Integrated,
    /// Microsoft Entra ID, with a token from the Azure CLI.
    EntraAzureCli,
    /// Microsoft Entra ID, with a token that the user supplies.
    EntraAccessToken,
}

impl MssqlAuth {
    /// True when the method reaches Microsoft Entra ID.
    pub fn is_entra(&self) -> bool {
        matches!(self, MssqlAuth::EntraAzureCli | MssqlAuth::EntraAccessToken)
    }
}

/// The options that apply to one connection.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ConnectionOptions {
    pub tls_mode: TlsMode,
    /// The path of a certificate authority file that signs the server
    /// certificate.
    pub ca_cert_path: Option<String>,
    pub connect_timeout_secs: u64,
    pub query_timeout_secs: u64,
    /// The largest number of rows one result set holds.
    pub max_rows: usize,
    /// The largest number of sessions the editor tabs open on this
    /// connection at one time.
    pub max_sessions: usize,
    /// True when the connection asks the server for a read-only session.
    pub read_only: bool,
    /// The name the server records for this client.
    pub application_name: Option<String>,

    /// The named instance of a MS SQL Server. The SQL Browser service
    /// resolves the port of a named instance.
    pub instance_name: Option<String>,
    /// True when MS SQL Server uses the credentials of the current user.
    /// A record that a former release wrote holds this flag alone, so the
    /// driver reads it when `mssql_auth` stands at its default.
    pub integrated_security: bool,
    /// How a MS SQL Server connection proves who the user is.
    pub mssql_auth: MssqlAuth,
    /// The path of the Azure CLI. An application that a desktop starts holds
    /// a short `PATH` that often misses the folder of the CLI, so the path
    /// can be given here.
    pub azure_cli_path: Option<String>,

    /// The file that holds a SQLite database.
    pub file_path: Option<String>,

    pub aws_region: Option<String>,
    pub aws_profile: Option<String>,
    /// Where the connection takes its AWS credentials from.
    pub aws_credential_source: AwsCredentialSource,
    /// The access key ID, which names the key and is no secret. The secret
    /// access key and the session token stay in the keychain.
    pub aws_access_key_id: Option<String>,
    pub athena_workgroup: Option<String>,
    /// The S3 location that Athena writes the results to.
    pub athena_output_location: Option<String>,
    pub athena_catalog: Option<String>,
    /// True when Athena may give the result of an earlier run of the same
    /// statement. A reused result costs nothing, because the engine scans
    /// no data for it.
    pub athena_result_reuse: bool,
    /// The age in minutes up to which a result may be reused.
    pub athena_result_reuse_max_age_minutes: u32,

    /// A complete connection string that replaces the fields above. Use it
    /// for an option the form does not show.
    pub connection_url: Option<String>,
}

impl Default for ConnectionOptions {
    fn default() -> Self {
        Self {
            tls_mode: TlsMode::default(),
            ca_cert_path: None,
            connect_timeout_secs: 15,
            query_timeout_secs: 300,
            max_rows: 10_000,
            max_sessions: crate::session::DEFAULT_SESSION_CAP,
            read_only: false,
            application_name: Some("SQL Explorer".to_string()),
            instance_name: None,
            integrated_security: false,
            mssql_auth: MssqlAuth::default(),
            azure_cli_path: None,
            file_path: None,
            aws_region: None,
            aws_profile: None,
            aws_credential_source: AwsCredentialSource::default(),
            aws_access_key_id: None,
            athena_workgroup: None,
            athena_output_location: None,
            athena_catalog: None,
            athena_result_reuse: false,
            athena_result_reuse_max_age_minutes: 60,
            connection_url: None,
        }
    }
}

/// One saved connection. The password never reaches the settings file; the
/// secret store holds it.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SavedConnection {
    pub id: String,
    pub name: String,
    pub db_type: DbType,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub database: Option<String>,
    /// Present when the user just typed it, and when the connection is in
    /// flight. The record that reaches the settings file has `None` here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// The secret access key of an Athena connection. It follows the rule
    /// of the password: the keychain holds it, and the record that reaches
    /// the settings file has `None` here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws_secret_access_key: Option<String>,
    /// The session token of an Athena connection, under the rule of the
    /// password as well.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws_session_token: Option<String>,
    #[serde(default)]
    pub options: ConnectionOptions,
    /// A colour that marks the connection in the list, for example to
    /// separate a production server from a test server.
    #[serde(default)]
    pub color: Option<String>,
    /// The folder that holds the connection in the list.
    #[serde(default)]
    pub group: Option<String>,
}

impl SavedConnection {
    /// Returns the port to use, which is the port of the record or the
    /// default port of the engine.
    pub fn effective_port(&self) -> Option<u16> {
        self.port.or_else(|| self.db_type.default_port())
    }

    /// Returns the host to use, which is the host of the record or the
    /// local host.
    pub fn effective_host(&self) -> &str {
        match self.host.as_deref() {
            Some(host) if !host.is_empty() => host,
            _ => "localhost",
        }
    }

    /// Returns a copy without any secret, for the settings file. Every
    /// secret of a connection belongs in the keychain.
    pub fn without_secrets(&self) -> Self {
        Self {
            password: None,
            aws_secret_access_key: None,
            aws_session_token: None,
            ..self.clone()
        }
    }

    /// Returns the limits that apply to one execution of this connection.
    pub fn exec_options(&self) -> crate::db::ExecOptions {
        crate::db::ExecOptions {
            max_rows: self.options.max_rows,
            timeout_secs: self.options.query_timeout_secs,
        }
    }

    /// Reports the fields that the engine needs but the record does not
    /// hold.
    /// The authentication method of a MS SQL Server connection. A record
    /// that a former release wrote holds `integrated_security` alone, so
    /// that flag decides while the new field stands at its default.
    pub fn effective_auth(&self) -> MssqlAuth {
        if self.options.mssql_auth == MssqlAuth::SqlLogin && self.options.integrated_security {
            MssqlAuth::Integrated
        } else {
            self.options.mssql_auth
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() {
            return Err("The connection needs an identifier.".to_string());
        }
        if self.name.trim().is_empty() {
            return Err("The connection needs a name.".to_string());
        }
        match self.db_type {
            DbType::Sqlite => {
                let path = self.options.file_path.as_deref().unwrap_or("");
                if path.trim().is_empty() {
                    return Err("A SQLite connection needs the path of a file.".to_string());
                }
            }
            DbType::Athena => {
                if self
                    .options
                    .aws_region
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .is_empty()
                {
                    return Err("An Athena connection needs an AWS region.".to_string());
                }
                let workgroup = self.options.athena_workgroup.as_deref().unwrap_or("");
                let output = self.options.athena_output_location.as_deref().unwrap_or("");
                if workgroup.trim().is_empty() && output.trim().is_empty() {
                    return Err(
                        "An Athena connection needs a workgroup or an output location.".to_string(),
                    );
                }
            }
            DbType::Mssql => {
                if self.options.connection_url.is_some() && self.effective_auth().is_entra() {
                    return Err(
                        "A connection string carries its own authentication. Remove the string, \
                         or choose the SQL login."
                            .to_string(),
                    );
                }
                if self.options.connection_url.is_none() && self.effective_port().is_none() {
                    return Err("The connection needs a port.".to_string());
                }
            }
            _ => {
                if self.options.connection_url.is_none() && self.effective_port().is_none() {
                    return Err("The connection needs a port.".to_string());
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(db_type: DbType) -> SavedConnection {
        SavedConnection {
            id: "id".into(),
            name: "name".into(),
            db_type,
            host: Some("db.example.com".into()),
            port: None,
            user: Some("sa".into()),
            database: Some("master".into()),
            password: Some("secret".into()),
            aws_secret_access_key: None,
            aws_session_token: None,
            options: ConnectionOptions::default(),
            color: None,
            group: None,
        }
    }

    #[test]
    fn each_engine_has_a_dialect_and_a_default_port() {
        assert_eq!(DbType::Mssql.dialect(), Dialect::MsSql);
        assert_eq!(DbType::Athena.dialect(), Dialect::Athena);
        assert_eq!(DbType::Mysql.dialect(), Dialect::MySql);
        assert_eq!(DbType::Postgres.dialect(), Dialect::Postgres);
        assert_eq!(DbType::Sqlite.dialect(), Dialect::Sqlite);

        assert_eq!(DbType::Mssql.default_port(), Some(1433));
        assert_eq!(DbType::Mysql.default_port(), Some(3306));
        assert_eq!(DbType::Postgres.default_port(), Some(5432));
        assert_eq!(DbType::Athena.default_port(), None);
        assert_eq!(DbType::Sqlite.default_port(), None);
    }

    #[test]
    fn the_tls_mode_reports_what_it_demands() {
        assert!(!TlsMode::Disable.is_required());
        assert!(!TlsMode::Disable.verifies_certificate());

        assert!(!TlsMode::Prefer.is_required());
        assert!(!TlsMode::Prefer.verifies_certificate());

        assert!(TlsMode::Require.is_required());
        assert!(!TlsMode::Require.verifies_certificate());

        assert!(TlsMode::VerifyFull.is_required());
        assert!(TlsMode::VerifyFull.verifies_certificate());

        assert_eq!(TlsMode::default(), TlsMode::VerifyFull);
    }

    #[test]
    fn the_default_options_verify_the_certificate() {
        let options = ConnectionOptions::default();
        assert_eq!(options.tls_mode, TlsMode::VerifyFull);
        assert_eq!(options.connect_timeout_secs, 15);
        assert_eq!(options.query_timeout_secs, 300);
        assert_eq!(options.max_rows, 10_000);
        assert_eq!(options.max_sessions, crate::session::DEFAULT_SESSION_CAP);
        assert!(!options.read_only);
        assert!(!options.integrated_security);
        assert_eq!(options.application_name.as_deref(), Some("SQL Explorer"));
    }

    #[test]
    fn a_record_without_a_session_limit_takes_the_default() {
        let text = r#"{"tlsMode":"verifyFull","maxRows":42}"#;
        let options: ConnectionOptions = serde_json::from_str(text).unwrap();
        assert_eq!(options.max_rows, 42);
        assert_eq!(options.max_sessions, crate::session::DEFAULT_SESSION_CAP);
    }

    #[test]
    fn the_port_falls_back_to_the_default_of_the_engine() {
        let mut connection = base(DbType::Postgres);
        assert_eq!(connection.effective_port(), Some(5432));
        connection.port = Some(6000);
        assert_eq!(connection.effective_port(), Some(6000));
    }

    #[test]
    fn the_host_falls_back_to_the_local_host() {
        let mut connection = base(DbType::Mysql);
        assert_eq!(connection.effective_host(), "db.example.com");
        connection.host = Some(String::new());
        assert_eq!(connection.effective_host(), "localhost");
        connection.host = None;
        assert_eq!(connection.effective_host(), "localhost");
    }

    #[test]
    fn the_record_for_the_settings_file_holds_no_secret() {
        let mut connection = base(DbType::Athena);
        connection.aws_secret_access_key = Some("the-secret-key".into());
        connection.aws_session_token = Some("the-session-token".into());
        connection.options.aws_credential_source = AwsCredentialSource::Keys;
        connection.options.aws_access_key_id = Some("AKIAEXAMPLE".into());

        let stripped = connection.without_secrets();
        assert_eq!(stripped.password, None);
        assert_eq!(stripped.aws_secret_access_key, None);
        assert_eq!(stripped.aws_session_token, None);
        assert_eq!(stripped.name, connection.name);

        let text = serde_json::to_string(&stripped).unwrap();
        assert!(!text.contains("password"));
        assert!(!text.contains("awsSecretAccessKey"));
        assert!(!text.contains("awsSessionToken\""));
        assert!(!text.contains("the-secret-key"));
        assert!(!text.contains("the-session-token"));
        // The access key ID names a key and is no secret, so it stays.
        assert!(text.contains("AKIAEXAMPLE"));
    }

    #[test]
    fn a_record_of_a_former_release_takes_the_default_source_of_the_keys() {
        let text = r#"{"id":"a","name":"n","dbType":"athena","options":{}}"#;
        let connection: SavedConnection = serde_json::from_str(text).unwrap();
        assert_eq!(
            connection.options.aws_credential_source,
            AwsCredentialSource::Chain
        );
        assert_eq!(connection.options.aws_access_key_id, None);
        assert_eq!(connection.aws_secret_access_key, None);
    }

    #[test]
    fn the_limits_come_from_the_options() {
        let mut connection = base(DbType::Mssql);
        connection.options.max_rows = 42;
        connection.options.query_timeout_secs = 7;
        let options = connection.exec_options();
        assert_eq!(options.max_rows, 42);
        assert_eq!(options.timeout_secs, 7);
    }

    #[test]
    fn validation_accepts_a_complete_record() {
        assert_eq!(base(DbType::Mssql).validate(), Ok(()));
        assert_eq!(base(DbType::Mysql).validate(), Ok(()));
        assert_eq!(base(DbType::Postgres).validate(), Ok(()));
    }

    #[test]
    fn validation_needs_an_identifier_and_a_name() {
        let mut connection = base(DbType::Mssql);
        connection.id = "  ".into();
        assert!(connection.validate().unwrap_err().contains("identifier"));

        let mut connection = base(DbType::Mssql);
        connection.name = String::new();
        assert!(connection.validate().unwrap_err().contains("name"));
    }

    #[test]
    fn a_sqlite_record_needs_a_file() {
        let mut connection = base(DbType::Sqlite);
        assert!(connection.validate().unwrap_err().contains("file"));
        connection.options.file_path = Some("/tmp/a.db".into());
        assert_eq!(connection.validate(), Ok(()));
    }

    #[test]
    fn an_athena_record_needs_a_region_and_a_place_for_the_results() {
        let mut connection = base(DbType::Athena);
        assert!(connection.validate().unwrap_err().contains("region"));

        connection.options.aws_region = Some("us-east-1".into());
        assert!(connection.validate().unwrap_err().contains("workgroup"));

        connection.options.athena_workgroup = Some("primary".into());
        assert_eq!(connection.validate(), Ok(()));

        connection.options.athena_workgroup = None;
        connection.options.athena_output_location = Some("s3://bucket/out/".into());
        assert_eq!(connection.validate(), Ok(()));
    }

    #[test]
    fn a_network_record_needs_a_port_or_a_connection_string() {
        let mut connection = base(DbType::Mssql);
        connection.port = None;
        // The engine has a default port, so the record stays valid.
        assert_eq!(connection.validate(), Ok(()));

        let mut connection = base(DbType::Mssql);
        connection.db_type = DbType::Mssql;
        connection.port = None;
        connection.options.connection_url = Some("server=tcp:host,1433".into());
        assert_eq!(connection.validate(), Ok(()));
    }

    #[test]
    fn a_record_round_trips_through_json() {
        let connection = base(DbType::Athena);
        let text = serde_json::to_string(&connection).unwrap();
        assert!(text.contains("dbType"));
        assert!(text.contains("athena"));
        let back: SavedConnection = serde_json::from_str(&text).unwrap();
        assert_eq!(back, connection);
    }

    #[test]
    fn a_record_without_options_takes_the_defaults() {
        let text = r#"{"id":"a","name":"b","dbType":"mysql"}"#;
        let connection: SavedConnection = serde_json::from_str(text).unwrap();
        assert_eq!(connection.options, ConnectionOptions::default());
        assert_eq!(connection.password, None);
        assert_eq!(connection.host, None);
        assert_eq!(connection.effective_port(), Some(3306));
    }
    #[test]
    fn a_record_written_before_the_new_options_still_reads() {
        // The options carry `serde(default)`, so a file that a former
        // release wrote parses and the new fields take their defaults.
        let text = r#"{
            "id": "c1",
            "name": "Old",
            "dbType": "athena",
            "options": { "awsRegion": "us-east-1" }
        }"#;
        let parsed: SavedConnection = serde_json::from_str(text).unwrap();
        assert_eq!(parsed.options.aws_region.as_deref(), Some("us-east-1"));
        assert!(!parsed.options.athena_result_reuse);
        assert_eq!(parsed.options.athena_result_reuse_max_age_minutes, 60);
        assert_eq!(parsed.options.max_rows, 10_000);
    }
    #[test]
    fn a_connection_string_and_entra_id_together_are_refused() {
        let mut input = base(DbType::Mssql);
        input.port = Some(1433);
        input.options.mssql_auth = MssqlAuth::EntraAzureCli;
        input.options.connection_url = Some("Server=tcp:host,1433".into());
        let message = input.validate().err().unwrap();
        assert!(message.contains("carries its own authentication"));

        // The SQL login goes through, because the string holds its own login.
        input.options.mssql_auth = MssqlAuth::SqlLogin;
        assert!(input.validate().is_ok());
    }

    #[test]
    fn the_authentication_of_an_older_record_is_read_from_the_flag() {
        let mut input = base(DbType::Mssql);
        assert_eq!(input.effective_auth(), MssqlAuth::SqlLogin);
        input.options.integrated_security = true;
        assert_eq!(input.effective_auth(), MssqlAuth::Integrated);
        assert!(MssqlAuth::EntraAzureCli.is_entra());
        assert!(!MssqlAuth::Integrated.is_entra());
    }
}
