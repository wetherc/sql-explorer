//! The AWS Athena driver.
//!
//! Athena has no session. A statement is started, the driver waits for the
//! state to reach a final value, and then it reads the pages of the result.
//! The metadata comes from the data catalog through the same service.

use crate::db::drivers::{f64_to_json, rows_returned_message, CancelHandle, DatabaseDriver};
use crate::db::{
    AppColumn, ColumnInfo, Database, DriverCapabilities, ExecOptions, QueryParams, QueryResponse,
    QueryStats, ResultSet, Schema, Table,
};
use crate::error::{Error, Result};
use crate::sql::{split_statements, Dialect};
use crate::storage::SavedConnection;
use async_trait::async_trait;
use aws_sdk_athena::types::{
    QueryExecutionContext, QueryExecutionState, QueryExecutionStatistics, ResultConfiguration,
    ResultReuseByAgeConfiguration, ResultReuseConfiguration, Row as AthenaRow,
};
use aws_sdk_athena::Client;
use serde_json::Value as JsonValue;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct AthenaDriver {
    client: Client,
    /// The catalog that holds the databases. Athena names the default
    /// catalog `AwsDataCatalog`.
    catalog: String,
    database: Option<String>,
    workgroup: Option<String>,
    output_location: Option<String>,
    /// True when the service may give the result of an earlier run.
    result_reuse: bool,
    /// The age in minutes up to which a result may be reused.
    result_reuse_max_age_minutes: u32,
    /// The identifier of the statement that runs, so that a request to
    /// stop it can reach the service.
    running: Arc<Mutex<Option<String>>>,
}

/// Reads the numbers of one execution. A number that the service leaves out
/// stays absent, so that a missing figure is not read as a zero.
fn read_statistics(statistics: Option<&QueryExecutionStatistics>) -> QueryStats {
    let Some(statistics) = statistics else {
        return QueryStats::default();
    };
    QueryStats {
        scanned_bytes: statistics
            .data_scanned_in_bytes()
            .map(|value| value.max(0) as u64),
        engine_ms: statistics
            .engine_execution_time_in_millis()
            .map(|value| value.max(0) as u64),
        queue_ms: statistics
            .query_queue_time_in_millis()
            .map(|value| value.max(0) as u64),
        result_reused: statistics
            .result_reuse_information()
            .map(|information| information.reused_previous_result()),
    }
}

/// The name Athena gives to the catalog that AWS Glue provides.
pub const DEFAULT_CATALOG: &str = "AwsDataCatalog";

