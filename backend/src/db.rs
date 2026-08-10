//! The shared data model that every driver produces and every command
//! returns.

pub mod drivers;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// One value that the user interface binds into a statement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryParam {
    pub value: JsonValue,
}

pub type QueryParams = Vec<QueryParam>;

/// One column of a result set.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ColumnInfo {
    pub name: String,
    pub type_name: String,
}

impl ColumnInfo {
    pub fn new(name: impl Into<String>, type_name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            type_name: type_name.into(),
        }
    }
}

/// One result set. Rows are arrays and not objects, because a statement can
/// return two columns with the same name.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResultSet {
    pub columns: Vec<ColumnInfo>,
    pub rows: Vec<Vec<JsonValue>>,
    /// True when the driver stopped reading at the row limit.
    pub truncated: bool,
}

impl ResultSet {
    pub fn new(columns: Vec<ColumnInfo>) -> Self {
        Self {
            columns,
            rows: Vec::new(),
            truncated: false,
        }
    }
}

/// What one execution cost, for an engine that reports it. Athena charges
/// for the bytes it scans, so a user of Athena needs these numbers.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QueryStats {
    /// The bytes the engine read from storage.
    pub scanned_bytes: Option<u64>,
    /// The time the engine spent on the statement.
    pub engine_ms: Option<u64>,
    /// The time the statement waited before it started.
    pub queue_ms: Option<u64>,
    /// True when the engine gave the result of an earlier run, which costs
    /// nothing.
    pub result_reused: Option<bool>,
}

impl QueryStats {
    /// True when the record holds no number at all.
    pub fn is_empty(&self) -> bool {
        self.scanned_bytes.is_none()
            && self.engine_ms.is_none()
            && self.queue_ms.is_none()
            && self.result_reused.is_none()
    }

    /// Adds the numbers of another record to this one.
    pub fn add(&mut self, other: &QueryStats) {
        self.scanned_bytes = sum_option(self.scanned_bytes, other.scanned_bytes);
        self.engine_ms = sum_option(self.engine_ms, other.engine_ms);
        self.queue_ms = sum_option(self.queue_ms, other.queue_ms);
        // A run of statements counts as reused only when every part was.
        self.result_reused = match (self.result_reused, other.result_reused) {
            (None, value) => value,
            (value, None) => value,
            (Some(left), Some(right)) => Some(left && right),
        };
    }
}

/// Adds two numbers that may be absent, and keeps absent when both are.
fn sum_option(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (None, None) => None,
        (left, right) => Some(left.unwrap_or(0) + right.unwrap_or(0)),
    }
}

/// Everything one execution produced.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QueryResponse {
    pub results: Vec<ResultSet>,
    pub messages: Vec<String>,
    pub rows_affected: Option<u64>,
    pub elapsed_ms: u64,
    /// What the execution cost, when the engine reports it.
    pub stats: Option<QueryStats>,
}

/// Makes every column name different from the others, so that a JSON object
/// keeps one field for each column. A repeated name gets a number.
pub fn unique_column_names(columns: &[ColumnInfo]) -> Vec<String> {
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut names = Vec::with_capacity(columns.len());
    for column in columns {
        let base = if column.name.is_empty() {
            "column".to_string()
        } else {
            column.name.clone()
        };
        let count = seen.entry(base.clone()).or_insert(0);
        *count += 1;
        if *count == 1 {
            names.push(base);
        } else {
            names.push(format!("{base}_{count}"));
        }
    }
    names
}

/// The limits that apply to one execution.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecOptions {
    /// The largest number of rows the driver reads from one result set.
    pub max_rows: usize,
    /// The number of seconds after which the driver stops waiting.
    pub timeout_secs: u64,
}

