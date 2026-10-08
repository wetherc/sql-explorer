//! Writes an XLSX file one row at a time.
//!
//! An XLSX file is a ZIP container that holds a few XML parts. The `zip`
//! crate builds the container and this module builds the parts, so the
//! backend needs no spreadsheet library.
//!
//! Every text goes in the sheet as an inline string. That makes the file
//! larger than a shared table of strings would, and it keeps the writer to
//! one pass over the rows: a row is written and forgotten, so the memory
//! cost of an export does not grow with the number of rows.
//!
//! `frontend/src/lib/xlsx.ts` holds the same rules for the export of the
//! rows that the grid shows.

use crate::error::Result;
use serde_json::Value as JsonValue;
use std::io::{Seek, Write};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// The largest number of rows a sheet holds, the header row among them. A
/// sheet therefore holds one row of data fewer than this number.
pub const MAX_SHEET_ROWS: usize = 1_048_576;

/// The largest number of columns a sheet holds. The last column is XFD.
pub const MAX_SHEET_COLUMNS: usize = 16_384;

/// The name of the first sheet part inside the container.
#[cfg(test)]
const SHEET_PART: &str = "xl/worksheets/sheet1.xml";

/// The name of the part of the sheet with the given number, from 1.
fn sheet_part(number: usize) -> String {
    format!("xl/worksheets/sheet{number}.xml")
}

/// Writes the part that names the type of each part. Each sheet needs a
/// line of its own.
fn content_types(sheets: usize) -> String {
    let mut out = concat!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#,
        r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#,
        r#"<Default Extension="xml" ContentType="application/xml"/>"#,
        r#"<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>"#,
    )
    .to_string();
    for number in 1..=sheets {
        out.push_str(&format!(
            r#"<Override PartName="/{}" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#,
            sheet_part(number)
        ));
    }
    out.push_str("</Types>");
    out
}

const ROOT_RELATIONSHIPS: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>"#,
    "</Relationships>",
);

/// Writes the part that links the workbook to each of its sheets.
fn workbook_relationships(sheets: usize) -> String {
    let mut out = concat!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    )
    .to_string();
    for number in 1..=sheets {
        out.push_str(&format!(
            r#"<Relationship Id="rId{number}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{number}.xml"/>"#
        ));
    }
    out.push_str("</Relationships>");
    out
}

/// Names a fault of the container as a fault of the file. A writer of an
/// archive fails when the file below it fails, so the reader of the message
/// needs the words of a file and not of a format.
fn zip_fault(error: zip::result::ZipError) -> crate::error::Error {
    crate::error::Error::Io(std::io::Error::other(format!(
        "Couldn't write the Excel file: {error}"
    )))
}

