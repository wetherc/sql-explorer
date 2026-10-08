//! The spill files: the full rows of a result set in a temporary file on the
//! local disk.
//!
//! With the option on, a run reads past the row limit of the grid, up to the
//! export row limit. The window gets the rows up to the grid limit, and every
//! row of the set goes to a spill file. The file of a set that passes the grid
//! limit becomes a kept result, so an export of all rows reads the file and
//! does not run the statement again.
//!
//! The file contains every row of the set, also the rows that the grid shows,
//! because the backend keeps no copy of the rows that go to the window. The
//! rows stand in the file in the order that the driver gave them.
//!
//! Each row is a count of values and then each value. Each value is one type
//! byte and then its data. Every number is little-endian, and a text has a
//! length of four bytes, as in the chunk form of the window. The form is
//! private to one process, so it has no header and no version.
//!
//! The writes and the reads run on threads of their own, so a slow disk does
//! not stop the async threads that serve the other commands.
//!
//! Each process of the application writes its files in a folder of its own,
//! under the `spill` folder in the cache folder of the application. A lock
//! file beside each folder has an exclusive lock while its process runs. The
//! operating system ends the lock when the process ends, also after a crash.
//! At the start, a process removes each folder whose lock is free, so a
//! second running process does not remove the files of the first one.

use crate::db::sink::{RowSink, SinkControl};
use crate::db::{ColumnInfo, ExecOptions};
use crate::error::{Error, Result};
use crate::kept::KeptResults;
use serde_json::Value as JsonValue;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use tempfile::NamedTempFile;

mod sink;
pub use sink::SpillSink;

/// The name of the folder of the spill files, under the cache folder.
pub const SPILL_FOLDER: &str = "spill";

/// The extension of the lock file beside the folder of each process.
const LOCK_EXTENSION: &str = "lock";

/// The file under the `spill` folder that each start locks while it removes
/// the old folders and makes its own. Two starts at the same time then do not
/// remove the folder that the other start makes.
const START_LOCK: &str = ".start";

/// The bytes that a writer collects before it sends them to its thread. A
/// set whose rows take fewer bytes never opens a file unless the grid limit
/// cuts it, so a small result causes no write to the disk.
pub const SPILL_BATCH: usize = 256 * 1024;

/// The number of batches that wait for the writer thread. A driver that reads
/// faster than the disk writes then waits, so the memory of a spill stays
/// below this number of batches.
const SPILL_QUEUE: usize = 16;

/// The number of rows in one block that the reader thread sends.
const READ_BLOCK: u64 = 1000;

/// The number of blocks that wait for the sink of a read.
const READ_QUEUE: usize = 4;

/// The type byte of each value.
const VALUE_NULL: u8 = 0;
const VALUE_FALSE: u8 = 1;
const VALUE_TRUE: u8 = 2;
const VALUE_INT: u8 = 3;
const VALUE_UINT: u8 = 4;
const VALUE_FLOAT: u8 = 5;
const VALUE_TEXT: u8 = 6;
/// An array or an object, as its JSON text.
const VALUE_JSON: u8 = 7;

/// Adds one row to the buffer.
///
/// A whole number keeps its eight bytes, and a fraction keeps the bits of its
/// double, so the read gives each number back as it was. An array or an
/// object keeps its JSON text.
pub fn encode_row(buffer: &mut Vec<u8>, row: &[JsonValue]) {
    buffer.extend_from_slice(&(row.len() as u32).to_le_bytes());
    for value in row {
        match value {
            JsonValue::Null => buffer.push(VALUE_NULL),
            JsonValue::Bool(false) => buffer.push(VALUE_FALSE),
            JsonValue::Bool(true) => buffer.push(VALUE_TRUE),
            JsonValue::Number(number) => {
                if let Some(whole) = number.as_i64() {
                    buffer.push(VALUE_INT);
                    buffer.extend_from_slice(&whole.to_le_bytes());
                } else if let Some(whole) = number.as_u64() {
                    buffer.push(VALUE_UINT);
                    buffer.extend_from_slice(&whole.to_le_bytes());
                } else {
                    buffer.push(VALUE_FLOAT);
                    let fraction = number.as_f64().unwrap_or_default();
                    buffer.extend_from_slice(&fraction.to_le_bytes());
                }
            }
            JsonValue::String(text) => {
                buffer.push(VALUE_TEXT);
                write_text(buffer, text);
            }
            other => {
                buffer.push(VALUE_JSON);
                write_text(buffer, &other.to_string());
            }
        }
    }
}