impl AthenaDriver {
    pub async fn connect(connection: &SavedConnection) -> Result<Box<dyn DatabaseDriver>> {
        let region = connection
            .options
            .aws_region
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                Error::Configuration("An Athena connection needs an AWS region.".to_string())
            })?
            .to_string();

        let mut loader = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_config::Region::new(region));
        if let Some(profile) = trimmed(connection.options.aws_profile.as_deref()) {
            loader = loader.profile_name(profile);
        }
        let config = loader.load().await;

        let driver = AthenaDriver {
            client: Client::new(&config),
            catalog: trimmed(connection.options.athena_catalog.as_deref())
                .unwrap_or(DEFAULT_CATALOG)
                .to_string(),
            database: trimmed(connection.database.as_deref()).map(str::to_string),
            workgroup: trimmed(connection.options.athena_workgroup.as_deref()).map(str::to_string),
            output_location: trimmed(connection.options.athena_output_location.as_deref())
                .map(str::to_string),
            result_reuse: connection.options.athena_result_reuse,
            result_reuse_max_age_minutes: connection.options.athena_result_reuse_max_age_minutes,
            running: Arc::new(Mutex::new(None)),
        };

        // The credentials and the permissions are checked once, so that a
        // wrong profile is reported at the moment the user connects.
        driver.list_databases_inner().await?;
        Ok(Box::new(driver))
    }

    /// Starts one statement and waits for it to reach a final state.
    async fn run_statement(
        &self,
        statement: &str,
        options: &ExecOptions,
    ) -> Result<(Option<ResultSet>, QueryStats)> {
        let mut start = self
            .client
            .start_query_execution()
            .query_string(statement)
            .set_work_group(self.workgroup.clone());

        if let Some(database) = &self.database {
            start = start.query_execution_context(
                QueryExecutionContext::builder()
                    .database(database)
                    .catalog(&self.catalog)
                    .build(),
            );
        }
        if let Some(location) = &self.output_location {
            start = start.result_configuration(
                ResultConfiguration::builder()
                    .output_location(location)
                    .build(),
            );
        }
        if self.result_reuse {
            start = start.result_reuse_configuration(
                ResultReuseConfiguration::builder()
                    .result_reuse_by_age_configuration(
                        ResultReuseByAgeConfiguration::builder()
                            .enabled(true)
                            .max_age_in_minutes(self.result_reuse_max_age_minutes as i32)
                            .build(),
                    )
                    .build(),
            );
        }

        let started = start
            .send()
            .await
            .map_err(|error| describe(error, "The statement could not be started"))?;
        let execution_id = started.query_execution_id().unwrap_or_default().to_string();
        self.set_running(Some(execution_id.clone()));

        let outcome = self.wait_for(&execution_id, options).await;
        self.set_running(None);
        let stats = outcome?;

        let set = self.read_results(&execution_id, options).await?;
        Ok((set, stats))
    }

    /// Waits until the statement reaches a final state. The wait grows
    /// step by step, so that a short statement answers quickly and a long
    /// statement does not flood the service with requests.
    async fn wait_for(&self, execution_id: &str, options: &ExecOptions) -> Result<QueryStats> {
        let deadline = Instant::now() + Duration::from_secs(options.timeout_secs.max(1));
        let mut wait = Duration::from_millis(200);

        loop {
            let execution = self
                .client
                .get_query_execution()
                .query_execution_id(execution_id)
                .send()
                .await
                .map_err(|error| describe(error, "The state of the statement could not be read"))?;

            let execution = execution
                .query_execution()
                .ok_or_else(|| Error::Athena("The service returned no statement.".to_string()))?;
            let status = execution.status();
            let state = status.and_then(|status| status.state());

            match state {
                Some(QueryExecutionState::Succeeded) => {
                    return Ok(read_statistics(execution.statistics()));
                }
                Some(QueryExecutionState::Failed) => {
                    let reason = status
                        .and_then(|status| status.athena_error())
                        .and_then(|error| error.error_message())
                        .or_else(|| status.and_then(|status| status.state_change_reason()))
                        .unwrap_or("The statement failed.")
                        .to_string();
                    return Err(Error::Athena(reason));
                }
                Some(QueryExecutionState::Cancelled) => return Err(Error::Cancelled),
                _ => {}
            }

            if Instant::now() >= deadline {
                let _ = self.stop(execution_id).await;
                return Err(Error::Timeout(options.timeout_secs));
            }
            tokio::time::sleep(wait).await;
            wait = next_wait(wait);
        }
    }

    /// Reads the pages of the result, up to the row limit.
    async fn read_results(
        &self,
        execution_id: &str,
        options: &ExecOptions,
    ) -> Result<Option<ResultSet>> {
        let mut token: Option<String> = None;
        let mut set: Option<ResultSet> = None;
        let mut first_page = true;

        loop {
            let page = self
                .client
                .get_query_results()
                .query_execution_id(execution_id)
                .set_next_token(token.clone())
                .max_results(1000)
                .send()
                .await
                .map_err(|error| describe(error, "The result could not be read"))?;

            let Some(result_set) = page.result_set() else {
                break;
            };

            if set.is_none() {
                let columns = result_set
                    .result_set_metadata()
                    .map(|metadata| {
                        metadata
                            .column_info()
                            .iter()
                            .map(|column| ColumnInfo::new(column.name(), column.r#type()))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                if columns.is_empty() {
                    break;
                }
                set = Some(ResultSet::new(columns));
            }

            let current = set.as_mut().expect("the result set exists");
            let rows = result_set.rows();
            // The first page repeats the column names in its first row.
            let rows = if first_page && !rows.is_empty() && is_header(&rows[0], &current.columns) {
                &rows[1..]
            } else {
                rows
            };
            first_page = false;

            for row in rows {
                if current.rows.len() >= options.max_rows {
                    current.truncated = true;
                    return Ok(set);
                }
                let values = row_to_json(row, &current.columns);
                current.rows.push(values);
            }

            token = page.next_token().map(str::to_string);
            if token.is_none() {
                break;
            }
        }

        Ok(set)
    }

    /// Asks the service to stop a statement.
    async fn stop(&self, execution_id: &str) -> Result<()> {
        self.client
            .stop_query_execution()
            .query_execution_id(execution_id)
            .send()
            .await
            .map_err(|error| describe(error, "The statement could not be stopped"))?;
        Ok(())
    }

    fn set_running(&self, value: Option<String>) {
        if let Ok(mut guard) = self.running.lock() {
            *guard = value;
        }
    }

    async fn list_databases_inner(&self) -> Result<Vec<Database>> {
        let mut names = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let page = self
                .client
                .list_databases()
                .catalog_name(&self.catalog)
                .set_next_token(token)
                .send()
                .await
                .map_err(|error| describe(error, "The databases could not be listed"))?;
            for database in page.database_list() {
                names.push(Database {
                    name: database.name().to_string(),
                });
            }
            token = page.next_token().map(str::to_string);
            if token.is_none() {
                break;
            }
        }
        names.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(names)
    }
}

#[async_trait]
impl DatabaseDriver for AthenaDriver {
    fn capabilities(&self) -> DriverCapabilities {
        DriverCapabilities {
            supports_schemas: false,
            supports_multiple_databases: true,
            supports_cancel: true,
            // Athena runs each statement on its own.
            supports_transactions: false,
        }
    }

    fn dialect(&self) -> Dialect {
        Dialect::Athena
    }

    async fn ping(&mut self) -> Result<()> {
        self.list_databases_inner().await.map(|_| ())
    }

    async fn execute_query(
        &mut self,
        query: &str,
        params: Option<&QueryParams>,
        options: &ExecOptions,
    ) -> Result<QueryResponse> {
        if params.is_some_and(|values| !values.is_empty()) {
            return Err(Error::Unsupported(
                "Athena takes no bound parameters through this client. Put the values into the \
                 statement."
                    .to_string(),
            ));
        }

        let started = Instant::now();
        let mut response = QueryResponse::default();
        let mut total = QueryStats::default();
        for statement in split_statements(query, Dialect::Athena) {
            let (set, stats) = self.run_statement(&statement, options).await?;
            total.add(&stats);
            if let Some(set) = set {
                response
                    .messages
                    .push(rows_returned_message(set.rows.len(), set.truncated));
                response.results.push(set);
            }
        }
        if !total.is_empty() {
            response.stats = Some(total);
        }
        response.elapsed_ms = started.elapsed().as_millis() as u64;
        Ok(response)
    }

    async fn list_databases(&mut self) -> Result<Vec<Database>> {
        self.list_databases_inner().await
    }

    /// Athena has no level between the database and the table.
    async fn list_schemas(&mut self, _database: &str) -> Result<Vec<Schema>> {
        Ok(Vec::new())
    }

    async fn list_tables(&mut self, database: &str, _schema: Option<&str>) -> Result<Vec<Table>> {
        let mut tables = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let page = self
                .client
                .list_table_metadata()
                .catalog_name(&self.catalog)
                .database_name(database)
                .set_next_token(token)
                .send()
                .await
                .map_err(|error| describe(error, "The tables could not be listed"))?;
            for table in page.table_metadata_list() {
                let name = table.name().to_string();
                tables.push(match table.table_type() {
                    Some(kind) if kind.eq_ignore_ascii_case("VIRTUAL_VIEW") => Table::view(name),
                    _ => Table::table(name),
                });
            }
            token = page.next_token().map(str::to_string);
            if token.is_none() {
                break;
            }
        }
        tables.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(tables)
    }

    async fn list_columns(
        &mut self,
        database: &str,
        _schema: Option<&str>,
        table: &str,
    ) -> Result<Vec<AppColumn>> {
        let metadata = self
            .client
            .get_table_metadata()
            .catalog_name(&self.catalog)
            .database_name(database)
            .table_name(table)
            .send()
            .await
            .map_err(|error| describe(error, "The columns could not be read"))?;

        let Some(table) = metadata.table_metadata() else {
            return Ok(Vec::new());
        };

        let mut columns: Vec<AppColumn> = table
            .columns()
            .iter()
            .map(|column| AppColumn {
                name: column.name().to_string(),
                data_type: column.r#type().unwrap_or("unknown").to_string(),
                nullable: true,
                is_primary_key: false,
            })
            .collect();

        // A partition key is a column of the table as well, and the
        // explorer marks it as a key.
        columns.extend(table.partition_keys().iter().map(|column| AppColumn {
            name: column.name().to_string(),
            data_type: column.r#type().unwrap_or("unknown").to_string(),
            nullable: false,
            is_primary_key: true,
        }));

        Ok(columns)
    }

    fn cancel_handle(&self) -> Option<Arc<dyn CancelHandle>> {
        Some(Arc::new(AthenaCancel {
            client: self.client.clone(),
            running: self.running.clone(),
        }))
    }
}

