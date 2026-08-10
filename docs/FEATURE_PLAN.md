# Feature plan

This plan adds sixteen features to the application. Each feature closes a
gap against the tools that the users of MS SQL Server and AWS Athena know.
The plan gives the design of each feature, the files it touches, and the
tests that prove it.

MS SQL Server and AWS Athena come first in every decision. MySQL, MariaDB
and PostgreSQL come next. SQLite gets the same features when the engine
supplies them.

## Rules for this work

The rules in `CLAUDE.md` apply to all of the work below.

- All text that is not code is in ASD-STE100 Simplified Technical English.
- The unit tests cover 100% of the lines, the branches and the functions.
- Each change goes directly on the `main` branch.
- The pre-commit hook runs the tests and the linters, and it stops a commit
  that fails.

## The features

The list keeps the numbers from the feature review, so that a reader can
match this plan against that list.

### 1. Script an object as DDL and as DML

The explorer menu gets four new commands: script the object as CREATE, as
SELECT, as INSERT, and as UPDATE. The command puts the text in a new tab.

The SELECT, INSERT and UPDATE forms come from the column list, so one
generator in `backend/src/sql.rs` serves every engine. The generator quotes
each name for the dialect, it writes one column on each line, and it marks
the key columns in the WHERE clause of the UPDATE form.

The CREATE form uses the command of the engine when the engine has one:

| Engine | Source of the CREATE text |
| --- | --- |
| MS SQL Server | `OBJECT_DEFINITION` for a view, and a built statement for a table |
| MySQL and MariaDB | `SHOW CREATE TABLE` |
| PostgreSQL | `pg_get_viewdef` for a view, and a built statement for a table |
| SQLite | `sqlite_master.sql` |
| Athena | `SHOW CREATE TABLE` |

A built statement holds the columns, the types, the null rule and the
primary key. The generic builder is the fallback for an engine that gives
no text.

### 2. Show the messages that the server sends

The Messages tab shows the text that the server sends beside the rows.

PostgreSQL sends `NOTICE`, `WARNING` and `INFO` on the connection object.
The driver reads them with `poll_message` on the connection task and puts
them in a shared buffer. `execute_query` drains that buffer into the
response.

MySQL and MariaDB already give the info text of each statement. The driver
keeps that behaviour.

MS SQL Server sends `PRINT` and low-severity `RAISERROR` as info tokens.
`tiberius` 0.12.3 decodes those tokens and then drops them, and the crate
gives no public way to read them. The application therefore cannot show
that text today. This plan does the part that is possible: the driver keeps
the severity, the line number, the procedure name and the error number of
every server error, and the Messages tab shows them. `docs/LIMITATIONS.md`
records the missing part and its cause.

Each message carries a level, so the interface can mark a warning.

### 3. Read the schema in the background

A new command `schema_snapshot` reads every table, every view and every
column of one database in one round trip. Each driver builds one query
against its catalog for this.

The frontend calls the command after a connection opens, and again when the
user changes the current database. The result goes in the explorer store as
a per-connection cache, and the editor completion reads it. The tree
continues to read a level at a time, because a tree must stay quick on a
server that holds many databases.

The snapshot has a bound on the number of columns it accepts, so that a
very large catalog cannot fill the memory. The bound is a setting.

### 4. Complete the columns of an alias

`frontend/src/lib/sql.ts` gets three new functions:

- `qualifierBefore` reads the name in front of the full stop at the cursor.
- `tableAliases` reads the FROM clause and the JOIN clauses of the
  statement, and returns a map from an alias to a table name.
- `completionsFor` takes the alias map and the qualifier. It offers the
  columns of the matched table alone when a qualifier is present. It puts
  the columns of the tables in the statement in front of the other names
  when no qualifier is present.

The alias reader steps over the quoted names of each dialect.

### 5. Show the query plan

The driver trait gets `explain`. The capability record gets
`supports_explain`. Each driver builds its own statement:

| Engine | Estimated plan | Actual plan |
| --- | --- | --- |
| MS SQL Server | `SET SHOWPLAN_XML ON` | `SET STATISTICS XML ON` |
| PostgreSQL | `EXPLAIN (FORMAT TEXT)` | `EXPLAIN (ANALYZE, BUFFERS)` |
| MySQL and MariaDB | `EXPLAIN` | `EXPLAIN ANALYZE` |
| Athena | `EXPLAIN` | `EXPLAIN ANALYZE` |
| SQLite | `EXPLAIN QUERY PLAN` | the same |

