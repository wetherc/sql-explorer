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
//!
//! The registry also keeps the count of the disk use of the spill files
//! (see `crate::spill`). A new spill that needs room past the cap releases
//! the oldest kept spill first.

use crate::db::sink::RowSink;
use crate::db::ExecOptions;
use crate::error::Result;
use crate::spill::{DiskUse, SpillFolder};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
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
/// its own frees it when the value drops.
pub enum KeptSource {
    /// A finished Athena statement. Athena keeps the full result in S3, and
    /// the read takes its pages again through `GetQueryResults`.
    AthenaExecution(crate::db::drivers::athena::KeptExecution),
    /// A file on the local disk with every row of the set. The read reads
    /// the file, and the drop removes it.
    SpillFile(crate::spill::SpillFile),
    /// A read that paused at the row limit on the session of the tab. Its
    /// statement stays open on the server, and the task of the read keeps
    /// the driver of the session. The export continues the read once, and
    /// the drop of the value releases it. See [`crate::pause`].
    PausedRead(crate::pause::PausedRead),
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
    /// A source that calls a function when it drops, for the tests of the
    /// registry lock.
    #[cfg(test)]
    Probe(#[allow(dead_code, reason = "the probe acts only through its drop")] tests::DropProbe),
}

impl KeptSource {
    /// Reads every row of the kept set into the sink, up to the row limit of
    /// the options. The read stops when the sink answers `Stop`. The caller
    /// keeps the time limit and the Stop button of the read.
    pub async fn read(&self, options: &ExecOptions, sink: &mut dyn RowSink) -> Result<()> {
        match *self {
            KeptSource::AthenaExecution(ref execution) => execution.read(options, sink).await,
            KeptSource::SpillFile(ref file) => file.read(options, sink).await,
            // The export continues a paused read through
            // `PausedRead::continue_into`, because the task of the read must
            // own the sink of the export.
            KeptSource::PausedRead(_) => Err(crate::error::Error::Unsupported(
                "A paused read gives its rows only to an export.".to_string(),
            )),
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
            KeptSource::Empty | KeptSource::Probe(_) => Ok(()),
        }
    }

    /// True for a spill file, which uses the local disk.
    pub fn is_spill(&self) -> bool {
        matches!(self, KeptSource::SpillFile(_))
    }

    /// The place of the full rows, as the window names it. The sources of
    /// the tests stand in for a result outside the application.
    pub fn origin(&self) -> KeptOrigin {
        match self {
            KeptSource::SpillFile(_) => KeptOrigin::Spill,
            KeptSource::PausedRead(_) => KeptOrigin::Paused,
            _ => KeptOrigin::Athena,
        }
    }

    /// The bytes of a spill file. Other sources use no local disk.
    pub fn saved_bytes(&self) -> Option<u64> {
        match self {
            KeptSource::SpillFile(file) => Some(file.bytes()),
            _ => None,
        }
    }

    /// The number of rows that a spill file contains. Other sources do not
    /// know their number of rows.
    pub fn saved_rows(&self) -> Option<u64> {
        match self {
            KeptSource::SpillFile(file) => Some(file.rows()),
            _ => None,
        }
    }
}

impl KeptSource {
    /// True while the source can still give its rows. A paused read that an
    /// export took, or that the end of its pause released, is not live, and
    /// the registry removes it.
    pub fn is_live(&self) -> bool {
        match self {
            KeptSource::PausedRead(read) => read.is_live(),
            _ => true,
        }
    }

    /// The seconds a paused read stays paused, for the window. Every other
    /// source gives `None`.
    pub fn paused_secs(&self) -> Option<u64> {
        match self {
            KeptSource::PausedRead(read) => Some(read.limit().as_secs()),
            _ => None,
        }
    }

