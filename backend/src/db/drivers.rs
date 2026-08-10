//! The interface that every engine implements, and the helpers the
//! implementations share.

pub mod athena;
pub mod mssql;
pub mod mysql;
pub mod postgres;
pub mod sqlite;

use crate::db::{
    AppColumn, Constraint, ConstraintKind, CreateQuery, Database, DriverCapabilities, ExecOptions,
    IndexInfo, Partition, QueryParams, QueryResponse, Routine, RoutineKind, Schema, SchemaSnapshot,
    SnapshotColumn, SnapshotRelation, Table, TableKind,
};
use crate::error::{Error, Result};
use crate::sql::Dialect;
use async_trait::async_trait;
use base64::Engine as _;
use serde_json::Value as JsonValue;
use std::sync::Arc;

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

    /// Runs a script and returns every result set it produced.
    async fn execute_query(
        &mut self,
        query: &str,
        params: Option<&QueryParams>,
        options: &ExecOptions,
    ) -> Result<QueryResponse>;

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
    ) -> Result<Vec<Partition>> {
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
                    kind: table.kind,
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
        _kind: TableKind,
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
        "The parameter {number} is a whole number outside the range this engine accepts."
    ))
}

/// The message a driver gives for a parameter whose type it cannot bind.
pub fn parameter_type_refused(value: &JsonValue) -> Error {
    Error::Configuration(format!(
        "The parameter {value} has a type this driver cannot send."
    ))
}

