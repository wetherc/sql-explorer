//! The kept results: result sets of a run that can give their full rows
//! again without a second run of the statement.
//!
//! A run stops its read at the row limit of the grid. For some sources, the
//! rows past the limit stay available after the run. An export of all rows
//! then reads them from the kept result, and an expensive statement does not
//! run a second time.
//!
//! The driver offers a source to the sink of the run when it cuts a set at
//! the row limit (`RowSink::keep_source`). The command of the run puts each
//! source of a cut set in the registry of the state, and the frame at the end
//! of the run gives the window the identifier of each kept set. The window
//! sends the identifier back to export the rows, and releases it when the
//! result leaves the interface.
//!
//! The registry has a bound on the number of entries and on their age, so an
//! entry that the window never releases goes at the latest at that age. A
//! disconnect of the connection releases each entry of the connection.

use crate::db::sink::RowSink;
use crate::db::ExecOptions;
use crate::error::Result;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// The most kept results the registry contains. A new entry past this
/// number removes the oldest entry.
pub const MAX_KEPT_RESULTS: usize = 64;

/// The age after which the registry removes a kept result. A source outside
/// the application, such as the result files of Athena in S3, can go before
/// this age, and the read then fails with an error that tells the user to run
/// the statement again.
pub const KEPT_RESULT_AGE: Duration = Duration::from_secs(12 * 60 * 60);

/// The place that gives the full rows of one kept result set again.
///
/// Each variant reads its rows into a sink, in the order and with the
/// columns of the set that the grid shows. A variant that owns a resource of
/// its own frees it when the value drops. Two more sources fit this type:
///
/// - A paused read on the session of the tab, such as a server cursor that
///   stopped at the row limit. Its read continues the cursor on that session,
///   so the read takes the driver of the session. A new run on the session
///   must release it first, because the run closes or replaces the cursor.
/// - A spill file, where the sink of the run writes the rows past the limit
///   to a local file. Its read reads the file, and its drop removes the file.
pub enum KeptSource {
    /// A finished Athena statement. Athena keeps the full result in S3, and
    /// the read takes its pages again through `GetQueryResults`.
    AthenaExecution(crate::db::drivers::athena::KeptExecution),
    /// A fixed list of rows, for the tests of the registry and the export.
    #[cfg(test)]
    Fixed {
        columns: Vec<crate::db::ColumnInfo>,
        rows: Vec<Vec<serde_json::Value>>,
    },
    /// A read that never ends, for the tests of the stop and the time limit.
    #[cfg(test)]
    Pending,
    /// A read that gives no set, for the tests of the export.
    #[cfg(test)]
    Empty,
}

impl KeptSource {
    /// Reads every row of the kept set into the sink, up to the row limit of
    /// the options. The read stops when the sink answers `Stop`. The caller
    /// keeps the time limit and the Stop button of the read.
    pub async fn read(&self, options: &ExecOptions, sink: &mut dyn RowSink) -> Result<()> {
        match *self {
            KeptSource::AthenaExecution(ref execution) => execution.read(options, sink).await,
            #[cfg(test)]
            KeptSource::Fixed {
                ref columns,
                ref rows,
            } => {
                sink.begin_set(columns.clone())?;
                let mut truncated = false;
                for (count, row) in rows.iter().enumerate() {
                    if count >= options.max_rows
                        || sink.row(row.clone())? == crate::db::sink::SinkControl::Stop
                    {
                        truncated = true;
                        break;
                    }
                }
                sink.end_set(truncated)
            }
            #[cfg(test)]
            KeptSource::Pending => std::future::pending().await,
            #[cfg(test)]
            KeptSource::Empty => Ok(()),
        }
    }
}

/// One entry of the registry.
pub struct KeptResult {
    /// The connection of the run. A disconnect releases the entry, and an
    /// export needs the connection to be open.
    pub connection_id: String,
    pub source: KeptSource,
    kept_at: Instant,
}

impl KeptResult {
    pub fn new(connection_id: &str, source: KeptSource) -> Self {
        Self {
            connection_id: connection_id.to_string(),
            source,
            kept_at: Instant::now(),
        }
    }
}