/// Escapes the five characters that XML reserves.
pub fn escape_xml(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

/// Drops the characters that XML 1.0 forbids. A database can hold such a
/// character, and a spreadsheet refuses to open a file that carries one.
#[cfg(test)]
pub fn strip_forbidden_xml(text: &str) -> String {
    text.chars()
        .filter(|character| {
            !matches!(
                character,
                '\u{0}'..='\u{8}' | '\u{B}' | '\u{C}' | '\u{E}'..='\u{1F}' | '\u{FFFE}' | '\u{FFFF}'
            )
        })
        .collect()
}

/// The largest number of significant digits that Excel keeps in a number.
const EXCEL_DIGITS: usize = 15;

/// The largest number of characters, counted in UTF-16 units, that Excel
/// accepts in one cell. A longer text makes Excel repair the file.
const MAX_CELL_UNITS: usize = 32_767;

/// True when the whole text is a decimal number: an optional sign, digits
/// with at most one decimal point, and an optional exponent. A spreadsheet
/// reads such a text as a number.
pub fn is_plain_number(text: &str) -> bool {
    let body = text.strip_prefix(['+', '-']).unwrap_or(text);
    let (mantissa, exponent) = match body.find(['e', 'E']) {
        Some(at) => (&body[..at], Some(&body[at + 1..])),
        None => (body, None),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = |part: &str| part.bytes().all(|byte| byte.is_ascii_digit());
    let exponent_ok = exponent.is_none_or(|power| {
        let power = power.strip_prefix(['+', '-']).unwrap_or(power);
        !power.is_empty() && digits(power)
    });
    !(whole.is_empty() && fraction.is_empty()) && digits(whole) && digits(fraction) && exponent_ok
}

/// Gives the text of a number cell when Excel can keep the value exactly:
/// a decimal number with at most 15 significant digits whose value is
/// finite. A longer number goes in as text, because Excel would round
/// 1234567890123456789 to 1234567890123456800.
///
/// A whole part with a zero in front of other digits, as in the code
/// `00123`, also goes in as text. A database never writes a number in that
/// form, and the number cell would lose the zeros.
fn excel_number(text: &str) -> Option<String> {
    if !is_plain_number(text) {
        return None;
    }
    let mantissa = text.split(['e', 'E']).next().unwrap_or(text);
    let whole = mantissa.trim_start_matches(['+', '-']).split('.').next();
    if whole.is_some_and(|whole| whole.len() > 1 && whole.starts_with('0')) {
        return None;
    }
    let significant = mantissa
        .trim_start_matches(['+', '-'])
        .replace('.', "")
        .trim_matches('0')
        .len();
    let value = text.parse::<f64>().ok().filter(|value| value.is_finite())?;
    // The display form is the shortest decimal text that gives the same
    // value, with no exponent.
    (significant <= EXCEL_DIGITS).then(|| value.to_string())
}

/// Cuts a text to the number of characters that one cell accepts.
#[cfg(test)]
fn cell_text(text: &str) -> &str {
    let mut units = 0;
    for (at, character) in text.char_indices() {
        units += character.len_utf16();
        if units > MAX_CELL_UNITS {
            return &text[..at];
        }
    }
    text
}

/// True for a character that XML 1.0 forbids.
fn forbidden_in_xml(character: char) -> bool {
    matches!(
        character,
        '\u{0}'..='\u{8}' | '\u{B}' | '\u{C}' | '\u{E}'..='\u{1F}' | '\u{FFFE}' | '\u{FFFF}'
    )
}

/// Adds a text cell to `out` in one pass over the text: the pass drops the
/// characters that XML forbids, escapes the reserved characters and stops at
/// the bound of a cell. Returns true when the text was cut at that bound.
fn push_text_cell(out: &mut String, reference: &str, text: &str) -> bool {
    out.push_str("<c r=\"");
    out.push_str(reference);
    out.push_str("\" t=\"inlineStr\"><is><t xml:space=\"preserve\">");
    let mut units = 0;
    let mut cut = false;
    for character in text
        .chars()
        .filter(|character| !forbidden_in_xml(*character))
    {
        units += character.len_utf16();
        if units > MAX_CELL_UNITS {
            cut = true;
            break;
        }
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out.push_str("</t></is></c>");
    cut
}

/// Writes a cell that holds a text.
fn text_cell(reference: &str, text: &str) -> String {
    let mut out = String::new();
    push_text_cell(&mut out, reference, text);
    out
}

/// True when a column of this type contains numbers, so that a text value of
/// the column can go in as a number cell. The test reads the first word of
/// the type name, without a length or a precision, so `decimal(10,2)`,
/// `double precision` and `int unsigned` all count as numeric.
pub fn is_numeric_type(type_name: &str) -> bool {
    let lower = type_name.trim().to_ascii_lowercase();
    let word = lower
        .split(|character: char| character == '(' || character.is_whitespace())
        .next()
        .unwrap_or("");
    matches!(
        word,
        "tinyint"
            | "smallint"
            | "mediumint"
            | "int"
            | "integer"
            | "bigint"
            | "int2"
            | "int4"
            | "int8"
            | "decimal"
            | "dec"
            | "numeric"
            | "number"
            | "float"
            | "float4"
            | "float8"
            | "double"
            | "real"
            | "money"
            | "smallmoney"
    )
}

/// Names a column of a spreadsheet: 1 gives A, 27 gives AA.
pub fn column_name(index: usize) -> String {
    let mut rest = index;
    let mut name = String::new();
    while rest > 0 {
        let remainder = (rest - 1) % 26;
        name.insert(0, (b'A' + remainder as u8) as char);
        rest = (rest - remainder - 1) / 26;
    }
    name
}

/// Cleans a name for a sheet. A sheet name holds at most 31 characters and
/// none of the characters that Excel reserves. Excel also refuses an
/// apostrophe at the start or the end of the name, and it keeps the name
/// `History` for a sheet of its own. Excel repairs a file that breaks one
/// of these rules.
pub fn sheet_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|character| match character {
            '\\' | '/' | '?' | '*' | '[' | ']' | ':' => '_',
            other => other,
        })
        .collect();
    let cleaned = cleaned.trim();
    let cleaned = if cleaned.is_empty() {
        "Result"
    } else {
        cleaned
    };
    let mut chars: Vec<char> = cleaned.chars().take(31).collect();
    // The cut to 31 characters can put an apostrophe at the end, so the
    // ends are read after the cut.
    let last = chars.len() - 1;
    for at in [0, last] {
        if chars[at] == '\'' {
            chars[at] = '_';
        }
    }
    let cleaned: String = chars.into_iter().collect();
    if cleaned.eq_ignore_ascii_case("history") {
        format!("{cleaned}_")
    } else {
        cleaned
    }
}

/// Writes the workbook part, which names each sheet of the file in order.
fn workbook_xml(sheets: &[String]) -> String {
    let mut out = concat!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
        r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" "#,
        r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">"#,
        "<sheets>",
    )
    .to_string();
    for (index, sheet) in sheets.iter().enumerate() {
        let number = index + 1;
        out.push_str(&format!(
            r#"<sheet name="{}" sheetId="{number}" r:id="rId{number}"/>"#,
            escape_xml(sheet)
        ));
    }
    out.push_str("</sheets></workbook>");
    out
}

