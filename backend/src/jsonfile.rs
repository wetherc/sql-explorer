//! Small JSON files that keep one object of named values each.
//!
//! Each file has a lock and a copy in memory. A read takes the copy, and a
//! change writes the whole file before the copy changes, so a failed write
//! leaves both the file and the copy as they were.
//!
//! A write goes to a temporary file first. The temporary file is flushed to
//! the disk and then renamed over the old one, so a crash during the write
//! leaves the old file or the new file, and never half of one.
//!
//! A file that is not valid JSON is renamed to `<name>.corrupt-<seconds>` and
//! the read continues with an empty object. The rename keeps the damaged data
//! for the user, and the next write cannot overwrite it.

use crate::error::{Error, Result};
use serde_json::{Map, Value as JsonValue};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// The named values of one file.
pub type Values = Map<String, JsonValue>;

/// The copy in memory of one file. `None` until the first read.
type Cached = Arc<Mutex<Option<Values>>>;

static FILES: Mutex<Option<HashMap<PathBuf, Cached>>> = Mutex::new(None);

/// The problems with the files that the window has not shown yet.
static PROBLEMS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn entry(path: &Path) -> Cached {
    let mut files = lock(&FILES);
    files
        .get_or_insert_with(HashMap::new)
        .entry(path.to_path_buf())
        .or_default()
        .clone()
}

/// Adds a problem for the window to show. The same text is kept once.
pub fn note_problem(text: String) {
    let mut problems = lock(&PROBLEMS);
    if !problems.contains(&text) {
        log::warn!("{text}");
        problems.push(text);
    }
}

/// Gives the problems that were found and forgets them.
pub fn take_problems() -> Vec<String> {
    std::mem::take(&mut *lock(&PROBLEMS))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Reads a file from the disk. A missing file is an empty object.
fn load(path: &Path) -> Result<Values> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Values::new()),
        Err(error) => {
            return Err(Error::Storage(format!(
                "{} couldn't be read: {error}",
                path.display()
            )))
        }
    };
    match serde_json::from_slice::<JsonValue>(&bytes) {
        Ok(JsonValue::Object(values)) => Ok(values),
        Ok(_) => keep_damaged(path, "it doesn't contain a JSON object"),
        Err(error) => keep_damaged(path, &error.to_string()),
    }
}

/// Renames a damaged file out of the way and starts with an empty object.
fn keep_damaged(path: &Path, reason: &str) -> Result<Values> {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let name = file_name(path);
    let backup = free_backup(path, &name, seconds);
    std::fs::rename(path, &backup).map_err(|error| {
        Error::Storage(format!(
            "{} is damaged ({reason}) and couldn't be moved aside: {error}",
            path.display()
        ))
    })?;
    note_problem(format!(
        "{name} couldn't be read ({reason}), so the app started without it. \
         The old file was kept as {}.",
        backup.display()
    ));
    Ok(Values::new())
}

/// Gives a name for the copy of a damaged file that no file has yet. A
/// rename onto a file that exists replaces it, so a second damaged file in
/// the same second would delete the first copy.
fn free_backup(path: &Path, name: &str, seconds: u64) -> PathBuf {
    let mut backup = path.with_file_name(format!("{name}.corrupt-{seconds}"));
    let mut count = 1;
    while backup.exists() {
        backup = path.with_file_name(format!("{name}.corrupt-{seconds}-{count}"));
        count += 1;
    }
    backup
}

/// Writes the values to a temporary file, flushes it, and renames it over
/// the file.
fn store(path: &Path, values: &Values) -> Result<()> {
    let failed = |error: std::io::Error| {
        Error::Storage(format!("{} couldn't be saved: {error}", path.display()))
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(failed)?;
    }
    let text = serde_json::to_vec_pretty(values)?;
    let temporary = path.with_file_name(format!("{}.tmp", file_name(path)));
    let write = || -> std::io::Result<()> {
        use std::io::Write;
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(&text)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    };
    if let Err(error) = write() {
        // A temporary file that stays would be read by nothing, so it goes.
        let _ = std::fs::remove_file(&temporary);
        return Err(failed(error));
    }
    sync_folder(path);
    Ok(())
}

/// Flushes the folder of a file, so the rename reaches the disk too. The
/// file has its new values at this point, so a failure goes to the log and
/// the write still counts as done. Windows cannot open a folder as a file,
/// so only Unix flushes the folder.
#[cfg(unix)]
fn sync_folder(path: &Path) {
    let Some(parent) = path.parent() else {
        return;
    };
    if let Err(error) = std::fs::File::open(parent).and_then(|folder| folder.sync_all()) {
        log::warn!(
            "The folder of {} couldn't be flushed: {error}",
            path.display()
        );
    }
}

#[cfg(not(unix))]
fn sync_folder(_path: &Path) {}

/// Gives a copy of the values of a file.
pub fn read(path: &Path) -> Result<Values> {
    let cached = entry(path);
    let mut slot = lock(&cached);
    if slot.is_none() {
        *slot = Some(load(path)?);
    }
    Ok(slot.clone().unwrap_or_default())
}