impl Default for ExecOptions {
    fn default() -> Self {
        Self {
            max_rows: 10_000,
            timeout_secs: 300,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Database {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Schema {
    pub name: String,
}

/// The kind of a relation. The explorer shows a different icon for each.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TableKind {
    Table,
    View,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Table {
    pub name: String,
    pub kind: TableKind,
}

impl Table {
    #[allow(clippy::self_named_constructors)]
    pub fn table(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: TableKind::Table,
        }
    }

    pub fn view(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: TableKind::View,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppColumn {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub is_primary_key: bool,
}

/// The kind of a routine that a schema holds.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RoutineKind {
    Procedure,
    Function,
}

/// One procedure or one function of a schema.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Routine {
    pub name: String,
    pub kind: RoutineKind,
}

/// One index of a relation, with the columns it covers in their order.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct IndexInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
    /// True when the index carries the primary key of the relation.
    pub primary: bool,
}

/// The kind of a constraint of a relation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConstraintKind {
    PrimaryKey,
    ForeignKey,
    Unique,
    Check,
}

/// One constraint of a relation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Constraint {
    pub name: String,
    pub kind: ConstraintKind,
    pub columns: Vec<String>,
    /// The relation a foreign key points at, or the rule of a check.
    pub detail: Option<String>,
}

/// One partition of a relation that holds its data in parts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Partition {
    /// The values of the partition columns, as the engine reports them.
    pub values: String,
}

/// One column of a relation in a snapshot. The snapshot carries the name and
/// the type alone, because the editor shows nothing else.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotColumn {
    pub name: String,
    pub data_type: String,
}

/// One relation in a snapshot, with its columns.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotRelation {
    pub name: String,
    pub schema: Option<String>,
    pub kind: TableKind,
    pub columns: Vec<SnapshotColumn>,
}

/// Every relation and every column of one database, read in as few round
/// trips as the engine allows. The editor offers these names as completions.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SchemaSnapshot {
    pub database: String,
    pub relations: Vec<SnapshotRelation>,
    /// The number of columns the snapshot holds.
    pub column_count: usize,
    /// False when the bound on the columns stopped the read, so the names
    /// are not the whole catalog.
    pub complete: bool,
}

/// The statement that reads the CREATE text of one object from the engine,
/// and the column of the answer that carries that text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateQuery {
    pub sql: String,
    pub column: usize,
}

impl CreateQuery {
    pub fn new(sql: impl Into<String>, column: usize) -> Self {
        Self {
            sql: sql.into(),
            column,
        }
    }
}

/// What one driver can do. The user interface hides the actions that the
/// active engine does not support. The default answers no to every question,
/// which serves a test that cares about one field alone.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DriverCapabilities {
    /// True when the engine puts schemas below databases.
    pub supports_schemas: bool,
    /// True when one connection can read more than one database.
    pub supports_multiple_databases: bool,
    /// True when the driver can stop a statement that runs.
    pub supports_cancel: bool,
    /// True when the engine has transactions.
    pub supports_transactions: bool,
    /// True when the driver lists the procedures and the functions of a
    /// schema.
    pub supports_routines: bool,
    /// True when the driver lists the indexes of a relation.
    pub supports_indexes: bool,
    /// True when the driver lists the constraints of a relation.
    pub supports_constraints: bool,
    /// True when the driver lists the partitions of a relation.
    pub supports_partitions: bool,
}

/// What the connection form needs to know about one engine. The form
/// shows only the fields the engine uses.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EngineInfo {
    pub db_type: crate::storage::DbType,
    pub label: String,
    pub dialect: crate::sql::Dialect,
    pub default_port: Option<u16>,
    pub uses_host: bool,
    pub uses_credentials: bool,
    pub uses_database: bool,
    pub uses_tls: bool,
    pub uses_file: bool,
    pub uses_aws: bool,
    pub supports_schemas: bool,
    pub supports_integrated_security: bool,
}

