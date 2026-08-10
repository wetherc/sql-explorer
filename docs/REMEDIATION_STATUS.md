# Remediation status

This file records what the remediation plan asked for and what the repository
now holds. Read it beside `REMEDIATION_PLAN.md`.

## Phase 0 — A green build

| Item | State | Note |
| --- | --- | --- |
| B1, B2 MySQL compile errors | Done | The driver was rewritten. |
| B3 Type errors of the frontend | Done | `vue-tsc` passes. |
| B4 Failing unit tests | Done | 637 frontend tests and 249 backend tests pass. |
| B5 Tauri 1 on current Linux | Done | The project moved to Tauri 2. |
| npm scripts, ESLint, Prettier | Done | Also rustfmt and clippy. |
| `Cargo.lock` in the repository | Done | The ignore rule was removed. |
| Linux prerequisites in the README | Done | WebKitGTK 4.1. |

## Phase 1 — Statements run

| Item | State | Note |
| --- | --- | --- |
| F1 The Execute button did nothing | Done | The editor is now a component of this repository. |
| F2 The editor was not bound | Done | The binding is explicit and tested. |
| F3 The mock hid both faults | Done | A test asserts that Run reaches the backend. |
| Errors reach the user | Done | A notice that stays, with the reason and advice. |
| Keyboard execution, run the selection | Done | Ctrl or Cmd with Enter, and with Shift for the script. |

## Phase 2 — MS SQL Server

| Item | State | Note |
| --- | --- | --- |
| S6 The reason was discarded | Done | A structured payload with a kind, a message and the causes. |
| S1, S7 The port was lost | Done | The configuration is built with the builder, not from a string. |
| S2 TLS was not compiled in | Done | The TLS of the library of the system is on. |
| S4 The manual encryption block | Done | Removed. The transport setting drives it. |
| S3 Named instances | Done | `TcpStream::connect_named` resolves the port. |
| S5 No timeouts | Done | A connect timeout and a statement timeout, both settings. |
| S8 No liveness handling | Done | A check before use, one reconnection, and an event to the interface. |
| S9 Duplicated auth features | Done | `winauth` on Windows only; the GSSAPI dependency is gone. |
| Sec2 Hardcoded trust | Done | The default verifies the certificate. |

## Phase 3 — Correctness

| Item | State | Note |
| --- | --- | --- |
| C1 MS SQL Server conversion panicked | Done | Every read uses `try_get` and every column type is covered. |
| C2 PostgreSQL conversion panicked | Done | Optional targets and a wide type table. |
| C3 Invalid schema query | Done | `NOT LIKE 'pg\_temp\_%'`. |
| C4 Cross-database browsing | Done | The catalog view is named for the selected database. |
| C5 One lock for every connection | Done | Each connection holds its own. |
| C6 One bad record broke the list | Done | A record that cannot be read is left out with a warning. |
| C7 `LIMIT` on MS SQL Server | Done | The backend builds the statement from a dialect table. |
| C8 PostgreSQL multi-statement | Done | The simple protocol, with the command tag of each statement. |
| C9 MySQL statement splitting | Done | Quotes, comments, dollar tags and `DELIMITER`. |
| C10 Parameters and messages | Done | Both are wired through. |

## Phase 4 — Features

| Item | State | Note |
| --- | --- | --- |
| The column level | Done | Tables and views expand into their columns. |
| Cancellation | Done | A handle that works while the driver is busy. |
| Row limits and a windowed grid | Done | The limit is enforced in Rust; the grid draws only the rows in view. |
| Export | Done | CSV, JSON and the clipboard. |
| History and saved statements | Done | Both persist. |
| The open tabs persist | Done | |
| Completion from the objects read | Done | |
| A test button and advanced options | Done | |
| Tree refresh and a filter box | Done | |
| SQLite | Done | |
| AWS Athena | Done | Added as a first-class engine. |

## Phase 5 — Interface

| Item | State | Note |
| --- | --- | --- |
| One theme system | Done | Two themes, no hardcoded colour, the editor follows. |
| An application bar and a status bar | Done | |
| Flex layout in place of fixed heights | Done | |
| The context menu | Done | Uses `:target`. |
| Empty states | Done | Each one points at the action that fills it. |
| The grid: sort, absent values, inspection | Done | |
| Keyboard and accessibility | Partly | The tree takes focus and answers Enter and Space, and the icon buttons carry labels. A full pass over every view is still open. |
| Vuetify | Done | Pinned to the stable 4.1 line, not a pre-release. |

## Phase 6 — Platform and security