/// Writes one cell of the sheet. A cell that holds no value is left out of
/// the row, which is the form a spreadsheet reads as an empty cell.
///
/// A number, and a text that holds only a number, go in as a number when
/// Excel can keep the value exactly, so that `SUM` reads a DECIMAL column
/// and every PostgreSQL column of the simple protocol. Any other number goes
/// in as text. This form reads each column as a numeric column.
#[cfg(test)]
pub fn cell_xml(reference: &str, value: &JsonValue) -> String {
    let mut out = String::new();
    push_cell(&mut out, reference, value, true);
    out
}

/// Adds one cell to `out`. A text value goes in as a number only when
/// `numeric` is true, so a text column keeps a code such as `+1555` or
/// `12E3` as text. A JSON number goes in as a number in each column. Returns
/// true when a text was cut at the bound of a cell.
fn push_cell(out: &mut String, reference: &str, value: &JsonValue, numeric: bool) -> bool {
    let owned;
    let text = match value {
        JsonValue::Null => return false,
        JsonValue::Bool(flag) => {
            let digit = u8::from(*flag);
            out.push_str(&format!("<c r=\"{reference}\" t=\"b\"><v>{digit}</v></c>"));
            return false;
        }
        JsonValue::String(text) => text.as_str(),
        other => {
            owned = other.to_string();
            owned.as_str()
        }
    };
    let as_number = match value {
        JsonValue::Number(_) => true,
        JsonValue::String(_) => numeric,
        _ => false,
    };
    if as_number {
        if let Some(number) = excel_number(text) {
            out.push_str(&format!("<c r=\"{reference}\"><v>{number}</v></c>"));
            return false;
        }
    }
    push_text_cell(out, reference, text)
}

/// Writes one row of the sheet, at the given number of the row. This form
/// reads each column as a numeric column.
#[cfg(test)]
pub fn row_xml(values: &[JsonValue], number: usize) -> String {
    let mut out = format!("<row r=\"{number}\">");
    for (index, value) in values.iter().enumerate() {
        push_cell(
            &mut out,
            &format!("{}{number}", column_name(index + 1)),
            value,
            true,
        );
    }
    out.push_str("</row>");
    out
}

/// Writes the row of the column names, each as a text, so that a column
/// named `2024` keeps its name as text.
fn header_xml(columns: &[String]) -> String {
    let mut out = "<row r=\"1\">".to_string();
    for (index, name) in columns.iter().enumerate() {
        out.push_str(&text_cell(&format!("{}1", column_name(index + 1)), name));
    }
    out.push_str("</row>");
    out
}