/// Renders a slice of bytes as a base64 text value, because JSON holds no
/// binary type.
pub fn bytes_to_json(bytes: &[u8]) -> JsonValue {
    JsonValue::String(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// Builds a JSON number from a floating point value. A value that is not a
/// number, such as infinity, becomes text so that the result stays valid
/// JSON.
pub fn f64_to_json(value: f64) -> JsonValue {
    match serde_json::Number::from_f64(value) {
        Some(number) => JsonValue::Number(number),
        None => JsonValue::String(value.to_string()),
    }
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
    kind: TableKind,
    column: SnapshotColumn,
) -> bool {
    if snapshot.column_count >= max_columns {
        snapshot.complete = false;
        return false;
    }
    let last = snapshot.relations.last_mut();
    let entry = match last {
        Some(entry) if entry.name == relation && entry.schema == schema => entry,
        _ => {
            snapshot.relations.push(SnapshotRelation {
                name: relation,
                schema,
                kind,
                columns: Vec::new(),
            });
            snapshot
                .relations
                .last_mut()
                .expect("the record was just added")
        }
    };
    entry.columns.push(column);
    snapshot.column_count += 1;
    true
}

/// Reads the word of `INFORMATION_SCHEMA` that names the kind of a relation.
pub fn table_kind(word: &str) -> TableKind {
    if word.trim().eq_ignore_ascii_case("VIEW") {
        TableKind::View
    } else {
        TableKind::Table
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
    let entry = match indexes.iter_mut().find(|index| index.name == name) {
        Some(entry) => entry,
        None => {
            indexes.push(IndexInfo {
                name,
                columns: Vec::new(),
                unique,
                primary,
            });
            indexes.last_mut().expect("the record was just added")
        }
    };
    if let Some(column) = column {
        entry.columns.push(column);
    }
}

/// Adds one column to the record of its constraint, and starts a record when
/// the constraint is new. A check constraint covers no column, so a row
/// without a column name still gives a record.
pub fn add_constraint_column(
    constraints: &mut Vec<Constraint>,
    name: String,
    kind: ConstraintKind,
    column: Option<String>,
    detail: Option<String>,
) {
    let entry = match constraints
        .iter_mut()
        .find(|constraint| constraint.name == name)
    {
        Some(entry) => entry,
        None => {
            constraints.push(Constraint {
                name,
                kind,
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

/// Reads the kind of a constraint from the word the engine reports. The
/// engines answer with the words of `INFORMATION_SCHEMA` or with the one
/// letter of PostgreSQL.
pub fn constraint_kind(word: &str) -> ConstraintKind {
    match word.trim().to_uppercase().as_str() {
        "PRIMARY KEY" | "P" => ConstraintKind::PrimaryKey,
        "FOREIGN KEY" | "F" => ConstraintKind::ForeignKey,
        "UNIQUE" | "U" => ConstraintKind::Unique,
        _ => ConstraintKind::Check,
    }
}

/// Reads the kind of a routine from the word the engine reports. A word that
/// is not `PROCEDURE` names a function, because an engine has other kinds of
/// function and no other kind of procedure.
pub fn routine_kind(word: &str) -> RoutineKind {
    if word.trim().eq_ignore_ascii_case("PROCEDURE") {
        RoutineKind::Procedure
    } else {
        RoutineKind::Function
    }
}

/// Adds a message that reports how many rows a statement changed.
pub fn rows_affected_message(count: u64) -> String {
    if count == 1 {
        "1 row affected.".to_string()
    } else {
        format!("{count} rows affected.")
    }
}

/// Adds a message that reports how many rows a statement returned.
pub fn rows_returned_message(count: usize, truncated: bool) -> String {
    let plural = if count == 1 { "row" } else { "rows" };
    if truncated {
        format!("{count} {plural} returned. The row limit stopped the read.")
    } else {
        format!("{count} {plural} returned.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            TableKind::Table,
            column("id"),
        ));
        assert!(add_snapshot_column(
            &mut snapshot,
            10,
            Some("dbo".into()),
            "orders".into(),
            TableKind::Table,
            column("total"),
        ));
        assert!(add_snapshot_column(
            &mut snapshot,
            10,
            Some("staging".into()),
            "orders".into(),
            TableKind::View,
            column("id"),
        ));

        assert_eq!(snapshot.relations.len(), 2);
        assert_eq!(snapshot.relations[0].columns.len(), 2);
        assert_eq!(snapshot.relations[1].schema.as_deref(), Some("staging"));
        assert_eq!(snapshot.relations[1].kind, TableKind::View);
        assert_eq!(snapshot.column_count, 3);
        assert!(snapshot.complete);
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
            TableKind::Table,
            column.clone(),
        ));
        assert!(!add_snapshot_column(
            &mut snapshot,
            1,
            None,
            "orders".into(),
            TableKind::Table,
            column,
        ));
        assert!(!snapshot.complete);
        assert_eq!(snapshot.column_count, 1);
    }

    #[test]
    fn the_word_of_the_catalog_names_a_view() {
        assert_eq!(table_kind("VIEW"), TableKind::View);
        assert_eq!(table_kind("BASE TABLE"), TableKind::Table);
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
    fn the_rows_of_a_constraint_fold_into_one_record() {
        let mut constraints: Vec<Constraint> = Vec::new();
        add_constraint_column(
            &mut constraints,
            "pk_orders".into(),
            ConstraintKind::PrimaryKey,
            Some("id".into()),
            None,
        );
        add_constraint_column(
            &mut constraints,
            "pk_orders".into(),
            ConstraintKind::PrimaryKey,
            Some("region".into()),
            None,
        );
        add_constraint_column(
            &mut constraints,
            "total_positive".into(),
            ConstraintKind::Check,
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
    fn the_word_of_the_engine_names_the_kind() {
        assert_eq!(constraint_kind("PRIMARY KEY"), ConstraintKind::PrimaryKey);
        assert_eq!(constraint_kind("p"), ConstraintKind::PrimaryKey);
        assert_eq!(constraint_kind("FOREIGN KEY"), ConstraintKind::ForeignKey);
        assert_eq!(constraint_kind("f"), ConstraintKind::ForeignKey);
        assert_eq!(constraint_kind("UNIQUE"), ConstraintKind::Unique);
        assert_eq!(constraint_kind("u"), ConstraintKind::Unique);
        assert_eq!(constraint_kind("c"), ConstraintKind::Check);
        assert_eq!(routine_kind("PROCEDURE"), RoutineKind::Procedure);
        assert_eq!(routine_kind("FUNCTION"), RoutineKind::Function);
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
        assert_eq!(error.kind(), crate::error::ErrorKind::Configuration);
        assert!(error.to_string().contains("18446744073709551615"));
    }

    #[test]
    fn a_parameter_of_a_type_that_cannot_be_sent_is_refused() {
        let error = parameter_type_refused(&serde_json::json!({ "a": 1 }));
        assert_eq!(error.kind(), crate::error::ErrorKind::Configuration);
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
    fn a_value_that_is_not_a_number_becomes_text() {
        assert_eq!(
            f64_to_json(f64::INFINITY),
            JsonValue::String("inf".to_string())
        );
        assert!(f64_to_json(f64::NAN).is_string());
    }

    #[test]
    fn the_row_messages_use_the_correct_number() {
        assert_eq!(rows_affected_message(1), "1 row affected.");
        assert_eq!(rows_affected_message(0), "0 rows affected.");
        assert_eq!(rows_affected_message(4), "4 rows affected.");
        assert_eq!(rows_returned_message(1, false), "1 row returned.");
        assert_eq!(rows_returned_message(3, false), "3 rows returned.");
        assert_eq!(
            rows_returned_message(2, true),
            "2 rows returned. The row limit stopped the read."
        );
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
    fn a_driver_that_keeps_no_create_text_gives_no_statement() {
        assert!(BareDriver
            .create_query(None, None, "t", TableKind::Table)
            .is_none());
    }
}