The toolbar gets a button with two choices: the estimated plan and the
actual plan. The plan opens in its own result tab.

An actual plan runs the statement. The button therefore asks the user to
confirm before it runs an actual plan.

### 6. Transaction control

`OpenConnection` gets a transaction flag. Three commands set it:
`begin_transaction`, `commit_transaction` and `rollback_transaction`. Each
one sends the keyword of the dialect through the driver and then sets the
flag.

The query toolbar gets an auto-commit switch and three buttons. The buttons
appear when the driver reports `supports_transactions`. The status bar
shows an open transaction in a warning colour.

The application starts a transaction before a statement when auto-commit is
off and no transaction is open. A tab that closes with an open transaction
asks the user to commit or to roll back.

Athena reports `supports_transactions` as false, so the buttons stay hidden
for it.

### 7. Athena cost control

`QueryResponse` gets a `stats` record with the scanned bytes, the engine
time and the price of the query. The Athena driver fills it from the
statistics of the query execution.

The price uses a rate for each terabyte. The rate is a setting, and its
default is the standard rate of 5.00 US dollars for each terabyte.

The status bar shows the cost of the last query and the total for the
session. The settings dialog holds the rate and a warning limit. A query
whose scan passes the warning limit raises a notice.

The connection form gets a switch for the result reuse of Athena, which
lowers the cost of a repeated query.

### 8. More object types in the tree

The tree gets folder nodes below a schema: Tables, Views, Procedures and
Functions. A table node gets folder nodes below it: Columns, Indexes and
Keys. An Athena table gets a Partitions folder.

The driver trait gets four methods, and each one has a default that returns
an empty list:

- `list_routines` gives the procedures and the functions of a schema.
- `list_indexes` gives the indexes of a table with their columns.
- `list_constraints` gives the primary keys, the foreign keys, the unique
  constraints and the check constraints of a table.
- `list_partitions` gives the partitions of an Athena table.

The capability record reports which methods the driver answers, so the tree
draws a folder only when the engine fills it.

### 9. Format the statement

The application takes the `sql-formatter` package. The package supports the
T-SQL dialect, the MySQL dialect, the PostgreSQL dialect, the SQLite
dialect and the Trino dialect that Athena uses.

A wrapper in `frontend/src/lib/sql.ts` maps the dialect of the connection
to the dialect of the formatter. The editor gets a format action, a toolbar
button and the key Shift+Alt+F. The action formats the selection when there
is one, and the whole text when there is none.

### 10. More export formats and a partial export

The export menu gets four new entries: Markdown, INSERT statements, Excel,
and an export of the selected rows.

The Excel file is a real XLSX file. The `fflate` package writes the ZIP
container, and `frontend/src/lib/xlsx.ts` writes the parts of the document.
The file therefore needs no large dependency.

A new backend command `write_binary_file` accepts the bytes as base64 text
and writes them to the path.

A large result needs an export that does not pass through the interface. A
new backend command `export_query` runs the statement again with a high row
limit and writes CSV or JSON straight to the file. The export menu offers
this for a result that the row limit stopped.

### 11. Keyboard commands and a command palette

`frontend/src/lib/commands.ts` holds a registry. Each record has an
identifier, a title, a key, a group and a function. `AppLayout` binds the
keys, and a new `CommandPalette.vue` lists the commands with a filter.

| Key | Command |
| --- | --- |
| Ctrl/Cmd + Enter | Run the statement |
| Ctrl/Cmd + Shift + Enter | Run the whole script |
| Ctrl/Cmd + T | New tab |
| Ctrl/Cmd + W | Close the tab |
| Ctrl/Cmd + Shift + P | Open the palette |
| Ctrl/Cmd + Shift + C | Stop the statement |
| Shift + Alt + F | Format the statement |
| Ctrl/Cmd + 1, 2, 3 | Show the connections, the explorer, the history |
| Ctrl/Cmd + , | Open the settings |
| F1 | Show the key list |

### 12. Edit the rows in the grid