fn write_text(buffer: &mut Vec<u8>, text: &str) {
    buffer.extend_from_slice(&(text.len() as u32).to_le_bytes());
    buffer.extend_from_slice(text.as_bytes());
}

/// The error for a spill file that does not contain the form that the
/// writer wrote.
fn damaged() -> Error {
    Error::Storage("The saved rows of this result are damaged. Run the query again.".to_string())
}

fn read_bytes<const N: usize>(reader: &mut impl Read) -> Result<[u8; N]> {
    let mut bytes = [0u8; N];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn read_text(reader: &mut impl Read) -> Result<String> {
    let length = u32::from_le_bytes(read_bytes(reader)?) as usize;
    let mut bytes = vec![0u8; length];
    reader.read_exact(&mut bytes)?;
    String::from_utf8(bytes).map_err(|_| damaged())
}

/// Reads one row that `encode_row` wrote.
pub fn decode_row(reader: &mut impl Read) -> Result<Vec<JsonValue>> {
    let count = u32::from_le_bytes(read_bytes(reader)?) as usize;
    let mut row = Vec::with_capacity(count);
    for _ in 0..count {
        let [tag] = read_bytes::<1>(reader)?;
        let value = match tag {
            VALUE_NULL => JsonValue::Null,
            VALUE_FALSE => JsonValue::Bool(false),
            VALUE_TRUE => JsonValue::Bool(true),
            VALUE_INT => JsonValue::from(i64::from_le_bytes(read_bytes(reader)?)),
            VALUE_UINT => JsonValue::from(u64::from_le_bytes(read_bytes(reader)?)),
            VALUE_FLOAT => serde_json::Number::from_f64(f64::from_le_bytes(read_bytes(reader)?))
                .map(JsonValue::Number)
                .ok_or_else(damaged)?,
            VALUE_TEXT => JsonValue::String(read_text(reader)?),
            VALUE_JSON => serde_json::from_str(&read_text(reader)?).map_err(|_| damaged())?,
            _ => return Err(damaged()),
        };
        row.push(value);
    }
    Ok(row)
}

/// The folder of the spill files of this process, with the open lock file
/// that tells other processes that the folder is in use. The lock ends when
/// the value drops or the process ends.
#[derive(Debug)]
pub struct SpillFolder {
    path: PathBuf,
    _lock: Option<std::fs::File>,
}

impl SpillFolder {
    /// The path of the folder.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// A folder without a lock, for the tests.
impl From<PathBuf> for SpillFolder {
    fn from(path: PathBuf) -> Self {
        Self { path, _lock: None }
    }
}

/// Removes a file or a folder. A path that is already gone is not an error.
fn remove_path(path: &Path) -> std::io::Result<()> {
    let removal = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    match removal {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// Removes the folder and the lock file of a process that ended. Gives false
/// when another process has the lock.
fn remove_ended(lock_path: &Path) -> Result<bool> {
    let lock = std::fs::File::open(lock_path)?;
    match lock.try_lock() {
        Err(std::fs::TryLockError::WouldBlock) => return Ok(false),
        locked => locked.map_err(std::io::Error::from)?,
    }
    remove_path(&lock_path.with_extension(""))?;
    remove_path(lock_path)?;
    Ok(true)
}

/// Removes each entry of the `spill` folder that no running process uses. A
/// folder without a lock file and a loose file are left over from a crash.
fn remove_unused(root: &Path) -> Result<()> {
    let mut locks = Vec::new();
    let mut others = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let path = entry?.path();
        if path.file_name() == Some(START_LOCK.as_ref()) {
            continue;
        }
        if path.extension() == Some(LOCK_EXTENSION.as_ref()) {
            locks.push(path);
        } else {
            others.push(path);
        }
    }
    for lock in &locks {
        remove_ended(lock)?;
    }
    for path in others {
        if !path.with_extension(LOCK_EXTENSION).exists() {
            remove_path(&path)?;
        }
    }
    Ok(())
}

/// Makes the `spill` folder under the cache folder, removes the folders of
/// the processes that ended, and makes the locked folder of this process.
pub fn prepare_folder(cache: &Path) -> Result<SpillFolder> {
    let root = cache.join(SPILL_FOLDER);
    std::fs::create_dir_all(&root)?;
    let start = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join(START_LOCK))?;
    start.lock()?;
    remove_unused(&root)?;

    // The lock file comes before the folder, so a folder without a lock
    // file is always a leftover.
    let name = uuid::Uuid::new_v4().simple().to_string();
    let path = root.join(&name);
    let lock = std::fs::File::create_new(path.with_extension(LOCK_EXTENSION))?;
    lock.try_lock().map_err(std::io::Error::from)?;
    std::fs::create_dir(&path)?;
    Ok(SpillFolder {
        path,
        _lock: Some(lock),
    })
}

/// Prepares the folder of the spill files under the cache folder of the
/// application, and gives it to the registry. A failure leaves the registry
/// without a folder, and the runs then spill nothing.
pub fn start_folder(cache: std::result::Result<PathBuf, tauri::Error>, kept: &KeptResults) {
    match cache
        .map_err(|error| Error::Storage(error.to_string()))
        .and_then(|cache| prepare_folder(&cache))
    {
        Ok(folder) => kept.set_spill_folder(folder),
        Err(error) => log::warn!("Full results can't be saved on this computer: {error}"),
    }
}

/// The bytes of every spill file of the process, as a count that each writer
/// and each kept file shares.
#[derive(Debug, Clone, Default)]
pub struct DiskUse(Arc<AtomicU64>);

impl DiskUse {
    /// The bytes of every spill file, also the files that a writer still
    /// writes.
    pub fn bytes(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }

    /// Adds the bytes when the total then stays at or below the cap.
    fn try_add(&self, bytes: u64, cap: u64) -> bool {
        self.0
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |used| {
                used.checked_add(bytes).filter(|total| *total <= cap)
            })
            .is_ok()
    }
}

/// The bytes that one spill added to the count of the disk use. The drop
/// gives the bytes back.
#[derive(Debug)]
struct Reservation {
    disk: DiskUse,
    bytes: u64,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.disk.0.fetch_sub(self.bytes, Ordering::SeqCst);
    }
}

