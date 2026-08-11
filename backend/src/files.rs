//! The commands that read and write the statement files of the user.
//!
//! The user chooses a folder through the dialog of the operating system, and
//! the backend records that folder as a root. Every later command refuses a
//! path that lies outside every root, after it resolves the links of the
//! path, so a link inside a root cannot step out of it. The interface
//! therefore cannot reach a file that the user did not accept.

use crate::error::{Error, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// The largest file that the editor accepts, in bytes.
pub const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;

/// What one entry of a folder is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EntryKind {
    Folder,
    File,
}

/// One entry of a folder, as the interface sees it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderEntry {
    pub name: String,
    pub path: String,
    pub kind: EntryKind,
}

/// Resolves the links of a path and returns the result.
fn resolved(path: &Path) -> Result<PathBuf> {
    std::fs::canonicalize(path).map_err(|error| {
        Error::Io(std::io::Error::new(
            error.kind(),
            format!("The path could not be read: {}", path.display()),
        ))
    })
}

/// The words that name a path outside every folder the user accepted.
fn outside_the_roots(path: &Path) -> Error {
    Error::Configuration(format!(
        "The path '{}' lies outside every folder that you opened.",
        path.display()
    ))
}

/// Holds a path against the folders the user accepted.
///
/// The path is resolved first, so a link inside a root that points somewhere
/// else is judged by where it lands. Each root is resolved as well, because a
/// root can itself sit under a link.
pub fn path_inside_roots(path: &Path, roots: &[PathBuf]) -> Result<PathBuf> {
    let target = resolved(path)?;
    for root in roots {
        let Ok(root) = resolved(root) else {
            // A root that is gone from the disk holds nothing any more.
            continue;
        };
        if target.starts_with(&root) {
            return Ok(target);
        }
    }
    Err(outside_the_roots(path))
}

/// Reads a folder that a record of the workspace names.
///
/// The record can be old, so the path must still be a folder on the disk.
/// A record that names something else, or nothing at all, brings no root
/// back.
pub fn root_from_record(path: &str) -> Option<PathBuf> {
    let candidate = PathBuf::from(path);
    candidate.is_dir().then_some(candidate)
}

/// The folder that holds a file, when that folder is on the disk.
///
/// A file that the user names in the save dialog lies outside every root, so
/// the folder of it becomes a root and the next write of the same tab passes
/// the guard.
pub fn folder_of(path: &Path) -> Option<PathBuf> {
    path.parent()
        .filter(|parent| parent.is_dir())
        .map(Path::to_path_buf)
}

