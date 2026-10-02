//! The SQLite driver.
//!
//! `rusqlite` is a blocking library, so every call runs on the thread pool
//! for blocking work. The connection is held behind a mutex that the
//! closure takes for the length of one call.

use crate::db::drivers::{
    add_index_column, bytes_to_json, f64_to_json, next_values, number_out_of_range, number_value,
    prefixed_plan, rows_affected_message, rows_returned_message, CancelHandle, DatabaseDriver,
    NumberValue,
};
use crate::db::sink::{RowSink, RunSummary, SinkControl};
use crate::db::{
    AppColumn, ColumnInfo, Constraint, ConstraintKind, CreateQuery, Database, DriverCapabilities,
    ExecOptions, IndexInfo, Message, ObjectType, PlanKind, QueryParams, QueryResponse, Schema,
    Table, TableFact, TableKind, Trigger,
};
use crate::error::{Error, Result};
use crate::sql::{split_statements, Dialect};
use crate::storage::SavedConnection;
use async_trait::async_trait;
use rusqlite::types::{Value as SqliteValue, ValueRef};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value as JsonValue;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

mod trigger;

/// True when SQLite reads the text as one or more whole statements. The
/// engine knows the `BEGIN ... END` body of a trigger, which the splitter of
/// the application does not know.
fn is_complete(text: &str) -> bool {
    match std::ffi::CString::new(text) {
        // SAFETY: the pointer names a string that ends with a NUL and that
        // lives until the call returns. `sqlite3_complete` only reads it.
        Ok(text) => unsafe { rusqlite::ffi::sqlite3_complete(text.as_ptr()) != 0 },
        // A NUL inside the text ends it for SQLite, so no join can help.
        Err(_) => true,
    }
}

/// Joins the fragments that the splitter cut out of one trigger. The
/// splitter ends a statement at each semicolon, so `CREATE TRIGGER t AFTER
/// INSERT ON a BEGIN INSERT INTO b VALUES(1); END` gives two fragments, and
/// SQLite refuses the first with "incomplete input". A fragment that SQLite
/// reads as incomplete takes the next fragment, until the text is whole or
/// the script ends.
fn join_trigger_bodies(fragments: Vec<String>) -> Vec<String> {
    let mut statements: Vec<String> = Vec::new();
    let mut open: Option<String> = None;
    for fragment in fragments {
        let text = match open.take() {
            Some(head) => format!("{head}; {fragment}"),
            None => fragment,
        };
        if is_complete(&format!("{text};")) {
            statements.push(text);
        } else {
            open = Some(text);
        }
    }
    statements.extend(open);
    statements
}

pub struct SqliteDriver {
    connection: Arc<Mutex<Connection>>,
    path: String,
    /// Stops the statement that runs, from outside the lock.
    interrupt: Arc<rusqlite::InterruptHandle>,
}

/// Asks SQLite to stop the statement that runs. The engine then returns an
/// error to the caller of the statement, and the connection stays usable.
struct SqliteCancel(Arc<rusqlite::InterruptHandle>);

#[async_trait]
impl CancelHandle for SqliteCancel {
    async fn cancel(&self) -> Result<()> {
        self.0.interrupt();
        Ok(())
    }
}

impl SqliteDriver {
    pub async fn connect(connection: &SavedConnection) -> Result<Box<dyn DatabaseDriver>> {
        let path = connection
            .options
            .file_path
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                Error::Configuration("A SQLite connection needs the path of a file.".to_string())
            })?
            .to_string();

        let flags = if connection.options.read_only {
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI
        } else {
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_URI
        };

        let open_path = path.clone();
        let handle =
            tokio::task::spawn_blocking(move || Connection::open_with_flags(&open_path, flags))
                .await
                .map_err(|error| Error::Connection(error.to_string()))??;

        let interrupt = Arc::new(handle.get_interrupt_handle());
        Ok(Box::new(SqliteDriver {
            connection: Arc::new(Mutex::new(handle)),
            path,
            interrupt,
        }))
    }

    /// Runs a closure against the connection on the pool for blocking work.
    async fn with_connection<T, F>(&self, work: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
    {
        let connection = self.connection.clone();
        tokio::task::spawn_blocking(move || {
            let guard = connection
                .lock()
                .map_err(|_| Error::Connection("The SQLite connection is not usable.".into()))?;
            work(&guard)
        })
        .await
        .map_err(|error| Error::Connection(error.to_string()))?
    }
}

#[async_trait]
impl DatabaseDriver for SqliteDriver {
    fn capabilities(&self) -> DriverCapabilities {
        DriverCapabilities {
            supports_schemas: true,
            supports_multiple_databases: false,
            supports_cancel: true,
            supports_transactions: true,
            supports_routines: false,
            supports_indexes: true,
            supports_constraints: true,
            supports_partitions: false,
            supports_explain: true,
            supports_materialized_views: false,
            supports_foreign_tables: false,
            supports_synonyms: false,
            supports_triggers: true,
            supports_view_triggers: true,
            supports_events: false,
        }
    }

    fn dialect(&self) -> Dialect {
        Dialect::Sqlite
    }

    /// An interrupt aborts the statement alone and the engine rolls back
    /// its transaction, so the connection stays fit for use.
    fn keeps_connection_after_stop(&self) -> bool {
        true
    }

    fn cancel_handle(&self) -> Option<Arc<dyn CancelHandle>> {
        Some(Arc::new(SqliteCancel(self.interrupt.clone())))
    }

    fn create_query(
        &self,
        _database: Option<&str>,
        schema: Option<&str>,
        table: &str,
        _kind: TableKind,
    ) -> Option<CreateQuery> {
        Some(create_query_text(schema_or_main(schema), table))
    }

    fn object_create_query(
        &self,
        _database: Option<&str>,
        schema: Option<&str>,
        _parent: Option<&str>,
        name: &str,
        object_type: ObjectType,
    ) -> Option<CreateQuery> {
        object_query_text(schema_or_main(schema), name, object_type)
    }

    async fn ping(&mut self) -> Result<()> {
        self.with_connection(|connection| {
            connection.query_row("SELECT 1", [], |_| Ok(()))?;
            Ok(())
        })
        .await
    }

    async fn holds_open_transaction(&mut self) -> Result<bool> {
        self.with_connection(|connection| Ok(!connection.is_autocommit()))
            .await
    }

    /// Runs the statements on the pool for blocking work and feeds the sink
    /// as the rows arrive. The closure sends blocks of rows through a bounded
    /// channel, and this side drives the sink. A `Stop` of the sink travels
    /// back through a shared flag that the closure reads on each row.
    async fn execute_stream(
        &mut self,
        query: &str,
        params: Option<&QueryParams>,
        options: &ExecOptions,
        sink: &mut dyn RowSink,
    ) -> Result<RunSummary> {
        let started = Instant::now();
        let bound = bind_params(params)?;
        let statements = join_trigger_bodies(split_statements(query, Dialect::Sqlite));
        let options = *options;
        let stop = Arc::new(AtomicBool::new(false));

        let (sender, mut receiver) = tokio::sync::mpsc::channel::<RowEvent>(EVENT_CAPACITY);
        let connection = self.connection.clone();
        let flag = stop.clone();
        let reader = tokio::task::spawn_blocking(move || {
            let guard = connection
                .lock()
                .map_err(|_| Error::Connection("The SQLite connection is not usable.".into()))?;
            let mut rows_affected: Option<u64> = None;
            let mut used = 0;
            for statement in statements {
                if flag.load(Ordering::Relaxed) {
                    break;
                }
                stream_statement(
                    &guard,
                    &statement,
                    &bound,
                    &mut used,
                    &options,
                    &sender,
                    &flag,
                    &mut rows_affected,
                )?;
            }
            Ok::<Option<u64>, Error>(rows_affected)
        });

        // A `Stop` ends the whole run, so the rows of the set that stopped
        // are dropped until its end arrives.
        let mut skip_rows = false;
        while let Some(event) = receiver.recv().await {
            match event {
                RowEvent::BeginSet(columns) => {
                    skip_rows = false;
                    sink.begin_set(columns)?;
                }
                RowEvent::Rows(rows) => {
                    for row in rows {
                        if skip_rows {
                            continue;
                        }
                        if sink.row(row)? == SinkControl::Stop {
                            skip_rows = true;
                            stop.store(true, Ordering::Relaxed);
                        }
                    }
                }
                RowEvent::EndSet(truncated) => sink.end_set(truncated)?,
                RowEvent::Message(message) => sink.message(message),
            }
        }

        let rows_affected = reader
            .await
            .map_err(|error| Error::Connection(error.to_string()))??;
        Ok(RunSummary {
            rows_affected,
            elapsed_ms: started.elapsed().as_millis() as u64,
            stats: None,
        })
    }

