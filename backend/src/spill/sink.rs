//! The sink of a run that keeps its full result sets in spill files.

use super::{SpillEnd, SpillWriter};
use crate::db::sink::{RowSink, SinkControl};
use crate::db::{ColumnInfo, Message};
use crate::error::Result;
use crate::kept::{KeptResults, KeptSource};
use std::path::PathBuf;

/// The message for a result set whose spill passed the cap of the disk use.
pub const FULL_MESSAGE: &str = "This result needs more disk space than Settings allows for saved \
     results, so it wasn't saved and the read stopped at the row limit. Export all rows will run \
     the query again.";

/// The message for a result set that the export row limit cut.
pub const EXPORT_LIMIT_MESSAGE: &str = "This result has more rows than the export row limit, so \
     it wasn't saved. Export all rows will run the query again.";

/// The message for a spill that the disk refused.
fn failed_message(error: &crate::error::Error) -> String {
    format!(
        "Couldn't save this result on this computer, so the read stopped at the row limit. \
         Export all rows will run the query again. {error}"
    )
}

/// A sink that gives the first rows of each set to the grid, and every row
/// of each set to a spill file.
///
/// The driver reads each set up to the export row limit. The rows up to the
/// row limit of the grid go to the grid, and the sink drops the rows past
/// it after the spill writer has them. A set that passes the grid limit
/// offers its spill file to the grid as a kept source when it ends, and the
/// grid gives the source to the registry with its cut set.
///
/// A spill that stops early takes the place of the grid limit again: the
/// sink answers `Stop` for the first row past the grid limit, and the read
/// ends there. A spill stops early when the spill files pass the cap of
/// their disk use, or when the disk refuses a write. After the cap stops one
/// spill, the later sets of the run do not spill. A set that the export row
/// limit cut keeps no file, because its file misses rows. A message of each
/// such case goes to the grid.
pub struct SpillSink<'k, G: RowSink> {
    grid: G,
    grid_rows: usize,
    folder: PathBuf,
    /// The cap of the disk use of every spill file, in bytes.
    cap: u64,
    kept: &'k KeptResults,
    /// The spill of the open set, while it goes on.
    writer: Option<SpillWriter>,
    /// True once the cap stopped a spill.
    full: bool,
    /// The rows of the open set that went to the grid.
    shown: usize,
    /// True when the open set had more rows than the grid takes.
    cut: bool,
}

impl<'k, G: RowSink> SpillSink<'k, G> {
    pub fn new(
        grid: G,
        grid_rows: usize,
        folder: PathBuf,
        cap: u64,
        kept: &'k KeptResults,
    ) -> Self {
        Self {
            grid,
            grid_rows,
            folder,
            cap,
            kept,
            writer: None,
            full: false,
            shown: 0,
            cut: false,
        }
    }

    /// Gives the grid back at the end of the run. A spill that did not end
    /// stops, and its file goes.
    pub fn into_grid(self) -> G {
        self.grid
    }

    /// Stops the spill of the open set, and tells the user why.
    fn give_up(&mut self, end: SpillEnd) {
        self.writer = None;
        let text = match end {
            SpillEnd::Full => {
                self.full = true;
                FULL_MESSAGE.to_string()
            }
            SpillEnd::Failed(error) => {
                log::warn!("A spill file failed: {error}");
                failed_message(&error)
            }
        };
        self.grid.message(Message::warning(text));
    }
}