    /// True when the source is a paused read on the session.
    fn pauses_on(&self, session: &Arc<crate::session::Session>) -> bool {
        matches!(self, KeptSource::PausedRead(read) if read.uses(session))
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

/// The place of the full rows of a kept set. The export menu and the grid
/// tell the user where the rows come from with this value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum KeptOrigin {
    /// The result files of an Athena statement in S3.
    Athena,
    /// A spill file on the local disk.
    Spill,
    /// A read that paused at the row limit on the server.
    Paused,
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
    pub origin: KeptOrigin,
    /// The moment that the registry got the set, in milliseconds since the
    /// Unix epoch, so the grid can show the age of the kept result.
    pub kept_at: u64,
    /// The number of rows that a spill file of the set contains, so the
    /// grid can tell the user that every row is on this computer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved_rows: Option<u64>,
    /// The bytes of the spill file of the set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved_bytes: Option<u64>,
    /// The seconds the read stays paused, for a set whose read paused at
    /// the row limit. The window shows the time that is left.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paused_secs: Option<u64>,
}

/// The time of the system clock in milliseconds since the Unix epoch. A
/// clock before the epoch gives zero.
pub fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
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
    /// The bytes of every spill file, also the files that a run still
    /// writes.
    disk: DiskUse,
    /// The folder of the spill files of this process. The start of the
    /// application sets it after it removes the files of the processes
    /// that ended, and a run writes no spill file before that.
    spill_folder: OnceLock<SpillFolder>,
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
        let kept_at = unix_millis();
        let mut entries = self.entries();
        let mut removed = Vec::new();
        let mut kept = Vec::with_capacity(sources.len());
        for (set, source) in sources {
            let id = kept_id(request_id, set);
            let set = KeptSet {
                set,
                id: id.clone(),
                origin: source.origin(),
                kept_at,
                saved_rows: source.saved_rows(),
                saved_bytes: source.saved_bytes(),
                paused_secs: source.paused_secs(),
            };
            let mut entry = KeptResult::new(connection_id, source);
            entry.kept_at = now;
            removed.extend(entries.insert(id, Arc::new(entry)));
            kept.push(set);
        }
        removed.extend(prune(&mut entries, now));
        // The drop of a spill file removes the file from the disk, so the
        // removed entries drop after the lock ends.
        drop(entries);
        drop(removed);
        kept
    }

    /// The kept result with the identifier, when the registry still
    /// contains it and it is younger than [`KEPT_RESULT_AGE`].
    pub fn get(&self, id: &str) -> Option<Arc<KeptResult>> {
        self.get_at(id, Instant::now())
    }

    fn get_at(&self, id: &str, now: Instant) -> Option<Arc<KeptResult>> {
        let mut entries = self.entries();
        let removed = prune(&mut entries, now);
        let entry = entries.get(id).cloned();
        drop(entries);
        drop(removed);
        entry
    }

    /// Removes one kept result. Returns true when the registry contained it.
    /// An export that reads the result at this moment keeps its own
    /// reference, so it ends normally.
    pub fn release(&self, id: &str) -> bool {
        // The lock ends at the end of the statement, before the drop of
        // the entry.
        let removed = self.entries().remove(id);
        removed.is_some()
    }

    /// Removes each kept result of one connection, and gives their number.
    pub fn release_connection(&self, connection_id: &str) -> usize {
        let removed: Vec<_> = self
            .entries()
            .extract_if(|_, entry| entry.connection_id == connection_id)
            .collect();
        removed.len()
    }

    /// Removes the oldest kept spill file, so a new spill gets its disk.
    /// Returns false when the registry contains no spill file. The file
    /// goes after the lock of the registry ends.
    pub fn release_oldest_spill(&self) -> bool {
        let oldest = {
            let mut entries = self.entries();
            let id = entries
                .iter()
                .filter(|(_, entry)| entry.source.is_spill())
                .min_by_key(|(_, entry)| entry.kept_at)
                .map(|(id, _)| id.clone());
            id.and_then(|id| entries.remove(&id))
        };
        oldest.is_some()
    }

    /// The count of the disk use of the spill files.
    pub fn disk_use(&self) -> DiskUse {
        self.disk.clone()
    }

    /// Sets the folder of the spill files. A second call changes nothing.
    pub fn set_spill_folder(&self, folder: impl Into<SpillFolder>) {
        let _ = self.spill_folder.set(folder.into());
    }

    /// The folder of the spill files, when the start of the application
    /// prepared it.
    pub fn spill_folder(&self) -> Option<PathBuf> {
        self.spill_folder
            .get()
            .map(|folder| folder.path().to_path_buf())
    }

    /// Removes each paused read on the session, and gives their number. A
    /// command that needs the driver of the session calls this first,
    /// because the task of a paused read keeps that driver. The drop of an
    /// entry releases its read, and the task then frees the driver.
    pub fn release_paused(&self, session: &Arc<crate::session::Session>) -> usize {
        let removed: Vec<_> = self
            .entries()
            .extract_if(|_, entry| entry.source.pauses_on(session))
            .collect();
        removed.len()
    }

    /// The number of kept results.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries().len()
    }
}

