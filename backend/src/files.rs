//! The commands that read and write the statement files of the user.
//!
//! The user chooses a folder through the dialog of the operating system, and
//! the backend records that folder as a root. A file that the user opens or
//! saves through a dialog becomes a grant for that one file, and its folder
//! stays out of reach. Every later command refuses a path that is neither a
//! grant nor inside a root, after it resolves the links of the path, so a
//! link inside a root cannot step out of it. A hidden entry under a root is
//! refused too, because the panel does not show it. The interface therefore
//! cannot reach a file that the user did not accept.

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
    path_accepted(path, roots, &[])
}

/// Holds a path against the folders and the single files the user accepted.
///
/// Each file of `files` is a resolved path, so a grant matches the resolved
/// target alone. A link that takes the place of a granted file later
/// resolves to a different path and is refused.
pub fn path_accepted(path: &Path, roots: &[PathBuf], files: &[PathBuf]) -> Result<PathBuf> {
    let target = resolved(path)?;
    if files.contains(&target) || inside_roots(&target, roots) {
        return Ok(target);
    }
    Err(outside_the_roots(path))
}

/// True when a resolved path lies under a root and no part of the path below
/// that root is hidden. A root that is gone from the disk holds nothing any
/// more.
fn inside_roots(target: &Path, roots: &[PathBuf]) -> bool {
    roots
        .iter()
        .filter_map(|root| resolved(root).ok())
        .any(|root| {
            target
                .strip_prefix(&root)
                .is_ok_and(|rest| !has_hidden_part(rest))
        })
}

/// True when one part of a relative path is a name that the panel hides.
fn has_hidden_part(path: &Path) -> bool {
    path.components()
        .any(|part| is_hidden(&part.as_os_str().to_string_lossy()))
}

/// The resolved path of a file that the user accepted in a dialog, for the
/// list of grants. A path that no file holds gives nothing.
pub fn grant_for(path: &Path) -> Option<PathBuf> {
    resolved(path).ok().filter(|file| file.is_file())
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
    write_bytes(path, contents.as_bytes())
}