/// Why a spill stopped before its end.
#[derive(Debug)]
pub enum SpillEnd {
    /// The file would pass the cap of the disk use.
    Full,
    /// The disk refused a write, or the writer thread stopped.
    Failed(Error),
}

/// One piece on its way to the writer thread.
enum Piece {
    Bytes(Vec<u8>),
    /// The rows are complete. The thread writes its buffer to the file and
    /// sends the file back on the channel.
    Finish(std::sync::mpsc::Sender<Result<NamedTempFile>>),
}

/// The error of the writer thread, kept for the writer.
type WriterFault = Arc<Mutex<Option<Error>>>;

/// The side of the writer thread that the writer keeps.
struct WriterThread {
    pieces: std::sync::mpsc::SyncSender<Piece>,
    fault: WriterFault,
}

impl WriterThread {
    /// Starts a thread that makes a temporary file in the folder and writes
    /// each piece to it. A queue that closes before a `Finish` piece drops
    /// the file, and the drop removes it.
    fn start(folder: &Path) -> Result<Self> {
        let (pieces, queue) = std::sync::mpsc::sync_channel(SPILL_QUEUE);
        let fault = WriterFault::default();
        let kept = Arc::clone(&fault);
        let folder = folder.to_path_buf();
        std::thread::Builder::new()
            .name("spill-writer".to_string())
            .spawn(move || {
                if let Err(error) = write_file(&folder, &queue) {
                    *kept.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
                }
            })?;
        Ok(Self { pieces, fault })
    }

    /// The error that stopped the thread. A thread that stopped without an
    /// error, such as one that panicked, gives a general error.
    fn fault(&self) -> Error {
        self.fault
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .unwrap_or_else(|| Error::Storage("The writer of the saved rows stopped.".to_string()))
    }

