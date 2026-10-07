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

    /// Wraps one value in single quotes and doubles a quote inside it, so
    /// that a name with a quote cannot break out of the literal.
    pub fn quote_literal(&self, value: &str) -> String {
        format!("'{}'", value.replace('\'', "''"))
    }

    /// Builds the quoted name of one relation. The levels that the engine
    /// holds decide which parts the name carries.
    pub fn qualified_name(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        table: &str,
    ) -> String {
        let mut parts: Vec<&str> = Vec::new();
        match self {
            // The database of a SQLite connection is the name of its file,
            // which no statement can name. The schema is `main`, `temp` or
            // the name of an attached database.
            Dialect::Sqlite => {
                if let Some(schema) = schema {
                    parts.push(schema);
                }
            }
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
        self.quote_qualified(&parts)
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
        let name = self.qualified_name(database, schema, table);

        match self {
            Dialect::MsSql => format!("SELECT TOP {limit} * FROM {name};"),
            _ => format!("SELECT * FROM {name} LIMIT {limit};"),
        }
    }

    /// True when a backslash starts an escape inside the quoted region that
    /// opens at `index`. MySQL reads every string so. PostgreSQL reads so a
    /// string with the prefix `E` alone, as in `E'it\'s'`.
    fn backslash_escapes(&self, chars: &[char], index: usize) -> bool {
        match self {
            Dialect::MySql => true,
            Dialect::Postgres => {
                chars[index] == '\''
                    && index > 0
                    && matches!(chars[index - 1], 'e' | 'E')
                    && (index < 2 || !in_a_word(chars[index - 2]))
            }
            _ => false,
        }
    }

    /// True when a dollar sign at `index` opens a tagged string literal. A
    /// dollar sign inside a name, as in `a$x$`, is part of the name.
    fn opens_dollar_quote(&self, chars: &[char], index: usize) -> bool {
        matches!(self, Dialect::Postgres) && (index == 0 || !in_a_word(chars[index - 1]))
    }

    /// True when a number sign starts a comment that runs to the end of
    /// the line.
    fn hash_comments(&self) -> bool {
        matches!(self, Dialect::MySql)
    }

    /// True when brackets quote an identifier. SQLite takes `[name]` as a
    /// quoted name for MS SQL Server compatibility.
    fn bracket_quotes(&self) -> bool {
        matches!(self, Dialect::MsSql | Dialect::Sqlite)
    }

    /// True when backticks quote an identifier. SQLite takes `` `name` ``
    /// as a quoted name for MySQL compatibility, and Athena quotes names in
    /// DDL with backticks.
    fn backtick_quotes(&self) -> bool {
        matches!(self, Dialect::MySql | Dialect::Sqlite | Dialect::Athena)
    }

    /// True when a block comment can hold another block comment.
    fn nested_block_comments(&self) -> bool {
        matches!(self, Dialect::MsSql | Dialect::Postgres)
    }

    /// True when the word `GO` on a line of its own ends a batch. MS SQL
    /// Server holds the variables and the temporary names of a batch until
    /// the batch ends, so the unit that the server compiles is the batch and
    /// not the statement.
    fn batch_separator(&self) -> bool {
        matches!(self, Dialect::MsSql)
    }
}

/// Reads the first word of a statement, in small letters. The reader steps
/// over the comments and the opening brackets that can stand in front of the
/// word, so `/* note */ (SELECT 1)` gives `select`. The dialect decides
/// whether a number sign also starts a comment, because MySQL accepts that
/// form and a script that opens with such a line still names a keyword. The
/// dialect also decides whether a block comment can hold another one, so the
/// reader ends the comment where the server ends it. MySQL runs the text of
/// a `/*!` comment, so the reader reads that text as a statement.
pub fn leading_keyword(statement: &str, dialect: Dialect) -> String {
    let chars: Vec<char> = statement.chars().collect();
    let mut skipped = String::new();
    let mut index = 0;
    while index < chars.len() {
        let current = chars[index];
        if current.is_whitespace() || current == '(' {
            index += 1;
            continue;
        }
        if (dialect.hash_comments() && current == '#') || opens_dash_comment(&chars, index, dialect)
        {
            index = copy_to_end_of_line(&chars, index, &mut skipped);
            continue;
        }
        if let Some(next) = executable_comment_body(&chars, index, dialect) {
            index = next;
            continue;
        }
        if current == '/' && chars.get(index + 1) == Some(&'*') {
            index =
                copy_block_comment(&chars, index, &mut skipped, dialect.nested_block_comments());
            continue;
        }
        break;
    }
    let mut word = String::new();
    while index < chars.len() && (chars[index].is_alphanumeric() || chars[index] == '_') {
        word.push(chars[index].to_ascii_lowercase());
        index += 1;
    }
    word
}

/// Finds a MySQL comment whose text the server runs, which is `/*!` or the
/// MariaDB form `/*M!`, each with an optional version number. Returns the
/// position of the first character of the text, or `None` when no such
/// comment starts at the given position. The closing `*/` is punctuation to
/// the readers, so they need no position for it.
fn executable_comment_body(chars: &[char], index: usize, dialect: Dialect) -> Option<usize> {
    if dialect != Dialect::MySql
        || chars.get(index) != Some(&'/')
        || chars.get(index + 1) != Some(&'*')
    {
        return None;
    }
    let mut cursor = index + 2;
    if chars.get(cursor) == Some(&'M') {
        cursor += 1;
    }
    if chars.get(cursor) != Some(&'!') {
        return None;
    }
    cursor += 1;
    while chars.get(cursor).is_some_and(|c| c.is_ascii_digit()) {
        cursor += 1;
    }
    Some(cursor)
}

/// The words that change data. A statement that has one of these words
/// outside a quoted region or a comment is refused for an export, because a
/// common table expression can contain an INSERT, an UPDATE or a DELETE
/// behind a leading WITH, and a SELECT can write through an INTO clause.
const WRITE_WORDS: [&str; 15] = [
    "insert", "update", "delete", "merge", "create", "drop", "alter", "truncate", "grant",
    "revoke", "deny", "exec", "execute", "call", "into",
];

/// The words that start a statement of MS SQL Server that changes data, the
/// server or the transaction. MS SQL Server needs no semicolon between two
/// statements, so `SELECT 1 KILL 57` is two statements, and the second one
/// has no reading keyword in front of it. Each word is reserved, so a bare
/// column name cannot use it.
const MSSQL_WRITE_WORDS: [&str; 14] = [
    "kill",
    "shutdown",
    "dbcc",
    "backup",
    "restore",
    "reconfigure",
    "begin",
    "commit",
    "rollback",
    "save",
    "bulk",
    "checkpoint",
    "writetext",
    "updatetext",
];

/// True when a script only reads. The export to a file runs the script a
/// second time, so it must refuse a script that changes data.
///
/// Each statement must start with a reading keyword, and no statement may
/// have a writing word outside a quoted region or a comment. The check reads
/// the text alone, so a function of the server that writes can still pass.
pub fn only_reads(script: &str, dialect: Dialect) -> bool {
    let statements: Vec<String> = split_batches(script, dialect)
        .iter()
        .flat_map(|batch| split_statements(&batch.text, dialect))
        .collect();
    if statements.is_empty() {
        return false;
    }
    statements.iter().all(|statement| {
        matches!(
            leading_keyword(statement, dialect).as_str(),
            "select" | "with" | "show"
        ) && !holds_a_write_word(statement, dialect)
    })
}

/// True when the statement has a writing word outside a quoted region or a
/// comment. PostgreSQL and MySQL lock the rows of a `SELECT` with
/// `FOR UPDATE`, and PostgreSQL also with `FOR NO KEY UPDATE`. That clause
/// changes no data, so the word `update` after `for` or `key` does not count
/// on these two engines.
fn holds_a_write_word(statement: &str, dialect: Dialect) -> bool {
    let row_locks = matches!(dialect, Dialect::Postgres | Dialect::MySql);
    let mut found = false;
    let mut previous = String::new();
    scan_words(statement, dialect, |word| {
        let locks_rows =
            row_locks && word == "update" && matches!(previous.as_str(), "for" | "key");
        if (WRITE_WORDS.contains(&word) && !locks_rows)
            || (dialect == Dialect::MsSql && MSSQL_WRITE_WORDS.contains(&word))
        {
            found = true;
        }
        previous = word.to_string();
    });
    found
}