/// Removes the entries past [`KEPT_RESULT_AGE`] and the sources that are no
/// longer live, and then the oldest entries past [`MAX_KEPT_RESULTS`]. Gives
/// the removed entries, so the caller can drop them after the lock of the
/// registry ends.
#[must_use]
fn prune(entries: &mut HashMap<String, Arc<KeptResult>>, now: Instant) -> Vec<Arc<KeptResult>> {
    let mut removed: Vec<_> = entries
        .extract_if(|_, entry| {
            now.saturating_duration_since(entry.kept_at) >= KEPT_RESULT_AGE
                || !entry.source.is_live()
        })
        .map(|(_, entry)| entry)
        .collect();
    while entries.len() > MAX_KEPT_RESULTS {
        let oldest = entries
            .iter()
            .min_by_key(|(_, entry)| entry.kept_at)
            .map(|(id, _)| id.clone())
            .expect("the map has entries");
        removed.extend(entries.remove(&oldest));
    }
    removed
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

    /// A function that a source calls when it drops.
    pub(crate) struct DropProbe(Option<Box<dyn FnOnce() + Send + Sync>>);

    impl Drop for DropProbe {
        fn drop(&mut self) {
            if let Some(call) = self.0.take() {
                call();
            }
        }
    }

    /// A source that records, when it drops, whether the lock of the
    /// registry was free.
    fn probe(registry: &Arc<KeptResults>, free: &Arc<Mutex<Vec<bool>>>) -> KeptSource {
        let registry = registry.clone();
        let free = free.clone();
        KeptSource::Probe(DropProbe(Some(Box::new(move || {
            let unlocked = registry.entries.try_lock().is_ok();
            free.lock().unwrap().push(unlocked);
        }))))
    }

    #[tokio::test]
    async fn a_removed_entry_drops_after_the_lock_ends() {
        let registry = Arc::new(KeptResults::default());
        let free = Arc::new(Mutex::new(Vec::new()));
        let start = Instant::now();
        let later = start + KEPT_RESULT_AGE;

        // A release and the release of a connection.
        registry.keep_at("a", "c1", vec![(0, probe(&registry, &free))], start);
        let mut sink = BufferSink::new(10);
        let entry = registry.get_at("a:0", start).unwrap();
        entry
            .source
            .read(&ExecOptions::default(), &mut sink)
            .await
            .unwrap();
        drop(entry);
        assert!(registry.release("a:0"));
        registry.keep_at("b", "c2", vec![(0, probe(&registry, &free))], start);
        assert_eq!(registry.release_connection("c2"), 1);

        // The age limit, in a read and in a new entry.
        registry.keep_at("c", "c1", vec![(0, probe(&registry, &free))], start);
        assert!(registry.get_at("c:0", later).is_none());
        registry.keep_at("d", "c1", vec![(0, probe(&registry, &free))], start);
        registry.keep_at("e", "c1", vec![(0, fixed(1))], later);

        // The count limit, and a second entry with the same identifier.
        registry.keep_at("f", "c1", vec![(0, probe(&registry, &free))], later);
        registry.keep_at("f", "c1", vec![(0, fixed(1))], later);
        registry.keep_at("g", "c1", vec![(0, probe(&registry, &free))], later);
        for n in 0..MAX_KEPT_RESULTS {
            let at = later + Duration::from_secs(1 + n as u64);
            registry.keep_at(&format!("n{n}"), "c1", vec![(0, fixed(1))], at);
        }
        assert!(registry.get("g:0").is_none());

        assert_eq!(*free.lock().unwrap(), vec![true; 6]);
    }

    #[test]
    fn each_cut_set_gets_an_identifier_of_the_run_and_the_set() {
        let registry = KeptResults::default();
        let before = unix_millis();
        let kept = registry.keep("r1", "c1", vec![(0, fixed(1)), (2, fixed(1))]);
        let after = unix_millis();
        let ids: Vec<_> = kept.iter().map(|set| (set.set, set.id.as_str())).collect();
        assert_eq!(ids, vec![(0, "r1:0"), (2, "r1:2")]);
        assert!(kept
            .iter()
            .all(|set| set.kept_at >= before && set.kept_at <= after));
        assert_eq!(registry.len(), 2);
        assert_eq!(registry.get("r1:2").unwrap().connection_id, "c1");
        assert!(registry.get("r1:1").is_none());
        assert_eq!(
            serde_json::to_value(&kept[0]).unwrap(),
            json!({ "set": 0, "id": "r1:0", "origin": "athena", "keptAt": kept[0].kept_at })
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
    async fn a_spill_file_gives_its_rows_and_its_count() {
        let folder = tempfile::tempdir().unwrap();
        let registry = KeptResults::default();
        let file = crate::spill::tests::spill(folder.path(), &registry.disk_use(), 3);
        let bytes = file.bytes();
        assert_eq!(registry.disk_use().bytes(), bytes);
        let kept = registry.keep("r1", "c1", vec![(0, KeptSource::SpillFile(file))]);
        assert_eq!(kept[0].saved_rows, Some(3));
        assert_eq!(
            serde_json::to_value(&kept[0]).unwrap(),
            json!({
                "set": 0,
                "id": "r1:0",
                "origin": "spill",
                "keptAt": kept[0].kept_at,
                "savedRows": 3,
                "savedBytes": bytes,
            })
        );
        let entry = registry.get("r1:0").unwrap();
        assert!(entry.source.is_spill());
        assert!(!fixed(1).is_spill());
        assert_eq!(fixed(1).saved_rows(), None);
        assert_eq!(fixed(1).saved_bytes(), None);

        let mut sink = BufferSink::new(10);
        entry
            .source
            .read(&ExecOptions::default(), &mut sink)
            .await
            .unwrap();
        let set = sink.into_response(RunSummary::default()).results.remove(0);
        assert_eq!(set.rows.len(), 3);

        // The release removes the file when the last reader lets it go.
        assert!(registry.release("r1:0"));
        assert_eq!(registry.disk_use().bytes(), bytes);
        drop(entry);
        assert_eq!(registry.disk_use().bytes(), 0);
        assert!(crate::spill::tests::wait_for_empty(folder.path()));
    }

    #[test]
    fn the_oldest_spill_goes_first_and_other_sources_stay() {
        let folder = tempfile::tempdir().unwrap();
        let registry = KeptResults::default();
        let disk = registry.disk_use();
        let start = Instant::now();
        let spill =
            |rows| KeptSource::SpillFile(crate::spill::tests::spill(folder.path(), &disk, rows));
        registry.keep_at("athena", "c1", vec![(0, fixed(1))], start);
        registry.keep_at(
            "old",
            "c1",
            vec![(0, spill(1))],
            start + Duration::from_secs(1),
        );
        registry.keep_at(
            "new",
            "c1",
            vec![(0, spill(2))],
            start + Duration::from_secs(2),
        );
        assert!(registry.release_oldest_spill());
        assert!(registry.get("old:0").is_none());
        assert!(registry.get("new:0").is_some());
        assert!(registry.release_oldest_spill());
        assert!(!registry.release_oldest_spill());
        assert!(registry.get("athena:0").is_some());
        assert_eq!(disk.bytes(), 0);
    }

    #[test]
    fn the_folder_of_the_spill_files_is_set_once() {
        let registry = KeptResults::default();
        assert_eq!(registry.spill_folder(), None);
        registry.set_spill_folder(PathBuf::from("/first"));
        registry.set_spill_folder(PathBuf::from("/second"));
        assert_eq!(registry.spill_folder(), Some(PathBuf::from("/first")));
    }

    #[tokio::test]
    async fn a_paused_read_names_its_limit_and_ends_with_its_session() {
        use crate::pause::tests::{paused_chunk_read, RowsDriver};
        let read = paused_chunk_read(RowsDriver::new(5), 2).await;
        let session = read.slot().session.clone();
        let registry = KeptResults::default();
        let kept = registry.keep(
            "r1",
            "c1",
            vec![(0, KeptSource::PausedRead(read)), (1, fixed(1))],
        );
        assert_eq!(kept[0].paused_secs, Some(60));
        assert_eq!(kept[1].paused_secs, None);
        assert_eq!(kept[0].origin, KeptOrigin::Paused);
        let mut value = serde_json::to_value(&kept[0]).unwrap();
        value.as_object_mut().unwrap().remove("keptAt");
        assert_eq!(
            value,
            json!({ "set": 0, "id": "r1:0", "origin": "paused", "pausedSecs": 60 })
        );

        // A paused read gives its rows to an export alone.
        let entry = registry.get("r1:0").unwrap();
        let mut sink = BufferSink::new(10);
        let error = entry
            .source
            .read(&ExecOptions::default(), &mut sink)
            .await
            .unwrap_err();
        assert!(matches!(error, crate::error::Error::Unsupported(_)));
        drop(entry);

        // A release of another session leaves the read.
        let other = Arc::new(crate::session::Session::new(Box::new(RowsDriver::new(0))));
        assert_eq!(registry.release_paused(&other), 0);
        assert_eq!(registry.release_paused(&session), 1);
        assert_eq!(registry.len(), 1);
        // The release ends the statement, and the driver is free again.
        drop(session.driver.lock().await);
    }

    #[tokio::test]
    async fn a_paused_read_that_an_export_took_leaves_the_registry() {
        use crate::pause::tests::{paused_chunk_read, RowsDriver};
        let registry = KeptResults::default();
        let live = paused_chunk_read(RowsDriver::new(5), 2).await;
        registry.keep("r1", "c1", vec![(0, KeptSource::PausedRead(live))]);
        let read = paused_chunk_read(RowsDriver::new(5), 2).await;
        read.continue_into(Box::new(BufferSink::new(10)), 10)
            .await
            .unwrap();
        // The next change of the registry removes the read that is gone.
        registry.keep("r2", "c1", vec![(0, KeptSource::PausedRead(read))]);
        assert!(registry.get("r1:0").is_some());
        assert!(registry.get("r2:0").is_none());
        assert_eq!(registry.len(), 1);
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