    /// Puts one piece in the queue. A full queue makes the driver wait for
    /// the disk.
    fn send(&self, piece: Piece) -> Result<()> {
        crate::commands::wait_in_place(|| self.pieces.send(piece)).map_err(|_| self.fault())
    }
}

/// The body of the writer thread.
fn write_file(folder: &Path, queue: &std::sync::mpsc::Receiver<Piece>) -> Result<()> {
    let file = tempfile::Builder::new()
        .prefix("rows-")
        .suffix(".spill")
        .tempfile_in(folder)?;
    let mut out = BufWriter::new(file);
    if let Some(reply) = write_pieces(&mut out, queue)? {
        let file = out
            .into_inner()
            .map_err(std::io::IntoInnerError::into_error)
            .map_err(Error::from);
        let _ = reply.send(file);
    }
    Ok(())
}

/// Writes each piece of the queue in order. Gives the channel of the reply
/// when a `Finish` piece arrives, and nothing when the queue closes first.
fn write_pieces<W: Write>(
    out: &mut W,
    queue: &std::sync::mpsc::Receiver<Piece>,
) -> Result<Option<std::sync::mpsc::Sender<Result<NamedTempFile>>>> {
    for piece in queue {
        match piece {
            Piece::Bytes(bytes) => out.write_all(&bytes)?,
            Piece::Finish(reply) => return Ok(Some(reply)),
        }
    }
    Ok(None)
}

/// The writer of the spill file of one result set.
///
/// The sink of the run calls the writer on an async thread. The writer
/// encodes each row into a buffer and sends a full buffer to its thread. The
/// thread starts with the first full buffer, so a small set opens no file.
/// Each buffer first takes its bytes from the cap of the disk use. A drop
/// before `finish` stops the thread, which removes the file.
pub struct SpillWriter {
    folder: PathBuf,
    columns: Vec<ColumnInfo>,
    cap: u64,
    buffer: Vec<u8>,
    rows: u64,
    reserved: Reservation,
    thread: Option<WriterThread>,
}

impl SpillWriter {
    pub fn new(folder: PathBuf, columns: Vec<ColumnInfo>, disk: DiskUse, cap: u64) -> Self {
        Self {
            folder,
            columns,
            cap,
            buffer: Vec::new(),
            rows: 0,
            reserved: Reservation { disk, bytes: 0 },
            thread: None,
        }
    }

    /// Adds one row. `make_room` frees the disk of an older spill when the
    /// cap has no room for the next buffer, and gives false when nothing
    /// is left to free.
    pub fn write(
        &mut self,
        row: &[JsonValue],
        make_room: &mut dyn FnMut() -> bool,
    ) -> std::result::Result<(), SpillEnd> {
        encode_row(&mut self.buffer, row);
        self.rows += 1;
        if self.buffer.len() >= SPILL_BATCH {
            self.flush(make_room)?;
        }
        Ok(())
    }

    /// Sends the buffer to the writer thread, and starts the thread first
    /// when it is not running.
    fn flush(&mut self, make_room: &mut dyn FnMut() -> bool) -> std::result::Result<(), SpillEnd> {
        let bytes = self.buffer.len() as u64;
        while !self.reserved.disk.try_add(bytes, self.cap) {
            if !make_room() {
                return Err(SpillEnd::Full);
            }
        }
        self.reserved.bytes += bytes;
        if self.thread.is_none() {
            self.thread = Some(WriterThread::start(&self.folder).map_err(SpillEnd::Failed)?);
        }
        let thread = self.thread.as_ref().expect("the thread started above");
        thread
            .send(Piece::Bytes(std::mem::take(&mut self.buffer)))
            .map_err(SpillEnd::Failed)
    }