impl<G: RowSink> RowSink for SpillSink<'_, G> {
    fn begin_set(&mut self, columns: Vec<ColumnInfo>) -> Result<()> {
        self.shown = 0;
        self.cut = false;
        self.writer = (!self.full).then(|| {
            SpillWriter::new(
                self.folder.clone(),
                columns.clone(),
                self.kept.disk_use(),
                self.cap,
            )
        });
        self.grid.begin_set(columns)
    }

    fn row(&mut self, row: Vec<serde_json::Value>) -> Result<SinkControl> {
        if let Some(writer) = self.writer.as_mut() {
            let kept = self.kept;
            if let Err(end) = writer.write(&row, &mut || kept.release_oldest_spill()) {
                self.give_up(end);
            }
        }
        if self.shown < self.grid_rows {
            self.shown += 1;
            return self.grid.row(row);
        }
        self.cut = true;
        Ok(match self.writer {
            Some(_) => SinkControl::Continue,
            None => SinkControl::Stop,
        })
    }

    fn end_set(&mut self, truncated: bool) -> Result<()> {
        if let Some(writer) = self.writer.take().filter(|_| self.cut) {
            if truncated {
                self.grid
                    .message(Message::warning(EXPORT_LIMIT_MESSAGE.to_string()));
            } else {
                let kept = self.kept;
                match writer.finish(&mut || kept.release_oldest_spill()) {
                    Ok(file) => {
                        let (rows, bytes) = (file.rows(), file.bytes());
                        let total = kept.disk_use().bytes();
                        log::info!(
                            "Saved {rows} rows in a spill file of {bytes} bytes. The spill files \
                             use {total} bytes."
                        );
                        self.grid.keep_source(KeptSource::SpillFile(file));
                    }
                    Err(end) => self.give_up(end),
                }
            }
        }
        self.grid.end_set(truncated || self.cut)
    }

    fn message(&mut self, message: Message) {
        self.grid.message(message);
    }

    fn keep_source(&mut self, source: KeptSource) {
        self.grid.keep_source(source);
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{spill, wait_for_empty};
    use super::super::SPILL_BATCH;
    use super::*;
    use crate::db::sink::{BufferSink, RunSummary};
    use crate::db::QueryResponse;
    use serde_json::json;

    /// A grid that keeps its rows and the sources that the sink offers it.
    struct Grid {
        rows: BufferSink,
        sources: Vec<KeptSource>,
    }

    impl Grid {
        fn new(max_rows: usize) -> Self {
            Self {
                rows: BufferSink::new(max_rows),
                sources: Vec::new(),
            }
        }

        fn response(self) -> (QueryResponse, Vec<KeptSource>) {
            (self.rows.into_response(RunSummary::default()), self.sources)
        }
    }

    impl RowSink for Grid {
        fn begin_set(&mut self, columns: Vec<ColumnInfo>) -> Result<()> {
            self.rows.begin_set(columns)
        }
        fn row(&mut self, row: Vec<serde_json::Value>) -> Result<SinkControl> {
            self.rows.row(row)
        }
        fn end_set(&mut self, truncated: bool) -> Result<()> {
            self.rows.end_set(truncated)
        }
        fn message(&mut self, message: Message) {
            self.rows.message(message);
        }
        fn keep_source(&mut self, source: KeptSource) {
            self.sources.push(source);
        }
    }

    fn columns() -> Vec<ColumnInfo> {
        vec![ColumnInfo::new("n", "int")]
    }

    fn row(value: usize) -> Vec<serde_json::Value> {
        vec![json!(value)]
    }

    /// Sends one set of `rows` rows, and gives the answer to the last row.
    fn send_set<G: RowSink>(
        sink: &mut SpillSink<'_, G>,
        rows: usize,
        truncated: bool,
    ) -> SinkControl {
        sink.begin_set(columns()).unwrap();
        let mut last = SinkControl::Continue;
        for value in 0..rows {
            last = sink.row(row(value)).unwrap();
        }
        sink.end_set(truncated).unwrap();
        last
    }

    fn texts(response: &QueryResponse) -> Vec<&str> {
        response.messages.iter().map(|m| m.text.as_str()).collect()
    }

    #[tokio::test]
    async fn a_set_past_the_grid_limit_keeps_every_row_in_a_spill_file() {
        let folder = tempfile::tempdir().unwrap();
        let kept = KeptResults::default();
        let mut sink = SpillSink::new(Grid::new(2), 2, folder.path().into(), u64::MAX, &kept);
        assert_eq!(send_set(&mut sink, 5, false), SinkControl::Continue);
        // A second set that fits the grid keeps no file.
        assert_eq!(send_set(&mut sink, 2, false), SinkControl::Continue);
        sink.message(Message::info("done"));
        let (response, mut sources) = sink.into_grid().response();

        assert_eq!(response.results[0].rows, vec![row(0), row(1)]);
        assert!(response.results[0].truncated);
        assert!(!response.results[1].truncated);
        assert_eq!(texts(&response), vec!["done"]);
        assert_eq!(sources.len(), 1);
        let source = sources.remove(0);
        assert_eq!(source.saved_rows(), Some(5));
        assert!(kept.disk_use().bytes() > 0);

        let mut all = BufferSink::new(100);
        source
            .read(&crate::db::ExecOptions::default(), &mut all)
            .await
            .unwrap();
        let set = all.into_response(RunSummary::default()).results.remove(0);
        assert_eq!(set.rows, (0..5).map(row).collect::<Vec<_>>());
        drop(source);
        assert_eq!(kept.disk_use().bytes(), 0);
    }

    #[test]
    fn a_set_that_the_export_limit_cut_keeps_no_file() {
        let folder = tempfile::tempdir().unwrap();
        let kept = KeptResults::default();
        let mut sink = SpillSink::new(Grid::new(2), 2, folder.path().into(), u64::MAX, &kept);
        send_set(&mut sink, 4, true);
        let (response, sources) = sink.into_grid().response();
        assert!(sources.is_empty());
        assert!(response.results[0].truncated);
        assert_eq!(texts(&response), vec![EXPORT_LIMIT_MESSAGE]);
        assert_eq!(kept.disk_use().bytes(), 0);
    }

    #[test]
    fn a_spill_past_the_cap_stops_the_read_at_the_grid_limit() {
        let folder = tempfile::tempdir().unwrap();
        let kept = KeptResults::default();
        // An older spill uses the whole cap. Its release gives no room,
        // because a reader still has the file.
        let older = spill(folder.path(), &kept.disk_use(), 1);
        kept.keep("r0", "c1", vec![(0, KeptSource::SpillFile(older))]);
        let held = kept.get("r0:0").unwrap();
        let cap = kept.disk_use().bytes();
        let mut sink = SpillSink::new(Grid::new(10), 10, folder.path().into(), cap, &kept);
        // Each row takes 13 bytes, so a buffer fills before row 30,000.
        let mut answers = Vec::new();
        sink.begin_set(columns()).unwrap();
        for value in 0..30_000 {
            let answer = sink.row(row(value)).unwrap();
            answers.push(answer);
            if answer == SinkControl::Stop {
                break;
            }
        }
        sink.end_set(true).unwrap();
        // A later set of the run does not spill.
        assert_eq!(send_set(&mut sink, 11, false), SinkControl::Stop);
        let (response, sources) = sink.into_grid().response();
        assert!(sources.is_empty());
        assert_eq!(answers.last(), Some(&SinkControl::Stop));
        assert!(answers.len() > 10);
        assert_eq!(texts(&response), vec![FULL_MESSAGE]);
        // The older result went from the registry for the room.
        assert!(kept.get("r0:0").is_none());
        drop(held);
        assert_eq!(kept.disk_use().bytes(), 0);
        assert!(wait_for_empty(folder.path()));
    }

    #[test]
    fn a_spill_that_the_disk_refuses_keeps_the_grid_rows() {
        let folder = tempfile::tempdir().unwrap();
        let gone = folder.path().join("gone");
        let kept = KeptResults::default();
        // The cut of the set finds no folder for the file.
        let mut sink = SpillSink::new(Grid::new(2), 2, gone.clone(), u64::MAX, &kept);
        send_set(&mut sink, 3, false);
        let (response, sources) = sink.into_grid().response();
        assert!(sources.is_empty());
        assert_eq!(response.results[0].rows.len(), 2);
        assert!(texts(&response)[0].starts_with("Couldn't save this result"));

        // A buffer that finds no folder stops the read at the grid limit.
        let mut sink = SpillSink::new(Grid::new(2), 2, gone, u64::MAX, &kept);
        sink.begin_set(columns()).unwrap();
        let wide = vec![json!("x".repeat(SPILL_BATCH))];
        let mut answers = Vec::new();
        // The thread fails a short time after it starts, and a buffer that
        // goes into its queue before that time shows no failure.
        for _ in 0..1000 {
            let answer = sink.row(wide.clone()).unwrap();
            answers.push(answer);
            if answer == SinkControl::Stop {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        sink.end_set(false).unwrap();
        assert_eq!(answers[..2], [SinkControl::Continue, SinkControl::Continue]);
        assert_eq!(answers.last(), Some(&SinkControl::Stop));
        let (response, sources) = sink.into_grid().response();
        assert!(sources.is_empty());
        assert!(texts(&response)[0].starts_with("Couldn't save this result"));
    }

    #[test]
    fn a_source_of_the_driver_goes_to_the_grid() {
        let folder = tempfile::tempdir().unwrap();
        let kept = KeptResults::default();
        let mut sink = SpillSink::new(Grid::new(2), 2, folder.path().into(), u64::MAX, &kept);
        sink.begin_set(columns()).unwrap();
        sink.keep_source(crate::kept::tests::fixed(1));
        sink.end_set(false).unwrap();
        let (_, sources) = sink.into_grid().response();
        assert_eq!(sources.len(), 1);
    }
}
