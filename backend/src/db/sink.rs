//! The sink that receives the rows of one execution as the driver reads
//! them. A driver that streams into a sink holds one row at a time, and the
//! caller decides what the rows become: a buffered response or a file.

use crate::db::{ColumnInfo, Message, QueryResponse, QueryStats, ResultSet};
use crate::error::{Error, Result};
use serde_json::Value as JsonValue;

/// The answer a sink gives for one row. `Stop` tells the driver to end the
/// read, for example because a row limit is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkControl {
    Continue,
    Stop,
}

/// The numbers of one execution that travel beside the rows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunSummary {
    pub rows_affected: Option<u64>,
    pub elapsed_ms: u64,
    pub stats: Option<QueryStats>,
}

/// Where the rows of one execution go. The driver calls the methods in the
/// order of the run: `begin_set`, then the rows of the set, then `end_set`,
/// for each set. A message can arrive at any point of the run.
pub trait RowSink: Send {
    /// Starts one result set. Called once for each set of the run.
    fn begin_set(&mut self, columns: Vec<ColumnInfo>) -> Result<()>;

    /// Receives one row. The answer tells the driver to go on or to stop
    /// the read.
    fn row(&mut self, row: Vec<JsonValue>) -> Result<SinkControl>;

    /// Ends one result set. The flag reports a read that the driver itself
    /// stopped at a limit.
    fn end_set(&mut self, truncated: bool) -> Result<()>;

    /// Receives one message of the server.
    fn message(&mut self, message: Message);
}

/// A sink that keeps the rows in memory and builds a `QueryResponse`. It
/// keeps the rows of each set up to `max_rows`. A row past that bound is not
/// kept: the sink marks the set as truncated and answers `Stop`.
#[derive(Debug)]
pub struct BufferSink {
    max_rows: usize,
    results: Vec<ResultSet>,
    messages: Vec<Message>,
}

impl BufferSink {
    pub fn new(max_rows: usize) -> Self {
        Self {
            max_rows,
            results: Vec::new(),
            messages: Vec::new(),
        }
    }

    /// Builds the response of the run from the buffered sets and the numbers
    /// of the summary.
    pub fn into_response(self, summary: RunSummary) -> QueryResponse {
        QueryResponse {
            results: self.results,
            messages: self.messages,
            rows_affected: summary.rows_affected,
            elapsed_ms: summary.elapsed_ms,
            stats: summary.stats,
        }
    }

    /// The set that `begin_set` opened. A row or an end without an open set
    /// is an error of the driver, not of the user.
    fn open_set(&mut self) -> Result<&mut ResultSet> {
        self.results.last_mut().ok_or_else(|| {
            Error::Anyhow(anyhow::anyhow!(
                "The driver sent a row before it began a result set."
            ))
        })
    }
}

impl RowSink for BufferSink {
    fn begin_set(&mut self, columns: Vec<ColumnInfo>) -> Result<()> {
        self.results.push(ResultSet::new(columns));
        Ok(())
    }

    fn row(&mut self, row: Vec<JsonValue>) -> Result<SinkControl> {
        let max_rows = self.max_rows;
        let set = self.open_set()?;
        if set.rows.len() >= max_rows {
            set.truncated = true;
            return Ok(SinkControl::Stop);
        }
        set.rows.push(row);
        Ok(SinkControl::Continue)
    }

    fn end_set(&mut self, truncated: bool) -> Result<()> {
        let set = self.open_set()?;
        set.truncated = set.truncated || truncated;
        Ok(())
    }