    /// Writes the last rows and closes the file. The file then belongs to
    /// the spill file that this gives back.
    pub fn finish(
        mut self,
        make_room: &mut dyn FnMut() -> bool,
    ) -> std::result::Result<SpillFile, SpillEnd> {
        self.flush(make_room)?;
        let thread = self.thread.take().expect("the flush started the thread");
        let (reply, answer) = std::sync::mpsc::channel();
        thread
            .send(Piece::Finish(reply))
            .map_err(SpillEnd::Failed)?;
        let file = crate::commands::wait_in_place(|| answer.recv())
            .map_err(|_| SpillEnd::Failed(thread.fault()))?
            .map_err(SpillEnd::Failed)?;
        let disk = self.reserved.disk.clone();
        let reserved = std::mem::replace(&mut self.reserved, Reservation { disk, bytes: 0 });
        Ok(SpillFile {
            file: Arc::new(file),
            columns: std::mem::take(&mut self.columns),
            rows: self.rows,
            reserved,
        })
    }
}

/// The complete spill file of one result set. The drop removes the file and
/// gives its bytes back to the count of the disk use. A read keeps its own
/// handle of the file, so a read that runs during the drop ends normally.
#[derive(Debug)]
pub struct SpillFile {
    file: Arc<NamedTempFile>,
    columns: Vec<ColumnInfo>,
    rows: u64,
    reserved: Reservation,
}

impl SpillFile {
    /// The number of rows in the file.
    pub fn rows(&self) -> u64 {
        self.rows
    }

    /// The bytes of the file.
    pub fn bytes(&self) -> u64 {
        self.reserved.bytes
    }

    /// Reads every row of the file into the sink, up to the row limit of
    /// the options. The read stops when the sink answers `Stop`. A thread
    /// of the blocking pool reads the file and sends the rows in blocks.
    pub async fn read(&self, options: &ExecOptions, sink: &mut dyn RowSink) -> Result<()> {
        sink.begin_set(self.columns.clone())?;
        let (sender, mut blocks) = tokio::sync::mpsc::channel(READ_QUEUE);
        let file = Arc::clone(&self.file);
        let rows = self.rows;
        tokio::task::spawn_blocking(move || {
            if let Err(error) = read_blocks(&file, rows, &sender) {
                let _ = sender.blocking_send(Err(error));
            }
        });
        let mut count = 0usize;
        while let Some(block) = blocks.recv().await {
            for row in block? {
                if count >= options.max_rows || sink.row(row)? == SinkControl::Stop {
                    return sink.end_set(true);
                }
                count += 1;
            }
        }
        sink.end_set(false)
    }
}

