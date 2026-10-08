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
pub enum EntryType {
    Folder,
    File,
}

/// One entry of a folder, as the interface sees it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderEntry {
    pub name: String,
    pub path: String,
    pub entry_type: EntryType,
}

/// The number of bytes at the start of a file that the check for a binary
/// file reads.
const TEXT_CHECK_BYTES: usize = 8 * 1024;

/// Gives a function that turns a fault of the file system into an error that
/// names the path and the reason of the operating system, such as "No such
/// file or directory". The `ErrorKind` of the fault stays the same.
pub(crate) fn on_path<'a>(
    action: &'a str,
    path: &'a Path,
) -> impl FnOnce(std::io::Error) -> Error + 'a {
    move |error| {
        Error::Io(std::io::Error::new(
            error.kind(),
            format!("Couldn't {action} {}: {error}", path.display()),
        ))
    }
}

/// The name of the file at the end of a path, for a message.
fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

/// Resolves the links of a path and returns the result.
fn resolved(path: &Path) -> Result<PathBuf> {
    std::fs::canonicalize(path).map_err(on_path("find", path))
}

/// The words that name a path outside every folder the user accepted.
fn outside_the_roots(path: &Path) -> Error {
    Error::Invalid(format!(
        "The path '{}' isn't inside any folder you've opened.",
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
    for entry in std::fs::read_dir(path).map_err(on_path("read the folder", path))? {
        let entry = entry.map_err(on_path("read the folder", path))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if is_hidden(&name) {
            continue;
        }
        // An entry whose file type cannot be read is left out, because neither a
        // read nor a walk of it can work either.
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        entries.push(FolderEntry {
            name,
            path: entry.path().to_string_lossy().to_string(),
            entry_type: if file_type.is_dir() {
                EntryType::Folder
            } else {
                EntryType::File
            },
        });
    }
    entries.sort_by(|left, right| {
        let group = folder_first(left.entry_type).cmp(&folder_first(right.entry_type));
        group.then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    Ok(entries)
}

/// The order of the two types of entry: a folder stands above a file.
fn folder_first(entry_type: EntryType) -> u8 {
    match entry_type {
        EntryType::Folder => 0,
        EntryType::File => 1,
    }
}

/// Reads the text of a file that is small enough for the editor. A file with
/// a zero byte near its start is refused, because a text file has none and
/// the editor would show the bytes of an image or a program as noise.
#[cfg(test)]
pub fn read_text(path: &Path) -> Result<String> {
    Ok(read_text_file(path)?.0)
}

/// Reads the text of a file as [`read_text`] does, and gives the encoding
/// the file uses, so a later save writes the same encoding back.
pub fn read_text_file(path: &Path) -> Result<(String, TextEncoding)> {
    let size = std::fs::metadata(path)
        .map_err(on_path("read", path))?
        .len();
    if size > MAX_FILE_BYTES {
        return Err(Error::Invalid(format!(
            "{} is too large to open in the editor. The limit is {} MB.",
            file_name(path),
            MAX_FILE_BYTES / (1024 * 1024)
        )));
    }
    let bytes = std::fs::read(path).map_err(on_path("read", path))?;
    if looks_binary(&bytes) {
        return Err(Error::Invalid(format!(
            "{} doesn't look like a text file, so it wasn't opened.",
            file_name(path)
        )));
    }
    Ok(decode_with_encoding(bytes))
}

/// The encoding of a text file. The names match the ones the window sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextEncoding {
    Utf8,
    /// UTF-8 with a byte order mark at the start.
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Windows1252,
}

/// True when the first bytes of a file contain a zero byte. A UTF-16 file has a
/// zero byte in each ASCII character, so a file that starts with a UTF-16
/// byte order mark is text.
fn looks_binary(bytes: &[u8]) -> bool {
    if bytes.starts_with(b"\xFF\xFE") || bytes.starts_with(b"\xFE\xFF") {
        return false;
    }
    bytes.iter().take(TEXT_CHECK_BYTES).any(|&byte| byte == 0)
}

/// The characters of the bytes 0x80 to 0x9F in Windows-1252. The five bytes
/// that the code page leaves out keep the control character of the same
/// value, as the WHATWG decoder does.
const CP1252_HIGH: [char; 32] = [
    '\u{20AC}', '\u{81}', '\u{201A}', '\u{192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{2C6}', '\u{2030}', '\u{160}', '\u{2039}', '\u{152}', '\u{8D}', '\u{17D}', '\u{8F}',
    '\u{90}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{2DC}', '\u{2122}', '\u{161}', '\u{203A}', '\u{153}', '\u{9D}', '\u{17E}', '\u{178}',
];

/// Reads the text of the bytes of a file.
///
/// A byte order mark names UTF-8, UTF-16 LE or UTF-16 BE, and the mark does
/// not go into the text. A leading mark in the text makes the first word of
/// the statement unreadable to the checks of the backend. Bytes without a
/// mark that are not UTF-8 are read as Windows-1252, the code page in which
/// older Windows tools save a script. Every byte has a character in that
/// code page, so such a file always opens.
#[cfg(test)]
pub fn decode_text(bytes: Vec<u8>) -> String {
    decode_with_encoding(bytes).0
}

/// Reads the text of the bytes of a file as [`decode_text`] does, and gives
/// the encoding that the bytes use.
pub fn decode_with_encoding(bytes: Vec<u8>) -> (String, TextEncoding) {
    if let Some(rest) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
        return (
            String::from_utf8_lossy(rest).into_owned(),
            TextEncoding::Utf8Bom,
        );
    }
    if let Some(rest) = bytes.strip_prefix(b"\xFF\xFE") {
        return (utf16(rest, u16::from_le_bytes), TextEncoding::Utf16Le);
    }
    if let Some(rest) = bytes.strip_prefix(b"\xFE\xFF") {
        return (utf16(rest, u16::from_be_bytes), TextEncoding::Utf16Be);
    }
    match String::from_utf8(bytes) {
        Ok(text) => (text, TextEncoding::Utf8),
        Err(error) => (
            error
                .as_bytes()
                .iter()
                .map(|&byte| match byte {
                    0x80..=0x9F => CP1252_HIGH[usize::from(byte - 0x80)],
                    _ => char::from(byte),
                })
                .collect(),
            TextEncoding::Windows1252,
        ),
    }
}

/// Gives the bytes of a text in an encoding, and the encoding of the bytes.
///
/// Windows-1252 has 256 characters. A text with a character outside them is
/// written as UTF-8 with a byte order mark, so no character is lost, and the
/// answer names that encoding so the window can tell the user.
pub fn encode_text(contents: &str, encoding: TextEncoding) -> (Vec<u8>, TextEncoding) {
    let utf16 = |unit: fn(u16) -> [u8; 2], mark: &[u8]| {
        let mut bytes = mark.to_vec();
        contents
            .encode_utf16()
            .for_each(|value| bytes.extend(unit(value)));
        bytes
    };
    match encoding {
        TextEncoding::Utf8 => (contents.as_bytes().to_vec(), encoding),
        TextEncoding::Utf8Bom => {
            let mut bytes = b"\xEF\xBB\xBF".to_vec();
            bytes.extend_from_slice(contents.as_bytes());
            (bytes, encoding)
        }
        TextEncoding::Utf16Le => (utf16(u16::to_le_bytes, b"\xFF\xFE"), encoding),
        TextEncoding::Utf16Be => (utf16(u16::to_be_bytes, b"\xFE\xFF"), encoding),
        TextEncoding::Windows1252 => match contents.chars().map(cp1252_byte).collect() {
            Some(bytes) => (bytes, encoding),
            None => encode_text(contents, TextEncoding::Utf8Bom),
        },
    }
}

/// The Windows-1252 byte of one character, when the code page has it.
fn cp1252_byte(character: char) -> Option<u8> {
    let code = u32::from(character);
    if code < 0x80 || (0xA0..=0xFF).contains(&code) {
        return u8::try_from(code).ok();
    }
    CP1252_HIGH
        .iter()
        .position(|&high| high == character)
        .and_then(|index| u8::try_from(0x80 + index).ok())
}

/// Reads UTF-16 units in the byte order that `unit` gives. A last odd byte
/// and a lone surrogate become U+FFFD.
fn utf16(bytes: &[u8], unit: fn([u8; 2]) -> u16) -> String {
    let units = bytes.chunks(2).map(|pair| match pair {
        [high, low] => unit([*high, *low]),
        _ => 0xFFFD,
    });
    char::decode_utf16(units)
        .map(|decoded| decoded.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

/// Writes the text of a file through a temporary file and a rename, so a
/// write that fails leaves the file that was there as it was. The tests use
/// it to make files.
#[cfg(test)]
pub fn write_text(path: &Path, contents: &str) -> Result<()> {
    write_bytes(path, contents.as_bytes())
}

/// Writes the text of a file in an encoding through a temporary file and a
/// rename, so a write that fails leaves the file that was there as it was.
/// Gives the encoding it used. See [`encode_text`].
pub fn write_text_as(path: &Path, contents: &str, encoding: TextEncoding) -> Result<TextEncoding> {
    let (bytes, used) = encode_text(contents, encoding);
    write_bytes(path, &bytes)?;
    Ok(used)
}

/// Writes the bytes of a file through a temporary file and a rename. A stop
/// in the middle of the write therefore leaves the temporary file and not a
/// file that holds a part of the content.
pub fn write_bytes(path: &Path, contents: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut temp = temp_file_beside(path)?;
    temp.write_all(contents).map_err(on_path("save", path))?;
    // The content goes to the disk before the rename. A power loss after
    // the rename otherwise can leave an empty file at the path.
    temp.as_file().sync_all().map_err(on_path("save", path))?;
    // A failed rename drops the temporary file, which removes it.
    temp.persist(path)
        .map_err(|error| on_path("save", path)(error.error))?;
    sync_folder_of(path);
    Ok(())
}

/// The folder that holds the file at `path`. A bare file name gives the
/// current folder.
fn folder_of(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

/// Writes the entries of the folder that holds `path` to the disk, so a
/// rename into that folder stays after a power loss. Windows cannot open a
/// folder as a file, and some file systems refuse the sync of a folder. The
/// rename is then already done, so a fault here is ignored.
pub fn sync_folder_of(path: &Path) {
    #[cfg(unix)]
    if let Ok(folder) = std::fs::File::open(folder_of(path)) {
        let _ = folder.sync_all();
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Creates the temporary file for a write of `path`, in the same folder so
/// that the rename at the end stays on one disk.
///
/// The temporary file has a random name and is created only when no entry
/// of that name exists, so a link that waits in the folder cannot send the
/// write to another file. It takes the permissions of the file it replaces,
/// so a file that only its owner can read stays that way.
pub fn temp_file_beside(path: &Path) -> Result<tempfile::NamedTempFile> {
    let folder = folder_of(path);
    let mut prefix = std::ffi::OsString::from(".");
    prefix.push(path.file_name().unwrap_or_default());
    prefix.push(".");
    let mut builder = tempfile::Builder::new();
    builder.prefix(&prefix).suffix(".part");
    // A new file takes the mode that the umask of the process allows, as a
    // plain create does. The temporary file otherwise gets 0600.
    #[cfg(unix)]
    builder.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o666));
    let temp = builder.tempfile_in(folder).map_err(on_path("save", path))?;
    if let Ok(existing) = std::fs::metadata(path) {
        temp.as_file()
            .set_permissions(existing.permissions())
            .map_err(on_path("save", path))?;
    }
    Ok(temp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_encoding_round_trips() {
        let text = "SELECT 'caf\u{e9} \u{20ac}'";
        for encoding in [
            TextEncoding::Utf8,
            TextEncoding::Utf8Bom,
            TextEncoding::Utf16Le,
            TextEncoding::Utf16Be,
            TextEncoding::Windows1252,
        ] {
            let (bytes, used) = encode_text(text, encoding);
            assert_eq!(used, encoding);
            assert_eq!(decode_with_encoding(bytes), (text.to_string(), encoding));
        }
    }

    #[test]
    fn windows_1252_without_the_character_saves_as_utf8_with_a_mark() {
        let (bytes, used) = encode_text("\u{3b1}", TextEncoding::Windows1252);
        assert_eq!(used, TextEncoding::Utf8Bom);
        assert!(bytes.starts_with(b"\xEF\xBB\xBF"));
        // The five bytes that the code page leaves out keep their value.
        let (bytes, _) = encode_text("\u{81}\u{ff}", TextEncoding::Windows1252);
        assert_eq!(bytes, [0x81, 0xFF]);
    }

    #[test]
    fn a_file_is_written_and_read_in_its_encoding() {
        let path = temp_folder("encoding").join("a.sql");
        let used = write_text_as(&path, "SELECT '\u{e9}'", TextEncoding::Utf16Le).unwrap();
        assert_eq!(used, TextEncoding::Utf16Le);
        let (text, encoding) = read_text_file(&path).unwrap();
        assert_eq!(text, "SELECT '\u{e9}'");
        assert_eq!(encoding, TextEncoding::Utf16Le);
        assert_eq!(
            serde_json::to_value(TextEncoding::Utf8Bom).unwrap(),
            serde_json::json!("utf8bom")
        );
        assert_eq!(
            serde_json::to_value(TextEncoding::Windows1252).unwrap(),
            serde_json::json!("windows1252")
        );
    }

    /// Builds a folder of the tests with a name of its own. The name has the
    /// process ID and a count, so two runs of the tests at the same time
    /// and two calls with the same name use different folders.
    fn temp_folder(name: &str) -> PathBuf {
        static COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let count = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "sql-explorer-files-{name}-{}-{count}",
            std::process::id()
        ));
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
        assert_eq!(error.category(), crate::error::ErrorCategory::Invalid);
        assert!(error.to_string().contains("isn't inside any folder"));

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
        assert_eq!(error.category(), crate::error::ErrorCategory::Invalid);
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
        assert!(error.to_string().contains("isn't inside any folder"));
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
        assert_eq!(entries[0].entry_type, EntryType::Folder);
        assert_eq!(entries[1].entry_type, EntryType::File);
        assert!(entries[1].path.ends_with("A.sql"));
    }

    #[test]
    fn a_folder_that_is_not_there_is_reported() {
        let root = temp_folder("list-gone");
        let gone = root.join("nowhere");
        let error = read_folder(&gone).err().unwrap();
        assert!(error
            .to_string()
            .starts_with(&format!("Couldn't read the folder {}: ", gone.display())));
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
        assert_eq!(error.category(), crate::error::ErrorCategory::Invalid);
        assert_eq!(
            error.to_string(),
            "large.sql is too large to open in the editor. The limit is 5 MB."
        );

        let gone = root.join("gone.sql");
        let error = read_text(&gone).err().unwrap();
        assert_eq!(error.category(), crate::error::ErrorCategory::Io);
        let text = error.to_string();
        assert!(
            text.starts_with(&format!("Couldn't read {}: ", gone.display())),
            "{text}"
        );
        assert!(text.contains("No such file or directory") || text.contains("cannot find"));
    }

    #[test]
    fn a_file_with_a_zero_byte_near_its_start_is_refused() {
        let root = temp_folder("read-binary");
        let image = root.join("logo.png");
        std::fs::write(&image, b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR").unwrap();
        let error = read_text(&image).err().unwrap();
        assert_eq!(error.category(), crate::error::ErrorCategory::Invalid);
        assert_eq!(
            error.to_string(),
            "logo.png doesn't look like a text file, so it wasn't opened."
        );

        // A zero byte past the first 8 KB is not checked.
        let late = root.join("late.sql");
        let mut bytes = vec![b'-'; TEXT_CHECK_BYTES];
        bytes.push(0);
        std::fs::write(&late, &bytes).unwrap();
        assert!(read_text(&late).is_ok());

        // UTF-16 text has a zero byte in each ASCII character.
        let wide = root.join("wide.sql");
        std::fs::write(&wide, b"\xFF\xFES\x001\x00").unwrap();
        assert_eq!(read_text(&wide).unwrap(), "S1");
        let big = root.join("big.sql");
        std::fs::write(&big, b"\xFE\xFF\x00S\x001").unwrap();
        assert_eq!(read_text(&big).unwrap(), "S1");
    }

    #[test]
    fn a_path_that_is_gone_names_the_path_and_the_reason() {
        let root = temp_folder("resolve-gone");
        let gone = root.join("gone.sql");
        let error = path_inside_roots(&gone, std::slice::from_ref(&root))
            .err()
            .unwrap();
        assert_eq!(error.category(), crate::error::ErrorCategory::Io);
        assert!(error
            .to_string()
            .starts_with(&format!("Couldn't find {}: ", gone.display())));
        assert_eq!(file_name(Path::new("/")), "/");
    }

    #[test]
    fn a_byte_order_mark_names_the_encoding_and_leaves_the_text() {
        assert_eq!(decode_text(b"\xEF\xBB\xBFSELECT 1".to_vec()), "SELECT 1");
        assert_eq!(decode_text(b"\xFF\xFES\x001\x00".to_vec()), "S1");
        assert_eq!(decode_text(b"\xFE\xFF\x00S\x001".to_vec()), "S1");
        // A last odd byte and a lone surrogate become U+FFFD.
        assert_eq!(decode_text(b"\xFF\xFES\x00\x31".to_vec()), "S\u{FFFD}");
        assert_eq!(decode_text(b"\xFF\xFE\x00\xD8".to_vec()), "\u{FFFD}");
    }

    #[test]
    fn text_that_is_not_utf8_is_read_as_windows_1252() {
        assert_eq!(decode_text("SELECT 'é'".as_bytes().to_vec()), "SELECT 'é'");
        assert_eq!(decode_text(b"SELECT '\xE9'".to_vec()), "SELECT 'é'");
        assert_eq!(
            decode_text(b"\x80\x81\x9F".to_vec()),
            "\u{20AC}\u{81}\u{178}"
        );
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
    fn the_sync_of_a_folder_ignores_a_folder_that_does_not_open() {
        let root = temp_folder("sync-folder");
        assert_eq!(folder_of(&root.join("a.sql")), root.as_path());
        assert_eq!(folder_of(Path::new("a.sql")), Path::new("."));
        assert_eq!(folder_of(Path::new("/")), Path::new("."));
        sync_folder_of(&root.join("a.sql"));
        // The folder is not there, so the sync does nothing.
        sync_folder_of(&root.join("missing").join("a.sql"));
    }

    #[test]
    fn a_write_to_a_relative_path_uses_the_current_folder() {
        let name = format!("sql-explorer-files-relative-{}.sql", std::process::id());
        let _ = std::fs::remove_file(&name);

        write_text(Path::new(&name), "SELECT 1").unwrap();
        assert_eq!(std::fs::read_to_string(&name).unwrap(), "SELECT 1");
        std::fs::remove_file(&name).unwrap();
    }

    #[test]
    fn a_write_that_cannot_finish_leaves_no_temporary_file() {
        let root = temp_folder("write-fail");
        // The path names a folder, so the rename onto it cannot work.
        let target = root.join("busy");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("held.sql"), "SELECT 1").unwrap();

        let error = write_text(&target, "SELECT 1").err().unwrap();
        assert!(error
            .to_string()
            .starts_with(&format!("Couldn't save {}: ", target.display())));
        assert!(leftovers(&root).is_empty());
    }

    #[test]
    fn a_write_into_a_folder_that_is_gone_is_reported() {
        let root = temp_folder("write-gone");
        let target = root.join("nowhere").join("a.sql");
        let error = write_text(&target, "SELECT 1").err().unwrap();
        assert!(error
            .to_string()
            .starts_with(&format!("Couldn't save {}: ", target.display())));
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