/// Writes the sheets of a workbook into a ZIP container, one row at a time.
///
/// The sheet parts go in first, one after the other, and each sheet stays
/// open until the next sheet starts or until `finish`. The parts that name
/// the sheets go in at `finish`, when the number of sheets is known. A row
/// that arrives past the bound of a sheet is left out and reported, so the
/// caller can mark the result as truncated.
pub struct SheetWriter<W: Write + Seek> {
    zip: ZipWriter<W>,
    /// The names of the sheets, in the order of the file.
    sheets: Vec<String>,
    /// The number of rows written to the open sheet, the header row among
    /// them.
    rows: usize,
    /// The letters of each column, made once for each sheet.
    letters: Vec<String>,
    /// For each column, true when a text value can go in as a number.
    numeric: Vec<bool>,
    /// The buffer of one row, used again for each row.
    line: String,
    /// The number of text cells cut at the bound of a cell.
    cut_cells: u64,
}

impl<W: Write + Seek> SheetWriter<W> {
    /// Starts the container and writes the header row of the sheet. Each
    /// column counts as numeric, so a text that contains a number goes in as a
    /// number. `create_typed` takes the numeric flag of each column.
    #[cfg(test)]
    pub fn create(writer: W, sheet: &str, columns: &[String]) -> Result<Self> {
        Self::create_typed(writer, sheet, columns, vec![true; columns.len()])
    }

    /// Starts the container and writes the header row of the first sheet.
    /// `numeric` gives, for each column, whether a text value can go in as a
    /// number cell. A column without a flag counts as a text column.
    ///
    /// A result with more columns than a sheet holds gives an error. Excel
    /// repairs a file with a column past XFD, and a cut of the columns would
    /// drop data with no sign of it in the file.
    pub fn create_typed(
        writer: W,
        sheet: &str,
        columns: &[String],
        numeric: Vec<bool>,
    ) -> Result<Self> {
        check_columns(columns)?;
        let mut writer = Self {
            zip: ZipWriter::new(writer),
            sheets: Vec::new(),
            rows: 0,
            letters: Vec::new(),
            numeric: Vec::new(),
            line: String::new(),
            cut_cells: 0,
        };
        writer.start_sheet(sheet, columns, numeric)?;
        Ok(writer)
    }

    /// Closes the open sheet and starts the next one with its header row.
    /// The caller gives each sheet a name that no other sheet of the file
    /// has, because Excel repairs a file with two sheets of the same name.
    pub fn next_sheet(
        &mut self,
        sheet: &str,
        columns: &[String],
        numeric: Vec<bool>,
    ) -> Result<()> {
        check_columns(columns)?;
        self.zip.write_all(SHEET_END.as_bytes())?;
        self.start_sheet(sheet, columns, numeric)
    }

    /// Starts the part of a new sheet and writes its header row.
    fn start_sheet(&mut self, sheet: &str, columns: &[String], numeric: Vec<bool>) -> Result<()> {
        self.sheets.push(sheet_name(sheet));
        // A full sheet can pass 4 GB, and a ZIP entry above that size needs
        // the ZIP64 fields, which the writer adds only when it is told first.
        self.zip
            .start_file(
                sheet_part(self.sheets.len()),
                part_options().large_file(true),
            )
            .map_err(zip_fault)?;
        self.zip.write_all(
            concat!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
                r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#,
                "<sheetData>",
            )
            .as_bytes(),
        )?;
        self.zip.write_all(header_xml(columns).as_bytes())?;
        self.rows = 1;
        self.letters = (1..=columns.len()).map(column_name).collect();
        self.numeric = numeric;
        Ok(())
    }

    /// Writes one row of data. Returns false when the sheet is full, and
    /// the row is then left out.
    pub fn row(&mut self, values: &[JsonValue]) -> Result<bool> {
        if self.rows >= MAX_SHEET_ROWS {
            return Ok(false);
        }
        self.write_row(values)?;
        Ok(true)
    }

    /// The number of text cells that were cut at the bound of a cell.
    pub fn cut_cells(&self) -> u64 {
        self.cut_cells
    }

    /// Sets the count of the rows, so that a test reaches the bound of a
    /// sheet without a million rows.
    #[cfg(test)]
    pub fn set_rows(&mut self, rows: usize) {
        self.rows = rows;
    }

    fn write_row(&mut self, values: &[JsonValue]) -> Result<()> {
        use std::fmt::Write as _;
        let number = self.rows + 1;
        let line = &mut self.line;
        line.clear();
        let _ = write!(line, "<row r=\"{number}\">");
        let mut reference = String::new();
        for (index, value) in values.iter().enumerate() {
            reference.clear();
            match self.letters.get(index) {
                Some(letters) => reference.push_str(letters),
                None => reference.push_str(&column_name(index + 1)),
            }
            let _ = write!(reference, "{number}");
            let numeric = self.numeric.get(index).copied().unwrap_or(false);
            if push_cell(line, &reference, value, numeric) {
                self.cut_cells += 1;
            }
        }
        line.push_str("</row>");
        self.zip.write_all(line.as_bytes())?;
        self.rows += 1;
        Ok(())
    }

    /// Closes the open sheet, writes the parts that name the sheets, and
    /// closes the container. Gives the writer back.
    pub fn finish(mut self) -> Result<W> {
        self.zip.write_all(SHEET_END.as_bytes())?;
        let count = self.sheets.len();
        for (name, body) in [
            ("[Content_Types].xml", content_types(count)),
            ("_rels/.rels", ROOT_RELATIONSHIPS.to_string()),
            ("xl/workbook.xml", workbook_xml(&self.sheets)),
            ("xl/_rels/workbook.xml.rels", workbook_relationships(count)),
        ] {
            self.zip
                .start_file(name, part_options())
                .map_err(zip_fault)?;
            self.zip.write_all(body.as_bytes())?;
        }
        self.zip.finish().map_err(zip_fault)
    }
}

