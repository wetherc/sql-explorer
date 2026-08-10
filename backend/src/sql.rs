//! Dialect rules: identifier quoting, literal quoting, preview statements
//! and a statement splitter that respects quotes and comments.

use serde::{Deserialize, Serialize};

/// The SQL dialect of one engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Dialect {
    MsSql,
    MySql,
    Postgres,
    Sqlite,
    Athena,
}

impl Dialect {
    /// Wraps one identifier in the quotes of the dialect and doubles the
    /// closing quote inside the name. This keeps a name that holds a quote,
    /// a space or a keyword safe.
    pub fn quote_identifier(&self, name: &str) -> String {
        match self {
            Dialect::MsSql => format!("[{}]", name.replace(']', "]]")),
            Dialect::MySql => format!("`{}`", name.replace('`', "``")),
            Dialect::Postgres | Dialect::Sqlite | Dialect::Athena => {
                format!("\"{}\"", name.replace('"', "\"\""))
            }
        }
    }

    /// Joins the parts of a qualified name with a period. Empty parts are
    /// removed, so a missing schema does not make an empty component.
    pub fn quote_qualified(&self, parts: &[&str]) -> String {
        parts
            .iter()
            .filter(|part| !part.is_empty())
            .map(|part| self.quote_identifier(part))
            .collect::<Vec<_>>()
            .join(".")
    }

    /// Builds the statement that reads the first rows of one relation.
    /// MS SQL Server has no `LIMIT`, so it gets a `TOP` clause.
    pub fn preview_query(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        table: &str,
        limit: usize,
    ) -> String {
        let mut parts: Vec<&str> = Vec::new();
        match self {
            // One SQLite connection holds one database, so a qualified
            // name would name an attached database that does not exist.
            Dialect::Sqlite => {}
            // MySQL has no schema level between the database and the table.
            Dialect::MySql => {
                if let Some(database) = database {
                    parts.push(database);
                }
            }
            _ => {
                if let Some(database) = database {
                    parts.push(database);
                }
                if let Some(schema) = schema {
                    parts.push(schema);
                }
            }
        }
        parts.push(table);
        let name = self.quote_qualified(&parts);

        match self {
            Dialect::MsSql => format!("SELECT TOP {limit} * FROM {name};"),
            _ => format!("SELECT * FROM {name} LIMIT {limit};"),
        }
    }

    /// True when a backslash starts an escape inside a string literal.
    fn backslash_escapes(&self) -> bool {
        matches!(self, Dialect::MySql)
    }

    /// True when a number sign starts a comment that runs to the end of
    /// the line.
    fn hash_comments(&self) -> bool {
        matches!(self, Dialect::MySql)
    }

    /// True when brackets quote an identifier.
    fn bracket_quotes(&self) -> bool {
        matches!(self, Dialect::MsSql)
    }

    /// True when a dollar sign starts a tagged string literal.
    fn dollar_quotes(&self) -> bool {
        matches!(self, Dialect::Postgres)
    }

    /// True when a block comment can hold another block comment.
    fn nested_block_comments(&self) -> bool {
        matches!(self, Dialect::MsSql | Dialect::Postgres)
    }
}

/// Reads the first word of a statement, in small letters. The reader steps
/// over the comments and the opening brackets that can stand in front of the
/// word, so `/* note */ (SELECT 1)` gives `select`.
pub fn leading_keyword(statement: &str) -> String {
    let bytes: Vec<char> = statement.chars().collect();
    let mut index = 0;
    while index < bytes.len() {
        let current = bytes[index];
        if current.is_whitespace() || current == '(' {
            index += 1;
            continue;
        }
        if current == '-' && bytes.get(index + 1) == Some(&'-') {
            while index < bytes.len() && bytes[index] != '\n' {
                index += 1;
            }
            continue;
        }
        if current == '/' && bytes.get(index + 1) == Some(&'*') {
            index += 2;
            while index < bytes.len()
                && !(bytes[index] == '*' && bytes.get(index + 1) == Some(&'/'))
            {
                index += 1;
            }
            index += 2;
            continue;
        }
        break;
    }
    let mut word = String::new();
    while index < bytes.len() && (bytes[index].is_alphanumeric() || bytes[index] == '_') {
        word.push(bytes[index].to_ascii_lowercase());
        index += 1;
    }
    word
}