/// Lists the engines this build supports.
pub fn supported_engines() -> Vec<EngineInfo> {
    use crate::storage::DbType;
    vec![
        EngineInfo {
            db_type: DbType::Mssql,
            dialect: DbType::Mssql.dialect(),
            label: "MS SQL Server".to_string(),
            default_port: DbType::Mssql.default_port(),
            uses_host: true,
            uses_credentials: true,
            uses_database: true,
            uses_tls: true,
            uses_file: false,
            uses_aws: false,
            supports_schemas: true,
            supports_integrated_security: true,
        },
        EngineInfo {
            db_type: DbType::Athena,
            dialect: DbType::Athena.dialect(),
            label: "AWS Athena".to_string(),
            default_port: None,
            uses_host: false,
            uses_credentials: false,
            uses_database: true,
            uses_tls: false,
            uses_file: false,
            uses_aws: true,
            supports_schemas: false,
            supports_integrated_security: false,
        },
        EngineInfo {
            db_type: DbType::Postgres,
            dialect: DbType::Postgres.dialect(),
            label: "PostgreSQL".to_string(),
            default_port: DbType::Postgres.default_port(),
            uses_host: true,
            uses_credentials: true,
            uses_database: true,
            uses_tls: true,
            uses_file: false,
            uses_aws: false,
            supports_schemas: true,
            supports_integrated_security: false,
        },
        EngineInfo {
            db_type: DbType::Mysql,
            dialect: DbType::Mysql.dialect(),
            label: "MySQL or MariaDB".to_string(),
            default_port: DbType::Mysql.default_port(),
            uses_host: true,
            uses_credentials: true,
            uses_database: true,
            uses_tls: true,
            uses_file: false,
            uses_aws: false,
            supports_schemas: false,
            supports_integrated_security: false,
        },
        EngineInfo {
            db_type: DbType::Sqlite,
            dialect: DbType::Sqlite.dialect(),
            label: "SQLite".to_string(),
            default_port: None,
            uses_host: false,
            uses_credentials: false,
            uses_database: false,
            uses_tls: false,
            uses_file: true,
            uses_aws: false,
            supports_schemas: false,
            supports_integrated_security: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_engine_is_listed_with_the_fields_it_uses() {
        use crate::storage::DbType;
        let engines = supported_engines();
        assert_eq!(engines.len(), 5);

        let mssql = &engines[0];
        assert_eq!(mssql.db_type, DbType::Mssql);
        assert_eq!(mssql.dialect, crate::sql::Dialect::MsSql);
        assert_eq!(mssql.default_port, Some(1433));
        assert!(mssql.uses_tls);
        assert!(mssql.supports_integrated_security);

        let athena = &engines[1];
        assert_eq!(athena.db_type, DbType::Athena);
        assert!(athena.uses_aws);
        assert!(!athena.uses_host);
        assert!(!athena.uses_credentials);

        let sqlite = engines
            .iter()
            .find(|e| e.db_type == DbType::Sqlite)
            .unwrap();
        assert!(sqlite.uses_file);
        assert!(!sqlite.uses_database);

        let mysql = engines.iter().find(|e| e.db_type == DbType::Mysql).unwrap();
        assert!(!mysql.supports_schemas);

        let postgres = engines
            .iter()
            .find(|e| e.db_type == DbType::Postgres)
            .unwrap();
        assert!(postgres.supports_schemas);

        // The list survives a round trip through JSON.
        let text = serde_json::to_string(&engines).unwrap();
        assert!(text.contains("defaultPort"));
        assert_eq!(
            serde_json::from_str::<Vec<EngineInfo>>(&text).unwrap(),
            engines
        );
    }

    #[test]
    fn a_new_result_set_starts_empty_and_complete() {
        let set = ResultSet::new(vec![ColumnInfo::new("id", "int")]);
        assert!(set.rows.is_empty());
        assert!(!set.truncated);
        assert_eq!(set.columns[0], ColumnInfo::new("id", "int"));
    }

    #[test]
    fn the_default_response_is_empty() {
        let response = QueryResponse::default();
        assert!(response.results.is_empty());
        assert!(response.messages.is_empty());
        assert_eq!(response.rows_affected, None);
        assert_eq!(response.elapsed_ms, 0);
    }

    #[test]
    fn the_default_limits_are_set() {
        let options = ExecOptions::default();
        assert_eq!(options.max_rows, 10_000);
        assert_eq!(options.timeout_secs, 300);
    }

    #[test]
    fn tables_and_views_carry_their_kind() {
        assert_eq!(Table::table("t").kind, TableKind::Table);
        assert_eq!(Table::view("v").kind, TableKind::View);
        assert_eq!(
            serde_json::to_value(TableKind::View).unwrap(),
            serde_json::json!("view")
        );
    }

    #[test]
    fn the_model_types_round_trip_through_json() {
        let column = AppColumn {
            name: "id".into(),
            data_type: "int".into(),
            nullable: false,
            is_primary_key: true,
        };
        let text = serde_json::to_string(&column).unwrap();
        assert!(text.contains("isPrimaryKey"));
        let back: AppColumn = serde_json::from_str(&text).unwrap();
        assert_eq!(back, column);

        let database = Database { name: "a".into() };
        assert_eq!(
            serde_json::from_str::<Database>(&serde_json::to_string(&database).unwrap()).unwrap(),
            database
        );
        let schema = Schema { name: "b".into() };
        assert_eq!(
            serde_json::from_str::<Schema>(&serde_json::to_string(&schema).unwrap()).unwrap(),
            schema
        );
        let table = Table::table("c");
        assert_eq!(
            serde_json::from_str::<Table>(&serde_json::to_string(&table).unwrap()).unwrap(),
            table
        );

        let param = QueryParam {
            value: serde_json::json!(1),
        };
        assert_eq!(
            serde_json::from_str::<QueryParam>(&serde_json::to_string(&param).unwrap())
                .unwrap()
                .value,
            param.value
        );

        let options = ExecOptions::default();
        let parsed: ExecOptions =
            serde_json::from_str(&serde_json::to_string(&options).unwrap()).unwrap();
        assert_eq!(parsed.max_rows, options.max_rows);

        let response = QueryResponse::default();
        assert_eq!(
            serde_json::from_str::<QueryResponse>(&serde_json::to_string(&response).unwrap())
                .unwrap(),
            response
        );

        let capabilities = DriverCapabilities {
            supports_schemas: true,
            supports_multiple_databases: true,
            supports_transactions: true,
            supports_indexes: true,
            ..DriverCapabilities::default()
        };
        assert_eq!(
            serde_json::from_str::<DriverCapabilities>(
                &serde_json::to_string(&capabilities).unwrap()
            )
            .unwrap(),
            capabilities
        );
    }
    #[test]
    fn a_repeated_column_name_gets_a_number() {
        let columns = vec![
            ColumnInfo::new("id", "int"),
            ColumnInfo::new("id", "int"),
            ColumnInfo::new("", "int"),
        ];
        assert_eq!(
            unique_column_names(&columns),
            vec!["id".to_string(), "id_2".to_string(), "column".to_string()]
        );
    }
    #[test]
    fn the_numbers_of_two_executions_add_up() {
        let mut total = QueryStats::default();
        assert!(total.is_empty());

        total.add(&QueryStats {
            scanned_bytes: Some(1000),
            engine_ms: Some(20),
            queue_ms: None,
            result_reused: Some(true),
        });
        total.add(&QueryStats {
            scanned_bytes: Some(500),
            engine_ms: None,
            queue_ms: Some(5),
            result_reused: Some(false),
        });

        assert!(!total.is_empty());
        assert_eq!(total.scanned_bytes, Some(1500));
        assert_eq!(total.engine_ms, Some(20));
        assert_eq!(total.queue_ms, Some(5));
        // A run counts as reused only when every statement of it was.
        assert_eq!(total.result_reused, Some(false));
    }

    #[test]
    fn a_number_that_the_engine_leaves_out_stays_absent() {
        let mut total = QueryStats::default();
        total.add(&QueryStats::default());
        assert_eq!(total.scanned_bytes, None);
        assert_eq!(total.result_reused, None);

        let mut kept = QueryStats {
            result_reused: Some(true),
            ..QueryStats::default()
        };
        kept.add(&QueryStats::default());
        assert_eq!(kept.result_reused, Some(true));
    }

    #[test]
    fn a_response_carries_the_numbers_through_json() {
        let response = QueryResponse {
            stats: Some(QueryStats {
                scanned_bytes: Some(2048),
                engine_ms: Some(31),
                queue_ms: Some(2),
                result_reused: Some(false),
            }),
            ..QueryResponse::default()
        };
        let text = serde_json::to_string(&response).unwrap();
        assert!(text.contains("scannedBytes"));
        assert!(text.contains("resultReused"));
        assert_eq!(
            serde_json::from_str::<QueryResponse>(&text).unwrap(),
            response
        );
    }
}