/// Walks a statement and gives each bare word to `visit`, in small letters.
/// A word inside a quoted region or a comment is text and is not given.
fn scan_words(sql: &str, dialect: Dialect, visit: impl FnMut(&str)) {
    scan_code(sql, dialect, visit, |_| {});
}

/// True when the statement contains a `?` placeholder outside quoted regions
/// and comments. A `?` inside a string, as in `SELECT 'why?'`, is text.
pub fn has_placeholder(sql: &str, dialect: Dialect) -> bool {
    let mut found = false;
    scan_code(sql, dialect, |_| {}, |c| found |= c == '?');
    found
}

/// Walks a statement outside its quoted regions and comments. Each bare word
/// goes to `visit`, in small letters, and each other character of code goes
/// to `other`.
fn scan_code(
    sql: &str,
    dialect: Dialect,
    mut visit: impl FnMut(&str),
    mut other: impl FnMut(char),
) {
    let chars: Vec<char> = sql.chars().collect();
    let mut skipped = String::new();
    let mut index = 0usize;

    while index < chars.len() {
        let c = chars[index];
        // The copy helpers write the region they step over into a buffer.
        // The words do not need that text, so the buffer is emptied here.
        skipped.clear();

        if opens_dash_comment(&chars, index, dialect) {
            index = copy_to_end_of_line(&chars, index, &mut skipped);
            continue;
        }
        if dialect.hash_comments() && c == '#' {
            index = copy_to_end_of_line(&chars, index, &mut skipped);
            continue;
        }
        // MySQL runs the text of this comment, so its words count.
        if let Some(next) = executable_comment_body(&chars, index, dialect) {
            index = next;
            continue;
        }
        if c == '/' && chars.get(index + 1) == Some(&'*') {
            index =
                copy_block_comment(&chars, index, &mut skipped, dialect.nested_block_comments());
            continue;
        }
        if c == '\'' || c == '"' {
            index = copy_quoted(
                &chars,
                index,
                c,
                dialect.backslash_escapes(&chars, index),
                &mut skipped,
            );
            continue;
        }
        if c == '`' && dialect.backtick_quotes() {
            index = copy_quoted(&chars, index, '`', false, &mut skipped);
            continue;
        }
        if c == '[' && dialect.bracket_quotes() {
            index = copy_bracket(&chars, index, &mut skipped);
            continue;
        }
        if c == '$' && dialect.opens_dollar_quote(&chars, index) {
            if let Some(next) = copy_dollar_quoted(&chars, index, &mut skipped) {
                index = next;
                continue;
            }
        }

        if c.is_alphanumeric() || c == '_' {
            let mut word = String::new();
            while index < chars.len() && (chars[index].is_alphanumeric() || chars[index] == '_') {
                word.push(chars[index].to_ascii_lowercase());
                index += 1;
            }
            visit(&word);
            continue;
        }

        other(c);
        index += 1;
    }
}

/// One batch of a script, with the number of runs the script asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct Batch {
    /// The text of the batch, without the separator that ends it.
    pub text: String,
    /// How many times the batch runs. `GO 3` gives three runs.
    pub runs: u32,
}

/// Splits a script into batches. A batch is the unit that the server
/// compiles, so the statements of one batch share their variables and their
/// temporary names.
///
/// Only MS SQL Server has a separator. The word `GO` ends a batch when it
/// stands alone on a line, outside a quoted region and outside a comment. A
/// number after the word says how many times the batch runs, and a line
/// comment may follow. Every other dialect gives the whole script as one
/// batch. A script of blank space alone gives no batch.
pub fn split_batches(script: &str, dialect: Dialect) -> Vec<Batch> {
    let mut batches: Vec<Batch> = Vec::new();
    if !dialect.batch_separator() {
        push_batch(&mut batches, &mut script.to_string(), 1);
        return batches;
    }

    // MS SQL Server is the one dialect with a separator, so the walk below
    // reads the quotes and the comments of that dialect alone.
    let chars: Vec<char> = script.chars().collect();
    let mut current = String::new();
    let mut index = 0usize;

    while index < chars.len() {
        let c = chars[index];

        // The separator holds a whole line, so it is read at a line start
        // alone. Every character that the walk steps over goes into the
        // buffer, so the end of the buffer tells where the line starts.
        if current.is_empty() || current.ends_with('\n') {
            if let Some((runs, next_index)) = read_batch_separator(&chars, index) {
                push_batch(&mut batches, &mut current, runs);
                index = next_index;
                continue;
            }
        }

        if c == '-' && chars.get(index + 1) == Some(&'-') {
            index = copy_to_end_of_line(&chars, index, &mut current);
            continue;
        }
        if c == '/' && chars.get(index + 1) == Some(&'*') {
            index = copy_block_comment(&chars, index, &mut current, true);
            continue;
        }
        if c == '\'' || c == '"' {
            index = copy_quoted(&chars, index, c, false, &mut current);
            continue;
        }
        if c == '[' {
            index = copy_bracket(&chars, index, &mut current);
            continue;
        }

        current.push(c);
        index += 1;
    }

    push_batch(&mut batches, &mut current, 1);
    batches
}

/// Adds the buffer to the list of batches when it holds more than blank
/// space, then clears the buffer.
fn push_batch(batches: &mut Vec<Batch>, current: &mut String, runs: u32) {
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        batches.push(Batch {
            text: trimmed.to_string(),
            runs,
        });
    }
    current.clear();
}

/// Reads a batch separator that starts at the given position. Returns the
/// number of runs and the position after the line of the separator, or `None`
/// when the line holds something else.
fn read_batch_separator(chars: &[char], index: usize) -> Option<(u32, usize)> {
    let mut cursor = skip_blanks(chars, index);
    let word: String = chars.get(cursor..cursor + 2)?.iter().collect();
    if !word.eq_ignore_ascii_case("go") {
        return None;
    }
    cursor += 2;
    // A longer word that starts with these two letters is not the separator.
    if chars
        .get(cursor)
        .is_some_and(|&c| c.is_alphanumeric() || c == '_')
    {
        return None;
    }

    cursor = skip_blanks(chars, cursor);
    let mut digits = String::new();
    while let Some(&c) = chars.get(cursor) {
        if !c.is_ascii_digit() {
            break;
        }
        digits.push(c);
        cursor += 1;
    }
    cursor = skip_blanks(chars, cursor);
    if chars.get(cursor) == Some(&'-') && chars.get(cursor + 1) == Some(&'-') {
        while chars.get(cursor).is_some_and(|&c| c != '\n') {
            cursor += 1;
        }
    }

    match chars.get(cursor) {
        None => {}
        Some('\n') => cursor += 1,
        Some('\r') if chars.get(cursor + 1) == Some(&'\n') => cursor += 2,
        // The line carries more than the separator, so it is text.
        Some(_) => return None,
    }
    // A count that no number can hold, and the count zero, both give one run.
    let runs = digits.parse::<u32>().ok().filter(|runs| *runs > 0);
    Some((runs.unwrap_or(1), cursor))
}

/// Steps over the spaces and the tabs that start at the given position.
fn skip_blanks(chars: &[char], mut index: usize) -> usize {
    while matches!(chars.get(index), Some(' ') | Some('\t')) {
        index += 1;
    }
    index
}

/// Puts a MySQL statement between two `DELIMITER` commands when the
/// statement splitter would cut it. A trigger or an event with a
/// `BEGIN ... END` body has a semicolon after each statement of the body,
/// and a split there sends a part of the body to the server. A statement
/// that the splitter keeps whole stays bare.
pub fn within_delimiter(text: &str) -> String {
    if split_statements(text, Dialect::MySql).len() <= 1 {
        return text.to_string();
    }
    let delimiter = free_delimiter(text);
    // A text whose last line contains a `--` comment would hide a terminator on
    // that line, so the terminator then goes on a line of its own.
    let last_line = text.rsplit('\n').next().unwrap_or("");
    let joint = if last_line.contains("--") { "\n" } else { "" };
    format!("DELIMITER {delimiter}\n{text}{joint}{delimiter}\nDELIMITER ;")
}