/// True when a statement only reads. The export to a file runs the statement
/// a second time, so it must refuse a statement that changes data.
pub fn only_reads(statement: &str) -> bool {
    matches!(
        leading_keyword(statement).as_str(),
        "select" | "with" | "show"
    )
}

/// Splits a script into single statements. The splitter keeps a semicolon
/// that is inside a string, an identifier or a comment, so a statement that
/// holds one of these stays whole.
///
/// The MySQL `DELIMITER` command changes the terminator for the statements
/// that follow it.
pub fn split_statements(script: &str, dialect: Dialect) -> Vec<String> {
    let chars: Vec<char> = script.chars().collect();
    let mut statements: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut delimiter: String = ";".to_string();
    let mut index = 0usize;
    let mut at_line_start = true;

    while index < chars.len() {
        let c = chars[index];

        // A `DELIMITER` command occupies a whole line and is not sent to
        // the server.
        if at_line_start && current.trim().is_empty() && dialect == Dialect::MySql {
            if let Some((new_delimiter, next_index)) = read_delimiter_command(&chars, index) {
                delimiter = new_delimiter;
                current.clear();
                index = next_index;
                at_line_start = true;
                continue;
            }
        }
        at_line_start = c == '\n';

        // A line comment runs to the end of the line.
        if c == '-' && chars.get(index + 1) == Some(&'-') {
            let end = copy_to_end_of_line(&chars, index, &mut current);
            index = end;
            continue;
        }
        if dialect.hash_comments() && c == '#' {
            let end = copy_to_end_of_line(&chars, index, &mut current);
            index = end;
            continue;
        }
        if c == '/' && chars.get(index + 1) == Some(&'*') {
            index =
                copy_block_comment(&chars, index, &mut current, dialect.nested_block_comments());
            continue;
        }

        // Quoted regions.
        if c == '\'' {
            index = copy_quoted(
                &chars,
                index,
                '\'',
                dialect.backslash_escapes(),
                &mut current,
            );
            continue;
        }
        if c == '"' {
            index = copy_quoted(
                &chars,
                index,
                '"',
                dialect.backslash_escapes(),
                &mut current,
            );
            continue;
        }
        if c == '`' && dialect == Dialect::MySql {
            index = copy_quoted(&chars, index, '`', false, &mut current);
            continue;
        }
        if c == '[' && dialect.bracket_quotes() {
            index = copy_bracket(&chars, index, &mut current);
            continue;
        }
        if c == '$' && dialect.dollar_quotes() {
            if let Some(next) = copy_dollar_quoted(&chars, index, &mut current) {
                index = next;
                continue;
            }
        }

        // The terminator ends the statement.
        if starts_with(&chars, index, &delimiter) {
            push_statement(&mut statements, &mut current);
            index += delimiter.chars().count();
            continue;
        }

        current.push(c);
        index += 1;
    }

    push_statement(&mut statements, &mut current);
    statements
}

/// Adds the buffer to the list when it holds more than blank space, then
/// clears the buffer.
fn push_statement(statements: &mut Vec<String>, current: &mut String) {
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        statements.push(trimmed.to_string());
    }
    current.clear();
}

/// True when the characters at the given position start with the needle.
fn starts_with(chars: &[char], index: usize, needle: &str) -> bool {
    let needle: Vec<char> = needle.chars().collect();
    if needle.is_empty() || index + needle.len() > chars.len() {
        return false;
    }
    chars[index..index + needle.len()] == needle[..]
}

