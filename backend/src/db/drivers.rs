//! The interface that every engine implements, and the helpers the
//! implementations share.

pub mod athena;
pub mod mssql;
pub mod mysql;
pub mod postgres;
pub mod sqlite;

use crate::db::{
    AppColumn, Constraint, CreateQuery, Database, DriverCapabilities, ExecOptions, IndexInfo,
    Partition, QueryParams, QueryResponse, Routine, Schema, Table, TableKind,
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