| Item | State | Note |
| --- | --- | --- |
| Tauri 2 | Done | |
| Passwords in the keychain | Done | A store in memory takes over where no keychain answers. |
| A restrictive content policy | Done | Set in `tauri.conf.json`. |
| Quoting helpers per dialect | Done | In `sql.rs`, used by every metadata query. |
| Signing and release automation | Open | Needs certificates and a release pipeline. |

## Features

The work after the remediation added these features. `docs/LIMITATIONS.md`
records what each one cannot do.

| Item | State | Note |
| --- | --- | --- |
| Script an object as DDL and as DML | Done | The backend builds the text and quotes every name. A built CREATE holds no index, no default and no constraint, so the interface names it a draft. |
| The messages that the server sends | Done | Severity, code, line and procedure. MS SQL Server gives no `PRINT` text, because `tiberius` drops the info tokens. |
| The schema in the background | Done | One command reads every relation of a database. The read holds the lock of the connection, so a statement of the user waits for it. |
| Completion of the columns of an alias | Done | The index is keyed by the qualified name, and one provider serves the whole application. |
| The plan of a statement | Done | Estimated and actual, in a result tab. The request covers one statement. |
| Transaction control | Open | Needs one session for each tab, a session generation that the reconnection path checks, and an attention packet for MS SQL Server. |
| Athena cost control | Done | The scan, the price at a rate the user sets, and the reuse of an earlier result with an age. |
| More object types in the tree | Done | Routines, indexes, constraints and partitions, under folders. |
| Format the statement | Done | A provider for the language, so the keys and the menu of the editor reach it. |
| More export formats and a partial export | Done | Markdown, INSERT statements and Excel. A whole result goes to a file from the backend. |
| Keyboard commands and a command palette | Done | One registry holds every key, and the palette lists them. |
| Edit the rows in the grid | Open | Needs transaction control first. |
| Query parameters | Done | Named parameters, bound by every engine but Athena, which takes literals. |
| Microsoft Entra ID authentication | Done | A token from the Azure CLI or from the user. The token stays out of the settings file. |
| Table properties | Done | One command collects the facts, the columns, the indexes and the constraints. |
| Keep a result | Done | A pinned tab holds its rows and the time of its run against the next statement. |

## Decisions that the plan asked for

1. **Tauri 2 first.** Done before the feature work, so nothing was built twice.
2. **Error detail in the interface.** Confirmed. The payload carries the kind,
   the message and the chain of causes.
3. **Vuetify.** Pinned to the stable 4.1 line.
4. **Engines.** MS SQL Server and AWS Athena are first-class. PostgreSQL, MySQL
   and MariaDB follow. SQLite is included. Redshift, DuckDB and BigQuery are not
   built, and the README no longer promises them.
5. **The transport default.** Verify the certificate, with three weaker settings
   the user can select.

## What is still open

- Signing and release automation for the three platforms.
- Integration tests against a live MS SQL Server, MySQL, PostgreSQL and Athena.
  The unit tests cover the configuration, the type conversion and the statement
  splitting, and SQLite is covered end to end.
- A full accessibility pass over every view.
- Transaction control in the interface. The drivers report whether the engine
  has transactions, but no button uses it yet. Three parts come first: one
  session for each tab, a session generation that the reconnection path
  checks, and an attention packet for MS SQL Server, which today keeps a
  stopped statement and its locks.
- The edit of a row in the grid, which needs transaction control.

## Test coverage

The frontend suite covers every statement, every function and every line of
`src/`, and every branch of the modules that carry logic:

```
Statements   : 100%   (2300/2300)
Functions    : 100%   (835/835)
Lines        : 100%   (2186/2186)
Branches     : 97.7%  (1532/1568)
```

The thirty-six branches that remain are one artifact, and no test can remove
them. Vue compiles a `v-if` that has no `v-else` into a conditional whose
second arm is a cached comment node. The tool that measures coverage never
records that arm as run, even when one test renders the block and another
test leaves it out. Every one of the thirty-six sits in a template, on such a
`v-if`, and each module of `src/lib`, `src/stores` and `src/types` holds every
branch. The threshold
in `vitest.config.ts` therefore holds `src/lib`, `src/stores` and
`src/types` at 100% on all four measures, and holds the views at 100% on
statements, functions and lines with a floor of 96% on branches.

The backend suite holds 249 tests. Every module has tests for its own
behaviour: the error kinds, the dialect rules, the statement splitter, the
parameter rewrite, the connection records, the history, the settings files,
the state of the open connections, the time limit of a statement, and each
driver's configuration, catalog statements and type conversion. SQLite is
covered end to end, because it needs no server.