/// The terminators that `within_delimiter` tries first, in order.
const DELIMITERS: [&str; 4] = ["$$", "$$$", "//", ";;"];

/// Finds a terminator that occurs first at the end of the text. A body can
/// contain `$$` in a string, in a comment or in a name such as `@a$$b`. A
/// name ends the statement at its `$$`. The search ignores quotes and
/// comments, so the text also runs in the `mysql` client and in other
/// tools with a different splitter. A terminator that starts inside the
/// last characters of the text also ends the text early, as `$$` after
/// `END l$`. A run of one character that is longer than the text cannot
/// occur in the text. The text cannot end with both `$` and `/`, so a run
/// of one of the two characters always fits.
fn free_delimiter(text: &str) -> String {
    let free = |delimiter: &String| {
        format!("{text}{delimiter}").find(delimiter.as_str()) == Some(text.len())
    };
    DELIMITERS
        .into_iter()
        .map(String::from)
        .chain((4..).flat_map(|length| ["$".repeat(length), "/".repeat(length)]))
        .find(free)
        .expect("a long run of one character is free")
}

/// Splits a script into single statements. The splitter keeps a semicolon
/// that is inside a string, an identifier or a comment, so a statement that
/// holds one of these stays whole.
///
/// The MySQL `DELIMITER` command changes the terminator for the statements
/// that follow it. A PostgreSQL routine with a `BEGIN ATOMIC ... END` body
/// stays whole, because each statement of the body ends with a semicolon.
pub fn split_statements(script: &str, dialect: Dialect) -> Vec<String> {
    let chars: Vec<char> = script.chars().collect();
    let mut statements: Vec<String> = Vec::new();
    let mut current = String::new();
    // The terminator is held as characters, because the walk holds each
    // character of the script against it and a build of that list for each
    // character would cost the length of the script again and again.
    let mut delimiter: Vec<char> = vec![';'];
    let mut index = 0usize;
    let mut at_line_start = true;
    // True when the statement in the buffer holds more than blank space and
    // comments. A comment above a `DELIMITER` line, as in a dump file, does
    // not hide the command.
    let mut code_seen = false;
    let mut words = BodyWords::default();

    while index < chars.len() {
        let c = chars[index];

        // A `DELIMITER` command occupies a whole line and is not sent to
        // the server. A comment in front of it is dropped with it.
        if at_line_start && !code_seen && dialect == Dialect::MySql {
            if let Some((new_delimiter, next_index)) = read_delimiter_command(&chars, index) {
                delimiter = new_delimiter.chars().collect();
                current.clear();
                index = next_index;
                at_line_start = true;
                continue;
            }
        }
        at_line_start = c == '\n';

        // A line comment runs to the end of the line, and its line break
        // starts the next line.
        if opens_dash_comment(&chars, index, dialect) || dialect.hash_comments() && c == '#' {
            index = copy_to_end_of_line(&chars, index, &mut current);
            at_line_start = chars[index - 1] == '\n';
            continue;
        }
        if c == '/' && chars.get(index + 1) == Some(&'*') {
            // MySQL runs the text of `/*!` and `/*M!` comments as code.
            code_seen |= executable_comment_body(&chars, index, dialect).is_some();
            index =
                copy_block_comment(&chars, index, &mut current, dialect.nested_block_comments());
            continue;
        }
        if !c.is_whitespace() && !starts_with(&chars, index, &delimiter) {
            code_seen = true;
        }

        // Quoted regions.
        if c == '\'' {
            index = copy_quoted(
                &chars,
                index,
                '\'',
                dialect.backslash_escapes(&chars, index),
                &mut current,
            );
            continue;
        }
        if c == '"' {
            index = copy_quoted(
                &chars,
                index,
                '"',
                dialect.backslash_escapes(&chars, index),
                &mut current,
            );
            continue;
        }
        if c == '`' && dialect.backtick_quotes() {
            index = copy_quoted(&chars, index, '`', false, &mut current);
            continue;
        }
        if c == '[' && dialect.bracket_quotes() {
            index = copy_bracket(&chars, index, &mut current);
            continue;
        }
        if c == '$' && dialect.opens_dollar_quote(&chars, index) {
            if let Some(next) = copy_dollar_quoted(&chars, index, &mut current) {
                index = next;
                continue;
            }
        }

        if dialect == Dialect::Postgres
            && (c.is_alphanumeric() || c == '_')
            && (index == 0 || !in_a_word(chars[index - 1]))
        {
            let start = index;
            while chars.get(index).is_some_and(|&c| in_a_word(c)) {
                index += 1;
            }
            let word: String = chars[start..index].iter().collect();
            words.read(&word.to_ascii_lowercase());
            current.push_str(&word);
            continue;
        }

        // The terminator ends the statement.
        if words.depth == 0 && starts_with(&chars, index, &delimiter) {
            push_statement(&mut statements, &mut current, code_seen);
            index += delimiter.len();
            code_seen = false;
            words = BodyWords::default();
            continue;
        }

        current.push(c);
        index += 1;
    }

    push_statement(&mut statements, &mut current, code_seen);
    statements
}

/// Follows the words of one PostgreSQL statement to find a routine body in
/// the form `BEGIN ATOMIC ... END`. The body can hold `CASE ... END`, so
/// each `CASE` inside the body also waits for an `END`.
#[derive(Default)]
struct BodyWords {
    /// The first word of the statement.
    first: String,
    /// The word in front of the current word.
    previous: String,
    /// The number of `BEGIN ATOMIC` and `CASE` words that have no `END` yet.
    depth: usize,
}

impl BodyWords {
    /// Reads the next bare word of the statement, in small letters.
    fn read(&mut self, word: &str) {
        if self.first.is_empty() {
            self.first = word.to_string();
        }
        if self.depth > 0 && word == "case" {
            self.depth += 1;
        } else if self.depth > 0 && word == "end" {
            self.depth -= 1;
        } else if word == "atomic" && self.previous == "begin" && self.first == "create" {
            self.depth += 1;
        }
        self.previous = word.to_string();
    }
}

/// Adds the buffer to the list when it contains code, then clears the buffer.
/// A buffer of comments and blank space alone is no statement, so a comment
/// after the last semicolon of a script does not go to the server, where an
/// engine such as MS SQL Server or SQLite can refuse an empty statement.
/// `frontend/src/lib/sql.ts` uses the same rule.
fn push_statement(statements: &mut Vec<String>, current: &mut String, code_seen: bool) {
    let trimmed = current.trim();
    if code_seen && !trimmed.is_empty() {
        statements.push(trimmed.to_string());
    }
    current.clear();
}

/// The values that the user gave for the named parameters of a statement.
/// The keys are the names without the colon.
pub type ParamValues = serde_json::Map<String, serde_json::Value>;

/// A statement whose named parameters carry the placeholders of the dialect,
/// with the names in the order the values must be sent.
#[derive(Debug, PartialEq)]
pub struct Prepared {
    pub sql: String,
    pub order: Vec<String>,
}