    /// SQLite holds one plan, which it gives without running the statement.
    /// A request for the actual plan therefore gives the same rows, with a
    /// message that says so.
    async fn explain(
        &mut self,
        query: &str,
        params: Option<&QueryParams>,
        kind: PlanKind,
        options: &ExecOptions,
    ) -> Result<QueryResponse> {
        let statement = prefixed_plan(query, Dialect::Sqlite, "EXPLAIN QUERY PLAN")?;
        let mut response = self.execute_query(&statement, params, options).await?;
        if kind.runs_the_statement() {
            response.messages.push(Message::info(
                "SQLite reports one plan, so this is the plan it builds before the run.",
            ));
        }
        Ok(response)
    }

    /// One SQLite connection holds one file. The name of the file stands
    /// for the database.
    async fn list_databases(&mut self) -> Result<Vec<Database>> {
        let name = std::path::Path::new(&self.path)
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.clone());
        Ok(vec![Database { name }])
    }

    /// Lists the schemas of the connection: `main`, `temp` when the
    /// connection made a temporary object, and each attached database.
    ///
    /// The explorer reads on a connection of its own, and a temporary table
    /// and an attached database belong to the connection that made them. A
    /// `CREATE TEMP TABLE` or an `ATTACH` in the tab of a query therefore
    /// does not show in the explorer.
    async fn list_schemas(&mut self, _database: &str) -> Result<Vec<Schema>> {
        self.with_connection(|connection| {
            let mut statement =
                connection.prepare("SELECT name FROM pragma_database_list ORDER BY seq")?;
            let names = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?;
            Ok(names.into_iter().map(|name| Schema { name }).collect())
        })
        .await
    }

    async fn list_tables(&mut self, _database: &str, schema: Option<&str>) -> Result<Vec<Table>> {
        let master = master_of(schema_or_main(schema));
        self.with_connection(move |connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT name, type FROM {master} \
                 WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' \
                 ORDER BY type, name"
            ))?;
            let rows = statement.query_map([], |row| {
                let name: String = row.get(0)?;
                let kind: String = row.get(1)?;
                Ok(if kind == "view" {
                    Table::view(name)
                } else {
                    Table::table(name)
                })
            })?;
            let mut tables = Vec::new();
            for table in rows {
                tables.push(table?);
            }
            Ok(tables)
        })
        .await
    }

    async fn list_columns(
        &mut self,
        _database: &str,
        schema: Option<&str>,
        table: &str,
    ) -> Result<Vec<AppColumn>> {
        let place = [table.to_string(), schema_or_main(schema).to_string()];
        self.with_connection(move |connection| {
            let mut statement = connection
                .prepare("SELECT name, type, \"notnull\", pk FROM pragma_table_info(?1, ?2)")?;
            let rows = statement.query_map([&place[0], &place[1]], |row| {
                let name: String = row.get(0)?;
                let data_type: String = row.get(1)?;
                let not_null: i64 = row.get(2)?;
                let primary_key: i64 = row.get(3)?;
                Ok(AppColumn {
                    name,
                    data_type: if data_type.is_empty() {
                        "any".to_string()
                    } else {
                        data_type
                    },
                    nullable: not_null == 0,
                    is_primary_key: primary_key > 0,
                })
            })?;
            let mut columns = Vec::new();
            for column in rows {
                columns.push(column?);
            }
            Ok(columns)
        })
        .await
    }

    /// Counts the rows of one table. SQLite keeps no such figure, so the
    /// count is read with a statement. The file is local, so the read costs
    /// little. A view gives no facts, because a count of a view runs the
    /// whole query of the view.
    async fn table_facts(
        &mut self,
        _database: &str,
        schema: Option<&str>,
        table: &str,
    ) -> Result<Vec<TableFact>> {
        let schema = schema_or_main(schema);
        let name = Dialect::Sqlite.quote_qualified(&[schema, table]);
        let master = master_of(schema);
        let table = table.to_string();
        self.with_connection(move |connection| {
            let views: i64 = connection.query_row(
                &format!("SELECT COUNT(*) FROM {master} WHERE type = 'view' AND name = ?1"),
                [&table],
                |row| row.get(0),
            )?;
            if views > 0 {
                return Ok(Vec::new());
            }
            let count: i64 =
                connection.query_row(&format!("SELECT COUNT(*) FROM {name}"), [], |row| {
                    row.get(0)
                })?;
            Ok(vec![TableFact::new("Rows", count.to_string())])
        })
        .await
    }

    /// Reads the indexes from the two pragmas of SQLite. One row of the
    /// answer carries one column of one index, so the rows are folded into
    /// one record for each index.
    async fn list_indexes(
        &mut self,
        _database: &str,
        schema: Option<&str>,
        table: &str,
    ) -> Result<Vec<IndexInfo>> {
        let place = [table.to_string(), schema_or_main(schema).to_string()];
        self.with_connection(move |connection| {
            let mut statement = connection.prepare(INDEX_QUERY)?;
            let rows = statement.query_map([&place[0], &place[1]], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)? != 0,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?;
            let mut indexes: Vec<IndexInfo> = Vec::new();
            for row in rows {
                let (name, unique, origin, column) = row?;
                add_index_column(&mut indexes, name, unique, origin == "pk", column);
            }
            Ok(indexes)
        })
        .await
    }

    /// Reads the triggers of one relation from the catalog of its schema,
    /// and the temporary triggers on it from the catalog of `temp`. SQLite
    /// has no switch that disables a trigger.
    async fn list_triggers(
        &mut self,
        _database: &str,
        schema: Option<&str>,
        table: &str,
    ) -> Result<Vec<Trigger>> {
        let schema = schema_or_main(schema).to_string();
        let table = table.to_string();
        self.with_connection(move |connection| {
            let read = |master: &str| -> Result<Vec<(String, String)>> {
                let mut statement = connection.prepare(&format!(
                    "SELECT name, sql FROM {master} \
                     WHERE type = 'trigger' AND tbl_name = ?1 AND sql IS NOT NULL \
                     ORDER BY name"
                ))?;
                let rows = statement
                    .query_map([&table], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect::<rusqlite::Result<Vec<(String, String)>>>()?;
                Ok(rows)
            };
            let mut rows = read(&master_of(&schema))?;
            let in_temp = schema.eq_ignore_ascii_case("temp");
            if !in_temp {
                let shadowed: i64 = connection.query_row(
                    "SELECT COUNT(*) FROM temp.sqlite_master \
                     WHERE type IN ('table', 'view') AND name = ?1",
                    [&table],
                    |row| row.get(0),
                )?;
                for (name, sql) in read("temp.sqlite_master")? {
                    let target = trigger::trigger_head(&sql).target_schema;
                    if temp_trigger_is_on(&schema, target.as_deref(), shadowed > 0) {
                        rows.push((name, sql));
                    }
                }
            }
            Ok(rows
                .into_iter()
                .map(|(name, sql)| {
                    let head = trigger::trigger_head(&sql);
                    Trigger {
                        name,
                        timing: head.timing,
                        events: head.event.into_iter().collect(),
                        enabled: true,
                    }
                })
                .collect())
        })
        .await
    }

    /// Reads the primary key and the foreign keys. SQLite holds no name for
    /// either, so the name comes from the columns.
    async fn list_constraints(
        &mut self,
        _database: &str,
        schema: Option<&str>,
        table: &str,
    ) -> Result<Vec<Constraint>> {
        let schema = schema_or_main(schema).to_string();
        let master = master_of(&schema);
        let place = [table.to_string(), schema];
        self.with_connection(move |connection| {
            let mut keys = connection
                .prepare("SELECT name FROM pragma_table_info(?1, ?2) WHERE pk > 0 ORDER BY pk")?;
            let key_columns: Vec<String> = keys
                .query_map([&place[0], &place[1]], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?;

            let mut constraints: Vec<Constraint> = Vec::new();
            if !key_columns.is_empty() {
                constraints.push(Constraint {
                    name: format!("Primary key on {}", key_columns.join(", ")),
                    kind: ConstraintKind::PrimaryKey,
                    columns: key_columns,
                    detail: None,
                });
            }

            let mut foreign = connection.prepare(
                "SELECT id, \"from\", \"table\", \"to\" FROM pragma_foreign_key_list(?1, ?2) \
                 ORDER BY id, seq",
            )?;
            let rows = foreign.query_map([&place[0], &place[1]], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?;
            let mut current: Option<i64> = None;
            for row in rows {
                let (id, column, target, target_column) = row?;
                let detail = match target_column {
                    Some(name) => format!("{target}({name})"),
                    None => target,
                };
                match (current, constraints.last_mut()) {
                    (Some(seen), Some(last)) if seen == id => last.columns.push(column),
                    _ => {
                        current = Some(id);
                        constraints.push(Constraint {
                            name: format!("Foreign key to {detail}"),
                            kind: ConstraintKind::ForeignKey,
                            columns: vec![column],
                            detail: Some(detail),
                        });
                    }
                }
            }
            // A UNIQUE constraint carries no name of its own. SQLite builds
            // an index for it with the origin 'u', and the columns of that
            // index are the columns of the constraint.
            let mut unique = connection.prepare(UNIQUE_QUERY)?;
            let rows = unique.query_map([&place[0], &place[1]], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })?;
            let mut groups: Vec<(String, Vec<String>)> = Vec::new();
            for row in rows {
                let (index_name, column) = row?;
                match groups.last_mut() {
                    Some((held, columns)) if *held == index_name => {
                        if let Some(column) = column {
                            columns.push(column);
                        }
                    }
                    _ => groups.push((index_name, column.into_iter().collect())),
                }
            }
            for (_, columns) in groups {
                if columns.is_empty() {
                    continue;
                }
                constraints.push(Constraint {
                    name: format!("Unique on {}", columns.join(", ")),
                    kind: ConstraintKind::Unique,
                    columns,
                    detail: None,
                });
            }

            // SQLite holds no pragma for a check, so the text of the table
            // gives it.
            let mut created = connection.prepare(&format!(
                "SELECT sql FROM {master} WHERE type = 'table' AND name = ?1 \
                 AND sql IS NOT NULL"
            ))?;
            let text: Option<String> = created
                .query_map([&place[0]], |row| row.get::<_, String>(0))?
                .next()
                .transpose()?;
            for expression in check_expressions(text.as_deref().unwrap_or("")) {
                constraints.push(Constraint {
                    name: format!("Check ({expression})"),
                    kind: ConstraintKind::Check,
                    columns: Vec::new(),
                    detail: Some(expression),
                });
            }

            Ok(constraints)
        })
        .await
    }
}

/// Reads one column of each index that a UNIQUE constraint made. The origin
/// 'u' names such an index, and the origin 'c' names an index that a CREATE
/// INDEX statement made.
const UNIQUE_QUERY: &str = "SELECT list.name, info.name \
     FROM pragma_index_list(?1, ?2) AS list \
     LEFT JOIN pragma_index_info(list.name, ?2) AS info \
     WHERE list.origin = 'u' \
     ORDER BY list.name, info.seqno";

/// True when the characters at the place hold the word, with no letter, no
/// digit and no low line on either side of it.
fn holds_word(chars: &[char], at: usize, word: &str) -> bool {
    let inside = |c: char| c.is_alphanumeric() || c == '_';
    if at > 0 && inside(chars[at - 1]) {
        return false;
    }
    let mut index = at;
    for wanted in word.chars() {
        match chars.get(index) {
            Some(current) if current.to_ascii_lowercase() == wanted => index += 1,
            _ => return false,
        }
    }
    !chars.get(index).copied().is_some_and(inside)
}

/// Steps over a quoted region that starts at the opening mark and ends at
/// the closing one. A doubled closing mark stands for the mark itself.
fn skip_quoted(chars: &[char], at: usize, closing: char) -> usize {
    let mut index = at + 1;
    while index < chars.len() {
        if chars[index] == closing {
            if chars.get(index + 1) == Some(&closing) {
                index += 2;
                continue;
            }
            return index + 1;
        }
        index += 1;
    }
    index
}

/// Reads the text inside the brackets that start at the place, and gives
/// the place after the closing bracket. A bracket inside a quoted region
/// counts for nothing.
fn balanced(chars: &[char], at: usize) -> (String, usize) {
    let mut index = at + 1;
    let mut depth = 1usize;
    let start = index;
    while index < chars.len() && depth > 0 {
        let current = chars[index];
        match current {
            '\'' | '"' | '`' => {
                index = skip_quoted(chars, index, current);
                continue;
            }
            '[' => {
                index = skip_quoted(chars, index, ']');
                continue;
            }
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            break;
        }
        index += 1;
    }
    let text: String = chars[start..index.min(chars.len())].iter().collect();
    (text.trim().to_string(), (index + 1).min(chars.len()))
}

/// Reads the CHECK expressions of a CREATE TABLE statement.
///
/// The reader steps over the string literals, the quoted names and the
/// comments, and it takes the expression in the brackets that follow each
/// CHECK keyword of the body of the table.
pub fn check_expressions(sql: &str) -> Vec<String> {
    let chars: Vec<char> = sql.chars().collect();
    let mut found: Vec<String> = Vec::new();
    let mut index = 0usize;
    let mut depth = 0usize;
    while index < chars.len() {
        let current = chars[index];
        match current {
            '\'' | '"' | '`' => {
                index = skip_quoted(&chars, index, current);
                continue;
            }
            '[' => {
                index = skip_quoted(&chars, index, ']');
                continue;
            }
            '-' if chars.get(index + 1) == Some(&'-') => {
                while index < chars.len() && chars[index] != '\n' {
                    index += 1;
                }
                continue;
            }
            '/' if chars.get(index + 1) == Some(&'*') => {
                index += 2;
                while index < chars.len()
                    && !(chars[index] == '*' && chars.get(index + 1) == Some(&'/'))
                {
                    index += 1;
                }
                index = (index + 2).min(chars.len());
                continue;
            }
            '(' => {
                depth += 1;
                index += 1;
                continue;
            }
            ')' => {
                depth = depth.saturating_sub(1);
                index += 1;
                continue;
            }
            _ => {}
        }
        if depth == 1 && holds_word(&chars, index, "check") {
            let mut after = index + "check".len();
            while after < chars.len() && chars[after].is_whitespace() {
                after += 1;
            }
            if chars.get(after) == Some(&'(') {
                let (text, next) = balanced(&chars, after);
                found.push(text);
                index = next;
                continue;
            }
        }
        index += 1;
    }
    found
}

/// Reads one column of one index for each row. The `origin` column of the
/// pragma names `pk` for the index that carries the primary key.
const INDEX_QUERY: &str = "SELECT list.name, list.\"unique\", list.origin, info.name \
     FROM pragma_index_list(?1, ?2) AS list \
     LEFT JOIN pragma_index_info(list.name, ?2) AS info \
     ORDER BY list.name, info.seqno";

/// The schema that a request names. A request without one reads `main`.
fn schema_or_main(schema: Option<&str>) -> &str {
    schema.unwrap_or("main")
}

/// The quoted name of the catalog table of one schema. Each schema keeps its
/// own `sqlite_master`, and the name `temp.sqlite_master` reads the catalog
/// of the temporary objects.
fn master_of(schema: &str) -> String {
    format!("{}.sqlite_master", Dialect::Sqlite.quote_identifier(schema))
}

/// Builds the statement that reads the CREATE text of one object. SQLite
/// keeps the text of every object in `sqlite_master`, so a table and a view
/// come from the same place.
fn create_query_text(schema: &str, table: &str) -> CreateQuery {
    CreateQuery::new(
        format!(
            "SELECT sql FROM {} WHERE name = {} AND sql IS NOT NULL;",
            master_of(schema),
            Dialect::Sqlite.quote_literal(table)
        ),
        0,
    )
}

/// True when a temporary trigger fires on a relation of the schema. The
/// `ON` clause of a temporary trigger can name a table of any schema. A
/// clause without a schema names the temporary table of that name when one
/// exists, and else the table of `main`. `shadowed` is true when `temp` has
/// a relation of the name.
fn temp_trigger_is_on(schema: &str, target: Option<&str>, shadowed: bool) -> bool {
    match target {
        Some(target) => target.eq_ignore_ascii_case(schema),
        None => !shadowed && schema.eq_ignore_ascii_case("main"),
    }
}

/// Builds the statement that reads the CREATE text of one trigger. The
/// trigger is in the catalog of the schema, or in the catalog of `temp` when
/// it is a temporary trigger on a table of another schema. The first match
/// wins. SQLite has no scheduled events.
fn object_query_text(schema: &str, name: &str, object_type: ObjectType) -> Option<CreateQuery> {
    if object_type != ObjectType::Trigger {
        return None;
    }
    let name = Dialect::Sqlite.quote_literal(name);
    let select = |master: &str| {
        format!(
            "SELECT sql FROM {master} WHERE type = 'trigger' AND name = {name} AND sql IS NOT NULL"
        )
    };
    let mut sql = select(&master_of(schema));
    if !schema.eq_ignore_ascii_case("temp") {
        sql = format!("{sql} UNION ALL {}", select("temp.sqlite_master"));
    }
    Some(CreateQuery::new(format!("{sql} LIMIT 1;"), 0))
}

/// One step of a run, sent from the blocking closure to the async side.
enum RowEvent {
    BeginSet(Vec<ColumnInfo>),
    Rows(Vec<Vec<JsonValue>>),
    EndSet(bool),
    Message(Message),
}

/// The number of rows one event carries. A block amortizes the cost of the
/// channel over many rows.
const ROW_BLOCK: usize = 1000;

/// The number of events the channel holds. The bound keeps the memory of a
/// fast read small while the sink works.
const EVENT_CAPACITY: usize = 4;

/// Sends one event, and reports a closed channel as a cancelled run. The
/// channel closes when the receiving future is dropped, for example when a
/// time limit ends the run.
fn send_event(sender: &tokio::sync::mpsc::Sender<RowEvent>, event: RowEvent) -> Result<()> {
    sender.blocking_send(event).map_err(|_| Error::Cancelled)
}

/// Runs one statement and sends what it produces through the channel. The
/// row limit stops the read of one set, and the stop flag ends it early.
///
/// `values` holds the values of the whole script. The statement takes as
/// many as it has places, and `used` counts the values that earlier
/// statements took.
#[allow(clippy::too_many_arguments)]
fn stream_statement(
    connection: &Connection,
    statement_text: &str,
    values: &[SqliteValue],
    used: &mut usize,
    options: &ExecOptions,
    sender: &tokio::sync::mpsc::Sender<RowEvent>,
    stop: &AtomicBool,
    rows_affected: &mut Option<u64>,
) -> Result<()> {
    let mut statement = connection.prepare(statement_text)?;
    let params = next_values(values, used, statement.parameter_count());
    let column_count = statement.column_count();

    if column_count == 0 {
        // `sqlite3_changes()` keeps the count of the last INSERT, UPDATE or
        // DELETE, so a CREATE, a BEGIN or a COMMIT after a write gives that
        // count again. Such a statement leaves the total count as it was.
        let before = connection.total_changes();
        let changed = statement.execute(rusqlite::params_from_iter(params.iter()))? as u64;
        let affected = if connection.total_changes() == before {
            0
        } else {
            changed
        };
        *rows_affected = Some(rows_affected.unwrap_or(0) + affected);
        send_event(sender, RowEvent::Message(rows_affected_message(affected)))?;
        return Ok(());
    }

    let columns: Vec<ColumnInfo> = (0..column_count)
        .map(|index| {
            ColumnInfo::new(
                statement.column_name(index).unwrap_or("?").to_string(),
                statement
                    .columns()
                    .get(index)
                    .and_then(|column| column.decl_type())
                    .unwrap_or("any")
                    .to_string(),
            )
        })
        .collect();
    send_event(sender, RowEvent::BeginSet(columns))?;

    // An INSERT, an UPDATE or a DELETE with RETURNING gives rows and also
    // changes rows. Its count is read after the rows, as for a statement
    // without columns.
    let writes = !statement.readonly();
    let before = connection.total_changes();
    let mut rows = statement.query(rusqlite::params_from_iter(params.iter()))?;
    let mut block: Vec<Vec<JsonValue>> = Vec::new();
    let mut count = 0usize;
    let mut truncated = false;
    while let Some(row) = rows.next()? {
        if stop.load(Ordering::Relaxed) || count >= options.max_rows {
            truncated = true;
            break;
        }
        block.push(
            (0..column_count)
                .map(|index| value_to_json(row.get_ref(index).unwrap_or(ValueRef::Null)))
                .collect(),
        );
        count += 1;
        if block.len() >= ROW_BLOCK {
            send_event(sender, RowEvent::Rows(std::mem::take(&mut block)))?;
        }
    }
    if !block.is_empty() {
        send_event(sender, RowEvent::Rows(block))?;
    }
    // The drop resets the statement, and SQLite sets the count of changes
    // when the statement ends.
    drop(rows);
    if writes {
        let affected = if connection.total_changes() == before {
            0
        } else {
            connection.changes()
        };
        *rows_affected = Some(rows_affected.unwrap_or(0) + affected);
    }
    send_event(
        sender,
        RowEvent::Message(rows_returned_message(count, truncated)),
    )?;
    send_event(sender, RowEvent::EndSet(truncated))?;
    Ok(())
}

/// Turns the JSON parameters into values the driver can bind.
pub fn bind_params(params: Option<&QueryParams>) -> Result<Vec<SqliteValue>> {
    let Some(params) = params else {
        return Ok(Vec::new());
    };
    let mut bound = Vec::new();
    for param in params {
        bound.push(match &param.value {
            JsonValue::Null => SqliteValue::Null,
            JsonValue::Bool(flag) => SqliteValue::Integer(i64::from(*flag)),
            JsonValue::String(text) => SqliteValue::Text(text.clone()),
            JsonValue::Number(number) => match number_value(number) {
                Some(NumberValue::Integer(value)) => SqliteValue::Integer(value),
                Some(NumberValue::Float(value)) => SqliteValue::Real(value),
                None => return Err(number_out_of_range(number)),
            },
            // SQLite has no structured type, so the value goes in as text.
            other => SqliteValue::Text(other.to_string()),
        });
    }
    Ok(bound)
}

/// Converts one value of the driver into JSON.
pub fn value_to_json(value: ValueRef<'_>) -> JsonValue {
    match value {
        ValueRef::Null => JsonValue::Null,
        ValueRef::Integer(number) => JsonValue::from(number),
        ValueRef::Real(number) => f64_to_json(number),
        ValueRef::Text(bytes) => match std::str::from_utf8(bytes) {
            Ok(text) => JsonValue::String(text.to_string()),
            Err(_) => bytes_to_json(bytes),
        },
        ValueRef::Blob(bytes) => bytes_to_json(bytes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Message, TriggerEvent, TriggerTiming};

    #[test]
    fn the_fragments_of_a_trigger_body_are_joined() {
        let script = "CREATE TABLE a(x); CREATE TRIGGER t AFTER INSERT ON a BEGIN \
                      INSERT INTO b VALUES(1); DELETE FROM c; END; SELECT 1";
        assert_eq!(
            join_trigger_bodies(split_statements(script, Dialect::Sqlite)),
            vec![
                "CREATE TABLE a(x)".to_string(),
                "CREATE TRIGGER t AFTER INSERT ON a BEGIN INSERT INTO b VALUES(1); \
                 DELETE FROM c; END"
                    .to_string(),
                "SELECT 1".to_string(),
            ]
        );
        // A body that never ends stays one statement, and SQLite reports it.
        assert_eq!(
            join_trigger_bodies(vec![
                "CREATE TRIGGER t BEGIN SELECT 1".into(),
                "SELECT 2".into()
            ]),
            vec!["CREATE TRIGGER t BEGIN SELECT 1; SELECT 2".to_string()]
        );
        // A NUL in the text ends the join.
        assert_eq!(
            join_trigger_bodies(vec!["SELECT '\0".into(), "SELECT 2".into()]),
            vec!["SELECT '\0".to_string(), "SELECT 2".to_string()]
        );
    }

    #[test]
    fn the_create_statement_reads_the_master_table() {
        let query = create_query_text("main", "it's");
        assert_eq!(
            query.sql,
            "SELECT sql FROM \"main\".sqlite_master WHERE name = 'it''s' AND sql IS NOT NULL;"
        );
        assert_eq!(query.column, 0);
    }
    use crate::db::QueryParam;
    use crate::storage::{ConnectionOptions, DbType};
    use std::sync::atomic::{AtomicU32, Ordering};

    fn connection_for(path: &str) -> SavedConnection {
        SavedConnection {
            id: "id".into(),
            name: "name".into(),
            db_type: DbType::Sqlite,
            host: None,
            port: None,
            user: None,
            database: None,
            password: None,
            aws_secret_access_key: None,
            aws_session_token: None,
            options: ConnectionOptions {
                file_path: Some(path.to_string()),
                ..ConnectionOptions::default()
            },
            color: None,
            group: None,
        }
    }

    /// Each test needs its own database. A shared cache name lets two tests
    /// that run at the same time lock each other out, so the counter below
    /// gives every call a name of its own.
    static MEMORY_DATABASE_COUNT: AtomicU32 = AtomicU32::new(0);

    async fn open_memory() -> Box<dyn DatabaseDriver> {
        let number = MEMORY_DATABASE_COUNT.fetch_add(1, Ordering::Relaxed);
        SqliteDriver::connect(&connection_for(&format!(
            "file:sqlite_test_{number}?mode=memory&cache=shared"
        )))
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn a_cancel_stops_a_statement_and_the_connection_stays_usable() {
        let mut driver = open_memory().await;
        assert!(driver.capabilities().supports_cancel);
        assert!(driver.keeps_connection_after_stop());
        let handle = driver.cancel_handle().expect("the driver can cancel");

        // A recursive query that counts to a thousand million runs long
        // enough for the interrupt to land.
        let long_query = "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c \
                          WHERE x < 1000000000) SELECT COUNT(*) FROM c";
        let options = ExecOptions::default();
        let work = driver.execute_query(long_query, None, &options);
        let stopper = async {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            handle.cancel().await
        };
        let (outcome, stopped) = tokio::join!(work, stopper);
        assert!(stopped.is_ok());
        assert!(outcome.is_err());

        // The same connection answers the next statement.
        let after = driver
            .execute_query("SELECT 1", None, &ExecOptions::default())
            .await
            .unwrap();
        assert_eq!(after.results.len(), 1);
    }

    #[tokio::test]
    async fn a_connection_needs_a_file_path() {
        let mut input = connection_for("  ");
        assert_eq!(
            SqliteDriver::connect(&input).await.err().unwrap().kind(),
            crate::error::ErrorKind::Configuration
        );
        input.options.file_path = None;
        assert!(SqliteDriver::connect(&input).await.is_err());
    }

    #[tokio::test]
    async fn a_plan_of_a_statement_names_the_steps_of_the_read() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("plan.db");
        let mut driver = SqliteDriver::connect(&connection_for(&path.to_string_lossy()))
            .await
            .unwrap();
        driver
            .execute_query(
                "CREATE TABLE person (id INTEGER PRIMARY KEY, name TEXT)",
                None,
                &ExecOptions::default(),
            )
            .await
            .unwrap();

        let estimated = driver
            .explain(
                "SELECT * FROM person WHERE id = 1;",
                None,
                PlanKind::Estimated,
                &ExecOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(estimated.results.len(), 1);
        let plan = estimated.results[0].rows[0]
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(plan.to_lowercase().contains("person"), "{plan}");

        // SQLite holds one plan, so the actual plan carries a message that
        // names what the rows are.
        let actual = driver
            .explain(
                "SELECT * FROM person",
                None,
                PlanKind::Actual,
                &ExecOptions::default(),
            )
            .await
            .unwrap();
        assert!(actual
            .messages
            .iter()
            .any(|message| message.text.contains("SQLite reports one plan")));
    }

    #[tokio::test]
    async fn a_plan_of_two_statements_is_refused() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("plan_two.db");
        let mut driver = SqliteDriver::connect(&connection_for(&path.to_string_lossy()))
            .await
            .unwrap();
        let error = driver
            .explain(
                "SELECT 1; SELECT 2",
                None,
                PlanKind::Estimated,
                &ExecOptions::default(),
            )
            .await
            .unwrap_err();
        assert_eq!(error.kind(), crate::error::ErrorKind::Configuration);
    }

    #[tokio::test]
    async fn a_read_only_connection_refuses_a_write() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("read_only.db");
        let path_text = path.to_string_lossy().into_owned();

        let mut driver = SqliteDriver::connect(&connection_for(&path_text))
            .await
            .unwrap();
        driver
            .execute_query("CREATE TABLE t (a INTEGER)", None, &ExecOptions::default())
            .await
            .unwrap();
        drop(driver);

        let mut input = connection_for(&path_text);
        input.options.read_only = true;
        let mut driver = SqliteDriver::connect(&input).await.unwrap();
        assert!(driver
            .execute_query("INSERT INTO t VALUES (1)", None, &ExecOptions::default())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn a_missing_file_in_a_missing_folder_gives_an_error() {
        let result = SqliteDriver::connect(&connection_for("/does/not/exist/a.db")).await;
        assert_eq!(
            result.err().unwrap().kind(),
            crate::error::ErrorKind::Database
        );
    }

    #[tokio::test]
    async fn the_driver_reports_what_it_can_do() {
        let driver = open_memory().await;
        assert_eq!(driver.dialect(), Dialect::Sqlite);
        let capabilities = driver.capabilities();
        assert!(capabilities.supports_schemas);
        assert!(!capabilities.supports_multiple_databases);
        assert!(capabilities.supports_transactions);
        assert!(capabilities.supports_cancel);
        assert!(driver.cancel_handle().is_some());
        // SQLite has triggers, also on a view, but no stored routine and no
        // scheduled event.
        assert!(!capabilities.supports_routines);
        assert!(capabilities.supports_triggers);
        assert!(capabilities.supports_view_triggers);
        assert!(!capabilities.supports_events);
    }

    #[tokio::test]
    async fn the_triggers_of_a_relation_give_their_time_and_their_event() {
        let mut driver = open_memory().await;
        driver
            .execute_query(
                "CREATE TABLE orders (id INTEGER, total REAL); \
                 CREATE TABLE other (id INTEGER); \
                 CREATE VIEW big AS SELECT * FROM orders WHERE total > 100; \
                 CREATE TRIGGER stamp UPDATE OF total ON orders BEGIN SELECT 1; END; \
                 CREATE TRIGGER audit AFTER INSERT ON orders BEGIN SELECT 1; END; \
                 CREATE TRIGGER elsewhere AFTER DELETE ON other BEGIN SELECT 1; END; \
                 CREATE TRIGGER write INSTEAD OF DELETE ON big BEGIN SELECT 1; END; \
                 CREATE TEMP TRIGGER watch BEFORE DELETE ON main.orders BEGIN SELECT 1; END; \
                 CREATE TEMP TRIGGER bare AFTER UPDATE ON orders BEGIN SELECT 1; END;",
                None,
                &ExecOptions::default(),
            )
            .await
            .unwrap();
        let trigger = |name: &str, timing, event| Trigger {
            name: name.into(),
            timing,
            events: vec![event],
            enabled: true,
        };

        // The triggers of the schema come first, and the temporary triggers
        // on the table follow.
        assert_eq!(
            driver.list_triggers("db", None, "orders").await.unwrap(),
            vec![
                trigger("audit", TriggerTiming::After, TriggerEvent::Insert),
                trigger("stamp", TriggerTiming::Before, TriggerEvent::Update),
                trigger("bare", TriggerTiming::After, TriggerEvent::Update),
                trigger("watch", TriggerTiming::Before, TriggerEvent::Delete),
            ]
        );
        assert_eq!(
            driver
                .list_triggers("db", Some("main"), "big")
                .await
                .unwrap(),
            vec![trigger(
                "write",
                TriggerTiming::InsteadOf,
                TriggerEvent::Delete
            )]
        );
        // The schema temp lists its own catalog alone.
        let temp = driver
            .list_triggers("db", Some("temp"), "orders")
            .await
            .unwrap();
        assert_eq!(temp.len(), 2);

        // The CREATE text of a temporary trigger on a table of main comes
        // from the catalog of temp.
        for (name, start) in [
            ("audit", "CREATE TRIGGER audit"),
            ("watch", "CREATE TRIGGER watch BEFORE DELETE ON main.orders"),
        ] {
            let query = driver
                .object_create_query(None, None, Some("orders"), name, ObjectType::Trigger)
                .unwrap();
            let text = driver
                .execute_query(&query.sql, None, &ExecOptions::default())
                .await
                .unwrap();
            let sql = text.results[0].rows[0][0].as_str().unwrap().to_string();
            assert!(sql.starts_with(start), "{sql}");
        }
        assert!(driver
            .object_create_query(None, None, None, "e", ObjectType::Event)
            .is_none());
    }

    #[test]
    fn a_temporary_trigger_belongs_to_the_schema_that_its_table_names() {
        assert!(temp_trigger_is_on("aux", Some("AUX"), true));
        assert!(!temp_trigger_is_on("main", Some("aux"), false));
        // A table without a schema is the temporary table of that name,
        // and else the table of main.
        assert!(temp_trigger_is_on("main", None, false));
        assert!(!temp_trigger_is_on("main", None, true));
        assert!(!temp_trigger_is_on("aux", None, false));
    }

    #[test]
    fn the_create_statement_of_a_trigger_also_reads_the_temporary_catalog() {
        let query = object_query_text("main", "it's", ObjectType::Trigger).unwrap();
        assert_eq!(
            query.sql,
            "SELECT sql FROM \"main\".sqlite_master WHERE type = 'trigger' AND name = 'it''s' \
             AND sql IS NOT NULL UNION ALL SELECT sql FROM temp.sqlite_master \
             WHERE type = 'trigger' AND name = 'it''s' AND sql IS NOT NULL LIMIT 1;"
        );
        let temp = object_query_text("temp", "t", ObjectType::Trigger).unwrap();
        assert!(!temp.sql.contains("UNION ALL"));
    }

    #[tokio::test]
    async fn a_query_returns_rows_and_a_message() {
        let mut driver = open_memory().await;
        let response = driver
            .execute_query(
                "SELECT 1 AS a, 'x' AS b, NULL AS c",
                None,
                &ExecOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].columns[0], ColumnInfo::new("a", "any"));
        assert_eq!(
            response.results[0].rows[0],
            vec![
                serde_json::json!(1),
                serde_json::json!("x"),
                JsonValue::Null
            ]
        );
        assert_eq!(response.messages, vec![Message::info("1 row returned.")]);
        assert!(!response.results[0].truncated);
    }

    #[tokio::test]
    async fn a_script_runs_every_statement_and_counts_the_changes() {
        let mut driver = open_memory().await;
        let response = driver
            .execute_query(
                "DROP TABLE IF EXISTS people; \
                 CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT NOT NULL); \
                 INSERT INTO people (name) VALUES ('Ada'), ('Grace'); \
                 SELECT name FROM people ORDER BY name;",
                None,
                &ExecOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].rows.len(), 2);
        assert_eq!(response.rows_affected, Some(2));
        assert!(response
            .messages
            .iter()
            .any(|message| message.text == "2 rows affected."));
    }

    #[tokio::test]
    async fn a_statement_that_changes_no_row_counts_no_row() {
        let mut driver = open_memory().await;
        let response = driver
            .execute_query(
                "CREATE TABLE people (name TEXT); \
                 INSERT INTO people VALUES ('Ada'), ('Grace'); \
                 CREATE TABLE places (name TEXT); \
                 BEGIN; COMMIT;",
                None,
                &ExecOptions::default(),
            )
            .await
            .unwrap();
        let counts: Vec<&str> = response.messages.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(
            counts,
            vec![
                "0 rows affected.",
                "2 rows affected.",
                "0 rows affected.",
                "0 rows affected.",
                "0 rows affected.",
            ]
        );
        assert_eq!(response.rows_affected, Some(2));
    }

    #[tokio::test]
    async fn the_row_limit_stops_the_read() {
        let mut driver = open_memory().await;
        driver
            .execute_query(
                "DROP TABLE IF EXISTS numbers; CREATE TABLE numbers (n INTEGER);\
                 INSERT INTO numbers VALUES (1),(2),(3);",
                None,
                &ExecOptions::default(),
            )
            .await
            .unwrap();
        let response = driver
            .execute_query(
                "SELECT n FROM numbers ORDER BY n",
                None,
                &ExecOptions {
                    max_rows: 2,
                    timeout_secs: 10,
                    one_statement: false,
                },
            )
            .await
            .unwrap();
        assert_eq!(response.results[0].rows.len(), 2);
        assert!(response.results[0].truncated);
        assert!(response.messages[0].text.contains("row limit"));
    }

    #[tokio::test]
    async fn a_statement_with_parameters_binds_them() {
        let mut driver = open_memory().await;
        let params: QueryParams = vec![
            QueryParam {
                value: serde_json::json!(4),
            },
            QueryParam {
                value: serde_json::json!("four"),
            },
        ];
        let response = driver
            .execute_query(
                "SELECT ?1 AS n, ?2 AS t",
                Some(&params),
                &ExecOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.results[0].rows[0],
            vec![serde_json::json!(4), serde_json::json!("four")]
        );
    }

    #[tokio::test]
    async fn each_statement_of_a_script_takes_its_own_values() {
        let mut driver = open_memory().await;
        let params: QueryParams = ["Ada", "Grace", "Ada"]
            .into_iter()
            .map(|name| QueryParam {
                value: serde_json::json!(name),
            })
            .collect();
        let response = driver
            .execute_query(
                "CREATE TABLE people (name TEXT); \
                 INSERT INTO people VALUES (?), (?); \
                 SELECT count(*) FROM people WHERE name = ?;",
                Some(&params),
                &ExecOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(response.rows_affected, Some(2));
        assert_eq!(response.results[0].rows[0], vec![serde_json::json!(1)]);
    }

    #[tokio::test]
    async fn a_script_creates_a_trigger_with_a_body() {
        let mut driver = open_memory().await;
        let response = driver
            .execute_query(
                "CREATE TABLE a(x); CREATE TABLE b(y); \
                 CREATE TRIGGER t AFTER INSERT ON a BEGIN INSERT INTO b VALUES(1); END; \
                 INSERT INTO a VALUES(5); SELECT count(*) FROM b;",
                None,
                &ExecOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(response.results[0].rows, vec![vec![serde_json::json!(1)]]);
    }

    #[tokio::test]
    async fn a_statement_that_is_not_valid_gives_a_database_error() {
        let mut driver = open_memory().await;
        let error = driver
            .execute_query("SELECT FROM", None, &ExecOptions::default())
            .await
            .unwrap_err();
        assert_eq!(error.kind(), crate::error::ErrorKind::Database);
    }

    #[tokio::test]
    async fn the_metadata_lists_the_tables_the_views_and_the_columns() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("meta.db");
        let mut driver = SqliteDriver::connect(&connection_for(&path.to_string_lossy()))
            .await
            .unwrap();
        driver
            .execute_query(
                "CREATE TABLE orders (id INTEGER PRIMARY KEY, total REAL, note TEXT); \
                 CREATE VIEW big_orders AS SELECT * FROM orders WHERE total > 10;",
                None,
                &ExecOptions::default(),
            )
            .await
            .unwrap();

        let databases = driver.list_databases().await.unwrap();
        assert_eq!(
            databases,
            vec![Database {
                name: "meta.db".into()
            }]
        );
        assert_eq!(
            driver.list_schemas("meta.db").await.unwrap(),
            vec![Schema {
                name: "main".into()
            }]
        );

        let tables = driver.list_tables("meta.db", None).await.unwrap();
        assert!(tables.contains(&Table::table("orders")));
        assert!(tables.contains(&Table::view("big_orders")));

        let columns = driver
            .list_columns("meta.db", None, "orders")
            .await
            .unwrap();
        assert_eq!(columns.len(), 3);
        assert_eq!(columns[0].name, "id");
        assert!(columns[0].is_primary_key);
        assert_eq!(columns[1].data_type, "REAL");
        assert!(columns[1].nullable);

        driver.ping().await.unwrap();
    }

    #[tokio::test]
    async fn the_metadata_lists_the_unique_and_the_check_constraints() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("marks.db");
        let mut driver = SqliteDriver::connect(&connection_for(&path.to_string_lossy()))
            .await
            .unwrap();
        driver
            .execute_query(
                "CREATE TABLE items ( \
                     id INTEGER PRIMARY KEY, \
                     code TEXT, region TEXT, \
                     total REAL CHECK (total > 0), \
                     UNIQUE (code, region), \
                     CONSTRAINT within_range CHECK (id < 100 AND code <> ')') \
                 );",
                None,
                &ExecOptions::default(),
            )
            .await
            .unwrap();

        let constraints = driver
            .list_constraints("marks.db", None, "items")
            .await
            .unwrap();
        let unique: Vec<&Constraint> = constraints
            .iter()
            .filter(|constraint| constraint.kind == ConstraintKind::Unique)
            .collect();
        assert_eq!(unique.len(), 1);
        assert_eq!(
            unique[0].columns,
            vec!["code".to_string(), "region".to_string()]
        );
        assert_eq!(unique[0].name, "Unique on code, region");

        let checks: Vec<&Constraint> = constraints
            .iter()
            .filter(|constraint| constraint.kind == ConstraintKind::Check)
            .collect();
        assert_eq!(checks.len(), 2);
        assert_eq!(checks[0].detail.as_deref(), Some("total > 0"));
        assert_eq!(checks[0].name, "Check (total > 0)");
        assert_eq!(
            checks[1].detail.as_deref(),
            Some("id < 100 AND code <> ')'")
        );
        assert!(checks[1].columns.is_empty());
    }

    #[test]
    fn a_check_of_a_table_is_read_over_the_comments_and_the_literals() {
        // A check outside the body of the table, a comment and a literal
        // that holds the word all give nothing.
        assert_eq!(
            check_expressions(
                "CREATE TABLE t ( \
                 a INT, -- CHECK (never)\n \
                 b TEXT DEFAULT 'CHECK (never)', \
                 /* CHECK (never) */ \
                 CHECK (a > (b - 1)) \
                 )"
            ),
            vec!["a > (b - 1)".to_string()]
        );
        // A quoted name that holds the word gives nothing either.
        assert_eq!(
            check_expressions("CREATE TABLE t (\"check\" INT)"),
            Vec::<String>::new()
        );
        assert_eq!(
            check_expressions("CREATE TABLE t ([check] INT)"),
            Vec::<String>::new()
        );
        assert_eq!(
            check_expressions("CREATE TABLE t (`check` INT)"),
            Vec::<String>::new()
        );
        // A keyword that is part of a longer word gives nothing.
        assert_eq!(
            check_expressions("CREATE TABLE t (checked INT)"),
            Vec::<String>::new()
        );
        // A CHECK with no brackets after it, and a text with no body.
        assert_eq!(
            check_expressions("CREATE TABLE t (a INT CHECK)"),
            Vec::<String>::new()
        );
        assert_eq!(check_expressions(""), Vec::<String>::new());
        // A quote and a bracket that never close leave the reader at the end.
        assert_eq!(
            check_expressions("CREATE TABLE t (a TEXT DEFAULT 'x"),
            Vec::<String>::new()
        );
        assert_eq!(
            check_expressions("CREATE TABLE t (CHECK (a > 'it''s'"),
            vec!["a > 'it''s'".to_string()]
        );
        // A closing bracket with no opening one holds the depth at none.
        assert_eq!(check_expressions(") CHECK (a)"), Vec::<String>::new());
        // A comment that never closes leaves the reader at the end as well.
        assert_eq!(
            check_expressions("CREATE TABLE t (/* never"),
            Vec::<String>::new()
        );
    }

    #[tokio::test]
    async fn the_metadata_lists_the_indexes_and_the_constraints() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("keys.db");
        let mut driver = SqliteDriver::connect(&connection_for(&path.to_string_lossy()))
            .await
            .unwrap();
        driver
            .execute_query(
                "CREATE TABLE customers (id INTEGER PRIMARY KEY, code TEXT); \
                 CREATE TABLE orders ( \
                     id INTEGER, region TEXT, customer INTEGER, \
                     PRIMARY KEY (id, region), \
                     FOREIGN KEY (customer) REFERENCES customers (id) \
                 ); \
                 CREATE UNIQUE INDEX orders_region ON orders (region, id);",
                None,
                &ExecOptions::default(),
            )
            .await
            .unwrap();

        let indexes = driver
            .list_indexes("keys.db", None, "orders")
            .await
            .unwrap();
        let named = indexes
            .iter()
            .find(|index| index.name == "orders_region")
            .expect("the index of the statement is listed");
        assert_eq!(named.columns, vec!["region".to_string(), "id".to_string()]);
        assert!(named.unique);
        assert!(!named.primary);
        assert!(indexes.iter().any(|index| index.primary));

        let constraints = driver
            .list_constraints("keys.db", None, "orders")
            .await
            .unwrap();
        assert_eq!(constraints[0].kind, ConstraintKind::PrimaryKey);
        assert_eq!(
            constraints[0].columns,
            vec!["id".to_string(), "region".to_string()]
        );
        assert_eq!(constraints[1].kind, ConstraintKind::ForeignKey);
        assert_eq!(constraints[1].columns, vec!["customer".to_string()]);
        assert_eq!(constraints[1].detail.as_deref(), Some("customers(id)"));

        // A table without a key or an index gives an empty list, and no
        // engine here holds a routine or a partition for SQLite.
        assert!(driver
            .list_constraints("keys.db", None, "customers")
            .await
            .unwrap()
            .iter()
            .all(|constraint| constraint.kind == ConstraintKind::PrimaryKey));
        assert!(driver
            .list_routines("keys.db", None)
            .await
            .unwrap()
            .is_empty());
        assert!(driver
            .list_partitions("keys.db", None, "orders")
            .await
            .unwrap()
            .partitions
            .is_empty());
    }

    #[test]
    fn an_index_on_an_expression_holds_no_column() {
        let mut indexes: Vec<IndexInfo> = Vec::new();
        add_index_column(&mut indexes, "by_total".into(), false, false, None);
        add_index_column(
            &mut indexes,
            "by_total".into(),
            false,
            false,
            Some("a".into()),
        );
        assert_eq!(indexes.len(), 1);
        assert_eq!(indexes[0].columns, vec!["a".to_string()]);
    }

    #[tokio::test]
    async fn the_snapshot_holds_every_relation_and_stops_at_the_bound() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("snap.db");
        let mut driver = SqliteDriver::connect(&connection_for(&path.to_string_lossy()))
            .await
            .unwrap();
        driver
            .execute_query(
                "CREATE TABLE orders (id INTEGER PRIMARY KEY, total REAL); \
                 CREATE VIEW big AS SELECT * FROM orders WHERE total > 10;",
                None,
                &ExecOptions::default(),
            )
            .await
            .unwrap();

        let snapshot = driver.schema_snapshot("snap.db", 100).await.unwrap();
        assert_eq!(snapshot.database, "snap.db");
        assert!(snapshot.complete);
        assert_eq!(snapshot.column_count, 4);
        let orders = snapshot
            .relations
            .iter()
            .find(|relation| relation.name == "orders")
            .unwrap();
        assert_eq!(orders.kind, TableKind::Table);
        assert_eq!(orders.schema.as_deref(), Some("main"));
        assert_eq!(orders.columns[0].name, "id");
        assert_eq!(orders.columns[1].data_type, "REAL");
        assert!(snapshot
            .relations
            .iter()
            .any(|relation| relation.kind == TableKind::View));

        // The bound stops the read before the second relation.
        let part = driver.schema_snapshot("snap.db", 1).await.unwrap();
        assert!(!part.complete);
        assert_eq!(part.relations.len(), 1);
    }

    #[tokio::test]
    async fn the_temporary_and_the_attached_schemas_are_read() {
        let directory = tempfile::tempdir().unwrap();
        let main_path = directory.path().join("home.db");
        let other_path = directory.path().join("other.db");
        let mut driver = SqliteDriver::connect(&connection_for(&main_path.to_string_lossy()))
            .await
            .unwrap();
        driver
            .execute_query(
                &format!(
                    "CREATE TABLE local (id INTEGER); \
                     CREATE TEMP TABLE scratch (id INTEGER PRIMARY KEY, note TEXT UNIQUE); \
                     INSERT INTO scratch (note) VALUES ('a'), ('b'); \
                     ATTACH DATABASE {} AS \"odd\"\"name\"; \
                     CREATE TABLE \"odd\"\"name\".parent (id INTEGER PRIMARY KEY); \
                     CREATE TABLE \"odd\"\"name\".items ( \
                         id INTEGER PRIMARY KEY, \
                         parent INTEGER REFERENCES parent (id), \
                         code TEXT, total REAL CHECK (total > 0), UNIQUE (code)); \
                     CREATE INDEX \"odd\"\"name\".items_total ON items (total); \
                     CREATE VIEW \"odd\"\"name\".cheap AS SELECT * FROM items;",
                    Dialect::Sqlite.quote_literal(&other_path.to_string_lossy())
                ),
                None,
                &ExecOptions::default(),
            )
            .await
            .unwrap();

        let names: Vec<String> = driver
            .list_schemas("home.db")
            .await
            .unwrap()
            .into_iter()
            .map(|schema| schema.name)
            .collect();
        assert_eq!(names, vec!["main", "temp", "odd\"name"]);

        let odd = Some("odd\"name");
        assert_eq!(
            driver.list_tables("home.db", None).await.unwrap(),
            vec![Table::table("local")]
        );
        assert_eq!(
            driver.list_tables("home.db", Some("temp")).await.unwrap(),
            vec![Table::table("scratch")]
        );
        assert_eq!(
            driver.list_tables("home.db", odd).await.unwrap(),
            vec![
                Table::table("items"),
                Table::table("parent"),
                Table::view("cheap")
            ]
        );

        let columns = driver
            .list_columns("home.db", Some("temp"), "scratch")
            .await
            .unwrap();
        assert_eq!(columns.len(), 2);
        assert!(columns[0].is_primary_key);
        assert_eq!(
            driver
                .table_facts("home.db", Some("temp"), "scratch")
                .await
                .unwrap(),
            vec![TableFact::new("Rows", "2")]
        );

        let indexes = driver.list_indexes("home.db", odd, "items").await.unwrap();
        assert!(indexes.iter().any(|index| index.name == "items_total"));
        let constraints = driver
            .list_constraints("home.db", odd, "items")
            .await
            .unwrap();
        let kinds: Vec<ConstraintKind> = constraints.iter().map(|c| c.kind).collect();
        assert_eq!(
            kinds,
            vec![
                ConstraintKind::PrimaryKey,
                ConstraintKind::ForeignKey,
                ConstraintKind::Unique,
                ConstraintKind::Check
            ]
        );

        // The CREATE text comes from the catalog of the schema.
        let query = driver
            .create_query(Some("home.db"), odd, "cheap", TableKind::View)
            .unwrap();
        let text = driver
            .execute_query(&query.sql, None, &ExecOptions::default())
            .await
            .unwrap();
        assert_eq!(
            text.results[0].rows[0][0],
            serde_json::json!("CREATE VIEW cheap AS SELECT * FROM items")
        );

        // The snapshot reads every schema.
        let snapshot = driver.schema_snapshot("home.db", 100).await.unwrap();
        let places: Vec<(Option<String>, String)> = snapshot
            .relations
            .iter()
            .map(|relation| (relation.schema.clone(), relation.name.clone()))
            .collect();
        assert!(places.contains(&(Some("temp".into()), "scratch".into())));
        assert!(places.contains(&(Some("odd\"name".into()), "items".into())));
    }

    #[tokio::test]
    async fn the_facts_of_a_relation_count_its_rows() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("facts.db");
        let mut driver = SqliteDriver::connect(&connection_for(&path.to_string_lossy()))
            .await
            .unwrap();
        driver
            .execute_query(
                "CREATE TABLE orders (id INTEGER); INSERT INTO orders VALUES (1), (2), (3);",
                None,
                &ExecOptions::default(),
            )
            .await
            .unwrap();

        let facts = driver
            .table_facts("facts.db", None, "orders")
            .await
            .unwrap();
        assert_eq!(facts, vec![TableFact::new("Rows", "3")]);

        // A name that holds a quote reaches the statement safely.
        driver
            .execute_query(
                "CREATE TABLE \"odd\"\"name\" (id INTEGER);",
                None,
                &ExecOptions::default(),
            )
            .await
            .unwrap();
        let odd = driver
            .table_facts("facts.db", None, "odd\"name")
            .await
            .unwrap();
        assert_eq!(odd, vec![TableFact::new("Rows", "0")]);
    }

    #[tokio::test]
    async fn a_view_gives_no_facts() {
        let mut driver = open_memory().await;
        driver
            .execute_query(
                "CREATE TABLE orders (id INTEGER); CREATE VIEW every AS SELECT * FROM orders;",
                None,
                &ExecOptions::default(),
            )
            .await
            .unwrap();
        let facts = driver.table_facts("db", None, "every").await.unwrap();
        assert!(facts.is_empty());
    }

    #[tokio::test]
    async fn a_write_with_returning_counts_its_changes() {
        let mut driver = open_memory().await;
        let response = driver
            .execute_query(
                "CREATE TABLE pets (name TEXT); \
                 INSERT INTO pets VALUES ('cat'), ('dog'), ('owl') RETURNING name; \
                 DELETE FROM pets WHERE name = 'none' RETURNING name; \
                 SELECT name FROM pets;",
                None,
                &ExecOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(response.results[0].rows.len(), 3);
        assert_eq!(response.results[1].rows.len(), 0);
        assert_eq!(response.rows_affected, Some(3));

        // A read leaves the count as it was, and a write that stops at the
        // row limit still counts every row that it changed.
        let limited = driver
            .execute_query(
                "UPDATE pets SET name = name || '!' RETURNING name",
                None,
                &ExecOptions {
                    max_rows: 1,
                    timeout_secs: 10,
                    one_statement: false,
                },
            )
            .await
            .unwrap();
        assert!(limited.results[0].truncated);
        assert_eq!(limited.rows_affected, Some(3));
        let read = driver
            .execute_query("SELECT 1", None, &ExecOptions::default())
            .await
            .unwrap();
        assert_eq!(read.rows_affected, None);
    }

    #[tokio::test]
    async fn a_database_name_falls_back_to_the_whole_path() {
        let mut driver = SqliteDriver::connect(&connection_for("file:x?mode=memory"))
            .await
            .unwrap();
        assert_eq!(driver.list_databases().await.unwrap().len(), 1);
    }

    #[test]
    fn the_parameters_accept_every_json_type() {
        let params: QueryParams = vec![
            QueryParam {
                value: JsonValue::Null,
            },
            QueryParam {
                value: serde_json::json!(true),
            },
            QueryParam {
                value: serde_json::json!("a"),
            },
            QueryParam {
                value: serde_json::json!(2),
            },
            QueryParam {
                value: serde_json::json!(2.5),
            },
            QueryParam {
                value: serde_json::json!([1, 2]),
            },
        ];
        let bound = bind_params(Some(&params)).unwrap();
        assert_eq!(bound.len(), 6);
        assert_eq!(bound[0], SqliteValue::Null);
        assert_eq!(bound[1], SqliteValue::Integer(1));
        assert_eq!(bound[3], SqliteValue::Integer(2));
        assert_eq!(bound[4], SqliteValue::Real(2.5));
        assert_eq!(bound[5], SqliteValue::Text("[1,2]".into()));
        assert!(bind_params(None).unwrap().is_empty());
    }

    #[test]
    fn a_number_that_is_too_large_is_refused() {
        let params: QueryParams = vec![QueryParam {
            value: serde_json::json!(18446744073709551615u64),
        }];
        assert_eq!(
            bind_params(Some(&params)).unwrap_err().kind(),
            crate::error::ErrorKind::Configuration
        );
    }

    /// A sink that records what it receives and stops after a set number of
    /// rows, to test the stream path of the driver.
    struct RecordingSink {
        stop_after: usize,
        sets_begun: usize,
        rows: Vec<Vec<JsonValue>>,
        ends: Vec<bool>,
        messages: Vec<Message>,
    }

    impl RecordingSink {
        fn new(stop_after: usize) -> Self {
            Self {
                stop_after,
                sets_begun: 0,
                rows: Vec::new(),
                ends: Vec::new(),
                messages: Vec::new(),
            }
        }
    }

    impl RowSink for RecordingSink {
        fn begin_set(&mut self, _columns: Vec<ColumnInfo>) -> Result<()> {
            self.sets_begun += 1;
            Ok(())
        }
        fn row(&mut self, row: Vec<JsonValue>) -> Result<SinkControl> {
            self.rows.push(row);
            if self.rows.len() >= self.stop_after {
                Ok(SinkControl::Stop)
            } else {
                Ok(SinkControl::Continue)
            }
        }
        fn end_set(&mut self, truncated: bool) -> Result<()> {
            self.ends.push(truncated);
            Ok(())
        }
        fn message(&mut self, message: Message) {
            self.messages.push(message);
        }
    }

    #[tokio::test]
    async fn a_stream_feeds_the_sink_and_reports_the_run() {
        let mut driver = open_memory().await;
        let mut sink = RecordingSink::new(usize::MAX);
        let summary = driver
            .execute_stream(
                "CREATE TABLE pets (name TEXT); \
                 INSERT INTO pets VALUES ('cat'), ('dog'); \
                 SELECT name FROM pets ORDER BY name;",
                None,
                &ExecOptions::default(),
                &mut sink,
            )
            .await
            .unwrap();
        assert_eq!(summary.rows_affected, Some(2));
        assert_eq!(sink.sets_begun, 1);
        assert_eq!(sink.rows.len(), 2);
        assert_eq!(sink.ends, vec![false]);
        assert!(sink
            .messages
            .iter()
            .any(|message| message.text == "2 rows returned."));
    }

    #[tokio::test]
    async fn a_stop_of_the_sink_ends_the_whole_run() {
        let mut driver = open_memory().await;
        driver
            .execute_query(
                "CREATE TABLE numbers (n INTEGER)",
                None,
                &ExecOptions::default(),
            )
            .await
            .unwrap();

        // The sink stops after the first row. The first statement gives more
        // blocks than the channel holds, so the reader is still inside that
        // statement when the flag lands, and the insert must not run.
        let mut sink = RecordingSink::new(1);
        driver
            .execute_stream(
                "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c \
                 WHERE x < 100000) SELECT x FROM c; \
                 INSERT INTO numbers VALUES (5);",
                None,
                &ExecOptions {
                    max_rows: 100_000,
                    timeout_secs: 300,
                    one_statement: false,
                },
                &mut sink,
            )
            .await
            .unwrap();
        assert_eq!(sink.sets_begun, 1);
        assert_eq!(sink.rows.len(), 1);
        assert_eq!(sink.ends, vec![true]);

        let count = driver
            .execute_query(
                "SELECT COUNT(*) FROM numbers",
                None,
                &ExecOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(count.results[0].rows[0], vec![serde_json::json!(0)]);
    }

    #[test]
    fn every_value_type_becomes_json() {
        assert_eq!(value_to_json(ValueRef::Null), JsonValue::Null);
        assert_eq!(value_to_json(ValueRef::Integer(3)), serde_json::json!(3));
        assert_eq!(value_to_json(ValueRef::Real(0.5)), serde_json::json!(0.5));
        assert_eq!(
            value_to_json(ValueRef::Text(b"hi")),
            serde_json::json!("hi")
        );
        assert_eq!(
            value_to_json(ValueRef::Text(&[0xff, 0xfe])),
            serde_json::json!("//4=")
        );
        assert_eq!(
            value_to_json(ValueRef::Blob(b"hi")),
            serde_json::json!("aGk=")
        );
    }
}
