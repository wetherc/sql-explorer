//! The interface that every engine implements, and the helpers the
//! implementations share.

pub mod athena;
#[cfg(test)]
pub mod live;
pub mod mssql;
pub mod mysql;
pub mod postgres;
pub mod sqlite;

use crate::db::sink::{BufferSink, RowSink, RunSummary};
use crate::db::{
    AppColumn, Constraint, ConstraintType, CreateQuery, Database, DriverCapabilities, ExecOptions,
    IndexInfo, Message, ObjectType, PartitionList, PlanMode, QueryParams, QueryResponse,
    RelationType, Routine, RoutineType, ScheduledEvent, Schema, SchemaSnapshot, SnapshotColumn,
    SnapshotRelation, Table, TableFact, Trigger, TriggerEvent, TriggerTiming,
};
use crate::error::{Error, Result};
use crate::sql::{split_statements, Dialect};
use async_trait::async_trait;
use base64::Engine as _;
use rustls::RootCertStore;
use rustls_pki_types::CertificateDer;
use serde_json::Value as JsonValue;
use std::sync::{Arc, OnceLock};

/// The operations the application asks of one open connection.
#[async_trait]
pub trait DatabaseDriver: Send + Sync {
    /// Reports what the engine behind this driver can do.
    fn capabilities(&self) -> DriverCapabilities;

    /// Returns the SQL dialect of the engine.
    fn dialect(&self) -> Dialect;

    /// Confirms that the connection still answers. The command layer calls
    /// this before it lends the connection out.
    async fn ping(&mut self) -> Result<()>;

    /// True when a connection that stood idle must be checked before it is
    /// used again. A driver that holds no session, such as Athena, answers
    /// false, and the command layer then skips the check.
    fn needs_ping(&self) -> bool {
        true
    }

    /// True when the session is inside a transaction that it did not end.
    /// The idle reaper keeps such a session, because a close of it rolls
    /// the work of the transaction back. A driver that holds no session,
    /// such as Athena, answers false.
    async fn holds_open_transaction(&mut self) -> Result<bool> {
        Ok(false)
    }

    /// True when the connection stays fit for use after a limit stopped a
    /// statement in the middle of its run.
    ///
    /// A driver that speaks a wire protocol answers false, because the stop
    /// leaves the connection in the middle of a message. A driver whose work
    /// runs outside the connection, such as Athena, and a driver whose engine
    /// aborts a statement cleanly, such as SQLite, answer true.
    fn keeps_connection_after_stop(&self) -> bool {
        false
    }

    /// Runs a script and sends each row to the sink as the read produces it.
    /// A driver that streams holds one row at a time, and the sink decides
    /// what the rows become.
    ///
    /// The default refuses. A driver keeps its own `execute_query` until it
    /// implements this method.
    async fn execute_stream(
        &mut self,
        _query: &str,
        _params: Option<&QueryParams>,
        _options: &ExecOptions,
        _sink: &mut dyn RowSink,
    ) -> Result<RunSummary> {
        Err(Error::Unsupported(
            "This driver can't stream rows.".to_string(),
        ))
    }

    /// Runs a script and returns every result set it produced. The default
    /// runs `execute_stream` into a buffer that keeps the rows up to the row
    /// limit. A driver overrides this method until it has `execute_stream`.
    async fn execute_query(
        &mut self,
        query: &str,
        params: Option<&QueryParams>,
        options: &ExecOptions,
    ) -> Result<QueryResponse> {
        let mut sink = BufferSink::new(options.max_rows);
        let summary = self
            .execute_stream(query, params, options, &mut sink)
            .await?;
        Ok(sink.into_response(summary))
    }

    async fn list_databases(&mut self) -> Result<Vec<Database>>;

    async fn list_schemas(&mut self, database: &str) -> Result<Vec<Schema>>;

    async fn list_tables(&mut self, database: &str, schema: Option<&str>) -> Result<Vec<Table>>;

    async fn list_columns(
        &mut self,
        database: &str,
        schema: Option<&str>,
        table: &str,
    ) -> Result<Vec<AppColumn>>;

    /// Lists the procedures and the functions of one schema. An engine that
    /// holds none answers with an empty list, and the capability record says
    /// so, which keeps the folder out of the tree.
    async fn list_routines(
        &mut self,
        _database: &str,
        _schema: Option<&str>,
    ) -> Result<Vec<Routine>> {
        Ok(Vec::new())
    }

    /// Lists the indexes of one relation, with the columns of each index.
    async fn list_indexes(
        &mut self,
        _database: &str,
        _schema: Option<&str>,
        _table: &str,
    ) -> Result<Vec<IndexInfo>> {
        Ok(Vec::new())
    }

    /// Lists the constraints of one relation.
    async fn list_constraints(
        &mut self,
        _database: &str,
        _schema: Option<&str>,
        _table: &str,
    ) -> Result<Vec<Constraint>> {
        Ok(Vec::new())
    }

    /// Lists the partitions of one relation that holds its data in parts.
    async fn list_partitions(
        &mut self,
        _database: &str,
        _schema: Option<&str>,
        _table: &str,
    ) -> Result<PartitionList> {
        Ok(PartitionList::default())
    }

    /// Lists the triggers of one relation. An engine without triggers
    /// answers with an empty list, and the capability record keeps the
    /// folder out of the tree.
    async fn list_triggers(
        &mut self,
        _database: &str,
        _schema: Option<&str>,
        _table: &str,
    ) -> Result<Vec<Trigger>> {
        Ok(Vec::new())
    }