/// Reads a `DELIMITER` command. Returns the new terminator and the position
/// after the command, or `None` when no command starts here.
fn read_delimiter_command(chars: &[char], index: usize) -> Option<(String, usize)> {
    let keyword: Vec<char> = "delimiter".chars().collect();
    if index + keyword.len() >= chars.len() {
        return None;
    }
    let found: String = chars[index..index + keyword.len()].iter().collect();
    if !found.eq_ignore_ascii_case("delimiter") {
        return None;
    }
    let mut cursor = index + keyword.len();
    if !matches!(chars.get(cursor), Some(' ') | Some('\t')) {
        return None;
    }
    while matches!(chars.get(cursor), Some(' ') | Some('\t')) {
        cursor += 1;
    }
    let mut value = String::new();
    while let Some(&c) = chars.get(cursor) {
        if c.is_whitespace() {
            break;
        }
        value.push(c);
        cursor += 1;
    }
    while let Some(&c) = chars.get(cursor) {
        cursor += 1;
        if c == '\n' {
            break;
        }
    }
    if value.is_empty() {
        None
    } else {
        Some((value, cursor))
    }
}

/// Copies the characters up to and including the end of the line.
fn copy_to_end_of_line(chars: &[char], mut index: usize, out: &mut String) -> usize {
    while let Some(&c) = chars.get(index) {
        out.push(c);
        index += 1;
        if c == '\n' {
            break;
        }
    }
    index
}

/// Copies a block comment. Counts the depth when the dialect allows a
/// comment inside a comment.
fn copy_block_comment(chars: &[char], mut index: usize, out: &mut String, nested: bool) -> usize {
    out.push('/');
    out.push('*');
    index += 2;
    let mut depth = 1usize;
    while index < chars.len() {
        if nested && chars[index] == '/' && chars.get(index + 1) == Some(&'*') {
            depth += 1;
            out.push('/');
            out.push('*');
            index += 2;
            continue;
        }
        if chars[index] == '*' && chars.get(index + 1) == Some(&'/') {
            depth -= 1;
            out.push('*');
            out.push('/');
            index += 2;
            if depth == 0 {
                break;
            }
            continue;
        }
        out.push(chars[index]);
        index += 1;
    }
    index
}

/// Copies a region that a quote character opens and closes. A doubled quote
/// stays inside the region.
fn copy_quoted(
    chars: &[char],
    mut index: usize,
    quote: char,
    backslash_escapes: bool,
    out: &mut String,
) -> usize {
    out.push(quote);
    index += 1;
    while index < chars.len() {
        let c = chars[index];
        if backslash_escapes && c == '\\' {
            out.push(c);
            index += 1;
            if let Some(&escaped) = chars.get(index) {
                out.push(escaped);
                index += 1;
            }
            continue;
        }
        if c == quote {
            if chars.get(index + 1) == Some(&quote) {
                out.push(quote);
                out.push(quote);
                index += 2;
                continue;
            }
            out.push(quote);
            index += 1;
            break;
        }
        out.push(c);
        index += 1;
    }
    index
}

/// Copies an identifier that brackets enclose. A doubled closing bracket
/// stays inside the name.
fn copy_bracket(chars: &[char], mut index: usize, out: &mut String) -> usize {
    out.push('[');
    index += 1;
    while index < chars.len() {
        let c = chars[index];
        if c == ']' {
            if chars.get(index + 1) == Some(&']') {
                out.push(']');
                out.push(']');
                index += 2;
                continue;
            }
            out.push(']');
            index += 1;
            break;
        }
        out.push(c);
        index += 1;
    }
    index
}

