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

/// The name of the sheet part inside the container.
const SHEET_PART: &str = "xl/worksheets/sheet1.xml";

const CONTENT_TYPES: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#,
    r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#,
    r#"<Default Extension="xml" ContentType="application/xml"/>"#,
    r#"<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>"#,
    r#"<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#,
    "</Types>",
);

const ROOT_RELATIONSHIPS: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>"#,
    "</Relationships>",
);

const WORKBOOK_RELATIONSHIPS: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>"#,
    "</Relationships>",
);

/// Names a fault of the container as a fault of the file. A writer of an
/// archive fails when the file below it fails, so the reader of the message
/// needs the words of a file and not of a format.
fn zip_fault(error: zip::result::ZipError) -> crate::error::Error {
    crate::error::Error::Io(std::io::Error::other(format!(
        "The Excel file could not be written: {error}"
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

/// Writes a cell that holds a text.
fn text_cell(reference: &str, text: &str) -> String {
    let text = escape_xml(cell_text(&strip_forbidden_xml(text)));
    format!(
        "<c r=\"{reference}\" t=\"inlineStr\"><is><t xml:space=\"preserve\">{text}</t></is></c>"
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

/// Writes the workbook part, which names the one sheet of the file.
fn workbook_xml(sheet: &str) -> String {
    format!(
        concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
            r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" "#,
            r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">"#,
            r#"<sheets><sheet name="{}" sheetId="1" r:id="rId1"/></sheets>"#,
            "</workbook>",
        ),
        escape_xml(sheet)
    )
}

/// Writes one cell of the sheet. A cell that holds no value is left out of
/// the row, which is the form a spreadsheet reads as an empty cell.
///
/// A number, and a text that holds only a number, go in as a number when
/// Excel can keep the value exactly, so that `SUM` reads a DECIMAL column
/// and every PostgreSQL column of the simple protocol. Any other number goes
/// in as text.
pub fn cell_xml(reference: &str, value: &JsonValue) -> String {
    let text = match value {
        JsonValue::Null => return String::new(),
        JsonValue::Bool(flag) => {
            let digit = u8::from(*flag);
            return format!("<c r=\"{reference}\" t=\"b\"><v>{digit}</v></c>");
        }
        JsonValue::String(text) => text.clone(),
        other => other.to_string(),
    };
    if matches!(value, JsonValue::Number(_) | JsonValue::String(_)) {
        if let Some(number) = excel_number(&text) {
            return format!("<c r=\"{reference}\"><v>{number}</v></c>");
        }
    }
    text_cell(reference, &text)
}

/// Writes one row of the sheet, at the given number of the row.
pub fn row_xml(values: &[JsonValue], number: usize) -> String {
    let mut out = format!("<row r=\"{number}\">");
    for (index, value) in values.iter().enumerate() {
        out.push_str(&cell_xml(
            &format!("{}{number}", column_name(index + 1)),
            value,
        ));
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

/// Writes one sheet into a ZIP container, one row at a time.
///
/// The static parts go in first, and the sheet part stays open until
/// `finish`. A row that arrives past the bound of a sheet is left out and
/// reported, so the caller can mark the result as truncated.
pub struct SheetWriter<W: Write + Seek> {
    zip: ZipWriter<W>,
    /// The number of rows written, the header row among them.
    rows: usize,
}

impl<W: Write + Seek> SheetWriter<W> {
    /// Starts the container and writes the header row of the sheet.
    ///
    /// A result with more columns than a sheet holds gives an error. Excel
    /// repairs a file with a column past XFD, and a cut of the columns would
    /// drop data with no sign of it in the file.
    pub fn create(writer: W, sheet: &str, columns: &[String]) -> Result<Self> {
        if columns.len() > MAX_SHEET_COLUMNS {
            return Err(crate::error::Error::Unsupported(format!(
                "An Excel sheet holds at most {MAX_SHEET_COLUMNS} columns, and the result has {} columns. Export the result as CSV or JSON.",
                columns.len()
            )));
        }
        let mut zip = ZipWriter::new(writer);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for (name, body) in [
            ("[Content_Types].xml", CONTENT_TYPES.to_string()),
            ("_rels/.rels", ROOT_RELATIONSHIPS.to_string()),
            ("xl/workbook.xml", workbook_xml(&sheet_name(sheet))),
            (
                "xl/_rels/workbook.xml.rels",
                WORKBOOK_RELATIONSHIPS.to_string(),
            ),
        ] {
            zip.start_file(name, options).map_err(zip_fault)?;
            zip.write_all(body.as_bytes())?;
        }

        zip.start_file(SHEET_PART, options).map_err(zip_fault)?;
        zip.write_all(
            concat!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
                r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#,
                "<sheetData>",
            )
            .as_bytes(),
        )?;

        zip.write_all(header_xml(columns).as_bytes())?;
        Ok(Self { zip, rows: 1 })
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

    /// Sets the count of the rows, so that a test reaches the bound of a
    /// sheet without a million rows.
    #[cfg(test)]
    pub fn set_rows(&mut self, rows: usize) {
        self.rows = rows;
    }

    fn write_row(&mut self, values: &[JsonValue]) -> Result<()> {
        let number = self.rows + 1;
        self.zip.write_all(row_xml(values, number).as_bytes())?;
        self.rows += 1;
        Ok(())
    }

    /// Closes the sheet and the container, and gives the writer back.
    pub fn finish(mut self) -> Result<W> {
        self.zip.write_all(b"</sheetData></worksheet>")?;
        self.zip.finish().map_err(zip_fault)
    }
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
}
