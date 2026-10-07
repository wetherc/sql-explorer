# SQL Explorer

An open source desktop client for SQL databases on macOS, Windows and Linux. It
is an alternative to SQL Server Management Studio (SSMS) and Azure Data Studio
for users who work on a Mac. It connects to MS SQL Server, AWS Athena,
PostgreSQL, MySQL, MariaDB and SQLite. It shows the objects of each server in a
tree, runs statements in tabs, and writes the results to a file.

The application is built with Vue 3, Vuetify and [Tauri 2](https://tauri.app/).
The website is at <https://sql-explorer.tbmh.org/>.

![The tree of objects, the editor and the rows of a result](docs/screenshots/overview.png)

## Screenshots

Every picture below shows sample connections and sample rows. No user and no
row in them is real.

### Connections

![The list of connections and the form of a MS SQL Server connection](docs/screenshots/connections.png)

The list groups the connections into folders and marks the state of each one.
The form shows the fields of the engine that the user chose. This connection
takes its login from Microsoft Entra ID through the Azure CLI, so the form
holds no password.

### Objects

![The menu of a table in the tree](docs/screenshots/explorer-menu.png)

The tree holds the databases, the schemas, the tables, the views and the
columns of each connection. A key column carries its own icon, and each column
shows its type. The menu of an object builds a preview statement and the CREATE,
SELECT, INSERT and UPDATE statements of that object. A trigger and an event
give their CREATE text from the catalog of the engine.

![The properties of a table](docs/screenshots/properties.png)

The Properties dialog holds the facts of a relation, its columns, its indexes
and its constraints, in one call to the backend.

### Statements

![The completion of the editor after an alias](docs/screenshots/completion.png)

The editor completes the names of the database. A name after a full stop gives
the columns of the table that the alias in front of the stop names, with the
type of each column.

![The values of the named parameters of a statement](docs/screenshots/parameters.png)

A statement that carries a name such as `:country` asks for the values before
it runs. Each value holds the form that the user chose, so a value stays text
when it looks like a number. The bar above the editor names the parameters and
their values.

![The rows of a result, with a filter, a sort and a selection](docs/screenshots/grid.png)

The result grid draws only the rows in view, so a large result stays quick. It
sorts, it filters, it marks a value that is absent, and it counts the rows that
the user selected.

![The forms that an export writes](docs/screenshots/export.png)

The rows go to a CSV, JSON, Markdown, INSERT or Excel file, or to the
clipboard. A whole result goes to a file from the backend, so the rows do not
pass through the interface.

![The plan of a statement](docs/screenshots/plan.png)

A plan tab shows the plan of one statement. The estimated plan needs no run.
The actual plan runs the statement, and the interface asks first.

![A statement on AWS Athena, with the data it scanned](docs/screenshots/athena.png)

For Athena the status bar reports the data that the statement scanned, the
cost of that scan and the cost of the session. The price for each terabyte
stands in the settings.

![The history and the statements that ran](docs/screenshots/history.png)

The panel holds every statement that ran, with its connection, its time and
its result. A second tab holds the statements that the user saved. Both
persist across a restart.

![The palette of commands](docs/screenshots/palette.png)

One registry holds every key of the application. The palette lists the
commands with the keys that reach them.

### Linux

These pictures come from the release build of the `.deb` package on Ubuntu
24.04, in the Openbox window manager. Five sample servers are open at the same
time: PostgreSQL 18, MS SQL Server 2022, MySQL 8.4, MariaDB 11.4 and SQLite.

![Five open connections in the tree and the rows of a MySQL result, in the dark theme](docs/screenshots/linux/overview.png)

The tree holds the objects of the five connections. Each editor tab runs on its
own connection, and this tab runs on MySQL.

![The list of connections and the form of a MS SQL Server connection, in the light theme](docs/screenshots/linux/connections.png)

The list puts the connections into folders. On Linux the passwords go to the
Secret Service of the desktop, for example GNOME Keyring or KWallet.

![A statement on MS SQL Server and the columns of its table in the tree](docs/screenshots/linux/mssql.png)

![The rows of a SQLite result with a filter, in the light theme](docs/screenshots/linux/sqlite.png)

The filter keeps 6 of the 18 rows of the result.

![The properties of a MariaDB table](docs/screenshots/linux/properties.png)

## What it does

**Connections**

- Five engines: MS SQL Server, AWS Athena, PostgreSQL, MySQL and MariaDB, and
  SQLite.
- The form shows the fields the selected engine uses and hides the rest.
- A test button opens the connection, confirms that it answers, and closes it.
- Transport settings: verify the certificate, encrypt without a check, encrypt
  when the server offers it, or send in clear text. The first is the default.
- A named MS SQL Server instance resolves its port through the SQL Browser
  service.
- Four ways to authenticate against MS SQL Server: a SQL login, the account of
  the user, Microsoft Entra ID through the Azure CLI, and an access token that
  the user gives. The account of the user reaches the server through SSPI on
  Windows and through Kerberos on macOS and Linux.
- An Athena connection takes its credentials from the AWS tools of the machine,
  with a profile name, or from an access key ID, a secret access key and an
  optional session token that the user pastes into the form. The two secrets go
  into the keychain of the operating system.
- An Athena connection can reuse the result of an earlier run up to an age that
  the user gives, which costs nothing because the engine scans no data.
- Timeouts for the connection and for a statement, a row limit, a read-only
  session on PostgreSQL, MySQL and SQLite, a read-only replica on MS SQL
  Server, an application name, folders and colours.
- One server session for each tab, so the statements of two tabs run at the
  same time. The temporary tables, the `SET` options and the transactions of
  a tab stay with the session of that tab. A session limit in the options of
  the connection bounds the sessions of one server, with six as the default.
  A session that is idle for 60 minutes closes. At the limit, the session that
  was not used for the longest time closes. A session in a transaction or with
  a running statement does not close.
- A second connection of the record reads the catalog for the explorer, so
  the tree does not wait behind a statement. The temporary tables and the
  attached databases of a tab do not show in the tree. A SQLite database in
  memory has one session alone, and the tree reads it on that session.
- Passwords go into the keychain of the operating system. The settings file
  contains no password. When the server of a saved connection changes and the user
  types no new password, the application removes the stored password. A copy
  of a connection asks for the password again.
- A connection that stops answering is opened again, and the interface shows
  the state of each connection.

**Statements**

- One editor for each tab, with syntax colours and completion that draws on the
  objects of the database. Completion after a full stop offers the columns of
  the table that the alias in front of the stop names.
- `Ctrl` or `Cmd` with `Enter` runs the statement under the cursor. The same
  keys with `Shift` run the whole script. A selection runs in place of the
  statement.
- One registry holds every key of the application, and a palette lists the
  commands with the keys that reach them.
- A script runs statement by statement. The splitter respects quotes, comments,
  dollar tags and the MySQL `DELIMITER` command. On MS SQL Server a script runs
  batch by batch, and a line that has only `GO` ends a batch.
- The formatter of the dialect lays out the statement, through the format
  command of the editor or a button.
- A statement that carries a name such as `:id` opens a dialog for the values.
  Each value holds the form the user chose, so a value stays text when it looks
  like a number. Every engine but Athena binds the values.
- A plan tab shows the plan of one statement. The estimated plan needs no run.
  The actual plan runs the statement, and the interface asks first.
- The Messages tab holds what the server sent, with the severity, the code, the
  line and the procedure of each message. The messages come in while the script
  runs. When a statement fails, the results panel shows the Messages tab.
- When the server gives the place of an error, the editor marks that place, and
  a "Go to line" button moves the cursor to it.
- A tab shows a mark while its statement runs, and a red dot when its last run
  failed.
- A statement that runs can be stopped with the Stop button or with `Ctrl` or
  `Cmd` with `Shift` and `C`. The time limit of the connection stops a
  statement that runs too long.
- The result grid draws only the rows in view, so a large result stays quick. It
  sorts, filters, marks a value that is absent, and opens a wide value.
- Results go to a CSV, JSON, Markdown, INSERT or Excel file, or to the
  clipboard. A whole result goes to a file from the backend, so the rows do not
  pass through the interface. An export of a whole result shows its progress
  and has a Stop button. An Excel export tells the user when it stops at the
  row limit of a sheet, or when it cuts the text of a cell.
- A result tab can be pinned. It then holds its rows and the time of its run
  against the next statement, so two results stand beside each other.
- The File menu of the operating system opens a query in a new tab, opens a
  query from a file, opens a folder of queries, and writes the query that
  stands open to its file. The keys of a desktop do the same four commands.
- A file keeps its encoding when it is saved: UTF-8, UTF-8 or UTF-16 with a
  byte order mark, or Windows-1252. When the text has a character that
  Windows-1252 cannot store, the file is saved as UTF-8 with a byte order mark,
  and a warning tells the user. A file that looks binary does not open.
- The status bar reports the rows and the time, and it counts the time up
  while a statement runs. For Athena it reports the data scanned with the cost
  at a rate the settings hold.
- The history and the saved statements persist, and so do the open tabs with
  their parameter values. When a settings file cannot be read, the application
  renames it to a `.corrupt` file, starts without it, and shows a notice.

**Objects**

- Databases, schemas, tables, views and columns. A key column carries its own
  icon, and each column shows its type.
- Folders hold the tables, the views, the routines, the indexes, the constraints
  and the partitions of a schema or a relation. PostgreSQL also gets folders for
  the materialized views and the foreign tables, and a partitioned table has its
  own icon. MS SQL Server also gets a folder for the synonyms.
- A Triggers folder below a table shows when each trigger runs, the changes
  that fire it (with the columns of an `UPDATE OF` clause on PostgreSQL and
  SQLite), and whether it is disabled. A PostgreSQL replica trigger shows
  the mark `replica`. MS SQL Server, PostgreSQL and SQLite also show the
  triggers of a view. MySQL and MariaDB get an Events folder with
  the schedule of each event. The MySQL and MariaDB CREATE text of a trigger
  has no `FOLLOWS` or `PRECEDES` clause, so a trigger made again from its text
  fires after the other triggers with the same timing and event. The guide
  tells how to keep the order.
- A filter box keeps the path down to each match.
- The context menu builds a preview statement in the backend, so every name is
  quoted for the engine. It also builds the statements of an object: a CREATE
  draft, a SELECT, an INSERT and an UPDATE. The CREATE of a table is a draft,
  because it contains no index, no default and no constraint.
- A Properties dialog holds the facts of a relation, its columns, its indexes
  and its constraints, in one call to the backend.
- When the user expands a database in the tree, the application reads all the
  relations of that database in the background. The completion of the editor
  then knows a name that the tree has not opened.
- A branch that cannot load shows the error and a Retry button.

## Prerequisites

1. **Node.js and pnpm.** Node 20.19 or later, or 22.12 or later. Install pnpm
   with `npm install -g pnpm`.
2. **Rust.** Install the toolchain through [rustup](https://rustup.rs/).
3. **The system libraries of Tauri.** See below for Linux.

### Linux

Tauri 2 needs WebKitGTK 4.1. On Ubuntu 24.04 or Debian 13:

```sh
sudo apt update
sudo apt install -y \
    libwebkit2gtk-4.1-dev \
    libgtk-3-dev \
    libayatana-appindicator3-dev \
    librsvg2-dev \
    libxdo-dev \
    libssl-dev \
    libkrb5-dev \
    clang \
    pkg-config \
    build-essential \
    curl wget file
```

The Kerberos headers of `libkrb5-dev` and the `clang` compiler build the
Integrated Security of MS SQL Server. The `.deb` and `.rpm` packages that the
build makes depend on the Kerberos and the OpenSSL libraries of the system.

### macOS

Install the command line tools of Xcode: `xcode-select --install`.

### Windows

Install the C++ build tools of Visual Studio and the WebView2 runtime.

## Setup

```sh
pnpm install
```

The `prepare` script points git at the hooks of this repository.

## Development

```sh
pnpm dev
```

This starts the Vite server and builds the Rust binary with hot reloading for
the interface.

## Building

```sh
pnpm build
```

This command makes the macOS application bundle and the `.dmg` image, and
then it cross-compiles the Windows NSIS installer. The `app` and `dmg` bundle
formats build on macOS only, so the command stops on an other operating
system. The macOS installers appear under `backend/target/release/bundle/`.
The Windows installer appears under
`backend/target/x86_64-pc-windows-msvc/release/bundle/nsis/`. To build only
one operating system, run `pnpm build:macos` or `pnpm build:windows`.

To build the Linux packages, run this command on Linux:

```sh
pnpm build:linux
```

The command makes a `.deb` package, an `.rpm` package and an AppImage under
`backend/target/release/bundle/`.

The Windows cross-compilation needs these tools on the macOS machine:

```sh
rustup target add x86_64-pc-windows-msvc
cargo install cargo-xwin
brew install nsis llvm
```

`cargo-xwin` downloads the Windows SDK and the MSVC headers on the first
build. The MSI format needs the WiX toolset, which runs on Windows only, so a
build on macOS makes the NSIS installer and not the MSI installer.

## Tests and linters

```sh
pnpm test           # every unit test, both halves and the vendored crates
pnpm test:unit      # the frontend only
pnpm test:backend   # the backend and the vendored crates only
pnpm test:coverage  # the frontend with a coverage report
pnpm test:coverage:backend  # the backend with a coverage report
pnpm lint           # Prettier, ESLint, vue-tsc, clippy and rustfmt checks
pnpm format         # Prettier and rustfmt
pnpm verify         # the linters and then the tests
```

A pre-commit hook runs the formatters, the linters and the unit tests with
the coverage gate of each half, and it stops a commit that does not pass. The
frontend gate is the set of thresholds in `frontend/vitest.config.ts`. The
backend gate is a line coverage of 91% and a function coverage of 86%, which
`.githooks/pre-commit` sets. The
hook runs the checks on the staged files alone: it puts the changes that are
not staged, and the files that git does not track, into a stash for the run,
and puts them back after it. Run `git commit --no-verify` to step past the
hook when you know why.

The backend gate needs `cargo-llvm-cov`:

```sh
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov
```

### Tests against a live server

The unit tests need no server. SQLite runs in memory, and the other drivers are
covered by tests of their configuration, their type conversion and their
statement splitting.

The live tests run the PostgreSQL, MySQL, MariaDB and MS SQL Server drivers
against real servers in Docker. Each live test has a name that starts with
`live_` and the `#[ignore]` mark, so `cargo test` and the pre-commit hook do
not run them. The coverage gate also ignores their files.

To run the live tests:

1. Start the servers:

   ```sh
   docker compose -f backend/live/compose.yaml up -d
   ```

2. Wait until `docker compose -f backend/live/compose.yaml ps` shows each
   server as healthy. The first start of MS SQL Server can take a minute.

3. Run the tests:

   ```sh
   pnpm test:live
   ```

4. Stop the servers when you do not need them:

   ```sh
   docker compose -f backend/live/compose.yaml down
   ```

The script `backend/live/run.sh` reads the URL of each server from one
variable. When a variable is not set, the script sets the URL of the server
in `compose.yaml`. A test that runs without its variable, for example
through `cargo test -- --ignored`, writes a line that says so, and it passes.

| Variable            | Default URL                                         |
| ------------------- | --------------------------------------------------- |
| `SQLX_LIVE_PG`      | `postgres://postgres:LivePg16pass@127.0.0.1:15416`  |
| `SQLX_LIVE_MYSQL`   | `mysql://root:LiveMysql84pass@127.0.0.1:13384`      |
| `SQLX_LIVE_MARIADB` | `mysql://root:LiveMaria11pass@127.0.0.1:13311`      |
| `SQLX_LIVE_MSSQL`   | `mssql://sa:Live%23Mssql2022Pass@127.0.0.1:11433`   |

Write a special character of a password as its percent code. For example,
the `#` of the SQL Server password is `%23`. To test the PostgreSQL 18
server of `compose.yaml`, run:

```sh
SQLX_LIVE_PG=postgres://postgres:LivePg18pass@127.0.0.1:15418 pnpm test:live
```

Each test makes a database with a unique name, loads a fixture of
`backend/live/fixtures` into it, and removes the database at the end. A test
that fails also removes its database. An argument selects the tests by
name, so `pnpm test:live live_mysql` runs the MySQL tests alone. The passwords in
`compose.yaml` are for these local test servers alone.

## Layout

```
backend/              The Rust half
  src/
    commands.rs       The commands the interface calls
    db.rs             The shared data model
    db/columnar.rs    The binary frames that send the rows to the interface
    db/drivers.rs     The helpers that the drivers share
    db/drivers/       One file for each engine
    db/sink.rs        The receiver of the rows of a run: a response or a file
    error.rs          The error type and the payload the interface receives
    files.rs          The query files, their folders and their encodings
    history.rs        The records of the history and the saved statements
    jsonfile.rs       The JSON files of the settings, written safely
    main.rs           The start of the application
    menu.rs           The File menu of the operating system
    script.rs         The statements that the explorer builds for a relation
    secrets.rs        The keychain of the operating system
    session.rs        The server sessions of each connection
    sql.rs            Quoting rules, the statement splitter and the parameters
    state.rs          The open connections and the statements that run
    storage.rs        The connection record and its options
    store.rs          The settings, the history and the saved statements
    xlsx.rs           The writer of Excel files
  live/               The Docker servers and the fixtures of the live tests
  vendor/             The patched copies of tiberius and tokio-postgres
frontend/             The Vue half
  src/
    components/       The views
    layouts/          The shell of the application
    lib/              The calls to the backend and the pure helpers
    plugins/          The set-up of Vuetify, Monaco and the icons
    stores/           The state of the interface
    types/            The types of the data that the backend sends
docs/                 The GitHub Pages site
  guide/              The user guide, which the app bundles and the site publishes
tests/fixtures/       Test data that the frontend and the backend both read
```

## What it does not do

- The interface has no transaction control and no edit of a row in the grid.
- The user guide in `docs/guide/` gives the limits of each engine, for example
  the way a parameter changes a batch on MS SQL Server. Read it before you
  report a defect.

## Notes on the design

**The backend builds every connection configuration.** No component joins a
connection string by hand. A joined string loses the port of a MS SQL Server
connection, because the parser of `tiberius` reads the port only from inside
the `server` value. It also loses a password that has a semicolon or a brace.

**Errors give their reason.** A failed command returns a category, a message,
the chain of causes and, when the server gives it, the line and the column of
the fault.

**Each connection has its own lock.** One slow statement does not block the
other connections.

**Rows are arrays and not objects.** A statement can return two columns with the
same name, and an object would lose one of them.

## Licence

MIT. See [LICENSE](LICENSE).