/// Asks the service to stop the statement that runs.
struct AthenaCancel {
    client: Client,
    running: Arc<Mutex<Option<String>>>,
}

#[async_trait]
impl CancelHandle for AthenaCancel {
    async fn cancel(&self) -> Result<()> {
        let execution_id = self
            .running
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
            .ok_or_else(|| Error::Athena("No statement is running.".to_string()))?;
        self.client
            .stop_query_execution()
            .query_execution_id(&execution_id)
            .send()
            .await
            .map_err(|error| describe(error, "The statement could not be stopped"))?;
        Ok(())
    }
}

/// Returns the text when it holds something other than blank space.
fn trimmed(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|text| !text.is_empty())
}

/// Turns a failure of the service into an error with the reason the
/// service gave.
fn describe<E: std::error::Error + 'static, R: std::fmt::Debug>(
    error: aws_sdk_athena::error::SdkError<E, R>,
    context: &str,
) -> Error {
    use std::error::Error as _;
    let mut reason = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        reason.push_str(": ");
        reason.push_str(&cause.to_string());
        source = cause.source();
    }
    Error::Athena(format!("{context}: {reason}"))
}

/// Doubles the wait between two checks, up to two seconds.
pub fn next_wait(current: Duration) -> Duration {
    std::cmp::min(current * 2, Duration::from_secs(2))
}