/// True when a character can stand inside the name of a parameter.
fn holds_a_name(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Walks a statement and gives each named parameter to `emit`, which returns
/// the text that takes its place.
///
/// A name inside a quoted region or a comment is text of the statement and it
/// stays as it is. Two colons together are the cast of PostgreSQL, so they
/// carry no name either.
fn scan_parameters(sql: &str, dialect: Dialect, mut emit: impl FnMut(&str) -> String) -> String {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = String::with_capacity(sql.len());
    let mut index = 0usize;
    // The open brackets of PostgreSQL, true for a subscript of an array. A
    // colon inside a subscript, as in `a[lo:hi]`, marks a slice and gives no
    // name. The brackets of an `ARRAY[...]` constructor contain values, so
    // `ARRAY[:ids]` names a parameter.
    let mut brackets: Vec<bool> = Vec::new();

    while index < chars.len() {
        let c = chars[index];

        if opens_dash_comment(&chars, index, dialect) {
            index = copy_to_end_of_line(&chars, index, &mut out);
            continue;
        }
        if dialect.hash_comments() && c == '#' {
            index = copy_to_end_of_line(&chars, index, &mut out);
            continue;
        }
        if c == '/' && chars.get(index + 1) == Some(&'*') {
            index = copy_block_comment(&chars, index, &mut out, dialect.nested_block_comments());
            continue;
        }
        if c == '\'' || c == '"' {
            index = copy_quoted(
                &chars,
                index,
                c,
                dialect.backslash_escapes(&chars, index),
                &mut out,
            );
            continue;
        }
        if c == '`' && dialect.backtick_quotes() {
            index = copy_quoted(&chars, index, '`', false, &mut out);
            continue;
        }
        if c == '[' && dialect.bracket_quotes() {
            index = copy_bracket(&chars, index, &mut out);
            continue;
        }
        if c == '$' && dialect.opens_dollar_quote(&chars, index) {
            if let Some(next) = copy_dollar_quoted(&chars, index, &mut out) {
                index = next;
                continue;
            }
        }

        if dialect == Dialect::Postgres {
            match c {
                '[' => brackets.push(opens_subscript(&chars, index)),
                ']' => {
                    brackets.pop();
                }
                _ => {}
            }
        }
        let in_subscript = brackets.last() == Some(&true);
        if c == ':' && !in_subscript {
            // The cast of PostgreSQL holds two colons.
            if chars.get(index + 1) == Some(&':') {
                out.push(':');
                out.push(':');
                index += 2;
                continue;
            }
        }
        if c == ':' && !in_subscript && !follows_a_value(&chars, index) {
            // A name starts with a letter or a low line, so the `:30` of a
            // time or the `:2` of a slice carries no name.
            let mut end = index + 1;
            while chars
                .get(end)
                .is_some_and(|&c| holds_a_name(c) && (end > index + 1 || !c.is_ascii_digit()))
            {
                end += 1;
            }
            if end > index + 1 {
                let name: String = chars[index + 1..end].iter().collect();
                out.push_str(&emit(&name));
                index = end;
                continue;
            }
        }

        out.push(c);
        index += 1;
    }

    out
}

/// True when the `[` at `index` opens the subscript of an array, as in
/// `a[1]` or `(f())[2]`, and false for the constructor `ARRAY[...]` and for a
/// bracket after an operator.
fn opens_subscript(chars: &[char], index: usize) -> bool {
    let before = chars[..index].iter().rposition(|c| !c.is_whitespace());
    let Some(at) = before else {
        return false;
    };
    let c = chars[at];
    if holds_a_name(c) {
        let start = chars[..=at]
            .iter()
            .rposition(|&c| !holds_a_name(c))
            .map_or(0, |at| at + 1);
        let word: String = chars[start..=at].iter().collect();
        return !word.eq_ignore_ascii_case("array");
    }
    matches!(c, ')' | ']' | '"')
}

/// True when the colon at `index` comes right after a name, a quoted name or
/// a closing bracket. Such a colon is part of the text, as in the Athena type
/// `struct<name:string>` or the `"t":x` of a JSON path, and names no
/// parameter.
fn follows_a_value(chars: &[char], index: usize) -> bool {
    index > 0 && {
        let c = chars[index - 1];
        holds_a_name(c) || matches!(c, '\'' | '"' | '`' | ']')
    }
}

/// Turns each `:name` of a statement into the placeholder of the dialect.
///
/// MS SQL Server and PostgreSQL number their placeholders, so a name that
/// stands twice keeps one number and its value travels once. Every other
/// engine marks a place with a question mark, so the value of a repeated name
/// travels once for each place.
pub fn rewrite_parameters(sql: &str, dialect: Dialect) -> Prepared {
    let numbered = matches!(dialect, Dialect::MsSql | Dialect::Postgres);
    let mut order: Vec<String> = Vec::new();
    let mut numbers: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    let sql = scan_parameters(sql, dialect, |name| {
        if !numbered {
            order.push(name.to_string());
            return "?".to_string();
        }
        let number = match numbers.get(name) {
            Some(number) => *number,
            None => {
                order.push(name.to_string());
                let number = order.len();
                numbers.insert(name.to_string(), number);
                number
            }
        };
        match dialect {
            Dialect::MsSql => format!("@P{number}"),
            _ => format!("${number}"),
        }
    });

    Prepared { sql, order }
}

/// Lists the names of the parameters of a statement, each name once, in the
/// order they stand in the text.
pub fn find_parameters(sql: &str, dialect: Dialect) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    scan_parameters(sql, dialect, |name| {
        if !names.iter().any(|held| held == name) {
            names.push(name.to_string());
        }
        String::new()
    });
    names
}

/// Puts the values of the parameters into the text of the statement.
///
/// Athena binds no value, so its parameters reach the service as literals.
/// Returns the name of the first parameter that has no value.
pub fn inline_parameters(
    sql: &str,
    dialect: Dialect,
    values: &ParamValues,
) -> std::result::Result<String, String> {
    let mut missing: Option<String> = None;
    let text = scan_parameters(sql, dialect, |name| match values.get(name) {
        Some(value) => json_literal(value, dialect),
        None => {
            if missing.is_none() {
                missing = Some(name.to_string());
            }
            String::new()
        }
    });
    match missing {
        Some(name) => Err(name),
        None => Ok(text),
    }
}

/// Writes one JSON value as a literal of SQL.
fn json_literal(value: &serde_json::Value, dialect: Dialect) -> String {
    match value {
        serde_json::Value::Null => "NULL".to_string(),
        serde_json::Value::Bool(flag) => if *flag { "true" } else { "false" }.to_string(),
        // A negative number goes in parentheses. After a minus sign, as in
        // `10-:n`, the bare text `10--1` would start a comment.
        serde_json::Value::Number(number) if number.to_string().starts_with('-') => {
            format!("({number})")
        }
        serde_json::Value::Number(number) => number.to_string(),
        serde_json::Value::String(text) => dialect.quote_literal(text),
        other => dialect.quote_literal(&other.to_string()),
    }
}