/// The identifier of one kept set, as the frame at the end of the run gives
/// it to the window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeptSet {
    /// The number of the set in the run, from zero, as the frames name it.
    pub set: u32,
    /// The identifier that `export_kept` and `release_kept` take.
    pub id: String,
}

/// The identifier of one kept set: the identifier of the request of the run
/// and the number of the set. The window makes a new request identifier for
/// each run, so two runs never give the same identifier.
pub fn kept_id(request_id: &str, set: u32) -> String {
    format!("{request_id}:{set}")
}

/// The registry of the kept results of every run.
#[derive(Default)]
pub struct KeptResults {
    entries: Mutex<HashMap<String, Arc<KeptResult>>>,
}

impl KeptResults {
    fn entries(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<KeptResult>>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Adds the sources of the cut sets of one run, and gives the identifier
    /// of each one for the frame at the end of the run.
    pub fn keep(
        &self,
        request_id: &str,
        connection_id: &str,
        sources: Vec<(u32, KeptSource)>,
    ) -> Vec<KeptSet> {
        self.keep_at(request_id, connection_id, sources, Instant::now())
    }

    fn keep_at(
        &self,
        request_id: &str,
        connection_id: &str,
        sources: Vec<(u32, KeptSource)>,
        now: Instant,
    ) -> Vec<KeptSet> {
        if sources.is_empty() {
            return Vec::new();
        }
        let mut entries = self.entries();
        let mut kept = Vec::with_capacity(sources.len());
        for (set, source) in sources {
            let id = kept_id(request_id, set);
            let mut entry = KeptResult::new(connection_id, source);
            entry.kept_at = now;
            entries.insert(id.clone(), Arc::new(entry));
            kept.push(KeptSet { set, id });
        }
        prune(&mut entries, now);
        kept
    }

    /// The kept result with the identifier, when the registry still
    /// contains it and it is younger than [`KEPT_RESULT_AGE`].
    pub fn get(&self, id: &str) -> Option<Arc<KeptResult>> {
        self.get_at(id, Instant::now())
    }

    fn get_at(&self, id: &str, now: Instant) -> Option<Arc<KeptResult>> {
        let mut entries = self.entries();
        prune(&mut entries, now);
        entries.get(id).cloned()
    }

    /// Removes one kept result. Returns true when the registry contained it.
    /// An export that reads the result at this moment keeps its own
    /// reference, so it ends normally.
    pub fn release(&self, id: &str) -> bool {
        self.entries().remove(id).is_some()
    }

    /// Removes each kept result of one connection, and gives their number.
    pub fn release_connection(&self, connection_id: &str) -> usize {
        let mut entries = self.entries();
        let before = entries.len();
        entries.retain(|_, entry| entry.connection_id != connection_id);
        before - entries.len()
    }

    /// The number of kept results.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries().len()
    }
}

