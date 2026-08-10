# SQL Explorer

A desktop client for SQL databases. It connects to MS SQL Server, AWS Athena,
PostgreSQL, MySQL, MariaDB and SQLite. It shows the objects of each server in a
tree, runs statements in tabs, and writes the results to a file.

The application is built with Vue 3, Vuetify and [Tauri 2](https://tauri.app/).

## What it does

**Connections**

- Five engines: MS SQL Server, AWS Athena, PostgreSQL, MySQL and MariaDB, and
  SQLite.
- The form shows the fields the selected engine uses and hides the rest.
- A test button opens the connection, confirms that it answers, and closes it.
- Transport settings: verify the certificate, encrypt without a check, encrypt
  when the server offers it, or send in clear text. The first is the default.
- A named MS SQL Server instance resolves its port through the SQL Browser
  service. Windows Integrated Security works on Windows.
- Timeouts for the connection and for a statement, a row limit, a read-only
  session, an application name, folders and colours.
- Passwords go into the keychain of the operating system. The settings file
  holds no password.
- A connection that stops answering is opened again, and the interface shows
  the state of each connection.

**Statements**

- One editor for each tab, with syntax colours and completion that draws on the
  objects the explorer has read.
- `Ctrl` or `Cmd` with `Enter` runs the statement under the cursor. The same
  keys with `Shift` run the whole script. A selection runs in place of the
  statement.
- A script runs statement by statement. The splitter respects quotes, comments,
  dollar tags and the MySQL `DELIMITER` command.
- A statement that runs can be stopped.
- The result grid draws only the rows in view, so a large result stays quick. It
  sorts, filters, marks a value that is absent, and opens a wide value.
- Results go to a CSV file, to a JSON file, or to the clipboard.
- The history and the saved statements persist, and so do the open tabs.

**Objects**

- Databases, schemas, tables, views and columns. A key column carries its own
  icon, and each column shows its type.
- A filter box keeps the path down to each match.
- The context menu builds a preview statement in the backend, so every name is
  quoted for the engine.

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
    pkg-config \
    build-essential \
    curl wget file
```

An earlier version of this application used Tauri 1, which needs
`libsoup-2.4` and `webkit2gtk-4.0`. Neither package ships on Ubuntu 24.04 or on
Debian 13, so that version could not be built there. Tauri 2 removes the
problem.

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

The installers appear under `backend/target/release/bundle/`.

## Tests and linters

```sh
pnpm test           # every unit test, both halves
pnpm test:unit      # the frontend only
pnpm test:coverage  # the frontend with a coverage report
pnpm lint           # ESLint, clippy and the formatters
pnpm format         # Prettier and rustfmt
pnpm verify         # the linters and then the tests
```

A pre-commit hook runs the formatters, the linters and the unit tests, and it
stops a commit that does not pass. Run `git commit --no-verify` to step past it
when you know why.

### Tests against a live server

The unit tests need no server. SQLite runs in memory, and the other drivers are
covered by tests of their configuration, their type conversion and their
statement splitting.

## Layout

```
backend/          The Rust half
  src/
    commands.rs   The commands the interface calls
    db.rs         The shared data model
    db/drivers/   One file for each engine
    error.rs      The error type and the payload the interface receives
    secrets.rs    The keychain of the operating system
    sql.rs        Quoting rules and the statement splitter
    state.rs      The open connections and the statements that run
    storage.rs    The connection record and its options
    store.rs      The settings, the history and the saved statements
frontend/         The Vue half
  src/
    components/   The views
    layouts/      The shell of the application
    lib/          The calls to the backend and the pure helpers
    stores/       The state of the interface
    types/        The shapes the backend sends
docs/             The state evaluation and the remediation plan
```

## Notes on the design

**The backend builds every connection configuration.** No component joins a
connection string by hand. An earlier version did, and the string lost the port
of every MS SQL Server connection, because the parser of `tiberius` reads the
port only from inside the `server` value. The same string lost any password that
held a semicolon or a brace.

**Errors carry their reason.** A failed command returns a kind, a message and
the chain of causes. An earlier version replaced every database error with one
fixed sentence, which made a set of deterministic faults look like intermittent
behaviour.

**Each connection holds its own lock.** One slow statement no longer blocks the
other connections.

**Rows are arrays and not objects.** A statement can return two columns with the
same name, and an object would lose one of them.

## Licence

MIT