/// True when the characters at the given position start with the needle.
fn starts_with(chars: &[char], index: usize, needle: &[char]) -> bool {
    if needle.is_empty() || index + needle.len() > chars.len() {
        return false;
    }
    chars[index..index + needle.len()] == *needle
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

/// True when two dashes at the given position start a comment that runs to
/// the end of the line. MySQL reads them so only when a blank, a control
/// character or the end of the text follows them, so `5--1` is a
/// subtraction there.
fn opens_dash_comment(chars: &[char], index: usize, dialect: Dialect) -> bool {
    chars[index] == '-'
        && chars.get(index + 1) == Some(&'-')
        && (dialect != Dialect::MySql
            || chars
                .get(index + 2)
                .is_none_or(|c| c.is_whitespace() || c.is_control()))
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

/// True when a character can stand inside a bare name. PostgreSQL and MySQL
/// accept a dollar sign after the first character.
fn in_a_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
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
        // A tag starts as a name starts, so `$1$` holds the parameter `$1`.
        if !(c.is_alphabetic() || c == '_' || (!tag.is_empty() && c.is_numeric())) {
            return None;
        }
        tag.push(c);
        cursor += 1;
    }
    if chars.get(cursor) != Some(&'$') {
        return None;
    }
    let opener: Vec<char> = format!("${tag}$").chars().collect();
    out.extend(opener.iter());
    cursor += 1;
    while cursor < chars.len() {
        if starts_with(chars, cursor, &opener) {
            out.extend(opener.iter());
            return Some(cursor + opener.len());
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
    fn a_name_becomes_the_placeholder_of_the_dialect() {
        let numbered = rewrite_parameters("SELECT * FROM t WHERE a = :id", Dialect::MsSql);
        assert_eq!(numbered.sql, "SELECT * FROM t WHERE a = @P1");
        assert_eq!(numbered.order, vec!["id".to_string()]);

        let postgres = rewrite_parameters("SELECT :a, :b", Dialect::Postgres);
        assert_eq!(postgres.sql, "SELECT $1, $2");
        assert_eq!(postgres.order, vec!["a".to_string(), "b".to_string()]);

        let marks = rewrite_parameters("SELECT :a, :b", Dialect::MySql);
        assert_eq!(marks.sql, "SELECT ?, ?");
        assert_eq!(marks.order, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn a_repeated_name_keeps_one_number_and_needs_one_value() {
        let numbered = rewrite_parameters("SELECT :id, :id", Dialect::Postgres);
        assert_eq!(numbered.sql, "SELECT $1, $1");
        assert_eq!(numbered.order, vec!["id".to_string()]);

        // A question mark holds no name, so the value travels twice.
        let marks = rewrite_parameters("SELECT :id, :id", Dialect::Sqlite);
        assert_eq!(marks.sql, "SELECT ?, ?");
        assert_eq!(marks.order, vec!["id".to_string(), "id".to_string()]);
    }

    #[test]
    fn a_name_inside_text_or_a_comment_stays_as_it_is() {
        let cases = [
            ("SELECT ':id'", Dialect::MsSql),
            ("SELECT \"a:id\" FROM t", Dialect::Postgres),
            ("SELECT `a:id` FROM t", Dialect::MySql),
            ("SELECT [a:id] FROM t", Dialect::MsSql),
            ("SELECT 1 -- :id\n", Dialect::Postgres),
            ("SELECT 1 # :id\n", Dialect::MySql),
            ("SELECT /* :id */ 1", Dialect::Postgres),
            ("SELECT $body$ :id $body$", Dialect::Postgres),
            ("SELECT 1::text", Dialect::Postgres),
        ];
        for (text, dialect) in cases {
            let prepared = rewrite_parameters(text, dialect);
            assert_eq!(prepared.sql, text, "{text}");
            assert!(prepared.order.is_empty(), "{text}");
        }
    }

    #[test]
    fn a_colon_that_holds_no_name_is_left_alone() {
        let prepared = rewrite_parameters("SELECT a : b", Dialect::MsSql);
        assert_eq!(prepared.sql, "SELECT a : b");
        assert!(prepared.order.is_empty());
    }

    #[test]
    fn a_colon_right_after_a_name_or_a_quote_names_no_parameter() {
        assert_eq!(
            find_parameters(
                "CREATE TABLE t (s struct<name:string, `n`:int>) WHERE x = :id",
                Dialect::Athena
            ),
            vec!["id"]
        );
        assert!(find_parameters("SELECT 'a':b, \"c\":d, [e]:f", Dialect::MsSql).is_empty());
        assert_eq!(
            find_parameters("SELECT (:a), x=:b", Dialect::MySql),
            vec!["a", "b"]
        );
    }

    #[test]
    fn an_array_constructor_holds_parameters() {
        assert_eq!(
            find_parameters(
                "SELECT ARRAY[:ids], array [ :more ], a[:lo]",
                Dialect::Postgres
            ),
            vec!["ids", "more"]
        );
        assert_eq!(
            find_parameters(
                "SELECT (f())[1:2], \"t\"[1:2], x[1][2:3], [:n]",
                Dialect::Postgres
            ),
            vec!["n"]
        );
    }

    #[test]
    fn a_chunk_of_comments_alone_is_no_statement() {
        for dialect in [
            Dialect::MsSql,
            Dialect::Postgres,
            Dialect::Sqlite,
            Dialect::Athena,
        ] {
            assert_eq!(
                split_statements("SELECT 1; -- end\n/* note */; ;", dialect),
                vec!["SELECT 1"],
                "{dialect:?}"
            );
            assert_eq!(
                split_statements("-- a\nSELECT 2", dialect),
                vec!["-- a\nSELECT 2"]
            );
        }
    }

    #[test]
    fn sqlite_quotes_names_with_brackets_and_backticks() {
        assert_eq!(
            split_statements("SELECT [a;b], `c;d` FROM t; SELECT 2", Dialect::Sqlite),
            vec!["SELECT [a;b], `c;d` FROM t", "SELECT 2"]
        );
        assert_eq!(
            find_parameters("SELECT [:a], `:b`, :c", Dialect::Sqlite),
            vec!["c"]
        );
    }

    #[test]
    fn a_slice_of_an_array_carries_no_name() {
        let prepared = rewrite_parameters(
            "SELECT a[1:2], a[lo:hi], a[b[1]:n] WHERE x = :id",
            Dialect::Postgres,
        );
        assert_eq!(
            prepared.sql,
            "SELECT a[1:2], a[lo:hi], a[b[1]:n] WHERE x = $1"
        );
        assert_eq!(prepared.order, vec!["id".to_string()]);
        // A closing bracket without an opening one leaves the count at zero.
        assert_eq!(find_parameters("SELECT ] :a", Dialect::Postgres), vec!["a"]);
        // A name starts with a letter or a low line, on every engine.
        assert!(find_parameters("SELECT 10:30", Dialect::MySql).is_empty());
        assert_eq!(
            find_parameters("SELECT :_1, :a1", Dialect::MySql),
            vec!["_1", "a1"]
        );
        // Brackets quote a name on MS SQL Server, so they count no slice.
        assert_eq!(
            find_parameters("SELECT [a] WHERE b = :c", Dialect::MsSql),
            vec!["c"]
        );
    }

    #[test]
    fn the_names_of_a_statement_are_listed_once_and_in_order() {
        let names = find_parameters("SELECT :b, :a, :b FROM t", Dialect::MsSql);
        assert_eq!(names, vec!["b".to_string(), "a".to_string()]);
        assert!(find_parameters("SELECT 1", Dialect::MsSql).is_empty());
    }

    #[test]
    fn the_values_of_athena_reach_the_statement_as_literals() {
        let mut values = ParamValues::new();
        values.insert("name".to_string(), serde_json::json!("O'Hara"));
        values.insert("count".to_string(), serde_json::json!(12));
        values.insert("flag".to_string(), serde_json::json!(true));
        values.insert("empty".to_string(), serde_json::Value::Null);
        values.insert("list".to_string(), serde_json::json!([1, 2]));

        let text = inline_parameters(
            "SELECT :name, :count, :flag, :empty, :list",
            Dialect::Athena,
            &values,
        )
        .unwrap();
        assert_eq!(text, "SELECT 'O''Hara', 12, true, NULL, '[1,2]'");
    }

    #[test]
    fn a_placeholder_counts_only_outside_text_and_comments() {
        assert!(has_placeholder("SELECT ?", Dialect::MySql));
        assert!(!has_placeholder(
            "SELECT 'why?', `a?` -- b?\n/* c? */ # d?",
            Dialect::MySql
        ));
        assert!(has_placeholder("SELECT 'a', x = ?", Dialect::MySql));
    }

    #[test]
    fn a_negative_value_goes_in_parentheses() {
        let mut values = ParamValues::new();
        values.insert("n".to_string(), serde_json::json!(-1));
        values.insert("f".to_string(), serde_json::json!(-2.5));
        let text = inline_parameters("SELECT 10-:n, :f", Dialect::Athena, &values).unwrap();
        assert_eq!(text, "SELECT 10-(-1), (-2.5)");
    }

    #[test]
    fn a_value_that_is_missing_names_itself() {
        let values = ParamValues::new();
        assert_eq!(
            inline_parameters("SELECT :id", Dialect::Athena, &values),
            Err("id".to_string())
        );
    }

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
    fn a_literal_doubles_a_quote() {
        assert_eq!(Dialect::MsSql.quote_literal("plain"), "'plain'");
        assert_eq!(Dialect::Postgres.quote_literal("it's"), "'it''s'");
    }

    #[test]
    fn a_qualified_name_holds_the_levels_of_the_engine() {
        assert_eq!(
            Dialect::MsSql.qualified_name(Some("Sales"), Some("dbo"), "Orders"),
            "[Sales].[dbo].[Orders]"
        );
        assert_eq!(
            Dialect::MySql.qualified_name(Some("shop"), Some("shop"), "orders"),
            "`shop`.`orders`"
        );
        assert_eq!(
            Dialect::Sqlite.qualified_name(Some("app.db"), None, "events"),
            "\"events\""
        );
        assert_eq!(
            Dialect::Sqlite.qualified_name(Some("app.db"), Some("aux"), "events"),
            "\"aux\".\"events\""
        );
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
            Dialect::Sqlite.preview_query(Some("app.db"), Some("main"), "events", 10),
            "SELECT * FROM \"main\".\"events\" LIMIT 10;"
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
    fn a_postgres_string_with_the_prefix_e_reads_a_backslash_escape() {
        assert_eq!(
            split_statements("SELECT E'it\\'s; ok'; SELECT 2", Dialect::Postgres),
            vec!["SELECT E'it\\'s; ok'", "SELECT 2"]
        );
        assert_eq!(
            split_statements("SELECT e'a\\'; b'", Dialect::Postgres),
            vec!["SELECT e'a\\'; b'"]
        );
        // The letter ends a longer name, so the string has no prefix.
        assert_eq!(
            split_statements("SELECT name'a\\'; b'", Dialect::Postgres),
            vec!["SELECT name'a\\'", "b'"]
        );
        assert_eq!(
            find_parameters("SELECT E'\\' :a' , :b", Dialect::Postgres),
            vec!["b"]
        );
        assert!(!only_reads(
            "SELECT E'\\''; DELETE FROM t",
            Dialect::Postgres
        ));
    }

    #[test]
    fn a_dollar_sign_inside_a_postgres_name_opens_no_string() {
        assert_eq!(
            split_statements("SELECT a$x$ FROM t; SELECT $x$;$x$", Dialect::Postgres),
            vec!["SELECT a$x$ FROM t", "SELECT $x$;$x$"]
        );
        assert_eq!(
            split_statements("$a$;$a$; SELECT 2", Dialect::Postgres),
            vec!["$a$;$a$", "SELECT 2"]
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
    fn a_dollar_tag_starts_with_a_letter_or_a_low_line() {
        // `$1$` is the parameter `$1` and a dollar sign, so it opens no tag.
        assert_eq!(
            split_statements("SELECT $1$; SELECT 2; SELECT $1$", Dialect::Postgres),
            vec!["SELECT $1$", "SELECT 2", "SELECT $1$"]
        );
        assert_eq!(
            split_statements(
                "SELECT $_1$ a; b $_1$; SELECT $t2$ c; $t2$",
                Dialect::Postgres
            ),
            vec!["SELECT $_1$ a; b $_1$", "SELECT $t2$ c; $t2$"]
        );
        assert_eq!(
            find_parameters("SELECT $1$ :a $1$", Dialect::Postgres),
            vec!["a".to_string()]
        );
    }

    #[test]
    fn a_begin_atomic_body_stays_in_one_statement() {
        let function = "CREATE FUNCTION f(x int) RETURNS int LANGUAGE sql \
                        BEGIN ATOMIC SELECT CASE WHEN x > 0 THEN 1 ELSE 0 END; \
                        SELECT x + 1; END";
        assert_eq!(
            split_statements(&format!("{function}; SELECT 2;"), Dialect::Postgres),
            vec![function, "SELECT 2"]
        );
        let procedure = "create procedure p() begin atomic \
                         insert into t values (1); /* ; */ insert into t values (2); end";
        assert_eq!(
            split_statements(&format!("{procedure};select 3"), Dialect::Postgres),
            vec![procedure, "select 3"]
        );
        // A body that no END closes runs to the end of the script.
        assert_eq!(
            split_statements(
                "CREATE FUNCTION f() BEGIN ATOMIC SELECT 1; SELECT 2",
                Dialect::Postgres
            ),
            vec!["CREATE FUNCTION f() BEGIN ATOMIC SELECT 1; SELECT 2"]
        );
        // Outside a CREATE statement, and in another dialect, the words
        // open no body. A CASE outside a body counts nothing.
        assert_eq!(
            split_statements("SELECT 1 AS begin, 2 atomic; SELECT 3", Dialect::Postgres),
            vec!["SELECT 1 AS begin, 2 atomic", "SELECT 3"]
        );
        assert_eq!(
            split_statements("SELECT CASE WHEN a THEN 1 END; SELECT 2", Dialect::Postgres),
            vec!["SELECT CASE WHEN a THEN 1 END", "SELECT 2"]
        );
        assert_eq!(
            split_statements(
                "CREATE FUNCTION f() BEGIN ATOMIC SELECT 1; END",
                Dialect::MySql
            ),
            vec!["CREATE FUNCTION f() BEGIN ATOMIC SELECT 1", "END"]
        );
        // A word that a quote or a name holds is no keyword.
        assert_eq!(
            split_statements(
                "CREATE TABLE begin_atomic (\"begin atomic\" int); SELECT 4",
                Dialect::Postgres
            ),
            vec![
                "CREATE TABLE begin_atomic (\"begin atomic\" int)",
                "SELECT 4"
            ]
        );
        assert_eq!(
            split_statements(
                "CREATE VIEW v AS SELECT a$begin atomic; SELECT 5",
                Dialect::Postgres
            ),
            vec!["CREATE VIEW v AS SELECT a$begin atomic", "SELECT 5"]
        );
    }

    #[test]
    fn two_dashes_start_a_mysql_comment_only_before_a_blank() {
        assert_eq!(
            split_statements("SELECT 5--1; SELECT 2;", Dialect::MySql),
            vec!["SELECT 5--1", "SELECT 2"]
        );
        assert_eq!(
            split_statements(
                "SELECT 1 -- a; b\nSELECT 2;--\tc;\nSELECT 3;--",
                Dialect::MySql
            ),
            vec!["SELECT 1 -- a; b\nSELECT 2", "--\tc;\nSELECT 3"]
        );
        // Every other dialect reads two dashes as a comment at once.
        assert_eq!(
            split_statements("SELECT 5--1; SELECT 2;", Dialect::Postgres),
            vec!["SELECT 5--1; SELECT 2;"]
        );
        assert_eq!(
            find_parameters("SELECT 5--:a\n, :b", Dialect::MySql),
            vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(
            find_parameters("SELECT 5-- :a\n, :b", Dialect::MySql),
            vec!["b".to_string()]
        );
        assert_eq!(leading_keyword("--x\nSELECT 1", Dialect::MySql), "");
        assert_eq!(leading_keyword("--\nSELECT 1", Dialect::MySql), "select");
        // The words after `5--` are code, so the export refuses the script.
        assert!(!only_reads("SELECT 5--1 INTO @x", Dialect::MySql));
        assert!(only_reads("SELECT 5-- 1 INTO @x", Dialect::MySql));
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
    fn a_body_with_semicolons_goes_between_delimiter_commands() {
        let text =
            "CREATE TRIGGER t BEFORE INSERT ON o FOR EACH ROW BEGIN SET @a = 1; SET @b = 2; END";
        let wrapped = within_delimiter(text);
        assert_eq!(wrapped, format!("DELIMITER $$\n{text}$$\nDELIMITER ;"));
        assert_eq!(split_statements(&wrapped, Dialect::MySql), vec![text]);
        // A text that the splitter keeps whole needs no command.
        let simple = "CREATE TRIGGER t BEFORE INSERT ON o FOR EACH ROW SET @a = ';'";
        assert_eq!(within_delimiter(simple), simple);
        // A text that ends in a comment gets the terminator on a new line.
        let commented = format!("{text} -- done");
        let wrapped = within_delimiter(&commented);
        assert_eq!(
            wrapped,
            format!("DELIMITER $$\n{commented}\n$$\nDELIMITER ;")
        );
        assert_eq!(split_statements(&wrapped, Dialect::MySql), vec![commented]);
    }

    #[test]
    fn a_body_that_contains_the_terminator_gets_another_one() {
        let body = |inner: &str| {
            format!("CREATE EVENT e ON SCHEDULE EVERY 1 DAY DO BEGIN {inner}; SELECT 2; END")
        };
        for (inner, delimiter) in [
            ("SELECT '$$'", "$$$"),
            ("SELECT 1 /* $$ */", "$$$"),
            ("SET @a$$b = 1", "$$$"),
            ("SELECT '$$$'", "//"),
            ("SELECT '$$$', '//'", ";;"),
            ("SELECT '$$$', '//', ';;'", "$$$$"),
            ("SELECT '$$$$$', '//', ';;'", "////"),
        ] {
            let text = body(inner);
            let wrapped = within_delimiter(&text);
            assert_eq!(
                wrapped,
                format!("DELIMITER {delimiter}\n{text}{delimiter}\nDELIMITER ;")
            );
            assert_eq!(split_statements(&wrapped, Dialect::MySql), vec![text]);
        }
    }

    #[test]
    fn a_body_that_ends_with_a_part_of_the_terminator_gets_another_one() {
        // A label can end with a dollar sign, and `$$` after it would end
        // the text one character early.
        let text = "CREATE EVENT e ON SCHEDULE EVERY 1 DAY DO l$: BEGIN SELECT 1; SELECT 2; END l$";
        let wrapped = within_delimiter(text);
        assert_eq!(wrapped, format!("DELIMITER //\n{text}//\nDELIMITER ;"));
        assert_eq!(split_statements(&wrapped, Dialect::MySql), vec![text]);
        // No run of dollar signs fits after a final dollar sign, so a run of
        // slashes follows.
        let text = "CREATE EVENT e ON SCHEDULE EVERY 1 DAY DO l$: BEGIN SELECT '//', ';;'; END l$";
        assert_eq!(free_delimiter(text), "////");
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
        // Text in front of the word on the same statement keeps it as text.
        assert_eq!(
            split_statements("SELECT 1\nDELIMITER //\nSELECT 2;", Dialect::MySql),
            vec!["SELECT 1\nDELIMITER //\nSELECT 2"]
        );
    }

    #[test]
    fn a_comment_in_front_of_the_delimiter_command_does_not_hide_it() {
        let script =
            "-- make p\nDELIMITER $$\nCREATE PROCEDURE p() BEGIN SELECT 1; END$$\nDELIMITER ;";
        assert_eq!(
            split_statements(script, Dialect::MySql),
            vec!["CREATE PROCEDURE p() BEGIN SELECT 1; END"]
        );
        let script = "# one\n/* two */\n  \nDELIMITER //\nSELECT 1; SELECT 2//";
        assert_eq!(
            split_statements(script, Dialect::MySql),
            vec!["SELECT 1; SELECT 2"]
        );
        // A comment at the end of the script is no statement.
        assert_eq!(
            split_statements("SELECT 1; -- end", Dialect::MySql),
            vec!["SELECT 1"]
        );
        // MySQL runs the text of an executable comment, so the word after it
        // belongs to that statement.
        assert_eq!(
            split_statements(
                "/*!40101 SET x = 1 */\nDELIMITER //\nSELECT 1;",
                Dialect::MySql
            ),
            vec!["/*!40101 SET x = 1 */\nDELIMITER //\nSELECT 1"]
        );
    }

    /// The text of each batch, which is what most of the tests below check.
    fn batch_texts(script: &str, dialect: Dialect) -> Vec<String> {
        split_batches(script, dialect)
            .into_iter()
            .map(|batch| batch.text)
            .collect()
    }

    #[test]
    fn the_word_go_ends_a_batch_of_ms_sql_server() {
        assert_eq!(
            batch_texts(
                "DECLARE @x int = 1;\nSELECT @x;\nGO\nSELECT 2;",
                Dialect::MsSql
            ),
            vec!["DECLARE @x int = 1;\nSELECT @x;", "SELECT 2;"]
        );
        // The word carries any mix of capitals, and blank space may stand
        // around it.
        assert_eq!(
            batch_texts("SELECT 1;\n  gO \t\nSELECT 2;", Dialect::MsSql),
            vec!["SELECT 1;", "SELECT 2;"]
        );
        // A separator at the start and at the end of a script gives no empty
        // batch.
        assert_eq!(
            batch_texts("GO\nSELECT 1;\nGO\n", Dialect::MsSql),
            vec!["SELECT 1;"]
        );
        // A separator on the last line, with no line end behind it.
        assert_eq!(
            batch_texts("SELECT 1;\r\nGO", Dialect::MsSql),
            vec!["SELECT 1;"]
        );
        // A line that ends with a return and a line feed.
        assert_eq!(
            batch_texts("SELECT 1;\r\nGO\r\nSELECT 2;", Dialect::MsSql),
            vec!["SELECT 1;", "SELECT 2;"]
        );
        assert!(batch_texts("  \n GO \n ", Dialect::MsSql).is_empty());
    }

    #[test]
    fn a_count_after_the_word_go_says_how_many_runs_the_batch_takes() {
        assert_eq!(
            split_batches("SELECT 1;\nGO 3\n", Dialect::MsSql),
            vec![Batch {
                text: "SELECT 1;".to_string(),
                runs: 3
            }]
        );
        // A count of zero and a count that no number can hold give one run.
        assert_eq!(
            split_batches("SELECT 1;\nGO 0\n", Dialect::MsSql)[0].runs,
            1
        );
        assert_eq!(
            split_batches("SELECT 1;\nGO 99999999999\n", Dialect::MsSql)[0].runs,
            1
        );
        // A comment may follow the separator.
        assert_eq!(
            split_batches("SELECT 1;\nGO 2 -- twice\nSELECT 2;", Dialect::MsSql),
            vec![
                Batch {
                    text: "SELECT 1;".to_string(),
                    runs: 2
                },
                Batch {
                    text: "SELECT 2;".to_string(),
                    runs: 1
                }
            ]
        );
    }

    #[test]
    fn a_line_that_holds_more_than_the_separator_is_text() {
        for script in [
            "SELECT 1;\nGOTO done\n",
            "SELECT 1;\nGO_1\n",
            "SELECT 1;\nGO SELECT 2;\n",
            "SELECT 1; GO\n",
            "SELECT 1;\nGO 2 3\n",
        ] {
            assert_eq!(batch_texts(script, Dialect::MsSql).len(), 1, "{script}");
        }
    }

    #[test]
    fn a_separator_inside_a_quote_or_a_comment_does_not_end_a_batch() {
        for script in [
            "SELECT 'a\nGO\nb';",
            "SELECT \"a\nGO\nb\";",
            "SELECT [a\nGO\nb];",
            "SELECT 1 -- GO\n;",
            "SELECT /* a\nGO\n */ 1;",
        ] {
            assert_eq!(batch_texts(script, Dialect::MsSql).len(), 1, "{script}");
        }
    }

    #[test]
    fn a_dialect_without_a_separator_gives_the_whole_script() {
        assert_eq!(
            batch_texts("SELECT 1;\nGO\nSELECT 2;", Dialect::Postgres),
            vec!["SELECT 1;\nGO\nSELECT 2;"]
        );
        assert!(batch_texts("   ", Dialect::Postgres).is_empty());
    }

    #[test]
    fn a_script_with_a_separator_can_still_be_exported() {
        assert!(only_reads("SELECT 1;\nGO\nSELECT 2;", Dialect::MsSql));
        assert!(!only_reads("SELECT 1;\nGO\nDELETE FROM t;", Dialect::MsSql));
        assert!(!only_reads("GO\n", Dialect::MsSql));
    }

    #[test]
    fn starts_with_handles_the_edges() {
        let chars: Vec<char> = "abc".chars().collect();
        assert!(starts_with(&chars, 0, &['a', 'b']));
        assert!(!starts_with(&chars, 2, &['b', 'c']));
        assert!(!starts_with(&chars, 0, &[]));
    }

    #[test]
    fn the_dialect_flags_match_the_engine() {
        let plain: Vec<char> = "'a'".chars().collect();
        assert!(Dialect::MySql.backslash_escapes(&plain, 0));
        assert!(!Dialect::Postgres.backslash_escapes(&plain, 0));
        assert!(!Dialect::Sqlite.backslash_escapes(&plain, 0));
        assert!(Dialect::MySql.hash_comments());
        assert!(!Dialect::MsSql.hash_comments());
        assert!(Dialect::MsSql.bracket_quotes());
        assert!(!Dialect::MySql.bracket_quotes());
        assert!(Dialect::Postgres.opens_dollar_quote(&['$'], 0));
        assert!(!Dialect::Sqlite.opens_dollar_quote(&['$'], 0));
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
        let any = Dialect::Postgres;
        assert_eq!(leading_keyword("SELECT 1", any), "select");
        assert_eq!(leading_keyword("  \n(select 1)", any), "select");
        assert_eq!(
            leading_keyword("-- a note\nUPDATE t SET a = 1", any),
            "update"
        );
        assert_eq!(leading_keyword("/* a note */ WITH x AS ()", any), "with");
        assert_eq!(leading_keyword("/* never closed", any), "");
        assert_eq!(leading_keyword("   ", any), "");
    }

    #[test]
    fn a_number_sign_comment_hides_no_keyword_in_mysql() {
        assert_eq!(
            leading_keyword("# a note\nSELECT 1", Dialect::MySql),
            "select"
        );
        assert_eq!(leading_keyword("# a note", Dialect::MySql), "");
        // The sign starts no comment in the other dialects, so the reader
        // stops at it and finds no word.
        assert_eq!(leading_keyword("# a note\nSELECT 1", Dialect::Postgres), "");
        assert!(only_reads("# a note\nSELECT 1", Dialect::MySql));
    }

    #[test]
    fn only_a_statement_that_reads_may_be_exported() {
        assert!(only_reads("SELECT * FROM t", Dialect::Postgres));
        assert!(only_reads(
            "with x as (select 1) select * from x",
            Dialect::Postgres
        ));
        assert!(only_reads("SHOW TABLES", Dialect::MySql));
        assert!(!only_reads("DELETE FROM t", Dialect::Postgres));
        assert!(!only_reads("EXEC do_work", Dialect::MsSql));
        assert!(!only_reads("   ", Dialect::Postgres));
    }

    #[test]
    fn a_writing_word_behind_a_reading_keyword_is_refused() {
        // PostgreSQL runs data changes inside a common table expression.
        assert!(!only_reads(
            "WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d",
            Dialect::Postgres
        ));
        // MS SQL Server puts data changes after the WITH clause.
        assert!(!only_reads(
            "WITH x AS (SELECT 1 AS a) UPDATE t SET a = 1",
            Dialect::MsSql
        ));
        assert!(!only_reads(
            "WITH x AS (SELECT 1 AS a) MERGE INTO t USING x ON 1 = 1",
            Dialect::MsSql
        ));
        // SELECT INTO writes a new relation.
        assert!(!only_reads("SELECT * INTO t2 FROM t", Dialect::MsSql));
        // A script refuses when one of its statements writes.
        assert!(!only_reads("SELECT 1; DELETE FROM t", Dialect::Postgres));
    }

    #[test]
    fn a_second_statement_of_ms_sql_server_without_a_semicolon_is_refused() {
        for script in [
            "SELECT 1\nKILL 57",
            "SELECT 1 SHUTDOWN",
            "SELECT 1 DBCC SHRINKFILE(1)",
            "SELECT 1 BACKUP DATABASE d TO DISK = 'x'",
            "SELECT 1 RESTORE DATABASE d FROM DISK = 'x'",
            "SELECT 1 RECONFIGURE",
            "SELECT 1 BEGIN TRAN",
            "SELECT 1 COMMIT",
            "SELECT 1 ROLLBACK",
            "SELECT 1 SAVE TRAN s",
            "SELECT 1 BULK INSERT t FROM 'x'",
            "SELECT 1 CHECKPOINT",
            "SELECT 1 WRITETEXT t.c @p 'x'",
            "SELECT 1 UPDATETEXT t.c @p 0 0 'x'",
        ] {
            assert!(!only_reads(script, Dialect::MsSql), "{script}");
        }
        // The same words are names on the other engines.
        assert!(only_reads("SELECT backup, save FROM t", Dialect::Postgres));
        assert!(only_reads("SELECT [kill] FROM t", Dialect::MsSql));
    }

    #[test]
    fn a_nested_comment_hides_no_keyword() {
        assert_eq!(
            leading_keyword("/* /* */ SELECT */ COPY t TO STDOUT", Dialect::Postgres),
            "copy"
        );
        assert!(!only_reads(
            "/* /* */ SELECT */ COPY t TO PROGRAM 'x'",
            Dialect::Postgres
        ));
        assert_eq!(
            leading_keyword("/* /* */ SELECT */ KILL 57", Dialect::MsSql),
            "kill"
        );
        // MySQL ends a comment at the first close, so the rest is a statement.
        assert_eq!(
            leading_keyword("/* /* */ SELECT 1", Dialect::MySql),
            "select"
        );
    }

    #[test]
    fn mysql_reads_the_text_of_an_executable_comment() {
        assert!(!only_reads(
            "SELECT * FROM t /*! INTO OUTFILE '/tmp/x' */",
            Dialect::MySql
        ));
        assert!(!only_reads(
            "SELECT * FROM t /*!50100 INTO OUTFILE '/tmp/x' */",
            Dialect::MySql
        ));
        assert!(!only_reads(
            "SELECT * FROM t /*M!100100 INTO OUTFILE '/tmp/x' */",
            Dialect::MySql
        ));
        assert_eq!(
            leading_keyword("/*!40101 SET x = 1 */", Dialect::MySql),
            "set"
        );
        assert_eq!(
            leading_keyword("/*M! DELETE FROM t */", Dialect::MySql),
            "delete"
        );
        assert!(only_reads("/*! SELECT 1 */", Dialect::MySql));
        // A plain comment and an optimizer hint stay comments.
        assert!(only_reads("SELECT /*+ delete */ 1", Dialect::MySql));
        assert!(only_reads("SELECT /*M delete */ 1", Dialect::MySql));
        // The other engines read the form as a plain comment.
        assert!(only_reads(
            "SELECT * FROM t /*! INTO OUTFILE 'x' */",
            Dialect::Postgres
        ));
    }

    #[test]
    fn a_row_lock_of_a_select_only_reads() {
        assert!(only_reads("SELECT * FROM t FOR UPDATE", Dialect::Postgres));
        assert!(only_reads(
            "SELECT * FROM t FOR NO KEY UPDATE OF t",
            Dialect::Postgres
        ));
        assert!(only_reads("SELECT * FROM t FOR UPDATE", Dialect::MySql));
        // MS SQL Server has no such clause, so the word still counts there.
        assert!(!only_reads("SELECT * FROM t FOR UPDATE", Dialect::MsSql));
        assert!(!only_reads(
            "WITH d AS (UPDATE t SET a = 1 RETURNING *) SELECT * FROM d FOR UPDATE",
            Dialect::Postgres
        ));
    }

    #[test]
    fn a_writing_word_inside_text_or_a_comment_still_reads() {
        assert!(only_reads(
            "SELECT * FROM t WHERE action = 'delete'",
            Dialect::Postgres
        ));
        assert!(only_reads("SELECT \"delete\" FROM t", Dialect::Postgres));
        assert!(only_reads("SELECT [delete] FROM t", Dialect::MsSql));
        assert!(only_reads("SELECT `delete` FROM t", Dialect::MySql));
        assert!(only_reads("SELECT 1 -- delete\n", Dialect::Postgres));
        assert!(only_reads("SELECT 1 # delete\n", Dialect::MySql));
        assert!(only_reads("SELECT /* delete */ 1", Dialect::Postgres));
        assert!(only_reads("SELECT $tag$ delete $tag$", Dialect::Postgres));
        // A longer word that contains a writing word is its own word.
        assert!(only_reads(
            "SELECT created_at, updates, deleted FROM t",
            Dialect::Postgres
        ));
    }
}

/// The splitter against the scripts that the frontend tests also read.
#[cfg(test)]
mod shared_fixture {
    use super::*;

    #[derive(Deserialize)]
    struct Fixture {
        cases: Vec<Case>,
    }

    #[derive(Deserialize)]
    struct Case {
        name: String,
        dialects: Vec<Dialect>,
        script: String,
        statements: Vec<String>,
    }

    /// The frontend test `frontend/src/lib/__tests__/splitterFixture.spec.ts`
    /// reads the same file, so both splitters find the same statements.
    #[test]
    fn the_splitter_finds_the_statements_of_the_shared_fixture() {
        let fixture: Fixture =
            serde_json::from_str(include_str!("../../tests/fixtures/splitter.json")).unwrap();
        assert!(!fixture.cases.is_empty());
        for case in fixture.cases {
            for dialect in case.dialects {
                let found: Vec<String> = split_batches(&case.script, dialect)
                    .iter()
                    .flat_map(|batch| split_statements(&batch.text, dialect))
                    .collect();
                assert_eq!(found, case.statements, "{} ({dialect:?})", case.name);
            }
        }
    }
}