    /// Lists the scheduled events of one database. An engine without events
    /// answers with an empty list.
    async fn list_events(
        &mut self,
        _database: &str,
        _schema: Option<&str>,
    ) -> Result<Vec<ScheduledEvent>> {
        Ok(Vec::new())
    }

    /// Reads the facts of one relation, such as the number of rows it holds
    /// and its size on disk. An engine that reports none answers with an
    /// empty list.
    async fn table_facts(
        &mut self,
        _database: &str,
        _schema: Option<&str>,
        _table: &str,
    ) -> Result<Vec<TableFact>> {
        Ok(Vec::new())
    }

    /// Reads every relation and every column of one database.
    ///
    /// The default walks the lists of the tree, which costs one call for each
    /// relation. An engine whose catalog answers in one statement overrides
    /// this method.
    ///
    /// The read stops when the columns reach the bound, and the answer then
    /// reports that it is not complete.
    async fn schema_snapshot(
        &mut self,
        database: &str,
        max_columns: usize,
    ) -> Result<SchemaSnapshot> {
        let mut snapshot = SchemaSnapshot {
            database: database.to_string(),
            complete: true,
            ..SchemaSnapshot::default()
        };
        let schemas = self.list_schemas(database).await?;
        let places: Vec<Option<String>> = if schemas.is_empty() {
            vec![None]
        } else {
            schemas
                .into_iter()
                .map(|schema| Some(schema.name))
                .collect()
        };

        for place in places {
            let tables = self.list_tables(database, place.as_deref()).await?;
            for table in tables {
                if snapshot.column_count >= max_columns {
                    snapshot.complete = false;
                    return Ok(snapshot);
                }
                let columns = self
                    .list_columns(database, place.as_deref(), &table.name)
                    .await?;
                snapshot.column_count += columns.len();
                snapshot.relations.push(SnapshotRelation {
                    name: table.name,
                    schema: place.clone(),
                    relation_type: table.relation_type,
                    columns: columns
                        .into_iter()
                        .map(|column| SnapshotColumn {
                            name: column.name,
                            data_type: column.data_type,
                        })
                        .collect(),
                });
            }
        }
        Ok(snapshot)
    }

    /// Reads the plan of one statement. The estimated plan does not run the
    /// statement. The actual plan runs it, so the interface asks the user
    /// first.
    ///
    /// A driver that gives no plan keeps this default and refuses.
    async fn explain(
        &mut self,
        _query: &str,
        _params: Option<&QueryParams>,
        _mode: PlanMode,
        _options: &ExecOptions,
    ) -> Result<QueryResponse> {
        Err(Error::Unsupported(
            "This database doesn't support execution plans.".to_string(),
        ))
    }

    /// Returns the statement that reads the CREATE text of one object from
    /// the engine. An engine that gives no such text returns `None`, and the
    /// command layer builds a draft from the column list instead.
    ///
    /// The method builds text alone and reaches no server, so a test can
    /// check the statement of every engine.
    fn create_query(
        &self,
        _database: Option<&str>,
        _schema: Option<&str>,
        _table: &str,
        _relation_type: RelationType,
    ) -> Option<CreateQuery> {
        None
    }

    /// Returns the statement that reads the CREATE text of one trigger or
    /// one event from the engine. `parent` names the relation of a trigger.
    /// An engine that gives no such text returns `None`. A draft cannot
    /// take the place of the text, because the catalog of the columns says
    /// nothing about the body of the object.
    fn object_create_query(
        &self,
        _database: Option<&str>,
        _schema: Option<&str>,
        _parent: Option<&str>,
        _name: &str,
        _object_type: ObjectType,
    ) -> Option<CreateQuery> {
        None
    }

    /// Returns a handle that can ask the server to stop a statement while
    /// the driver itself is busy with that statement. A driver that cannot
    /// do this returns `None`, and the command layer closes the connection
    /// instead.
    fn cancel_handle(&self) -> Option<Arc<dyn CancelHandle>> {
        None
    }
}

/// Stops a statement that runs. The handle holds everything it needs, so
/// it works while the driver is locked.
#[async_trait]
pub trait CancelHandle: Send + Sync {
    async fn cancel(&self) -> Result<()>;
}

/// The form a JSON number takes when a driver binds it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NumberValue {
    Integer(i64),
    Float(f64),
}

/// Selects the form of a JSON number.
///
/// Returns `None` for a whole number above the range of a 64-bit signed
/// integer. No engine here holds such a value, and a driver must refuse it
/// rather than turn it into a floating point value that has lost digits.
pub fn number_value(number: &serde_json::Number) -> Option<NumberValue> {
    if let Some(value) = number.as_i64() {
        return Some(NumberValue::Integer(value));
    }
    if number.is_f64() {
        return number.as_f64().map(NumberValue::Float);
    }
    None
}

/// The message a driver gives for a number it cannot bind.
pub fn number_out_of_range(number: &serde_json::Number) -> Error {
    Error::Configuration(format!(
        "Parameter value {number} is outside the integer range this database supports."
    ))
}

/// The message a driver gives for a parameter whose type it cannot bind.
pub fn parameter_type_refused(value: &JsonValue) -> Error {
    Error::Configuration(format!(
        "Parameter value {value} has a type this driver can't send."
    ))
}