/// Copies a string that a dollar tag encloses. Returns `None` when the
/// dollar sign does not open a tag.
fn copy_dollar_quoted(chars: &[char], index: usize, out: &mut String) -> Option<usize> {
    let mut cursor = index + 1;
    let mut tag = String::new();
    while let Some(&c) = chars.get(cursor) {
        if c == '$' {
            break;
        }
        if !(c.is_alphanumeric() || c == '_') {
            return None;
        }
        tag.push(c);
        cursor += 1;
    }
    if chars.get(cursor) != Some(&'$') {
        return None;
    }
    let opener = format!("${tag}$");
    out.push_str(&opener);
    cursor += 1;
    while cursor < chars.len() {
        if starts_with(chars, cursor, &opener) {
            out.push_str(&opener);
            return Some(cursor + opener.chars().count());
        }
        out.push(chars[cursor]);
        cursor += 1;
    }
    Some(cursor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_use_the_quotes_of_the_dialect() {
        assert_eq!(Dialect::MsSql.quote_identifier("dbo"), "[dbo]");
        assert_eq!(Dialect::MySql.quote_identifier("db"), "`db`");
        assert_eq!(Dialect::Postgres.quote_identifier("pub"), "\"pub\"");
        assert_eq!(Dialect::Sqlite.quote_identifier("t"), "\"t\"");
        assert_eq!(Dialect::Athena.quote_identifier("t"), "\"t\"");
    }

    #[test]
    fn a_quote_inside_a_name_is_doubled() {
        assert_eq!(Dialect::MsSql.quote_identifier("a]b"), "[a]]b]");
        assert_eq!(Dialect::MySql.quote_identifier("a`b"), "`a``b`");
        assert_eq!(Dialect::Postgres.quote_identifier("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn qualified_names_drop_the_empty_parts() {
        assert_eq!(
            Dialect::MsSql.quote_qualified(&["db", "dbo", "t"]),
            "[db].[dbo].[t]"
        );
        assert_eq!(
            Dialect::MsSql.quote_qualified(&["", "dbo", "t"]),
            "[dbo].[t]"
        );
        assert_eq!(Dialect::Postgres.quote_qualified(&[]), "");
    }

    #[test]
    fn the_preview_statement_matches_the_engine() {
        assert_eq!(
            Dialect::MsSql.preview_query(Some("Sales"), Some("dbo"), "Orders", 1000),
            "SELECT TOP 1000 * FROM [Sales].[dbo].[Orders];"
        );
        assert_eq!(
            Dialect::MySql.preview_query(Some("shop"), Some("shop"), "orders", 100),
            "SELECT * FROM `shop`.`orders` LIMIT 100;"
        );
        assert_eq!(
            Dialect::Postgres.preview_query(Some("shop"), Some("public"), "orders", 50),
            "SELECT * FROM \"shop\".\"public\".\"orders\" LIMIT 50;"
        );
        assert_eq!(
            Dialect::Athena.preview_query(None, Some("logs"), "events", 10),
            "SELECT * FROM \"logs\".\"events\" LIMIT 10;"
        );
        assert_eq!(
            Dialect::Sqlite.preview_query(Some("main"), None, "events", 10),
            "SELECT * FROM \"events\" LIMIT 10;"
        );
        assert_eq!(
            Dialect::MySql.preview_query(None, None, "orders", 5),
            "SELECT * FROM `orders` LIMIT 5;"
        );
        assert_eq!(
            Dialect::Postgres.preview_query(None, None, "orders", 5),
            "SELECT * FROM \"orders\" LIMIT 5;"
        );
    }

    #[test]
    fn a_plain_script_splits_on_the_semicolon() {
        let parts = split_statements("SELECT 1; SELECT 2;", Dialect::Postgres);
        assert_eq!(parts, vec!["SELECT 1", "SELECT 2"]);
    }

    #[test]
    fn a_script_without_a_final_semicolon_keeps_the_last_statement() {
        assert_eq!(
            split_statements("SELECT 1", Dialect::Postgres),
            vec!["SELECT 1"]
        );
        assert!(split_statements("   \n  ", Dialect::Postgres).is_empty());
    }

    #[test]
    fn a_semicolon_inside_a_string_does_not_split() {
        assert_eq!(
            split_statements("SELECT 'a;b'; SELECT 2", Dialect::Postgres),
            vec!["SELECT 'a;b'", "SELECT 2"]
        );
        assert_eq!(
            split_statements("SELECT 'it''s; ok'", Dialect::Postgres),
            vec!["SELECT 'it''s; ok'"]
        );
    }

    #[test]
    fn a_backslash_escape_holds_only_for_mysql() {
        assert_eq!(
            split_statements("SELECT 'a\\'; b'", Dialect::MySql),
            vec!["SELECT 'a\\'; b'"]
        );
        assert_eq!(
            split_statements("SELECT 'a\\'", Dialect::MySql),
            vec!["SELECT 'a\\'"]
        );
        assert_eq!(
            split_statements("SELECT 'a\\'; b'", Dialect::Postgres),
            vec!["SELECT 'a\\'", "b'"]
        );
    }

    #[test]
    fn a_semicolon_inside_an_identifier_does_not_split() {
        assert_eq!(
            split_statements("SELECT \"a;b\" FROM t", Dialect::Postgres),
            vec!["SELECT \"a;b\" FROM t"]
        );
        assert_eq!(
            split_statements("SELECT `a;b` FROM t", Dialect::MySql),
            vec!["SELECT `a;b` FROM t"]
        );
        assert_eq!(
            split_statements("SELECT [a;b] FROM t", Dialect::MsSql),
            vec!["SELECT [a;b] FROM t"]
        );
        assert_eq!(
            split_statements("SELECT [a]]b] FROM t", Dialect::MsSql),
            vec!["SELECT [a]]b] FROM t"]
        );
        assert_eq!(
            split_statements("SELECT \"a\"\"b;c\"", Dialect::Postgres),
            vec!["SELECT \"a\"\"b;c\""]
        );
    }

    #[test]
    fn a_semicolon_inside_a_comment_does_not_split() {
        assert_eq!(
            split_statements("SELECT 1 -- a; b\n; SELECT 2", Dialect::Postgres),
            vec!["SELECT 1 -- a; b", "SELECT 2"]
        );
        assert_eq!(
            split_statements("SELECT 1 # a; b\n; SELECT 2", Dialect::MySql),
            vec!["SELECT 1 # a; b", "SELECT 2"]
        );
        assert_eq!(
            split_statements("SELECT /* a; b */ 1; SELECT 2", Dialect::MySql),
            vec!["SELECT /* a; b */ 1", "SELECT 2"]
        );
        assert_eq!(
            split_statements("SELECT /* a /* b; */ c */ 1", Dialect::Postgres),
            vec!["SELECT /* a /* b; */ c */ 1"]
        );
        assert_eq!(
            split_statements("SELECT 1 -- trailing", Dialect::Postgres),
            vec!["SELECT 1 -- trailing"]
        );
        assert_eq!(
            split_statements("SELECT /* never closed ; 1", Dialect::MySql),
            vec!["SELECT /* never closed ; 1"]
        );
    }

    #[test]
    fn a_dollar_tag_holds_a_semicolon_for_postgres() {
        assert_eq!(
            split_statements(
                "CREATE FUNCTION f() AS $$ BEGIN; END; $$; SELECT 1",
                Dialect::Postgres
            ),
            vec!["CREATE FUNCTION f() AS $$ BEGIN; END; $$", "SELECT 1"]
        );
        assert_eq!(
            split_statements("SELECT $tag$ a; b $tag$", Dialect::Postgres),
            vec!["SELECT $tag$ a; b $tag$"]
        );
        // A dollar sign that does not open a tag is an ordinary character.
        assert_eq!(
            split_statements("SELECT $1 + 2; SELECT 3", Dialect::Postgres),
            vec!["SELECT $1 + 2", "SELECT 3"]
        );
        assert_eq!(
            split_statements("SELECT a $ b", Dialect::Postgres),
            vec!["SELECT a $ b"]
        );
        assert_eq!(
            split_statements("SELECT $$ never closed ;", Dialect::Postgres),
            vec!["SELECT $$ never closed ;"]
        );
        // Another dialect treats the dollar sign as an ordinary character.
        assert_eq!(
            split_statements("SELECT $$a;b$$", Dialect::MySql),
            vec!["SELECT $$a", "b$$"]
        );
    }

    #[test]
    fn the_delimiter_command_changes_the_terminator() {
        let script = "DELIMITER //\nCREATE PROCEDURE p() BEGIN SELECT 1; SELECT 2; END//\nDELIMITER ;\nSELECT 3;";
        let parts = split_statements(script, Dialect::MySql);
        assert_eq!(
            parts,
            vec![
                "CREATE PROCEDURE p() BEGIN SELECT 1; SELECT 2; END",
                "SELECT 3"
            ]
        );
    }

    #[test]
    fn a_delimiter_command_needs_a_value_and_a_space() {
        // No value after the keyword, so the text stays a statement.
        assert_eq!(
            split_statements("DELIMITER \nSELECT 1;", Dialect::MySql),
            vec!["DELIMITER \nSELECT 1"]
        );
        // No separator after the keyword.
        assert_eq!(
            split_statements("DELIMITERX;", Dialect::MySql),
            vec!["DELIMITERX"]
        );
        // The keyword at the very end of the script.
        assert_eq!(
            split_statements("DELIMITER", Dialect::MySql),
            vec!["DELIMITER"]
        );
        // Another dialect does not read the command.
        assert_eq!(
            split_statements("DELIMITER //\nSELECT 1;", Dialect::Postgres),
            vec!["DELIMITER //\nSELECT 1"]
        );
    }

    #[test]
    fn the_delimiter_command_is_read_only_at_the_start_of_a_statement() {
        assert_eq!(
            split_statements("SELECT 1;\nDELIMITER //\nSELECT 2//", Dialect::MySql),
            vec!["SELECT 1", "SELECT 2"]
        );
    }

    #[test]
    fn starts_with_handles_the_edges() {
        let chars: Vec<char> = "abc".chars().collect();
        assert!(starts_with(&chars, 0, "ab"));
        assert!(!starts_with(&chars, 2, "bc"));
        assert!(!starts_with(&chars, 0, ""));
    }

    #[test]
    fn the_dialect_flags_match_the_engine() {
        assert!(Dialect::MySql.backslash_escapes());
        assert!(!Dialect::Postgres.backslash_escapes());
        assert!(Dialect::MySql.hash_comments());
        assert!(!Dialect::MsSql.hash_comments());
        assert!(Dialect::MsSql.bracket_quotes());
        assert!(!Dialect::MySql.bracket_quotes());
        assert!(Dialect::Postgres.dollar_quotes());
        assert!(!Dialect::Sqlite.dollar_quotes());
        assert!(Dialect::MsSql.nested_block_comments());
        assert!(!Dialect::Sqlite.nested_block_comments());
    }

    #[test]
    fn the_dialect_round_trips_through_json() {
        let text = serde_json::to_string(&Dialect::MsSql).unwrap();
        assert_eq!(text, "\"msSql\"");
        assert_eq!(
            serde_json::from_str::<Dialect>(&text).unwrap(),
            Dialect::MsSql
        );
    }
    #[test]
    fn the_first_word_of_a_statement_is_read_over_the_comments() {
        assert_eq!(leading_keyword("SELECT 1"), "select");
        assert_eq!(leading_keyword("  \n(select 1)"), "select");
        assert_eq!(leading_keyword("-- a note\nUPDATE t SET a = 1"), "update");
        assert_eq!(leading_keyword("/* a note */ WITH x AS ()"), "with");
        assert_eq!(leading_keyword("/* never closed"), "");
        assert_eq!(leading_keyword("   "), "");
    }

    #[test]
    fn only_a_statement_that_reads_may_be_exported() {
        assert!(only_reads("SELECT * FROM t"));
        assert!(only_reads("with x as (select 1) select * from x"));
        assert!(only_reads("SHOW TABLES"));
        assert!(!only_reads("DELETE FROM t"));
        assert!(!only_reads("EXEC do_work"));
    }
}