/// True when the row repeats the column names. Athena puts such a row at
/// the top of the first page of a `SELECT` result.
pub fn is_header(row: &AthenaRow, columns: &[ColumnInfo]) -> bool {
    let data = row.data();
    if data.len() != columns.len() {
        return false;
    }
    data.iter()
        .zip(columns)
        .all(|(cell, column)| cell.var_char_value() == Some(column.name.as_str()))
}

/// Converts one row into an array of JSON values. Athena returns every
/// value as text, and the type of the column decides how it is read.
pub fn row_to_json(row: &AthenaRow, columns: &[ColumnInfo]) -> Vec<JsonValue> {
    let data = row.data();
    columns
        .iter()
        .enumerate()
        .map(
            |(index, column)| match data.get(index).and_then(|cell| cell.var_char_value()) {
                None => JsonValue::Null,
                Some(text) => typed_value(text, &column.type_name),
            },
        )
        .collect()
}

/// Reads a text value as the type the column declares, so that a number
/// stays a number in the result grid.
pub fn typed_value(text: &str, type_name: &str) -> JsonValue {
    match type_name {
        "boolean" => match text {
            "true" => JsonValue::Bool(true),
            "false" => JsonValue::Bool(false),
            _ => JsonValue::String(text.to_string()),
        },
        "tinyint" | "smallint" | "integer" | "int" | "bigint" => text
            .parse::<i64>()
            .map(JsonValue::from)
            .unwrap_or_else(|_| JsonValue::String(text.to_string())),
        "float" | "real" | "double" => text
            .parse::<f64>()
            .map(f64_to_json)
            .unwrap_or_else(|_| JsonValue::String(text.to_string())),
        _ => JsonValue::String(text.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_sdk_athena::types::Datum;

    #[test]
    fn the_wait_doubles_and_stops_at_two_seconds() {
        assert_eq!(
            next_wait(Duration::from_millis(200)),
            Duration::from_millis(400)
        );
        assert_eq!(
            next_wait(Duration::from_millis(1500)),
            Duration::from_secs(2)
        );
        assert_eq!(next_wait(Duration::from_secs(2)), Duration::from_secs(2));
    }

    fn row(values: &[Option<&str>]) -> AthenaRow {
        let mut builder = AthenaRow::builder();
        for value in values {
            let mut datum = Datum::builder();
            if let Some(value) = value {
                datum = datum.var_char_value(*value);
            }
            builder = builder.data(datum.build());
        }
        builder.build()
    }

    #[test]
    fn the_first_row_of_names_is_recognised() {
        let columns = vec![
            ColumnInfo::new("a", "integer"),
            ColumnInfo::new("b", "varchar"),
        ];
        assert!(is_header(&row(&[Some("a"), Some("b")]), &columns));
        assert!(!is_header(&row(&[Some("1"), Some("b")]), &columns));
        assert!(!is_header(&row(&[Some("a")]), &columns));
        assert!(!is_header(&row(&[None, Some("b")]), &columns));
    }

    #[test]
    fn a_row_gives_one_value_for_each_column() {
        let columns = vec![
            ColumnInfo::new("a", "integer"),
            ColumnInfo::new("b", "varchar"),
            ColumnInfo::new("c", "varchar"),
        ];
        assert_eq!(
            row_to_json(&row(&[Some("1"), None]), &columns),
            vec![serde_json::json!(1), JsonValue::Null, JsonValue::Null]
        );
    }

    #[test]
    fn a_text_value_is_read_as_the_type_of_the_column() {
        assert_eq!(typed_value("true", "boolean"), serde_json::json!(true));
        assert_eq!(typed_value("false", "boolean"), serde_json::json!(false));
        assert_eq!(typed_value("maybe", "boolean"), serde_json::json!("maybe"));
        assert_eq!(typed_value("42", "bigint"), serde_json::json!(42));
        assert_eq!(typed_value("42", "integer"), serde_json::json!(42));
        assert_eq!(typed_value("42", "int"), serde_json::json!(42));
        assert_eq!(typed_value("4", "tinyint"), serde_json::json!(4));
        assert_eq!(typed_value("4", "smallint"), serde_json::json!(4));
        assert_eq!(typed_value("x", "bigint"), serde_json::json!("x"));
        assert_eq!(typed_value("1.5", "double"), serde_json::json!(1.5));
        assert_eq!(typed_value("1.5", "float"), serde_json::json!(1.5));
        assert_eq!(typed_value("1.5", "real"), serde_json::json!(1.5));
        assert_eq!(typed_value("x", "double"), serde_json::json!("x"));
        assert_eq!(typed_value("abc", "varchar"), serde_json::json!("abc"));
    }

    #[test]
    fn blank_options_count_as_absent() {
        assert_eq!(trimmed(Some(" a ")), Some("a"));
        assert_eq!(trimmed(Some("  ")), None);
        assert_eq!(trimmed(None), None);
    }

    #[tokio::test]
    async fn a_connection_without_a_region_is_refused() {
        let connection = SavedConnection {
            id: "id".into(),
            name: "name".into(),
            db_type: crate::storage::DbType::Athena,
            host: None,
            port: None,
            user: None,
            database: None,
            password: None,
            options: crate::storage::ConnectionOptions::default(),
            color: None,
            group: None,
        };
        assert_eq!(
            AthenaDriver::connect(&connection)
                .await
                .err()
                .unwrap()
                .kind(),
            crate::error::ErrorKind::Configuration
        );
    }

    #[test]
    fn the_numbers_of_an_execution_are_read_as_numbers() {
        use aws_sdk_athena::types::{QueryExecutionStatistics, ResultReuseInformation};

        assert_eq!(read_statistics(None), QueryStats::default());

        let statistics = QueryExecutionStatistics::builder()
            .data_scanned_in_bytes(2048)
            .engine_execution_time_in_millis(120)
            .query_queue_time_in_millis(9)
            .result_reuse_information(
                ResultReuseInformation::builder()
                    .reused_previous_result(true)
                    .build(),
            )
            .build();
        let stats = read_statistics(Some(&statistics));
        assert_eq!(stats.scanned_bytes, Some(2048));
        assert_eq!(stats.engine_ms, Some(120));
        assert_eq!(stats.queue_ms, Some(9));
        assert_eq!(stats.result_reused, Some(true));
    }

    #[test]
    fn a_negative_figure_from_the_service_becomes_zero() {
        use aws_sdk_athena::types::QueryExecutionStatistics;
        let statistics = QueryExecutionStatistics::builder()
            .data_scanned_in_bytes(-5)
            .build();
        assert_eq!(read_statistics(Some(&statistics)).scanned_bytes, Some(0));
    }

    #[test]
    fn the_reuse_of_results_has_an_age_and_is_off_by_default() {
        let default = crate::storage::ConnectionOptions::default();
        assert!(!default.athena_result_reuse);
        assert_eq!(default.athena_result_reuse_max_age_minutes, 60);
    }
}