/// True when the name of an entry is one the panel hides.
fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// Reads the entries of one folder, with the folders first and each group in
/// the order of its names. Hidden entries stay out.
pub fn read_folder(path: &Path) -> Result<Vec<FolderEntry>> {
    let mut entries: Vec<FolderEntry> = Vec::new();
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if is_hidden(&name) {
            continue;
        }
        // An entry whose kind cannot be read is left out, because neither a
        // read nor a walk of it can work either.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        entries.push(FolderEntry {
            name,
            path: entry.path().to_string_lossy().to_string(),
            kind: if kind.is_dir() {
                EntryKind::Folder
            } else {
                EntryKind::File
            },
        });
    }
    entries.sort_by(|left, right| {
        let group = folder_first(left.kind).cmp(&folder_first(right.kind));
        group.then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    Ok(entries)
}

/// The order of the two kinds: a folder stands above a file.
fn folder_first(kind: EntryKind) -> u8 {
    match kind {
        EntryKind::Folder => 0,
        EntryKind::File => 1,
    }
}

/// Reads the text of a file that is small enough for the editor.
pub fn read_text(path: &Path) -> Result<String> {
    let size = std::fs::metadata(path)?.len();
    if size > MAX_FILE_BYTES {
        return Err(Error::Configuration(format!(
            "The file is larger than the editor accepts. The limit is {} MB.",
            MAX_FILE_BYTES / (1024 * 1024)
        )));
    }
    Ok(std::fs::read_to_string(path)?)
}

/// Writes the text of a file through a temporary file and a rename, so a
/// write that fails leaves the file that was there as it was.
pub fn write_text(path: &Path, contents: &str) -> Result<()> {
    let mut name = path.as_os_str().to_owned();
    name.push(".part");
    let temp_path = PathBuf::from(name);
    std::fs::write(&temp_path, contents)?;
    if let Err(error) = std::fs::rename(&temp_path, path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(error.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a folder of the tests with a name of its own.
    fn temp_folder(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("sql-explorer-files-{name}"));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn a_path_under_a_root_is_accepted_and_one_outside_is_refused() {
        let root = temp_folder("guard");
        let inside = root.join("inside.sql");
        std::fs::write(&inside, "SELECT 1").unwrap();
        let outside = temp_folder("guard-other").join("outside.sql");
        std::fs::write(&outside, "SELECT 1").unwrap();

        let roots = vec![root.clone()];
        assert!(path_inside_roots(&inside, &roots).is_ok());

        let error = path_inside_roots(&outside, &roots).err().unwrap();
        assert_eq!(error.kind(), crate::error::ErrorKind::Configuration);
        assert!(error.to_string().contains("outside every folder"));

        // A path that no file holds is refused as well.
        assert!(path_inside_roots(&root.join("gone.sql"), &roots).is_err());

        // With no root at all, every path is outside.
        assert!(path_inside_roots(&inside, &[]).is_err());
    }

    #[test]
    fn a_root_that_is_gone_holds_no_path() {
        let root = temp_folder("guard-gone");
        let file = root.join("a.sql");
        std::fs::write(&file, "SELECT 1").unwrap();
        let roots = vec![root.join("nowhere"), root.clone()];

        // The first root cannot be resolved, and the walk goes on to the next.
        assert!(path_inside_roots(&file, &roots).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_that_points_out_of_a_root_is_refused() {
        let root = temp_folder("guard-link");
        let away = temp_folder("guard-away");
        let secret = away.join("secret.sql");
        std::fs::write(&secret, "SELECT 1").unwrap();
        let link = root.join("link.sql");
        std::os::unix::fs::symlink(&secret, &link).unwrap();

        let error = path_inside_roots(&link, &[root]).err().unwrap();
        assert_eq!(error.kind(), crate::error::ErrorKind::Configuration);
    }

    #[test]
    fn a_record_of_the_workspace_brings_back_a_folder_alone() {
        let root = temp_folder("record");
        let file = root.join("a.sql");
        std::fs::write(&file, "SELECT 1").unwrap();

        assert_eq!(
            root_from_record(&root.to_string_lossy()),
            Some(root.clone())
        );
        // A file is not a folder, and a path that is gone brings nothing back.
        assert_eq!(root_from_record(&file.to_string_lossy()), None);
        assert_eq!(root_from_record(&root.join("gone").to_string_lossy()), None);
        assert_eq!(root_from_record(""), None);
    }

    #[test]
    fn a_file_names_the_folder_that_holds_it() {
        let root = temp_folder("folder-of");
        let file = root.join("a.sql");

        // The file itself does not need to be there, because the dialog
        // names a path before the write.
        assert_eq!(folder_of(&file), Some(root.clone()));
        // A folder that is not on the disk names nothing.
        assert_eq!(folder_of(&root.join("gone").join("a.sql")), None);
        assert_eq!(folder_of(Path::new("a.sql")), None);
    }

    #[test]
    fn a_folder_lists_its_entries_with_the_folders_first() {
        let root = temp_folder("list");
        std::fs::write(root.join("b.sql"), "SELECT 1").unwrap();
        std::fs::write(root.join("A.sql"), "SELECT 1").unwrap();
        std::fs::write(root.join(".hidden.sql"), "SELECT 1").unwrap();
        std::fs::create_dir(root.join("zeta")).unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();

        let entries = read_folder(&root).unwrap();

        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, vec!["zeta", "A.sql", "b.sql"]);
        assert_eq!(entries[0].kind, EntryKind::Folder);
        assert_eq!(entries[1].kind, EntryKind::File);
        assert!(entries[1].path.ends_with("A.sql"));
    }

    #[test]
    fn a_folder_that_is_not_there_is_reported() {
        let root = temp_folder("list-gone");
        assert!(read_folder(&root.join("nowhere")).is_err());
    }

    #[test]
    fn a_file_is_read_and_one_that_is_too_large_is_refused() {
        let root = temp_folder("read");
        let small = root.join("small.sql");
        std::fs::write(&small, "SELECT 1").unwrap();
        assert_eq!(read_text(&small).unwrap(), "SELECT 1");

        let large = root.join("large.sql");
        std::fs::write(&large, vec![b'-'; (MAX_FILE_BYTES + 1) as usize]).unwrap();
        let error = read_text(&large).err().unwrap();
        assert_eq!(error.kind(), crate::error::ErrorKind::Configuration);
        assert!(error.to_string().contains("larger than the editor accepts"));

        assert!(read_text(&root.join("gone.sql")).is_err());
    }

    #[test]
    fn a_write_goes_through_a_temporary_file() {
        let root = temp_folder("write");
        let file = root.join("out.sql");

        write_text(&file, "SELECT 1").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "SELECT 1");
        // The temporary file is gone once the write ends.
        assert!(!root.join("out.sql.part").exists());

        // A second write takes the place of the first.
        write_text(&file, "SELECT 2").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "SELECT 2");
    }

    #[test]
    fn a_write_that_cannot_finish_leaves_no_temporary_file() {
        let root = temp_folder("write-fail");
        // The path names a folder, so the rename onto it cannot work.
        let target = root.join("busy");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("held.sql"), "SELECT 1").unwrap();

        assert!(write_text(&target, "SELECT 1").is_err());
        assert!(!root.join("busy.part").exists());
    }
}