/// The body of the reader thread. It stops when the receiver goes, for
/// example because the sink stopped the read.
fn read_blocks(
    file: &NamedTempFile,
    rows: u64,
    sender: &tokio::sync::mpsc::Sender<Result<Vec<Vec<JsonValue>>>>,
) -> Result<()> {
    let mut reader = BufReader::new(file.reopen()?);
    let mut left = rows;
    while left > 0 {
        let take = left.min(READ_BLOCK);
        let mut block = Vec::with_capacity(take as usize);
        for _ in 0..take {
            block.push(decode_row(&mut reader)?);
        }
        left -= take;
        if sender.blocking_send(Ok(block)).is_err() {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::db::sink::{BufferSink, RunSummary};
    use crate::error::ErrorCategory;
    use serde_json::json;

    fn round_trip(row: &[JsonValue]) -> Vec<JsonValue> {
        let mut buffer = Vec::new();
        encode_row(&mut buffer, row);
        let mut reader = buffer.as_slice();
        let back = decode_row(&mut reader).unwrap();
        assert!(reader.is_empty());
        back
    }

    #[test]
    fn each_value_comes_back_as_it_was_written() {
        let row = vec![
            JsonValue::Null,
            json!(false),
            json!(true),
            json!(-42),
            json!(i64::MIN),
            json!(u64::MAX),
            json!(0.1),
            json!(1e300),
            json!(-0.0),
            json!(""),
            json!("text with ünïcode"),
            json!([1, "a", null]),
            json!({ "b": 1, "a": [true] }),
        ];
        let back = round_trip(&row);
        assert_eq!(back, row);
        // A fraction keeps its type, so the export writes it as it was.
        assert!(back[6].is_f64());
        assert!(back[5].is_u64());
        assert_eq!(round_trip(&[]), Vec::<JsonValue>::new());
    }

    #[test]
    fn a_damaged_file_gives_an_error() {
        let storage = |bytes: &[u8]| {
            let mut reader = bytes;
            decode_row(&mut reader).unwrap_err().category()
        };
        // An unknown type byte.
        assert_eq!(storage(&[1, 0, 0, 0, 99]), ErrorCategory::Storage);
        // Text that is not UTF-8.
        assert_eq!(
            storage(&[1, 0, 0, 0, VALUE_TEXT, 1, 0, 0, 0, 0xff]),
            ErrorCategory::Storage
        );
        // JSON text that does not parse.
        assert_eq!(
            storage(&[1, 0, 0, 0, VALUE_JSON, 1, 0, 0, 0, b'{']),
            ErrorCategory::Storage
        );
        // A double that JSON cannot contain.
        let mut nan = vec![1, 0, 0, 0, VALUE_FLOAT];
        nan.extend_from_slice(&f64::NAN.to_le_bytes());
        assert_eq!(storage(&nan), ErrorCategory::Storage);
        // A file that ends inside a row.
        assert_eq!(storage(&[2, 0, 0, 0, VALUE_NULL]), ErrorCategory::Io);
    }

    #[test]
    fn the_start_removes_only_the_folders_of_ended_processes() {
        let cache = tempfile::tempdir().unwrap();
        let root = cache.path().join(SPILL_FOLDER);
        // A first process that still runs, and a second one that ended.
        let running = prepare_folder(cache.path()).unwrap();
        let ended = prepare_folder(cache.path()).unwrap();
        let ended_path = ended.path().to_path_buf();
        drop(ended);
        assert_eq!(running.path().parent(), Some(root.as_path()));
        std::fs::write(running.path().join("rows-1.spill"), b"x").unwrap();
        std::fs::write(ended_path.join("rows-2.spill"), b"x").unwrap();
        // Leftovers of a crash: a loose file and a folder without a lock.
        std::fs::write(root.join("rows-left.spill"), b"x").unwrap();
        std::fs::create_dir(root.join("inner")).unwrap();

        let third = prepare_folder(cache.path()).unwrap();
        assert!(running.path().join("rows-1.spill").exists());
        assert!(!ended_path.exists());
        assert!(!ended_path.with_extension(LOCK_EXTENSION).exists());
        let mut names: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        let mut expected = vec![START_LOCK.to_string()];
        for folder in [&running, &third] {
            let name = folder.path().file_name().unwrap().to_str().unwrap();
            expected.push(name.to_string());
            expected.push(format!("{name}.{LOCK_EXTENSION}"));
        }
        expected.sort();
        assert_eq!(names, expected);

        // A cache path that is a file cannot contain the folder.
        let file = cache.path().join("file");
        std::fs::write(&file, b"x").unwrap();
        assert!(prepare_folder(&file).is_err());
    }

    #[test]
    fn a_path_that_is_gone_needs_no_removal() {
        let cache = tempfile::tempdir().unwrap();
        assert!(remove_path(&cache.path().join("gone")).is_ok());
        let file = cache.path().join("file");
        std::fs::write(&file, b"x").unwrap();
        assert!(remove_path(&file.join("below")).is_err());
    }

    #[test]
    fn the_start_gives_the_folder_to_the_registry() {
        let cache = tempfile::tempdir().unwrap();
        let kept = KeptResults::default();
        start_folder(Err(tauri::Error::UnknownPath), &kept);
        assert_eq!(kept.spill_folder(), None);
        start_folder(Ok(cache.path().to_path_buf()), &kept);
        let folder = kept.spill_folder().unwrap();
        assert_eq!(
            folder.parent(),
            Some(cache.path().join(SPILL_FOLDER).as_path())
        );
        assert!(folder.is_dir());
    }

    #[test]
    fn the_disk_use_stays_at_or_below_the_cap() {
        let disk = DiskUse::default();
        assert!(disk.try_add(60, 100));
        assert!(!disk.try_add(41, 100));
        assert!(disk.try_add(40, 100));
        assert!(!disk.try_add(u64::MAX, u64::MAX));
        assert_eq!(disk.bytes(), 100);
        drop(Reservation {
            disk: disk.clone(),
            bytes: 100,
        });
        assert_eq!(disk.bytes(), 0);
    }

    fn columns() -> Vec<ColumnInfo> {
        vec![ColumnInfo::new("n", "int"), ColumnInfo::new("text", "text")]
    }

    fn row(value: usize) -> Vec<JsonValue> {
        vec![json!(value), json!(format!("row {value}"))]
    }

    pub(crate) fn no_room() -> impl FnMut() -> bool {
        || false
    }

    /// Writes `rows` rows to a new spill file in the folder.
    pub(crate) fn spill(folder: &Path, disk: &DiskUse, rows: usize) -> SpillFile {
        let mut writer = SpillWriter::new(folder.to_path_buf(), columns(), disk.clone(), u64::MAX);
        for value in 0..rows {
            writer.write(&row(value), &mut no_room()).unwrap();
        }
        writer.finish(&mut no_room()).unwrap()
    }

    fn files_in(folder: &Path) -> usize {
        std::fs::read_dir(folder).unwrap().count()
    }

    #[tokio::test]
    async fn a_spill_file_gives_every_row_back_in_order() {
        let folder = tempfile::tempdir().unwrap();
        let disk = DiskUse::default();
        // Enough rows for several buffers and several blocks of the read.
        let rows = 30_000;
        let file = spill(folder.path(), &disk, rows);
        assert_eq!(file.rows(), rows as u64);
        assert!(file.bytes() > SPILL_BATCH as u64);
        assert_eq!(disk.bytes(), file.bytes());
        assert_eq!(files_in(folder.path()), 1);

        let mut sink = BufferSink::new(usize::MAX);
        let options = ExecOptions {
            max_rows: usize::MAX,
            ..ExecOptions::default()
        };
        file.read(&options, &mut sink).await.unwrap();
        let set = sink.into_response(RunSummary::default()).results.remove(0);
        assert_eq!(set.columns, columns());
        assert!(!set.truncated);
        assert_eq!(set.rows.len(), rows);
        assert!(set.rows.iter().enumerate().all(|(n, got)| *got == row(n)));

        // The drop removes the file and gives its bytes back.
        drop(file);
        assert_eq!(files_in(folder.path()), 0);
        assert_eq!(disk.bytes(), 0);
    }

    #[tokio::test]
    async fn a_read_stops_at_the_row_limit_or_at_a_stop_of_the_sink() {
        let folder = tempfile::tempdir().unwrap();
        let file = spill(folder.path(), &DiskUse::default(), 20_000);
        let options = ExecOptions {
            max_rows: 1500,
            ..ExecOptions::default()
        };
        let mut sink = BufferSink::new(usize::MAX);
        file.read(&options, &mut sink).await.unwrap();
        let set = sink.into_response(RunSummary::default()).results.remove(0);
        assert_eq!(set.rows.len(), 1500);
        assert!(set.truncated);

        let mut sink = BufferSink::new(3);
        file.read(&ExecOptions::default(), &mut sink).await.unwrap();
        let set = sink.into_response(RunSummary::default()).results.remove(0);
        assert_eq!(set.rows.len(), 3);
        assert!(set.truncated);

        // A limit equal to the rows of the file reads them all.
        let small = spill(folder.path(), &DiskUse::default(), 2);
        let mut sink = BufferSink::new(2);
        let options = ExecOptions {
            max_rows: 2,
            ..ExecOptions::default()
        };
        small.read(&options, &mut sink).await.unwrap();
        let set = sink.into_response(RunSummary::default()).results.remove(0);
        assert_eq!(set.rows.len(), 2);
        assert!(!set.truncated);
    }

    #[tokio::test]
    async fn a_file_with_fewer_rows_than_its_count_gives_an_error() {
        let folder = tempfile::tempdir().unwrap();
        let mut file = spill(folder.path(), &DiskUse::default(), 1);
        file.rows = 2;
        let mut sink = BufferSink::new(10);
        let error = file
            .read(&ExecOptions::default(), &mut sink)
            .await
            .unwrap_err();
        assert_eq!(error.category(), ErrorCategory::Io);
    }

    #[test]
    fn a_small_set_opens_no_file_until_it_finishes() {
        let folder = tempfile::tempdir().unwrap();
        let disk = DiskUse::default();
        let mut writer = SpillWriter::new(folder.path().to_path_buf(), columns(), disk.clone(), 1);
        writer.write(&row(1), &mut no_room()).unwrap();
        assert!(writer.thread.is_none());
        assert_eq!(files_in(folder.path()), 0);
        // A drop before the end leaves nothing.
        drop(writer);
        assert_eq!(files_in(folder.path()), 0);
        assert_eq!(disk.bytes(), 0);
    }

    #[test]
    fn a_spill_past_the_cap_frees_older_spills_or_stops() {
        let folder = tempfile::tempdir().unwrap();
        let disk = DiskUse::default();
        let cap = (SPILL_BATCH * 3) as u64;
        // Another spill already uses most of the cap.
        assert!(disk.try_add(cap - 10, cap));
        let mut freed = 0;
        let mut make_room = || {
            if freed > 0 {
                return false;
            }
            freed += 1;
            disk.0.fetch_sub(cap - 10, Ordering::SeqCst);
            true
        };
        let mut writer =
            SpillWriter::new(folder.path().to_path_buf(), columns(), disk.clone(), cap);
        let mut written = 0;
        let end = loop {
            if let Err(end) = writer.write(&row(written), &mut make_room) {
                break end;
            }
            written += 1;
        };
        assert!(matches!(end, SpillEnd::Full));
        assert_eq!(freed, 1);
        // The spill took no more than the cap.
        assert!(disk.bytes() <= cap);
        drop(writer);
        assert_eq!(disk.bytes(), 0);
        assert!(wait_for_empty(folder.path()));
    }

    /// Waits for a writer thread to remove its file.
    pub(crate) fn wait_for_empty(folder: &Path) -> bool {
        for _ in 0..200 {
            if files_in(folder) == 0 {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn a_folder_that_is_gone_fails_the_spill() {
        let folder = tempfile::tempdir().unwrap();
        let gone = folder.path().join("gone");
        let mut writer = SpillWriter::new(gone.clone(), columns(), DiskUse::default(), u64::MAX);
        writer.write(&row(1), &mut no_room()).unwrap();
        let Err(SpillEnd::Failed(error)) = writer.finish(&mut no_room()) else {
            panic!("the spill had no folder");
        };
        assert_eq!(error.category(), ErrorCategory::Io);

        // A thread that stops without an error gives a general error.
        let mut writer = SpillWriter::new(gone, columns(), DiskUse::default(), u64::MAX);
        let (pieces, queue) = std::sync::mpsc::sync_channel(1);
        drop(queue);
        writer.thread = Some(WriterThread {
            pieces,
            fault: WriterFault::default(),
        });
        let Err(SpillEnd::Failed(error)) = writer.finish(&mut no_room()) else {
            panic!("the thread was gone");
        };
        assert_eq!(error.category(), ErrorCategory::Storage);
    }

    #[test]
    fn a_thread_that_drops_the_reply_fails_the_finish() {
        let (pieces, queue) = std::sync::mpsc::sync_channel(4);
        let fault = WriterFault::default();
        let mut writer = SpillWriter::new(PathBuf::new(), columns(), DiskUse::default(), u64::MAX);
        writer.thread = Some(WriterThread {
            pieces,
            fault: Arc::clone(&fault),
        });
        *fault.lock().unwrap() = Some(Error::Storage("disk full".to_string()));
        let reader = std::thread::spawn(move || {
            // The thread takes the pieces and drops the reply unanswered.
            for piece in queue {
                drop(piece);
            }
        });
        let Err(SpillEnd::Failed(error)) = writer.finish(&mut no_room()) else {
            panic!("the reply never came");
        };
        assert_eq!(error.to_string(), "disk full");
        reader.join().unwrap();
    }

    #[test]
    fn the_writer_thread_stops_at_a_failed_write_or_a_closed_queue() {
        struct Refusing;
        impl Write for Refusing {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("disk full"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let (pieces, queue) = std::sync::mpsc::sync_channel(4);
        pieces.send(Piece::Bytes(vec![1])).unwrap();
        assert!(write_pieces(&mut Refusing, &queue).is_err());
        drop(pieces);
        assert!(write_pieces(&mut Vec::new(), &queue).unwrap().is_none());
        Refusing.flush().unwrap();
    }
}