/// Removes the entries past [`KEPT_RESULT_AGE`], and then the oldest entries
/// past [`MAX_KEPT_RESULTS`].
fn prune(entries: &mut HashMap<String, Arc<KeptResult>>, now: Instant) {
    entries.retain(|_, entry| now.saturating_duration_since(entry.kept_at) < KEPT_RESULT_AGE);
    while entries.len() > MAX_KEPT_RESULTS {
        let oldest = entries
            .iter()
            .min_by_key(|(_, entry)| entry.kept_at)
            .map(|(id, _)| id.clone())
            .expect("the map has entries");
        entries.remove(&oldest);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::db::sink::{BufferSink, RunSummary};
    use crate::db::ColumnInfo;
    use serde_json::json;

    /// A source of `rows` rows in one column `n`.
    pub(crate) fn fixed(rows: usize) -> KeptSource {
        KeptSource::Fixed {
            columns: vec![ColumnInfo::new("n", "int")],
            rows: (0..rows).map(|row| vec![json!(row)]).collect(),
        }
    }

    #[test]
    fn each_cut_set_gets_an_identifier_of_the_run_and_the_set() {
        let registry = KeptResults::default();
        let kept = registry.keep("r1", "c1", vec![(0, fixed(1)), (2, fixed(1))]);
        assert_eq!(
            kept,
            vec![
                KeptSet {
                    set: 0,
                    id: "r1:0".into()
                },
                KeptSet {
                    set: 2,
                    id: "r1:2".into()
                },
            ]
        );
        assert_eq!(registry.len(), 2);
        assert_eq!(registry.get("r1:2").unwrap().connection_id, "c1");
        assert!(registry.get("r1:1").is_none());
        assert_eq!(
            serde_json::to_value(&kept[0]).unwrap(),
            json!({ "set": 0, "id": "r1:0" })
        );
    }

    #[test]
    fn a_run_without_a_cut_set_keeps_nothing() {
        let registry = KeptResults::default();
        assert!(registry.keep("r1", "c1", Vec::new()).is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn a_release_removes_one_result() {
        let registry = KeptResults::default();
        registry.keep("r1", "c1", vec![(0, fixed(1)), (1, fixed(1))]);
        let reading = registry.get("r1:0").unwrap();
        assert!(registry.release("r1:0"));
        assert!(!registry.release("r1:0"));
        assert!(registry.get("r1:0").is_none());
        assert!(registry.get("r1:1").is_some());
        // A read that took the result before the release still has it.
        assert_eq!(reading.connection_id, "c1");
    }

    #[test]
    fn a_disconnect_releases_each_result_of_the_connection() {
        let registry = KeptResults::default();
        registry.keep("r1", "c1", vec![(0, fixed(1)), (1, fixed(1))]);
        registry.keep("r2", "c2", vec![(0, fixed(1))]);
        assert_eq!(registry.release_connection("c1"), 2);
        assert_eq!(registry.release_connection("c1"), 0);
        assert!(registry.get("r2:0").is_some());
    }

    #[test]
    fn a_result_past_its_age_is_gone() {
        let registry = KeptResults::default();
        let start = Instant::now();
        registry.keep_at("r1", "c1", vec![(0, fixed(1))], start);
        let before = start + KEPT_RESULT_AGE - Duration::from_secs(1);
        assert!(registry.get_at("r1:0", before).is_some());
        assert!(registry.get_at("r1:0", start + KEPT_RESULT_AGE).is_none());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn the_oldest_results_go_past_the_bound() {
        let registry = KeptResults::default();
        let start = Instant::now();
        for run in 0..=MAX_KEPT_RESULTS {
            registry.keep_at(
                &format!("r{run}"),
                "c1",
                vec![(0, fixed(1))],
                start + Duration::from_millis(run as u64),
            );
        }
        assert_eq!(registry.len(), MAX_KEPT_RESULTS);
        assert!(registry.get("r0:0").is_none());
        assert!(registry.get(&format!("r{MAX_KEPT_RESULTS}:0")).is_some());
    }

    #[tokio::test]
    async fn a_fixed_source_reads_its_rows_up_to_the_limit() {
        let options = ExecOptions {
            max_rows: 2,
            ..ExecOptions::default()
        };
        let mut sink = BufferSink::new(10);
        fixed(3).read(&options, &mut sink).await.unwrap();
        let set = sink.into_response(RunSummary::default()).results.remove(0);
        assert_eq!(set.rows.len(), 2);
        assert!(set.truncated);

        // A sink that stops the read cuts the set as well.
        let mut sink = BufferSink::new(1);
        fixed(2)
            .read(&ExecOptions::default(), &mut sink)
            .await
            .unwrap();
        let set = sink.into_response(RunSummary::default()).results.remove(0);
        assert_eq!(set.rows.len(), 1);
        assert!(set.truncated);
    }

    #[tokio::test]
    async fn a_pending_source_never_ends() {
        let mut sink = BufferSink::new(10);
        let options = ExecOptions::default();
        let read = KeptSource::Pending.read(&options, &mut sink);
        assert!(tokio::time::timeout(Duration::from_millis(20), read)
            .await
            .is_err());
    }
}