The grid gets an edit mode. The mode is available when the result came from
one table and that table has a primary key. The application knows the table
when the result came from a preview or from a script, and it also reads a
simple `SELECT ... FROM one_table` statement to find the name.

The user changes a cell, adds a row, or marks a row for deletion. The grid
holds the changes and marks the cells. The Apply button opens a dialog that
shows the statements. The user reads them and confirms.

The application sends the changes to a new command `apply_row_edits`. The
command builds each statement with bound parameters and dialect quoting,
runs them inside one transaction, and rolls the transaction back when one
statement fails.

The mode stays off for Athena, because Athena tables hold no primary key
and the engine has no row update.

### 13. Query parameters

A statement can hold a named parameter in the form `:name`. Before a run,
the application finds the names that have no value and asks for them in a
dialog. The values stay with the tab, so a second run needs no dialog.

The rewrite happens in `backend/src/sql.rs`. It turns each name into the
placeholder of the dialect and it builds the value list in the right order:

| Engine | Placeholder |
| --- | --- |
| MS SQL Server | `@P1` |
| PostgreSQL | `$1` |
| MySQL, MariaDB and SQLite | `?` |
| Athena | `?` |

The rewrite steps over the text inside the quotes, the comments and the
casts of PostgreSQL that use two colons.

### 14. Microsoft Entra ID authentication

The connection form gets an authentication list for MS SQL Server: SQL
login, Windows integrated security, Microsoft Entra ID with the Azure CLI,
and an access token that the user supplies.

The Azure CLI choice runs `az account get-access-token --resource
https://database.windows.net/` and reads the token from the JSON output.
The driver gives the token to `AuthMethod::aad_token`.

The token has a life of about one hour. The health check therefore asks for
a new token when it opens the connection again.

### 15. Table properties

The tree menu gets a Properties command. It opens a dialog with four parts:
the general facts, the columns, the indexes and the constraints. The
general part holds the row count, the size on disk and the date of the last
change when the engine gives them.

A new command `table_details` collects the parts in one call.

### 16. Keep a result

Each result tab gets a pin. A pinned tab stays when the next statement
runs, and its title holds the time of the run. The user can therefore
compare a new result against an old one. A pinned tab has a close button of
its own.

## The order of the work

The work goes in eight packages. Each package is one commit or a small
group of commits, and each commit passes the hook.

1. **The shared model.** The new records and the new capability flags in
   `backend/src/db.rs`, and the new helpers in `backend/src/sql.rs`. This
   package holds the parameter rewrite, the script generator and the plan
   statements.
2. **The driver trait.** The new methods with their defaults, and the
   implementation of each one for the five drivers.
3. **The commands.** The new commands, the transaction state, the binary
   write, the streaming export and the Entra ID authentication.
4. **The frontend core.** The types, the API wrappers, the alias
   completion, the formatter wrapper, the new export writers and the
   command registry.
5. **The explorer.** The folder nodes, the new object types, the background
   snapshot and the properties dialog.
6. **The query view.** The plan tab, the messages, the pinned results, the
   transaction controls, the parameter dialog and the format action.
7. **The grid.** The row selection, the edit mode and the new exports.
8. **The shell.** The command palette, the key bindings, the key list and
   the settings.

The documents come last. `README.md` gets the new features.
`docs/LIMITATIONS.md` records the info tokens of MS SQL Server.
`docs/REMEDIATION_STATUS.md` gets a row for each feature.

## Risks

**The catalog queries are wide.** Each new list method runs a query against
a catalog view. A query that is wrong fails at run time and not at build
time. Each driver therefore gets a test that checks the text of the query
against the shape the engine needs.

**The row editor changes data.** A wrong WHERE clause changes more rows
than the user expects. The editor therefore builds the WHERE clause from
the primary key alone, it binds every value as a parameter, it runs inside
a transaction, and it shows the statements before it runs them. The command
refuses a table that has no primary key.

**The Athena price is an estimate.** The rate changes by region and by
contract. The status bar therefore names the figure an estimate, and the
rate is a setting.

**The formatter adds a dependency.** `sql-formatter` is a large package.
The wrapper keeps it behind one function, so a later change of package
touches one file.

**The token of Entra ID expires.** A long session loses its connection when
the token expires. The reconnection path therefore asks the Azure CLI for a
new token each time.