/// The text that closes the part of a sheet.
const SHEET_END: &str = "</sheetData></worksheet>";

/// The options of each part of the container.
fn part_options() -> SimpleFileOptions {
    SimpleFileOptions::default().compression_method(CompressionMethod::Deflated)
}

/// Refuses a result with more columns than a sheet allows.
fn check_columns(columns: &[String]) -> Result<()> {
    if columns.len() > MAX_SHEET_COLUMNS {
        return Err(crate::error::Error::Unsupported(format!(
            "Excel sheets allow at most {MAX_SHEET_COLUMNS} columns, but this result has {}. Export it as CSV or JSON instead.",
            columns.len()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{Cursor, Read};

    /// Reads one part out of a container that a test wrote.
    fn part_of(bytes: Vec<u8>, name: &str) -> String {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut part = archive.by_name(name).unwrap();
        let mut text = String::new();
        part.read_to_string(&mut text).unwrap();
        text
    }

    #[test]
    fn the_five_characters_of_xml_are_escaped() {
        assert_eq!(
            escape_xml(r#"a&b<c>d"e'f"#),
            "a&amp;b&lt;c&gt;d&quot;e&apos;f"
        );
        assert_eq!(escape_xml("plain"), "plain");
    }

    #[test]
    fn the_characters_that_xml_forbids_are_dropped() {
        let text = "a\u{0}b\u{8}c\u{B}d\u{C}e\u{E}f\u{1F}g";
        assert_eq!(strip_forbidden_xml(text), "abcdefg");
        assert_eq!(
            strip_forbidden_xml("a\u{FFFE}b\u{FFFF}c\u{FFFD}"),
            "abc\u{FFFD}"
        );
        // A tab, a newline and a return are allowed and stay.
        assert_eq!(strip_forbidden_xml("a\tb\nc\rd"), "a\tb\nc\rd");
    }

    #[test]
    fn a_column_takes_its_name_from_its_place() {
        assert_eq!(column_name(1), "A");
        assert_eq!(column_name(26), "Z");
        assert_eq!(column_name(27), "AA");
        assert_eq!(column_name(52), "AZ");
        assert_eq!(column_name(703), "AAA");
        assert_eq!(column_name(0), "");
    }

    #[test]
    fn a_sheet_name_holds_no_reserved_character_and_no_long_text() {
        assert_eq!(sheet_name(r"a/b\c?d*e[f]g:h"), "a_b_c_d_e_f_g_h");
        assert_eq!(sheet_name("   "), "Result");
        assert_eq!(sheet_name(""), "Result");
        assert_eq!(sheet_name(&"x".repeat(40)), "x".repeat(31));
    }

    #[test]
    fn a_sheet_name_has_no_apostrophe_at_an_end_and_is_not_history() {
        assert_eq!(sheet_name("'q'"), "_q_");
        assert_eq!(sheet_name("'"), "_");
        assert_eq!(sheet_name("it's"), "it's");
        // The cut to 31 characters can leave an apostrophe at the end.
        assert_eq!(
            sheet_name(&format!("{}'abc", "x".repeat(30))),
            format!("{}_", "x".repeat(30))
        );
        assert_eq!(sheet_name("History"), "History_");
        assert_eq!(sheet_name(" hIsToRy "), "hIsToRy_");
        assert_eq!(sheet_name("History 2"), "History 2");
    }

    #[test]
    fn a_sheet_refuses_more_columns_than_excel_holds() {
        let names: Vec<String> = (0..=MAX_SHEET_COLUMNS).map(|i| i.to_string()).collect();
        let error = SheetWriter::create(Cursor::new(Vec::new()), "Result", &names)
            .err()
            .expect("too many columns");
        assert!(matches!(error, crate::error::Error::Unsupported(_)));
        assert!(error.to_string().contains("16384"));

        let widest = &names[..MAX_SHEET_COLUMNS];
        let writer = SheetWriter::create(Cursor::new(Vec::new()), "Result", widest).unwrap();
        let sheet = part_of(writer.finish().unwrap().into_inner(), SHEET_PART);
        assert!(sheet.contains("<c r=\"XFD1\" t=\"inlineStr\">"));
    }

    #[test]
    fn each_type_of_value_takes_its_own_form_of_cell() {
        assert_eq!(cell_xml("A1", &JsonValue::Null), "");
        assert_eq!(cell_xml("A1", &json!(12.5)), "<c r=\"A1\"><v>12.5</v></c>");
        assert_eq!(
            cell_xml("B2", &json!(true)),
            "<c r=\"B2\" t=\"b\"><v>1</v></c>"
        );
        assert_eq!(
            cell_xml("B3", &json!(false)),
            "<c r=\"B3\" t=\"b\"><v>0</v></c>"
        );
        assert_eq!(
            cell_xml("C1", &json!("a<b")),
            "<c r=\"C1\" t=\"inlineStr\"><is><t xml:space=\"preserve\">a&lt;b</t></is></c>"
        );
        // A value of another type goes in as its JSON text.
        assert_eq!(
            cell_xml("D1", &json!({ "a": 1 })),
            "<c r=\"D1\" t=\"inlineStr\"><is><t xml:space=\"preserve\">{&quot;a&quot;:1}</t></is></c>"
        );
    }

    #[test]
    fn a_number_goes_in_as_a_number_when_excel_keeps_it_exactly() {
        let number = |text: &str| format!("<c r=\"A1\"><v>{text}</v></c>");
        let text = |text: &str| {
            format!("<c r=\"A1\" t=\"inlineStr\"><is><t xml:space=\"preserve\">{text}</t></is></c>")
        };
        for (value, expected) in [
            ("-5", "-5"),
            ("-10.00", "-10"),
            ("1.25", "1.25"),
            ("+.5", "0.5"),
            ("0", "0"),
            ("0.5", "0.5"),
            ("1e21", "1000000000000000000000"),
            ("123456789012345", "123456789012345"),
            ("0.000123456789012345", "0.000123456789012345"),
        ] {
            assert_eq!(cell_xml("A1", &json!(value)), number(expected), "{value}");
        }
        // Sixteen significant digits, an infinite value and a text that is
        // not a number stay text.
        for value in ["1234567890123456", "1e400", "12a", "-", "00123", "-07.5"] {
            assert_eq!(cell_xml("A1", &json!(value)), text(value), "{value}");
        }
        assert_eq!(
            cell_xml("A1", &json!(1234567890123456789_i64)),
            text("1234567890123456789")
        );
        assert_eq!(cell_xml("A1", &json!(-7)), number("-7"));
    }

    #[test]
    fn a_long_text_is_cut_to_the_bound_of_a_cell() {
        let long = "a".repeat(MAX_CELL_UNITS + 5);
        assert_eq!(cell_text(&long).len(), MAX_CELL_UNITS);
        // A character outside the basic plane counts as two units and is
        // never split.
        let wide = format!("{}\u{1F600}", "a".repeat(MAX_CELL_UNITS - 1));
        assert_eq!(cell_text(&wide), "a".repeat(MAX_CELL_UNITS - 1));
        assert_eq!(cell_text("short"), "short");
    }

    #[test]
    fn the_header_keeps_a_name_that_looks_like_a_number_as_text() {
        assert_eq!(
            header_xml(&["2024".to_string()]),
            "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t xml:space=\"preserve\">2024</t></is></c></row>"
        );
    }

    #[test]
    fn a_row_names_each_cell_by_its_column_and_its_number() {
        let row = row_xml(&[json!(1), JsonValue::Null, json!("x")], 3);
        assert_eq!(
            row,
            "<row r=\"3\"><c r=\"A3\"><v>1</v></c>\
             <c r=\"C3\" t=\"inlineStr\"><is><t xml:space=\"preserve\">x</t></is></c></row>"
        );
    }

    #[test]
    fn a_sheet_holds_its_header_and_its_rows() {
        let names = vec!["id".to_string(), "name".to_string()];
        let mut writer = SheetWriter::create(Cursor::new(Vec::new()), "Query 1", &names).unwrap();
        assert!(writer.row(&[json!(1), json!("a")]).unwrap());
        assert!(writer.row(&[json!(2), JsonValue::Null]).unwrap());
        let bytes = writer.finish().unwrap().into_inner();

        let sheet = part_of(bytes.clone(), SHEET_PART);
        assert!(sheet.starts_with(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#));
        assert!(sheet.contains(
            "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t xml:space=\"preserve\">id</t></is></c>"
        ));
        assert!(sheet.contains("<row r=\"2\"><c r=\"A2\"><v>1</v></c>"));
        assert!(sheet.contains("<row r=\"3\"><c r=\"A3\"><v>2</v></c></row>"));
        assert!(sheet.ends_with("</sheetData></worksheet>"));

        // The container holds the four static parts as well.
        let workbook = part_of(bytes.clone(), "xl/workbook.xml");
        assert!(workbook.contains(r#"<sheet name="Query 1" sheetId="1" r:id="rId1"/>"#));
        assert!(part_of(bytes.clone(), "[Content_Types].xml").contains("/xl/workbook.xml"));
        assert!(part_of(bytes.clone(), "_rels/.rels").contains("xl/workbook.xml"));
        assert!(part_of(bytes, "xl/_rels/workbook.xml.rels").contains("worksheets/sheet1.xml"));
    }

    #[test]
    fn each_sheet_gets_a_part_and_a_name_of_its_own() {
        let mut writer =
            SheetWriter::create(Cursor::new(Vec::new()), "Result 1", &["id".to_string()]).unwrap();
        writer.set_rows(MAX_SHEET_ROWS);
        assert!(!writer.row(&[json!(1)]).unwrap());
        writer
            .next_sheet("Result 2", &["name".to_string()], vec![false])
            .unwrap();
        // The next sheet starts with room for its rows again.
        assert!(writer.row(&[json!("12")]).unwrap());
        let wide: Vec<String> = (0..=MAX_SHEET_COLUMNS).map(|i| i.to_string()).collect();
        assert!(writer.next_sheet("Result 3", &wide, Vec::new()).is_err());
        let bytes = writer.finish().unwrap().into_inner();

        let second = part_of(bytes.clone(), "xl/worksheets/sheet2.xml");
        assert!(second.contains(">name</t>"));
        // The column of the second sheet is a text column.
        assert!(second.contains(
            "<row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t xml:space=\"preserve\">12</t>"
        ));
        let workbook = part_of(bytes.clone(), "xl/workbook.xml");
        assert!(workbook.contains(r#"<sheet name="Result 1" sheetId="1" r:id="rId1"/>"#));
        assert!(workbook.contains(r#"<sheet name="Result 2" sheetId="2" r:id="rId2"/>"#));
        assert!(part_of(bytes.clone(), "[Content_Types].xml").contains("/xl/worksheets/sheet2.xml"));
        let links = part_of(bytes, "xl/_rels/workbook.xml.rels");
        assert!(links.contains(r#"Id="rId2""#) && links.contains("worksheets/sheet2.xml"));
    }

    #[test]
    fn a_sheet_without_a_row_holds_its_header_alone() {
        let writer =
            SheetWriter::create(Cursor::new(Vec::new()), "Result", &["id".to_string()]).unwrap();
        let sheet = part_of(writer.finish().unwrap().into_inner(), SHEET_PART);
        assert!(sheet.contains("<sheetData><row r=\"1\">"));
        assert!(sheet.ends_with("</row></sheetData></worksheet>"));
    }

    #[test]
    fn a_row_past_the_bound_of_a_sheet_is_left_out() {
        let mut writer =
            SheetWriter::create(Cursor::new(Vec::new()), "Result", &["id".to_string()]).unwrap();
        // The header holds the first row, so the count starts at one.
        writer.rows = MAX_SHEET_ROWS - 1;
        assert!(writer.row(&[json!(1)]).unwrap());
        assert!(!writer.row(&[json!(2)]).unwrap());

        let sheet = part_of(writer.finish().unwrap().into_inner(), SHEET_PART);
        assert!(sheet.contains(&format!("<row r=\"{MAX_SHEET_ROWS}\">")));
        assert!(!sheet.contains(&format!("<row r=\"{}\">", MAX_SHEET_ROWS + 1)));
    }

    #[test]
    fn a_text_column_keeps_a_text_that_looks_like_a_number_as_text() {
        let names = vec!["phone".to_string(), "amount".to_string(), "n".to_string()];
        let mut writer =
            SheetWriter::create_typed(Cursor::new(Vec::new()), "Result", &names, vec![false, true])
                .unwrap();
        assert!(writer
            .row(&[json!("+1555"), json!("1.50"), json!("12E3")])
            .unwrap());
        assert!(writer.row(&[json!(7), json!(2), json!(3)]).unwrap());
        let sheet = part_of(writer.finish().unwrap().into_inner(), SHEET_PART);
        assert!(sheet.contains(
            "<c r=\"A2\" t=\"inlineStr\"><is><t xml:space=\"preserve\">+1555</t></is></c>"
        ));
        assert!(sheet.contains("<c r=\"B2\"><v>1.5</v></c>"));
        // A column past the flags counts as a text column.
        assert!(sheet.contains("<c r=\"C2\" t=\"inlineStr\">"));
        // A JSON number stays a number in a text column.
        assert!(sheet.contains("<c r=\"A3\"><v>7</v></c>"));
    }

    #[test]
    fn the_writer_counts_the_cut_cells_and_leaves_out_a_row_of_a_full_sheet() {
        let mut writer =
            SheetWriter::create(Cursor::new(Vec::new()), "Result", &["t".to_string()]).unwrap();
        let long = "\u{1}".to_string() + &"<".repeat(MAX_CELL_UNITS + 1);
        assert!(writer.row(&[json!(long)]).unwrap());
        assert!(writer.row(&[json!("short")]).unwrap());
        assert_eq!(writer.cut_cells(), 1);
        writer.set_rows(MAX_SHEET_ROWS);
        assert!(!writer.row(&[json!(1)]).unwrap());
        let sheet = part_of(writer.finish().unwrap().into_inner(), SHEET_PART);
        assert!(sheet.contains(&"&lt;".repeat(MAX_CELL_UNITS)));
        assert!(!sheet.contains(&"&lt;".repeat(MAX_CELL_UNITS + 1)));
        assert!(!sheet.contains('\u{1}'));
    }

    #[test]
    fn a_row_wider_than_the_header_still_names_each_cell() {
        let mut writer =
            SheetWriter::create(Cursor::new(Vec::new()), "Result", &["a".to_string()]).unwrap();
        assert!(writer.row(&[json!(1), json!("x")]).unwrap());
        let sheet = part_of(writer.finish().unwrap().into_inner(), SHEET_PART);
        assert!(sheet.contains("<c r=\"B2\" t=\"inlineStr\">"));
    }

    #[test]
    fn a_numeric_type_is_found_from_the_first_word_of_its_name() {
        for name in [
            "int",
            "DECIMAL(10,2)",
            "double precision",
            "int unsigned",
            "float8",
            "money",
        ] {
            assert!(is_numeric_type(name), "{name}");
        }
        for name in ["varchar", "text", "any", "date", "bit", ""] {
            assert!(!is_numeric_type(name), "{name}");
        }
    }
}