/// Writes the bytes of a file through a temporary file and a rename. A stop
/// in the middle of the write therefore leaves the temporary file and not a
/// file that holds a part of the content.
pub fn write_bytes(path: &Path, contents: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut temp = temp_file_beside(path)?;
    temp.write_all(contents)?;
    // A failed rename drops the temporary file, which removes it.
    temp.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// Creates the temporary file for a write of `path`, in the same folder so
/// that the rename at the end stays on one disk.
///
/// The temporary file has a random name and is created only when no entry
/// of that name exists, so a link that waits in the folder cannot send the
/// write to another file. It takes the permissions of the file it replaces,
/// so a file that only its owner can read stays that way.
pub fn temp_file_beside(path: &Path) -> Result<tempfile::NamedTempFile> {
    let folder = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let mut prefix = std::ffi::OsString::from(".");
    prefix.push(path.file_name().unwrap_or_default());
    prefix.push(".");
    let mut builder = tempfile::Builder::new();
    builder.prefix(&prefix).suffix(".part");
    // A new file takes the mode that the umask of the process allows, as a
    // plain create does. The temporary file otherwise gets 0600.
    #[cfg(unix)]
    builder.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o666));
    let temp = builder.tempfile_in(folder)?;
    if let Ok(existing) = std::fs::metadata(path) {
        temp.as_file().set_permissions(existing.permissions())?;
    }
    Ok(temp)
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
    fn a_hidden_entry_under_a_root_is_refused() {
        let root = temp_folder("guard-hidden");
        std::fs::create_dir(root.join(".config")).unwrap();
        let inner = root.join(".config").join("a.sql");
        std::fs::write(&inner, "SELECT 1").unwrap();
        let dotfile = root.join(".bashrc");
        std::fs::write(&dotfile, "echo").unwrap();
        let roots = vec![root.clone()];

        assert!(path_inside_roots(&inner, &roots).is_err());
        assert!(path_inside_roots(&dotfile, &roots).is_err());
        // A root whose own path is hidden still holds the entries in it.
        let hidden_root = root.join(".config");
        assert!(path_inside_roots(&inner, &[hidden_root]).is_ok());
    }

    #[test]
    fn a_granted_file_passes_and_its_folder_does_not() {
        let folder = temp_folder("grant");
        let file = folder.join("a.sql");
        std::fs::write(&file, "SELECT 1").unwrap();
        let beside = folder.join("b.sql");
        std::fs::write(&beside, "SELECT 2").unwrap();
        let grants = vec![grant_for(&file).unwrap()];

        assert!(path_accepted(&file, &[], &grants).is_ok());
        let error = path_accepted(&beside, &[], &grants).err().unwrap();
        assert!(error.to_string().contains("outside every folder"));
        // A root still admits the paths under it.
        assert!(path_accepted(&beside, std::slice::from_ref(&folder), &grants).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_that_takes_the_place_of_a_granted_file_is_refused() {
        let folder = temp_folder("grant-link");
        let away = temp_folder("grant-link-away");
        let file = folder.join("a.sql");
        std::fs::write(&file, "SELECT 1").unwrap();
        let grants = vec![grant_for(&file).unwrap()];

        std::fs::remove_file(&file).unwrap();
        let secret = away.join("secret");
        std::fs::write(&secret, "keep").unwrap();
        std::os::unix::fs::symlink(&secret, &file).unwrap();

        assert!(path_accepted(&file, &[], &grants).is_err());
    }

    #[test]
    fn a_grant_names_a_file_that_is_on_the_disk() {
        let folder = temp_folder("grant-for");
        let file = folder.join("a.sql");
        std::fs::write(&file, "SELECT 1").unwrap();

        assert_eq!(
            grant_for(&file),
            Some(std::fs::canonicalize(&file).unwrap())
        );
        // A folder and a path that is gone give no grant.
        assert_eq!(grant_for(&folder), None);
        assert_eq!(grant_for(&folder.join("gone.sql")), None);
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

    /// The names of the temporary files that a write left in a folder.
    fn leftovers(folder: &Path) -> Vec<String> {
        std::fs::read_dir(folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .filter(|name| name.ends_with(".part"))
            .collect()
    }

    #[test]
    fn a_write_goes_through_a_temporary_file() {
        let root = temp_folder("write");
        let file = root.join("out.sql");

        write_text(&file, "SELECT 1").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "SELECT 1");
        // The temporary file is gone once the write ends.
        assert!(leftovers(&root).is_empty());

        // A second write takes the place of the first.
        write_text(&file, "SELECT 2").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "SELECT 2");
    }

    #[test]
    fn a_write_of_bytes_goes_through_a_temporary_file() {
        let root = temp_folder("write-bytes");
        let file = root.join("out.bin");

        write_bytes(&file, &[0, 1, 2, 255]).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), vec![0, 1, 2, 255]);
        assert!(leftovers(&root).is_empty());
    }

    #[test]
    fn a_write_to_a_relative_path_uses_the_current_folder() {
        let name = "sql-explorer-files-relative.sql";
        let _ = std::fs::remove_file(name);

        write_text(Path::new(name), "SELECT 1").unwrap();
        assert_eq!(std::fs::read_to_string(name).unwrap(), "SELECT 1");
        std::fs::remove_file(name).unwrap();
    }

    #[test]
    fn a_write_that_cannot_finish_leaves_no_temporary_file() {
        let root = temp_folder("write-fail");
        // The path names a folder, so the rename onto it cannot work.
        let target = root.join("busy");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("held.sql"), "SELECT 1").unwrap();

        assert!(write_text(&target, "SELECT 1").is_err());
        assert!(leftovers(&root).is_empty());
    }

    #[test]
    fn a_write_into_a_folder_that_is_gone_is_reported() {
        let root = temp_folder("write-gone");
        assert!(write_text(&root.join("nowhere").join("a.sql"), "SELECT 1").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_that_waits_beside_the_file_is_not_followed() {
        let root = temp_folder("write-link");
        let away = temp_folder("write-link-away");
        let victim = away.join("victim");
        std::fs::write(&victim, "keep").unwrap();
        std::os::unix::fs::symlink(&victim, root.join("q.sql.part")).unwrap();

        write_text(&root.join("q.sql"), "SELECT 1").unwrap();

        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
        assert_eq!(
            std::fs::read_to_string(root.join("q.sql")).unwrap(),
            "SELECT 1"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_write_keeps_the_mode_of_the_file_it_replaces() {
        use std::os::unix::fs::PermissionsExt;
        let root = temp_folder("write-mode");
        let file = root.join("secret.sql");
        std::fs::write(&file, "SELECT 1").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();

        write_text(&file, "SELECT 2").unwrap();
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);

        // A new file is readable by its owner, as a plain create makes it.
        let fresh = root.join("fresh.sql");
        write_text(&fresh, "SELECT 3").unwrap();
        let mode = std::fs::metadata(&fresh).unwrap().permissions().mode();
        assert_eq!(mode & 0o600, 0o600);
    }
}