    fn message(&mut self, message: Message) {
        self.messages.push(message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::MessageLevel;
    use crate::error::ErrorKind;

    fn columns() -> Vec<ColumnInfo> {
        vec![ColumnInfo::new("id", "int")]
    }

    fn row(value: i64) -> Vec<JsonValue> {
        vec![serde_json::json!(value)]
    }

    #[test]
    fn a_run_with_two_sets_keeps_each_set_apart() {
        let mut sink = BufferSink::new(10);
        sink.begin_set(columns()).unwrap();
        assert_eq!(sink.row(row(1)).unwrap(), SinkControl::Continue);
        sink.end_set(false).unwrap();
        sink.begin_set(vec![ColumnInfo::new("name", "text")])
            .unwrap();
        assert_eq!(
            sink.row(vec![serde_json::json!("a")]).unwrap(),
            SinkControl::Continue
        );
        assert_eq!(
            sink.row(vec![serde_json::json!("b")]).unwrap(),
            SinkControl::Continue
        );
        sink.end_set(false).unwrap();

        let response = sink.into_response(RunSummary::default());
        assert_eq!(response.results.len(), 2);
        assert_eq!(response.results[0].rows, vec![row(1)]);
        assert_eq!(response.results[0].columns, columns());
        assert_eq!(response.results[1].rows.len(), 2);
        assert!(!response.results[0].truncated);
        assert!(!response.results[1].truncated);
    }

    #[test]
    fn a_row_past_the_bound_stops_the_read_and_marks_the_set() {
        let mut sink = BufferSink::new(2);
        sink.begin_set(columns()).unwrap();
        assert_eq!(sink.row(row(1)).unwrap(), SinkControl::Continue);
        assert_eq!(sink.row(row(2)).unwrap(), SinkControl::Continue);
        assert_eq!(sink.row(row(3)).unwrap(), SinkControl::Stop);
        sink.end_set(false).unwrap();

        let response = sink.into_response(RunSummary::default());
        assert_eq!(response.results[0].rows, vec![row(1), row(2)]);
        assert!(response.results[0].truncated);
    }

    #[test]
    fn a_run_that_holds_the_bound_exactly_is_not_truncated() {
        let mut sink = BufferSink::new(2);
        sink.begin_set(columns()).unwrap();
        sink.row(row(1)).unwrap();
        sink.row(row(2)).unwrap();
        sink.end_set(false).unwrap();

        let response = sink.into_response(RunSummary::default());
        assert_eq!(response.results[0].rows.len(), 2);
        assert!(!response.results[0].truncated);
    }

    #[test]
    fn the_driver_can_mark_a_set_as_truncated_itself() {
        let mut sink = BufferSink::new(10);
        sink.begin_set(columns()).unwrap();
        sink.row(row(1)).unwrap();
        sink.end_set(true).unwrap();

        let response = sink.into_response(RunSummary::default());
        assert!(response.results[0].truncated);
    }

    #[test]
    fn the_messages_keep_their_arrival_order() {
        let mut sink = BufferSink::new(10);
        sink.message(Message::warning("first"));
        sink.begin_set(columns()).unwrap();
        sink.row(row(1)).unwrap();
        sink.message(Message::info("second"));
        sink.end_set(false).unwrap();
        sink.message(Message::info("third"));

        let response = sink.into_response(RunSummary::default());
        let texts: Vec<&str> = response.messages.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(texts, vec!["first", "second", "third"]);
        assert_eq!(response.messages[0].level, MessageLevel::Warning);
    }

    #[test]
    fn an_empty_run_gives_an_empty_response() {
        let sink = BufferSink::new(10);
        let response = sink.into_response(RunSummary::default());
        assert!(response.results.is_empty());
        assert!(response.messages.is_empty());
        assert_eq!(response.rows_affected, None);
        assert_eq!(response.elapsed_ms, 0);
        assert_eq!(response.stats, None);
    }

    #[test]
    fn a_row_or_an_end_without_a_set_is_an_internal_error() {
        let mut sink = BufferSink::new(10);
        let error = sink.row(row(1)).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Internal);
        let error = sink.end_set(false).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Internal);
    }

    #[test]
    fn the_response_carries_the_numbers_of_the_summary() {
        let mut sink = BufferSink::new(10);
        sink.begin_set(columns()).unwrap();
        sink.row(row(1)).unwrap();
        sink.end_set(false).unwrap();

        let stats = QueryStats {
            scanned_bytes: Some(2048),
            ..QueryStats::default()
        };
        let response = sink.into_response(RunSummary {
            rows_affected: Some(3),
            elapsed_ms: 42,
            stats: Some(stats.clone()),
        });
        assert_eq!(response.rows_affected, Some(3));
        assert_eq!(response.elapsed_ms, 42);
        assert_eq!(response.stats, Some(stats));
    }

    #[test]
    fn a_bound_of_zero_keeps_no_row() {
        let mut sink = BufferSink::new(0);
        sink.begin_set(columns()).unwrap();
        assert_eq!(sink.row(row(1)).unwrap(), SinkControl::Stop);
        let response = sink.into_response(RunSummary::default());
        assert!(response.results[0].rows.is_empty());
        assert!(response.results[0].truncated);
    }

    // The sink crosses `await` points in the drivers, so it must be `Send`.
    #[test]
    fn the_buffer_sink_can_move_between_threads() {
        fn assert_send<T: Send>() {}
        assert_send::<BufferSink>();
        assert_send::<Box<dyn RowSink>>();
    }
}