/// Changes the values of a file and writes the file, under the lock of that
/// file. The copy in memory changes only after the write worked.
pub fn update<T>(path: &Path, change: impl FnOnce(&mut Values) -> T) -> Result<T> {
    let cached = entry(path);
    let mut slot = lock(&cached);
    let mut values = match slot.take() {
        Some(values) => values,
        None => load(path)?,
    };
    let before = values.clone();
    let answer = change(&mut values);
    match store(path, &values) {
        Ok(()) => {
            *slot = Some(values);
            Ok(answer)
        }
        Err(error) => {
            *slot = Some(before);
            Err(error)
        }
    }
}

/// Tests that read the list of problems take this lock, because the list is
/// shared by every test of the process.
#[cfg(test)]
pub static PROBLEM_TESTS: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn folder(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "sql-explorer-jsonfile-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn a_missing_file_is_empty_and_a_write_creates_it() {
        let path = folder("missing").join("deep").join("a.json");
        assert!(read(&path).unwrap().is_empty());
        update(&path, |values| values.insert("k".into(), json!(1))).unwrap();
        assert_eq!(read(&path).unwrap()["k"], json!(1));
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            serde_json::from_str::<JsonValue>(&text).unwrap(),
            json!({"k": 1})
        );
        assert!(!path.with_file_name("a.json.tmp").exists());
    }

    #[test]
    fn a_truncated_file_is_kept_aside_and_reported() {
        let _problems = lock(&PROBLEM_TESTS);
        let base = folder("truncated");
        let path = base.join("b.json");
        std::fs::write(&path, b"{\"k\": [1, 2").unwrap();
        assert!(read(&path).unwrap().is_empty());
        let kept: Vec<String> = std::fs::read_dir(&base)
            .unwrap()
            .map(|item| item.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(kept.iter().any(|name| name.starts_with("b.json.corrupt-")));
        assert!(!path.exists());
        let problems = take_problems();
        assert!(problems
            .iter()
            .any(|text| text.starts_with("b.json couldn't be read")));
    }

    #[test]
    fn a_file_that_is_not_an_object_is_kept_aside() {
        let path = folder("array").join("c.json");
        std::fs::write(&path, b"[1]").unwrap();
        assert!(read(&path).unwrap().is_empty());
        assert!(!path.exists());
    }

    #[test]
    fn a_failed_write_keeps_the_old_file_and_the_old_values() {
        let path = folder("failed").join("d.json");
        update(&path, |values| values.insert("k".into(), json!("old"))).unwrap();
        // A folder in the place of the temporary file makes the write fail.
        std::fs::create_dir_all(path.with_file_name("d.json.tmp")).unwrap();
        let failed = update(&path, |values| values.insert("k".into(), json!("new")));
        assert!(matches!(failed, Err(Error::Storage(_))));
        // The folder in the place of the temporary file is not a file, so
        // the clean-up leaves it.
        assert!(path.with_file_name("d.json.tmp").is_dir());
        assert_eq!(read(&path).unwrap()["k"], json!("old"));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("old"));
    }

    #[test]
    fn a_file_that_cannot_be_read_is_an_error() {
        // A folder in the place of the file cannot be read as a file.
        let path = folder("unreadable").join("e.json");
        std::fs::create_dir_all(&path).unwrap();
        assert!(matches!(read(&path), Err(Error::Storage(_))));
        assert!(matches!(update(&path, |_| ()), Err(Error::Storage(_))));
    }

    #[test]
    fn a_second_damaged_file_in_the_same_second_keeps_the_first_copy() {
        let base = folder("backup-name");
        let path = base.join("f.json");
        assert_eq!(
            free_backup(&path, "f.json", 7),
            base.join("f.json.corrupt-7")
        );
        std::fs::write(base.join("f.json.corrupt-7"), b"first").unwrap();
        assert_eq!(
            free_backup(&path, "f.json", 7),
            base.join("f.json.corrupt-7-1")
        );
        std::fs::write(base.join("f.json.corrupt-7-1"), b"second").unwrap();
        assert_eq!(
            free_backup(&path, "f.json", 7),
            base.join("f.json.corrupt-7-2")
        );
    }

    #[test]
    fn a_rename_that_fails_leaves_no_temporary_file() {
        // A folder with a file in it stands in the place of the file, so
        // the rename onto it fails after the temporary file is written.
        let path = folder("rename").join("h.json");
        std::fs::create_dir_all(path.join("inside")).unwrap();
        assert!(matches!(
            store(&path, &Values::new()),
            Err(Error::Storage(_))
        ));
        assert!(!path.with_file_name("h.json.tmp").exists());
    }

    #[test]
    fn a_folder_that_cannot_be_flushed_does_not_fail_the_write() {
        // The folder of the file is not there, so the flush fails and only
        // writes to the log.
        sync_folder(&folder("sync").join("missing").join("g.json"));
        sync_folder(Path::new("/"));
    }

    #[test]
    fn a_problem_is_kept_once() {
        let _problems = lock(&PROBLEM_TESTS);
        note_problem("same problem".into());
        note_problem("same problem".into());
        let problems = take_problems();
        assert_eq!(
            problems
                .iter()
                .filter(|text| *text == "same problem")
                .count(),
            1
        );
    }

    #[test]
    fn a_name_without_a_file_part_gives_the_whole_path() {
        assert_eq!(file_name(Path::new("/")), "/");
        assert_eq!(file_name(Path::new("/a/b.json")), "b.json");
    }
}