/// Renders a slice of bytes as a base64 text value, because JSON holds no
/// binary type.
pub fn bytes_to_json(bytes: &[u8]) -> JsonValue {
    JsonValue::String(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// Turns a four-byte float into JSON with the digits that the float shows.
///
/// A widening cast gives the double nearest to the float, and that double
/// shows 0.1 as 0.10000000149011612. The shortest text of the float reads
/// back as the double that the user wrote.
pub fn f32_to_json(value: f32) -> JsonValue {
    f64_to_json(value.to_string().parse().unwrap_or(f64::from(value)))
}

/// Builds a JSON number from a floating point value. A value that is not a
/// number, such as infinity, becomes text so that the result stays valid
/// JSON.
pub fn f64_to_json(value: f64) -> JsonValue {
    match serde_json::Number::from_f64(value) {
        Some(number) => JsonValue::Number(number),
        None => JsonValue::String(float_text(value)),
    }
}

/// Writes a floating point value as text. The special values use the words
/// that the database engines print (`NaN`, `Infinity`, `-Infinity`), where
/// Rust prints `inf`.
pub fn float_text(value: f64) -> String {
    if value.is_nan() {
        "NaN".to_owned()
    } else if value.is_infinite() {
        if value > 0.0 { "Infinity" } else { "-Infinity" }.to_owned()
    } else {
        value.to_string()
    }
}

/// Adds bytes to `out` as hexadecimal digits, two digits for each byte.
pub fn hex_text(out: &mut String, bytes: &[u8], upper: bool) {
    const LOWER: &[u8; 16] = b"0123456789abcdef";
    const UPPER: &[u8; 16] = b"0123456789ABCDEF";
    let digits = if upper { UPPER } else { LOWER };
    out.reserve(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(digits[usize::from(byte >> 4)]));
        out.push(char::from(digits[usize::from(byte & 0x0F)]));
    }
}

/// Reports the count of rows of the open result set and ends the set.
pub fn finish_set(sink: &mut dyn RowSink, count: usize, truncated: bool) -> Result<()> {
    sink.message(rows_returned_message(count, truncated));
    sink.end_set(truncated)
}

/// Runs a step of the opening of a connection under the time limit. A step
/// that does not finish reports the connection and not the statement, because
/// the advice for a slow statement does not fit a server that never answered.
pub async fn connect_within<F: std::future::Future>(
    limit_secs: u64,
    future: F,
) -> Result<F::Output> {
    tokio::time::timeout(std::time::Duration::from_secs(limit_secs), future)
        .await
        .map_err(|_| {
            Error::Connection(format!(
                "The server didn't finish opening the connection within {limit_secs} seconds."
            ))
        })
}

/// Returns the trimmed text when it contains something other than blank space.
pub fn non_empty(value: &Option<String>) -> Option<&str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

/// Writes a size in bytes in the largest unit that keeps it above one, so a
/// reader sees "1.5 GB" and not a long row of digits.
pub fn size_text(bytes: u64) -> String {
    const STEP: f64 = 1024.0;
    let units = ["bytes", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= STEP && unit + 1 < units.len() {
        value /= STEP;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", units[0])
    } else {
        format!("{value:.1} {}", units[unit])
    }
}

/// Takes the one statement of a plan request.
///
/// A plan covers one statement, because the keyword that asks for it stands
/// in front of that statement. A request with two statements is therefore
/// refused, and the trailing semicolon goes, so that the statement fits
/// behind a keyword.
///
/// A batch separator carries no plan of its own, so a text that holds more
/// than one batch is refused as well.
pub fn single_statement(query: &str, dialect: Dialect) -> Result<String> {
    let batches = crate::sql::split_batches(query, dialect);
    if batches.len() > 1 {
        return Err(Error::Configuration(format!(
            "The script contains {} batches. Select a single statement to see its plan.",
            batches.len()
        )));
    }
    let statements = match batches.first() {
        Some(batch) => split_statements(&batch.text, dialect),
        None => Vec::new(),
    };
    match statements.len() {
        0 => Err(Error::Configuration(
            "There's no statement to show a plan for.".to_string(),
        )),
        1 => Ok(statements[0]
            .trim()
            .trim_end_matches(';')
            .trim()
            .to_string()),
        count => Err(Error::Configuration(format!(
            "The script contains {count} statements. Select a single statement to see its plan."
        ))),
    }
}

/// Builds the plan statement of an engine that puts a keyword in front of
/// the statement.
pub fn prefixed_plan(query: &str, dialect: Dialect, prefix: &str) -> Result<String> {
    let statement = single_statement(query, dialect)?;
    Ok(format!("{prefix} {statement}"))
}

/// Adds one column of one relation to a snapshot, and starts a record when
/// the relation is new. The rows must arrive in the order of the relation,
/// because the record of a relation that comes back a second time starts
/// again.
///
/// Returns false when the columns have reached the bound, and the caller then
/// stops reading and reports that the snapshot is not complete.
pub fn add_snapshot_column(
    snapshot: &mut SchemaSnapshot,
    max_columns: usize,
    schema: Option<String>,
    relation: String,
    relation_type: RelationType,
    column: SnapshotColumn,
) -> bool {
    if !add_snapshot_relation(snapshot, max_columns, schema, relation, relation_type) {
        return false;
    }
    snapshot
        .relations
        .last_mut()
        .expect("the record was just found or added")
        .columns
        .push(column);
    snapshot.column_count += 1;
    true
}

/// Starts the record of a relation in a snapshot when the relation is new,
/// and adds no column. A relation with no known column, such as a synonym
/// that points out of the database, uses this function. The bound applies
/// as in [`add_snapshot_column`].
pub fn add_snapshot_relation(
    snapshot: &mut SchemaSnapshot,
    max_columns: usize,
    schema: Option<String>,
    relation: String,
    relation_type: RelationType,
) -> bool {
    if snapshot.column_count >= max_columns {
        snapshot.complete = false;
        return false;
    }
    let known = matches!(
        snapshot.relations.last(),
        Some(entry) if entry.name == relation && entry.schema == schema
    );
    if !known {
        snapshot.relations.push(SnapshotRelation {
            name: relation,
            schema,
            relation_type,
            columns: Vec::new(),
        });
    }
    true
}

/// Reads the word of `INFORMATION_SCHEMA` that names the type of a relation.
/// MySQL names the views of its own schemas `SYSTEM VIEW`.
pub fn relation_type(word: &str) -> RelationType {
    let word = word.trim();
    if word.eq_ignore_ascii_case("VIEW") || word.eq_ignore_ascii_case("SYSTEM VIEW") {
        RelationType::View
    } else {
        RelationType::Table
    }
}

/// Adds one column to the record of its index, and starts a record when the
/// index is new. Every engine reports one column of one index in each row of
/// the answer, so every driver folds the rows this way.
///
/// A row without a column name gives an index with no column, which an
/// engine reports for an index on an expression.
pub fn add_index_column(
    indexes: &mut Vec<IndexInfo>,
    name: String,
    unique: bool,
    primary: bool,
    column: Option<String>,
) {
    let entry = index_entry(indexes, name, unique, primary);
    if let Some(column) = column {
        entry.columns.push(column);
    }
}

/// Adds one `INCLUDE` column to the record of its index, and starts a record
/// when the index is new.
pub fn add_included_column(
    indexes: &mut Vec<IndexInfo>,
    name: String,
    unique: bool,
    primary: bool,
    column: String,
) {
    index_entry(indexes, name, unique, primary)
        .included
        .push(column);
}

/// Finds the record of an index by its name, or adds a record with no column.
fn index_entry(
    indexes: &mut Vec<IndexInfo>,
    name: String,
    unique: bool,
    primary: bool,
) -> &mut IndexInfo {
    // The rows of one record come together, so the search starts at the
    // last record and a row of the current record costs one comparison.
    match indexes.iter().rposition(|index| index.name == name) {
        Some(position) => &mut indexes[position],
        None => {
            indexes.push(IndexInfo {
                name,
                columns: Vec::new(),
                unique,
                primary,
                included: Vec::new(),
            });
            indexes.last_mut().expect("the record was just added")
        }
    }
}

/// Adds one column to the record of its constraint, and starts a record when
/// the constraint is new. A check constraint covers no column, so a row
/// without a column name still gives a record.
pub fn add_constraint_column(
    constraints: &mut Vec<Constraint>,
    name: String,
    constraint_type: ConstraintType,
    column: Option<String>,
    detail: Option<String>,
) {
    let entry = match constraints
        .iter_mut()
        .rev()
        .find(|constraint| constraint.name == name)
    {
        Some(entry) => entry,
        None => {
            constraints.push(Constraint {
                name,
                constraint_type,
                columns: Vec::new(),
                detail,
            });
            constraints.last_mut().expect("the record was just added")
        }
    };
    if let Some(column) = column {
        entry.columns.push(column);
    }
}

/// Reads the type of a constraint from the word the engine reports. The
/// engines answer with the words of `INFORMATION_SCHEMA` or with the one
/// letter of PostgreSQL. A word that no rule names gives a check.
pub fn constraint_type(word: &str) -> ConstraintType {
    match word.trim().to_uppercase().as_str() {
        "PRIMARY KEY" | "P" => ConstraintType::PrimaryKey,
        "FOREIGN KEY" | "F" => ConstraintType::ForeignKey,
        "UNIQUE" | "U" => ConstraintType::Unique,
        "X" => ConstraintType::Exclusion,
        "T" => ConstraintType::Trigger,
        "N" => ConstraintType::NotNull,
        "DEFAULT" => ConstraintType::Default,
        _ => ConstraintType::Check,
    }
}

/// Adds one event to the record of its trigger, and starts a record when the
/// trigger is new. MS SQL Server reports one event of one trigger in each
/// row, so the rows are folded into one record for each trigger.
pub fn add_trigger_event(
    triggers: &mut Vec<Trigger>,
    name: String,
    timing: TriggerTiming,
    enabled: bool,
    event: Option<TriggerEvent>,
) {
    let entry = match triggers
        .iter_mut()
        .rev()
        .find(|trigger| trigger.name == name)
    {
        Some(entry) => entry,
        None => {
            triggers.push(Trigger {
                name,
                timing,
                events: Vec::new(),
                enabled,
                replica: false,
                update_columns: Vec::new(),
            });
            triggers.last_mut().expect("the record was just added")
        }
    };
    if let Some(event) = event {
        if !entry.events.contains(&event) {
            entry.events.push(event);
        }
    }
}

/// Reads the word of the catalog that names the change that fires a trigger.
/// A word that names no such change gives `None`.
pub fn trigger_event(word: &str) -> Option<TriggerEvent> {
    match word.trim().to_uppercase().as_str() {
        "INSERT" => Some(TriggerEvent::Insert),
        "UPDATE" => Some(TriggerEvent::Update),
        "DELETE" => Some(TriggerEvent::Delete),
        "TRUNCATE" => Some(TriggerEvent::Truncate),
        _ => None,
    }
}

/// Reads the word of the catalog that names the time a trigger runs. A
/// word that names no time gives `AFTER`, which is the time that MS SQL
/// Server calls `FOR`.
pub fn trigger_timing(word: &str) -> TriggerTiming {
    match word.trim().to_uppercase().as_str() {
        "BEFORE" => TriggerTiming::Before,
        "INSTEAD OF" => TriggerTiming::InsteadOf,
        _ => TriggerTiming::After,
    }
}

/// Reads the type of a routine from the word the engine reports. A word that
/// is not `PROCEDURE` names a function, because an engine has other types of
/// function and no other type of procedure.
pub fn routine_type(word: &str) -> RoutineType {
    if word.trim().eq_ignore_ascii_case("PROCEDURE") {
        RoutineType::Procedure
    } else {
        RoutineType::Function
    }
}

/// Adds a message that reports how many rows a statement changed.
pub fn rows_affected_message(count: u64) -> Message {
    if count == 1 {
        Message::info("1 row affected.")
    } else {
        Message::info(format!("{count} rows affected."))
    }
}

/// Adds a message that reports how many rows a statement returned.
pub fn rows_returned_message(count: usize, truncated: bool) -> Message {
    let plural = if count == 1 { "row" } else { "rows" };
    if truncated {
        Message::warning(format!(
            "{count} {plural} returned. Stopped at the row limit."
        ))
    } else {
        Message::info(format!("{count} {plural} returned."))
    }
}

/// Takes the values of the next statement of a script from the list of all
/// values. An engine that marks each place with `?` counts the places of a
/// statement when it prepares it, and the values of the script stand in the
/// order of those places. `used` counts the values that earlier statements
/// took.
pub fn next_values<T: Clone>(values: &[T], used: &mut usize, count: usize) -> Vec<T> {
    let start = (*used).min(values.len());
    let end = (start + count).min(values.len());
    *used = end;
    values[start..end].to_vec()
}

/// The roots that the operating system trusts and that `rustls` can read.
/// These come from the keychain on macOS, from the certificate store on
/// Windows, and from the certificate files of OpenSSL on Linux, so a company
/// authority that the system trusts is trusted by the drivers too. A read of
/// the system roots takes about 100 ms, so the list is read once in each run
/// of the application. A root that the system gets after the start is used
/// after a restart. The list is empty when the system gives no usable root.
pub fn system_roots() -> &'static [CertificateDer<'static>] {
    static ROOTS: OnceLock<Vec<CertificateDer<'static>>> = OnceLock::new();
    ROOTS.get_or_init(|| usable_roots(rustls_native_certs::load_native_certs().certs))
}

/// Keeps the certificates that a store of trusted roots accepts.
fn usable_roots(certificates: Vec<CertificateDer<'static>>) -> Vec<CertificateDer<'static>> {
    certificates
        .into_iter()
        .filter(|certificate| RootCertStore::empty().add(certificate.clone()).is_ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plan_takes_one_statement_without_its_semicolon() {
        assert_eq!(
            single_statement("SELECT 1;", Dialect::MsSql).unwrap(),
            "SELECT 1"
        );
        assert_eq!(
            prefixed_plan("SELECT 1", Dialect::Postgres, "EXPLAIN").unwrap(),
            "EXPLAIN SELECT 1"
        );
    }

    #[test]
    fn a_plan_refuses_no_statement_and_more_than_one() {
        let none = single_statement("   ", Dialect::MsSql).unwrap_err();
        assert!(none.to_string().contains("no statement"));

        let many = single_statement("SELECT 1; SELECT 2;", Dialect::MsSql).unwrap_err();
        assert!(many.to_string().contains("2 statements"));

        let refused = prefixed_plan("SELECT 1; SELECT 2", Dialect::MsSql, "EXPLAIN").unwrap_err();
        assert!(refused.to_string().contains("Select a single statement"));
    }

    #[test]
    fn a_size_takes_the_largest_unit_that_fits() {
        assert_eq!(size_text(0), "0 bytes");
        assert_eq!(size_text(512), "512 bytes");
        assert_eq!(size_text(2048), "2.0 KB");
        assert_eq!(size_text(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(size_text(3 * 1024 * 1024 * 1024), "3.0 GB");
        // The largest unit holds, however big the figure is.
        assert_eq!(size_text(2048_u64 * 1024 * 1024 * 1024 * 1024), "2048.0 TB");
    }

    #[test]
    fn a_float_that_is_not_a_number_uses_the_engine_words() {
        assert_eq!(float_text(f64::NAN), "NaN");
        assert_eq!(float_text(f64::INFINITY), "Infinity");
        assert_eq!(float_text(f64::NEG_INFINITY), "-Infinity");
        assert_eq!(float_text(1.5), "1.5");
        assert_eq!(f64_to_json(f64::INFINITY), JsonValue::from("Infinity"));
        assert_eq!(f64_to_json(2.5), serde_json::json!(2.5));
    }

    #[test]
    fn hex_text_writes_two_digits_for_each_byte() {
        let mut out = String::from("0x");
        hex_text(&mut out, &[0x00, 0xAB, 0x0F], true);
        assert_eq!(out, "0x00AB0F");
        let mut lower = String::new();
        hex_text(&mut lower, &[0xAB, 0xCD], false);
        assert_eq!(lower, "abcd");
    }

    #[test]
    fn finish_set_reports_the_count_and_ends_the_set() {
        let mut sink = BufferSink::new(10);
        sink.begin_set(vec![]).unwrap();
        finish_set(&mut sink, 3, true).unwrap();
        let response = sink.into_response(RunSummary {
            rows_affected: None,
            elapsed_ms: 0,
            stats: None,
        });
        assert!(response.results[0].truncated);
        assert_eq!(response.messages.len(), 1);
    }

    #[tokio::test]
    async fn connect_within_reports_a_slow_server_as_a_connection_error() {
        let late = connect_within(0, std::future::pending::<()>())
            .await
            .unwrap_err();
        assert!(matches!(late, Error::Connection(ref text) if text.contains("within 0 seconds")));
        assert_eq!(connect_within(5, async { 7 }).await.unwrap(), 7);
    }

    #[test]
    fn non_empty_trims_and_drops_blank_text() {
        assert_eq!(non_empty(&Some("  db ".into())), Some("db"));
        assert_eq!(non_empty(&Some("   ".into())), None);
        assert_eq!(non_empty(&None), None);
    }
    use crate::db::MessageLevel;

    #[test]
    fn the_rows_of_a_snapshot_fold_into_one_record_for_each_relation() {
        let mut snapshot = SchemaSnapshot {
            database: "Sales".into(),
            complete: true,
            ..SchemaSnapshot::default()
        };
        let column = |name: &str| SnapshotColumn {
            name: name.to_string(),
            data_type: "int".to_string(),
        };
        assert!(add_snapshot_column(
            &mut snapshot,
            10,
            Some("dbo".into()),
            "orders".into(),
            RelationType::Table,
            column("id"),
        ));
        assert!(add_snapshot_column(
            &mut snapshot,
            10,
            Some("dbo".into()),
            "orders".into(),
            RelationType::Table,
            column("total"),
        ));
        assert!(add_snapshot_column(
            &mut snapshot,
            10,
            Some("staging".into()),
            "orders".into(),
            RelationType::View,
            column("id"),
        ));

        assert_eq!(snapshot.relations.len(), 2);
        assert_eq!(snapshot.relations[0].columns.len(), 2);
        assert_eq!(snapshot.relations[1].schema.as_deref(), Some("staging"));
        assert_eq!(snapshot.relations[1].relation_type, RelationType::View);
        assert_eq!(snapshot.column_count, 3);
        assert!(snapshot.complete);
    }

    #[test]
    fn a_relation_with_no_column_gets_a_record_of_its_own() {
        let mut snapshot = SchemaSnapshot {
            complete: true,
            ..SchemaSnapshot::default()
        };
        let column = SnapshotColumn {
            name: "id".into(),
            data_type: "int".into(),
        };
        assert!(add_snapshot_column(
            &mut snapshot,
            1,
            Some("dbo".into()),
            "orders".into(),
            RelationType::Table,
            column,
        ));
        assert!(add_snapshot_relation(
            &mut snapshot,
            2,
            Some("dbo".into()),
            "far".into(),
            RelationType::Synonym,
        ));
        // A second row of the same relation adds no second record.
        assert!(add_snapshot_relation(
            &mut snapshot,
            2,
            Some("dbo".into()),
            "far".into(),
            RelationType::Synonym,
        ));
        assert_eq!(snapshot.relations.len(), 2);
        assert_eq!(snapshot.relations[1].relation_type, RelationType::Synonym);
        assert!(snapshot.relations[1].columns.is_empty());
        assert_eq!(snapshot.column_count, 1);

        // At the bound, a relation with no column also stops the read.
        assert!(!add_snapshot_relation(
            &mut snapshot,
            1,
            None,
            "late".into(),
            RelationType::Synonym,
        ));
        assert!(!snapshot.complete);
        assert_eq!(snapshot.relations.len(), 2);
    }

    #[test]
    fn the_bound_stops_a_snapshot_and_marks_it_as_a_part() {
        let mut snapshot = SchemaSnapshot {
            complete: true,
            ..SchemaSnapshot::default()
        };
        let column = SnapshotColumn {
            name: "id".into(),
            data_type: "int".into(),
        };
        assert!(add_snapshot_column(
            &mut snapshot,
            1,
            None,
            "orders".into(),
            RelationType::Table,
            column.clone(),
        ));
        assert!(!add_snapshot_column(
            &mut snapshot,
            1,
            None,
            "orders".into(),
            RelationType::Table,
            column,
        ));
        assert!(!snapshot.complete);
        assert_eq!(snapshot.column_count, 1);
    }

    #[test]
    fn the_word_of_the_catalog_names_a_view() {
        assert_eq!(relation_type("VIEW"), RelationType::View);
        assert_eq!(relation_type("SYSTEM VIEW"), RelationType::View);
        assert_eq!(relation_type("view"), RelationType::View);
        assert_eq!(relation_type("BASE TABLE"), RelationType::Table);
    }

    #[test]
    fn the_rows_of_an_index_fold_into_one_record() {
        let mut indexes: Vec<IndexInfo> = Vec::new();
        add_index_column(
            &mut indexes,
            "by_name".into(),
            true,
            false,
            Some("a".into()),
        );
        add_index_column(
            &mut indexes,
            "by_name".into(),
            true,
            false,
            Some("b".into()),
        );
        add_index_column(&mut indexes, "on_lower".into(), false, false, None);
        assert_eq!(indexes.len(), 2);
        assert_eq!(indexes[0].columns, vec!["a".to_string(), "b".to_string()]);
        assert!(indexes[0].unique);
        assert!(indexes[1].columns.is_empty());
    }

    #[test]
    fn each_statement_takes_the_values_of_its_places() {
        let values = [1, 2, 3];
        let mut used = 0;
        assert_eq!(next_values(&values, &mut used, 2), vec![1, 2]);
        assert_eq!(next_values(&values, &mut used, 0), Vec::<i32>::new());
        assert_eq!(next_values(&values, &mut used, 5), vec![3]);
        assert_eq!(used, 3);
        assert_eq!(next_values(&values, &mut used, 1), Vec::<i32>::new());
    }

    #[test]
    fn a_certificate_that_is_not_a_root_is_left_out() {
        assert!(usable_roots(vec![CertificateDer::from(vec![0, 1, 2])]).is_empty());
        let system = system_roots();
        assert_eq!(usable_roots(system.to_vec()).len(), system.len());
    }

    #[test]
    fn an_include_column_goes_after_the_key_of_its_index() {
        let mut indexes: Vec<IndexInfo> = Vec::new();
        add_included_column(&mut indexes, "cover".into(), false, false, "c".into());
        add_index_column(&mut indexes, "cover".into(), false, false, Some("a".into()));
        add_included_column(&mut indexes, "cover".into(), false, false, "d".into());
        assert_eq!(indexes.len(), 1);
        assert_eq!(indexes[0].columns, vec!["a".to_string()]);
        assert_eq!(indexes[0].included, vec!["c".to_string(), "d".to_string()]);
    }

    #[test]
    fn the_rows_of_a_constraint_fold_into_one_record() {
        let mut constraints: Vec<Constraint> = Vec::new();
        add_constraint_column(
            &mut constraints,
            "pk_orders".into(),
            ConstraintType::PrimaryKey,
            Some("id".into()),
            None,
        );
        add_constraint_column(
            &mut constraints,
            "pk_orders".into(),
            ConstraintType::PrimaryKey,
            Some("region".into()),
            None,
        );
        add_constraint_column(
            &mut constraints,
            "total_positive".into(),
            ConstraintType::Check,
            None,
            Some("total > 0".into()),
        );
        assert_eq!(constraints.len(), 2);
        assert_eq!(
            constraints[0].columns,
            vec!["id".to_string(), "region".to_string()]
        );
        assert!(constraints[1].columns.is_empty());
        assert_eq!(constraints[1].detail.as_deref(), Some("total > 0"));
    }

    #[test]
    fn the_word_of_the_engine_names_the_type() {
        assert_eq!(constraint_type("PRIMARY KEY"), ConstraintType::PrimaryKey);
        assert_eq!(constraint_type("p"), ConstraintType::PrimaryKey);
        assert_eq!(constraint_type("FOREIGN KEY"), ConstraintType::ForeignKey);
        assert_eq!(constraint_type("f"), ConstraintType::ForeignKey);
        assert_eq!(constraint_type("UNIQUE"), ConstraintType::Unique);
        assert_eq!(constraint_type("u"), ConstraintType::Unique);
        assert_eq!(constraint_type("c"), ConstraintType::Check);
        assert_eq!(constraint_type("CHECK"), ConstraintType::Check);
        assert_eq!(constraint_type("x"), ConstraintType::Exclusion);
        assert_eq!(constraint_type("t"), ConstraintType::Trigger);
        assert_eq!(constraint_type("n"), ConstraintType::NotNull);
        assert_eq!(constraint_type("DEFAULT"), ConstraintType::Default);
        assert_eq!(routine_type("PROCEDURE"), RoutineType::Procedure);
        assert_eq!(routine_type("FUNCTION"), RoutineType::Function);
    }

    #[test]
    fn a_whole_number_binds_as_an_integer() {
        let number = serde_json::json!(-7);
        assert_eq!(
            number_value(number.as_number().unwrap()),
            Some(NumberValue::Integer(-7))
        );
    }

    #[test]
    fn a_number_with_a_fraction_binds_as_a_floating_point_value() {
        let number = serde_json::json!(1.5);
        assert_eq!(
            number_value(number.as_number().unwrap()),
            Some(NumberValue::Float(1.5))
        );
    }

    #[test]
    fn a_whole_number_above_the_range_is_refused() {
        let number = serde_json::json!(18446744073709551615u64);
        let number = number.as_number().unwrap();
        assert_eq!(number_value(number), None);
        let error = number_out_of_range(number);
        assert_eq!(error.category(), crate::error::ErrorCategory::Configuration);
        assert!(error.to_string().contains("18446744073709551615"));
    }

    #[test]
    fn a_parameter_of_a_type_that_cannot_be_sent_is_refused() {
        let error = parameter_type_refused(&serde_json::json!({ "a": 1 }));
        assert_eq!(error.category(), crate::error::ErrorCategory::Configuration);
    }

    #[test]
    fn bytes_become_base64_text() {
        assert_eq!(bytes_to_json(b"hi"), JsonValue::String("aGk=".into()));
        assert_eq!(bytes_to_json(b""), JsonValue::String(String::new()));
    }

    #[test]
    fn a_finite_number_stays_a_number() {
        assert_eq!(f64_to_json(1.5), serde_json::json!(1.5));
    }

    #[test]
    fn a_four_byte_float_gives_the_digits_that_it_shows() {
        assert_eq!(f32_to_json(0.1), serde_json::json!(0.1));
        assert_eq!(f32_to_json(16_777_217.0), serde_json::json!(16_777_216.0));
        assert!(f32_to_json(f32::NAN).is_string());
    }

    #[test]
    fn a_value_that_is_not_a_number_becomes_text() {
        assert_eq!(
            f64_to_json(f64::INFINITY),
            JsonValue::String("Infinity".to_string())
        );
        assert!(f64_to_json(f64::NAN).is_string());
    }

    #[test]
    fn the_row_messages_use_the_correct_number() {
        assert_eq!(rows_affected_message(1).text, "1 row affected.");
        assert_eq!(rows_affected_message(0).text, "0 rows affected.");
        assert_eq!(rows_affected_message(4).text, "4 rows affected.");
        assert_eq!(rows_returned_message(1, false).text, "1 row returned.");
        assert_eq!(rows_returned_message(3, false).level, MessageLevel::Info);
        let stopped = rows_returned_message(2, true);
        assert_eq!(stopped.text, "2 rows returned. Stopped at the row limit.");
        // A read that the limit stopped is a warning, because the answer is
        // not the whole result.
        assert_eq!(stopped.level, MessageLevel::Warning);
    }

    struct BareDriver;

    #[async_trait]
    impl DatabaseDriver for BareDriver {
        fn capabilities(&self) -> DriverCapabilities {
            DriverCapabilities::default()
        }
        fn dialect(&self) -> Dialect {
            Dialect::Sqlite
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

    #[tokio::test]
    async fn a_driver_that_cannot_cancel_has_no_handle() {
        let driver = BareDriver;
        assert!(driver.cancel_handle().is_none());
        assert_eq!(driver.dialect(), Dialect::Sqlite);
        assert!(!driver.capabilities().supports_cancel);
        assert!(!driver.capabilities().supports_transactions);
    }

    #[test]
    fn a_driver_needs_a_check_and_loses_its_connection_by_default() {
        let driver = BareDriver;
        assert!(driver.needs_ping());
        assert!(!driver.keeps_connection_after_stop());
    }

    #[test]
    fn a_driver_that_keeps_no_create_text_gives_no_statement() {
        assert!(BareDriver
            .create_query(None, None, "t", RelationType::Table)
            .is_none());
    }

    #[tokio::test]
    async fn a_driver_without_triggers_and_events_lists_none() {
        let mut driver = BareDriver;
        assert!(driver
            .list_triggers("db", None, "t")
            .await
            .unwrap()
            .is_empty());
        assert!(driver.list_events("db", None).await.unwrap().is_empty());
        for object_type in [ObjectType::Trigger, ObjectType::Event] {
            assert!(driver
                .object_create_query(None, None, Some("t"), "x", object_type)
                .is_none());
        }
    }

    #[test]
    fn the_events_of_a_trigger_fold_into_one_record() {
        let mut triggers = Vec::new();
        for (name, event) in [
            ("audit", Some(TriggerEvent::Insert)),
            ("audit", Some(TriggerEvent::Update)),
            ("audit", Some(TriggerEvent::Update)),
            ("other", None),
        ] {
            add_trigger_event(
                &mut triggers,
                name.into(),
                TriggerTiming::After,
                name == "audit",
                event,
            );
        }
        assert_eq!(
            triggers,
            vec![
                Trigger {
                    name: "audit".into(),
                    timing: TriggerTiming::After,
                    events: vec![TriggerEvent::Insert, TriggerEvent::Update],
                    enabled: true,
                    replica: false,
                    update_columns: Vec::new(),
                },
                Trigger {
                    name: "other".into(),
                    timing: TriggerTiming::After,
                    events: Vec::new(),
                    enabled: false,
                    replica: false,
                    update_columns: Vec::new(),
                },
            ]
        );
    }

    #[test]
    fn the_words_of_the_catalog_name_the_event_and_the_time() {
        assert_eq!(trigger_event(" insert "), Some(TriggerEvent::Insert));
        assert_eq!(trigger_event("UPDATE"), Some(TriggerEvent::Update));
        assert_eq!(trigger_event("Delete"), Some(TriggerEvent::Delete));
        assert_eq!(trigger_event("TRUNCATE"), Some(TriggerEvent::Truncate));
        assert_eq!(trigger_event("CREATE_TABLE"), None);
        assert_eq!(trigger_timing("before"), TriggerTiming::Before);
        assert_eq!(trigger_timing("INSTEAD OF"), TriggerTiming::InsteadOf);
        assert_eq!(trigger_timing("AFTER"), TriggerTiming::After);
        assert_eq!(trigger_timing("FOR"), TriggerTiming::After);
    }

    /// A driver that has `execute_stream` alone, to prove that the default
    /// `execute_query` buffers the streamed rows.
    struct StreamDriver;

    #[async_trait]
    impl DatabaseDriver for StreamDriver {
        fn capabilities(&self) -> DriverCapabilities {
            DriverCapabilities::default()
        }
        fn dialect(&self) -> Dialect {
            Dialect::Sqlite
        }
        async fn ping(&mut self) -> Result<()> {
            Ok(())
        }
        async fn execute_stream(
            &mut self,
            _query: &str,
            _params: Option<&QueryParams>,
            _options: &ExecOptions,
            sink: &mut dyn RowSink,
        ) -> Result<RunSummary> {
            use crate::db::sink::SinkControl;
            use crate::db::ColumnInfo;
            sink.begin_set(vec![ColumnInfo::new("id", "int")])?;
            for value in 0..3 {
                if sink.row(vec![serde_json::json!(value)])? == SinkControl::Stop {
                    break;
                }
            }
            sink.end_set(false)?;
            sink.message(Message::info("done"));
            Ok(RunSummary {
                rows_affected: None,
                elapsed_ms: 7,
                stats: None,
            })
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

    #[tokio::test]
    async fn the_default_execute_query_buffers_the_streamed_rows() {
        let mut driver = StreamDriver;
        let options = ExecOptions {
            max_rows: 2,
            ..ExecOptions::default()
        };
        let response = driver
            .execute_query("SELECT 1", None, &options)
            .await
            .unwrap();
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].rows.len(), 2);
        assert!(response.results[0].truncated);
        assert_eq!(response.messages[0].text, "done");
        assert_eq!(response.elapsed_ms, 7);
    }

    /// A driver that overrides nothing of the execution pair.
    struct NoStreamDriver;

    #[async_trait]
    impl DatabaseDriver for NoStreamDriver {
        fn capabilities(&self) -> DriverCapabilities {
            DriverCapabilities::default()
        }
        fn dialect(&self) -> Dialect {
            Dialect::Sqlite
        }
        async fn ping(&mut self) -> Result<()> {
            Ok(())
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

    #[tokio::test]
    async fn a_driver_without_a_stream_refuses_the_default_execution() {
        let mut driver = NoStreamDriver;
        let error = driver
            .execute_query("SELECT 1", None, &ExecOptions::default())
            .await
            .unwrap_err();
        assert_eq!(error.category(), crate::error::ErrorCategory::Unsupported);
    }
}
