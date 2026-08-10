# SQL Explorer — State Evaluation & Remediation Plan

_Assessment date: 2026-08-10. Evaluated commit: `69162da` ("WIP: Attempt to fix MySQL driver compilation")._

---

## 1. Verdict

**The project does not currently build, and the one thing it exists to do — run a query — is
wired up incorrectly and cannot work even if it did build.**

Three independent blockers, each verified by running the toolchain rather than by reading:

| Check | Result |
| --- | --- |
| `cargo check` (backend) | **fails** — 2 hard type errors in `mysql.rs` |
| `vue-tsc -b` (`pnpm build`) | **fails** — 50+ type errors across 8 files |
| `vitest run` (frontend) | **23 of 46 tests fail** (4 of 6 suites red) |
| Tauri v1 system deps on current Linux | **unavailable** — needs `libsoup-2.4` / `webkit2gtk-4.0`, dropped by Ubuntu 24.04+ |

The reported SQL Server flakiness is real and has **four distinct root causes**, all confirmed
against the `tiberius` 0.12.3 source. None of them are intermittent — they are deterministic
failures that only *look* flaky because the actual error text is discarded before it reaches the
UI (§4.4, S6).

The architecture underneath is sound: a clean `DatabaseDriver` trait, per-connection state, a
sensible Pinia store split. The problems are concentrated in wiring, error handling, and
type-mapping — not in the design. This is recoverable in roughly the effort it would take to
build it once more carefully, and most of the value lands in the first two phases.

---

## 2. How this was verified

Claims below are backed by executed commands, not inspection alone:

- **Backend compile errors.** The Tauri shell can't be compiled in this environment (§4.1, B5),
  so `src/db/`, `src/db.rs` and `src/error.rs` were extracted into a standalone crate with the
  same dependency set minus Tauri, and type-checked in isolation. Two errors surfaced; after
  patching only those two, the driver layer compiles clean. So the backend has exactly two
  compile errors, both in `mysql.rs`, and everything behind them is at least type-correct.
- **Frontend.** `pnpm install`, then `pnpm vitest run` and `pnpm exec vue-tsc -b --force`.
- **`tiberius` behaviour.** Read from the vendored crate source at
  `~/.cargo/registry/src/.../tiberius-0.12.3` — `client/config/ado_net.rs` (connection-string
  parsing), `client/config.rs` (feature gating), `client/connection.rs` (TLS negotiation),
  `row.rs` (panic semantics).
- **Monaco wiring.** Read from the installed `monaco-editor-vue3@1.0.5` type definitions.

---

## 3. What works today

Worth stating plainly, because the list of defects is long and the foundation is not the problem:

- The `DatabaseDriver` trait (`db/drivers.rs`) is a clean abstraction, correctly `Send + Sync`,
  and the three drivers implement it consistently.
- Multi-connection state is modelled properly end to end: `AppState.connections` is keyed by
  connection id, and every command takes a `connection_id`.
- The Pinia store split (`connection` / `connectionManager` / `explorer` / `query` / `tabs` /
  `navigation`) has sensible boundaries and per-tab query state.
- The Postgres and MSSQL drivers type-check and their metadata queries are broadly correct.
- Connection persistence via `tauri-plugin-store` works, and the connection CRUD UI is complete.
- `connectionManager.spec.ts` and `navigation.spec.ts` pass — the stores they cover are healthy.

---

## 4. Defect inventory

Severity: **P0** blocks build or core function · **P1** wrong results, crashes, or the reported
connectivity problem · **P2** security, robustness, polish.

### 4.1 P0 — The build is broken

| # | Defect | Location | Fix |
| --- | --- | --- | --- |
| B1 | `let (_, mut response) = self.with_conn(...)` destructures a `QueryResponse` as a 2-tuple. `with_conn` already unwraps the `(Conn, T)` pair and returns `T`. E0308. | `backend/src/db/drivers/mysql.rs:74` | `let mut response = self.with_conn(...).await?;` |
| B2 | `result.collect().await` returns `Result<Vec<Row>, Error>`, not `Vec<Result<Row, _>>`. E0308. | `backend/src/db/drivers/mysql.rs:176` | `let collected_rows: Vec<MySqlRow> = result.collect().await?;` and delete the manual re-collect loop below it. |
| B3 | `pnpm build` runs `vue-tsc -b` first and fails with 50+ errors, so no production bundle can be produced. Breakdown in §4.6. | `frontend/` | See §4.6 — mostly stale tests plus three real type bugs. |
| B4 | 23 of 46 unit tests fail. `connection.spec.ts` tests an API that no longer exists (`store.dbType`, `connect(string, dbType)`, `disconnect()` with no args); `explorer.spec.ts` asserts against a changed node shape. | `frontend/src/stores/__tests__/`, `components/__tests__/` | Rewrite against the current store signatures; delete assertions for removed behaviour. |
| B5 | Tauri v1 pulls `soup2-sys` → `libsoup-2.4` and `webkit2gtk-4.0`. Both were removed from Ubuntu 24.04+ and Debian 13 (only 3.0/4.1 ship). The `apt` line in `README.md` fails on any current distro. Tauri v1 is also EOL. | `backend/Cargo.toml`, `README.md` | Strategic — see Phase 6. Short term, document that Linux builds need Ubuntu 22.04 or a container. |

### 4.2 P0 — Query execution cannot work

| # | Defect | Location | Fix |
| --- | --- | --- | --- |
| F1 | **The Execute button is a no-op.** `monaco-editor-vue3@1.0.5` emits `editorDidMount`, but the template listens for `@mount`. `handleEditorMount` therefore never runs, `monacoInstance` stays `null`, and `handleExecute`'s guard `if (currentTab.value && monacoInstance.value)` is never true. Nothing is ever sent to the backend. | `frontend/src/components/QueryView.vue:23`, `:104`, `:114` | Listen for `@editorDidMount`. |
| F2 | The editor is also not data-bound. The component's prop is `value` with `update:value`; the template uses `v-model` (i.e. `modelValue` / `update:modelValue`). So `initialQuery` never reaches the editor and edits never reach the store. | `frontend/src/components/QueryView.vue:19` | Use `v-model:value="query"`, and drive `handleExecute` from that ref instead of reaching into the editor instance. |
| F3 | The test mock hides both of the above: `vitest.setup.ts` stubs Monaco with `modelValue`/`update:modelValue` props, so tests would pass against an interface the real package does not have. | `frontend/vitest.setup.ts:6-13` | Make the mock mirror the real contract (`value` / `update:value` / `editorDidMount`), then add a test that asserts Execute reaches `queryStore.executeQuery`. |

> These three together mean the app has never been able to run a query through the UI in its
> current state. Any connectivity debugging done through the UI was measuring the wrong thing.

### 4.3 P1 — SQL Server connectivity (the reported problem)

All four causes are deterministic. Confirmed against `tiberius-0.12.3` source.

| # | Root cause | Evidence | Fix |
| --- | --- | --- | --- |
| S1 | **The port is silently discarded.** The frontend builds `server=host;port=1433;...`, but `tiberius` only ever reads the port from *inside* the `server` value (`server=host,port`). There is no `port` key in its parser — `ado_net.rs::server()` splits `server` on `,` and ignores everything else. Every MSSQL connection therefore goes to the default 1433 regardless of what the user typed. | `tiberius-0.12.3/src/client/config/ado_net.rs:56-70`; `frontend/src/stores/connection.ts:22` | Emit `server=tcp:{host},{port}`. |
| S2 | **TLS is not compiled in at all.** `Cargo.toml` sets `default-features = false` for `tiberius` and enables none of `rustls` / `native-tls` / `vendored-openssl`. Without one of those, `ConfigString::encrypt()` is `#[cfg]`-ed out entirely, `encryption` is pinned to `NotSupported`, and `post_login_encryption` is a no-op stub. Consequence: **Azure SQL and any server with Force Encryption cannot be connected to at all**, and on servers that do allow it, credentials cross the wire with only TDS obfuscation. | `backend/Cargo.toml:22`; `tiberius/src/client/config.rs:349-364`, `:53-59`; `client/connection.rs:130-156` | Add `rustls` to the tiberius feature list; drop the manual encryption sniffing and let the connection string drive it. |
| S3 | **Named instances are ignored.** `from_ado_string` populates `instance_name` for `server=host\SQLEXPRESS`, and `sql-browser-tokio` *is* enabled — but `connect()` calls plain `TcpStream::connect(config.get_addr())`, which never consults the SQL Browser. Named instances on dynamic ports connect to the wrong port or fail. | `backend/src/db/drivers/mssql.rs:62-64`; `tiberius/src/client/config.rs:246` | Use `TcpStream::connect_named(&config)` from the `sql-browser-tokio` extension trait. |
| S4 | The hand-rolled encryption block lowercases the connection string and re-derives an `EncryptionLevel`, then overrides whatever the parser decided. With no TLS feature compiled it is dead weight; with one compiled it fights the parser and will override a correct `Encrypt=` value. `TrustServerCertificate` is already handled by the parser. | `backend/src/db/drivers/mssql.rs:39-57` | Delete the block. |
| S5 | **No connect timeout anywhere,** in any driver. An unreachable host hangs the `connect` command indefinitely; the UI spinner never resolves and there is no cancel. Reads as "flaky". | all three drivers | Wrap connects in `tokio::time::timeout`; make it a per-connection setting. |
| S6 | **The real error never reaches the user.** `Error::serialize` collapses every Tiberius/MySQL/Postgres error to the literal string `"Database error occurred."` and every IO error to `"Connection error occurred."`. The actual cause is written to `log::error!` only — invisible in a packaged desktop app. This is why the failures above present as unexplained flakiness rather than as bugs. | `backend/src/error.rs:47-52` | Return a structured `{ kind, message, detail }`. Desktop apps are single-tenant and local; there is no attacker to withhold detail from, and the cost is exactly this class of undiagnosable failure. |
| S7 | The generated connection string is malformed: `...TrustServerCertificate=true;` is followed by `;password=`, producing an empty `;;` pair. | `frontend/src/stores/connection.ts:22-28` | Build from an array and `join(';')`. |
| S8 | **No liveness handling.** A dropped socket leaves a dead client in `AppState.connections` forever; every subsequent query fails until the user manually disconnects and reconnects, with no indication why. | `backend/src/commands.rs`, `state.rs` | Health-check on borrow; auto-reconnect once with backoff; emit a connection-state event to the UI. |
| S9 | `integrated-auth-gssapi` is listed twice in the feature array, forces a `libkrb5` build dependency on every platform, and `winauth` is absent — so Windows Integrated Security does not work. | `backend/Cargo.toml:22` | De-duplicate; make it target-conditional (`winauth` on Windows, gssapi on Unix). |

### 4.4 P1 — Crashes and wrong results

| # | Defect | Location | Fix |
| --- | --- | --- | --- |
| C1 | **MSSQL result conversion panics on ordinary data.** `tiberius::Row::get` is documented to panic when the SQL→Rust conversion is impossible. `row_to_json` maps `ColumnType::Intn` to `i32` — but `Intn` is the nullable-integer type covering `TINYINT` through `BIGINT`, so any `BIGINT` column panics. The catch-all arm reads every remaining type as `&str`, so `DATETIME`, `UNIQUEIDENTIFIER`, `VARBINARY`, `DECIMAL` and `MONEY` all panic too. | `backend/src/db/drivers/mssql.rs:259-280`; panic documented at `tiberius/src/row.rs:384-389` | Switch to `try_get`, match on the real column type set, and fall back to a string rendering rather than a panic. |
| C2 | Postgres conversion panics on `NULL`: `row.get::<_, bool>(i)` and friends are non-`Option`, and `tokio_postgres::Row::get` panics on a null in a non-nullable target. Unmapped types are rendered as the literal text `"Unsupported type: …"` and silently become the cell's value. | `backend/src/db/drivers/postgres.rs:169-195` | Use `Option<T>` targets throughout; add `NUMERIC`, `DATE`, `TIMESTAMP(TZ)`, `UUID`, `JSON(B)`, `BYTEA`, and array types. |
| C3 | `list_schemas` for Postgres emits invalid SQL — `nspname.starts_with('pg_temp_')` is not PostgreSQL syntax. The call always errors, so **schema expansion is broken for every Postgres connection.** | `backend/src/db/drivers/postgres.rs:120` | `AND nspname NOT LIKE 'pg\_temp\_%'`. |
| C4 | **Cross-database browsing returns the wrong tables.** `list_tables`/`list_columns` query `INFORMATION_SCHEMA` (MSSQL) or `pg_tables` (Postgres), both of which are scoped to the *currently connected* database. The selected `database` argument is used only as a schema fallback. Expanding schema `dbo` under database `Foo` lists tables from whatever database the connection actually attached to. | `backend/src/commands.rs:97`; `mssql.rs:228`, `postgres.rs:133` | MSSQL: three-part-name the catalog view (`[{db}].INFORMATION_SCHEMA.TABLES`). Postgres: a connection is bound to one database — either open a per-database connection on expand, or scope the tree to the connected database only. |
| C5 | **One slow query freezes every connection.** `execute_query` holds the global `Mutex<HashMap<..>>` across the entire `await`. All connections are serialised behind one lock, and disconnect/cancel cannot run while a query is in flight. | `backend/src/commands.rs:65-67` (and every other command) | Store `Arc<Mutex<Box<dyn DatabaseDriver>>>` per connection; clone the `Arc` out, release the map lock, then await. |
| C6 | `get_connections` calls `.unwrap()` on `serde_json::from_value` for each stored entry — one malformed or schema-drifted record panics the command and the connection list never loads, with no way to recover from the UI. | `backend/src/commands.rs:121` | `filter_map` with a warning log, so one bad record can't take out the list. |
| C7 | MSSQL "Select Top 1000 Rows" generates `SELECT * FROM {table} LIMIT 1000;` — `LIMIT` is not T-SQL, and the table name is unqualified. The context-menu action always errors on the one engine it was presumably written for. | `frontend/src/components/DbExplorer.vue:111-114` | `SELECT TOP 1000 * FROM [{db}].[{schema}].[{table}];` |
| C8 | Postgres `execute_query` uses the extended protocol (`client.query`), which permits only one statement per call and returns rows only. Multi-statement scripts fail, and `INSERT`/`UPDATE`/DDL return an empty result with no row count and no message. | `backend/src/db/drivers/postgres.rs:78` | Use `simple_query` when there are no parameters, and map `CommandComplete` tags into `QueryResponse.messages`. |
| C9 | MySQL `execute_query` splits scripts on a bare `;`, which breaks on semicolons inside string literals, comments, or `DELIMITER` blocks. | `backend/src/db/drivers/mysql.rs:69` | Use a real statement splitter that respects quoting and comments. |
| C10 | `QueryResponse.messages` is populated by no driver, and `queryParams` is never sent by the frontend — so the parameter plumbing that exists end-to-end in Rust is unreachable. | `frontend/src/stores/query.ts:41`; all drivers | Wire `queryParams` through; populate `messages` with row counts and server notices. |

### 4.5 P2 — Security

| # | Defect | Location |
| --- | --- | --- |
| Sec1 | **Passwords are stored in plaintext** in `.settings.dat` via `tauri-plugin-store`. The `keyring` crate is already a declared dependency and is entirely unused — the intent was clearly there. | `backend/src/commands.rs:127-145`, `storage.rs:23` |
| Sec2 | `TrustServerCertificate=true` is hardcoded into every MSSQL connection string by the frontend, with no UI to turn it off. Once TLS is actually enabled (S2), this makes every connection trivially MITM-able. | `frontend/src/stores/connection.ts:22` |
| Sec3 | `"csp": null` disables the webview content-security-policy entirely. | `backend/tauri.conf.json` |
| Sec4 | Query text is interpolated into `format!` strings in the MySQL driver (`SHOW TABLES FROM \`{}\``) using identifiers that originate from server metadata. Low risk today, but it is the pattern that becomes an injection vector once the explorer accepts user-supplied filters. | `backend/src/db/drivers/mysql.rs:107`, `:126` |

### 4.6 Type errors and dead code

The 50+ `vue-tsc` errors are mostly stale tests, but three are real:

- `AppTreeview.vue:5` and `DbExplorer.vue:44` — `ExplorerNode.data.type` doesn't include
  `'connection'`, but both the tree and the explorer construct and compare against connection
  nodes. Add `'connection'` to the union in `stores/explorer.ts:16`.
- `QueryView.vue:63` — `splitpanes` ships no types; needs a `declare module` shim in `env.d.ts`.
- Unused imports (`computed` in two components, `watch` in `QueryView`) fail under
  `noUnusedLocals`.

Dead and stale artefacts to remove:

- **`ConnectionDialog.vue`** — referenced by nothing, and calls `connectionStore.connect(string, dbType)`,
  a signature that was replaced two refactors ago. Delete it.
- **`connection.spec.ts`** — tests the same removed API. Rewrite from scratch.
- **`GEMINI.md`** — another tool's session notes sitting at the repo root. Move to `docs/` or delete.
- **`backend/Cargo.lock`** is committed but simultaneously `.gitignore`d (`.gitignore:43`). For an
  application it should be committed — remove the ignore rule rather than the file.
- **`README.md`** documents `pnpm test:unit`, `pnpm lint` and `pnpm format`; none of these
  scripts exist. There is no linter or formatter configured at all.

---

## 5. Feature gaps

Measured against what the README advertises and what a SQL explorer needs to be usable.

**Core query workflow**
- No query cancellation — a runaway query can only be escaped by killing the app.
- No row limit, streaming, or pagination. Every result set is fully materialised in Rust, JSON
  -serialised across the IPC bridge, and handed to a non-virtualised `v-data-table`. A
  `SELECT *` against a large table will exhaust memory or freeze the UI.
- No "execute selection" and no `Ctrl`/`Cmd`+`Enter` shortcut.
- No rows-affected / elapsed-time feedback for non-`SELECT` statements.
- No result export (CSV, JSON, clipboard).
- No query history, no saved queries, no persistence of open tabs across restarts.
- No transaction control (begin/commit/rollback).

**Schema exploration**
- **The column level is never reached.** `list_columns` is implemented in all three drivers and
  registered as a command, but no frontend code calls it. Table nodes are created without a
  `children` array, so they render as leaves and cannot expand.
- No refresh action on the tree — schema changes require a full reconnect.
- No search/filter across the object tree.
- No views, indexes, keys, stored procedures, or DDL preview.
- MySQL `list_schemas` returns an empty vec by design, which is defensible, but nothing in the
  UI explains the resulting shape difference.

**Connections**
- No "Test Connection" button in the form.
- No advanced options: SSL mode, trust-certificate toggle, connection timeout, integrated auth,
  application name, read-only intent.
- No connection grouping/folders, no colour coding, no duplicate action.
- The connection form is engine-agnostic — it shows the same six fields regardless of engine.

**Engine coverage**
- README advertises MySQL, MariaDB, Postgres and MSSQL as supported, with Redshift, SQLite,
  DuckDB and BigQuery planned. Only three drivers exist. SQLite and DuckDB are by far the
  cheapest additions (no network, no TLS, no auth) and would prove the driver abstraction.

---

## 6. UI/UX gaps

- **The theme is incoherent.** `plugins/vuetify.ts` configures no theme, so Vuetify defaults to
  *light*, while `QueryTabs.vue` and `AppTreeview.vue` hardcode dark hex values (`#272727`,
  `#1E1E1E`, `#383838`) and Monaco is pinned to `vs-dark`. The result is dark panels floating in
  a light chrome. Define a proper Vuetify dark theme, drive the component colours from theme
  tokens, and bind Monaco's theme to it.
- Default window is **800×600** with no minimum size — far too small for a three-pane database
  IDE, and the layout collapses below roughly 900px.
- `.query-view { height: calc(100vh - 112px) }` is a magic number that breaks whenever the tab
  bar or toolbar changes height. Use flex layout.
- No application bar and no status bar. There is nowhere to surface the active connection, row
  count, elapsed time, or connection health.
- Errors appear only inside the results pane's "Messages" tab — a user who never clicks that tab
  sees a query silently do nothing. Needs a snackbar/toast plus inline error state.
- The explorer context menu uses `<v-menu absolute :style="{top,left}">`; `absolute` was removed
  in Vuetify 3+. Use `:target="[x, y]"`.
- `AppLayout.vue:38` renders the explorer only when `isConnected` — with no connections the
  sidebar is simply blank. Needs an empty state pointing at "New Connection".
- No per-node loading indicator; a single global `v-progress-linear` fires for any expansion.
- Results grid: no virtualisation, no column resize, no sort, no distinct `NULL` rendering, no
  cell-level copy, no wide-value inspection.
- No keyboard navigation anywhere, and no accessibility pass (the icon-only rail has tooltips
  but no `aria-label`s).
- Vuetify is pinned to `4.0.0-beta.2`, a pre-release, which is a live risk for a UI being built
  out — breaking changes will land without a semver signal.

---

## 7. Implementation plan

Sequenced so that each phase leaves the tree in a better verifiable state than it found it.
Estimates are working days for one developer.

### Phase 0 — Restore a green build (1–2 days)

The tree is currently unverifiable; nothing else can be trusted until this lands.

1. Fix `mysql.rs:74` and `mysql.rs:176` (B1, B2); remove the now-unused `TryStreamExt` import.
2. Fix the three real type errors (§4.6) and strip unused imports.
3. Delete `ConnectionDialog.vue`; rewrite `connection.spec.ts` and `explorer.spec.ts` against
   the current store APIs.
4. Add the missing npm scripts the README already promises (`test:unit`, `lint`, `format`), and
   configure ESLint + Prettier.
5. Add CI: `cargo check`, `cargo clippy -D warnings`, `cargo test`, `vue-tsc`, `vitest run`.
6. Un-ignore `backend/Cargo.lock`; correct the README's Linux prerequisites.

**Done when:** `cargo check`, `vue-tsc -b`, and `vitest run` all pass, and CI enforces it.

### Phase 1 — Make queries actually run (1–2 days)

7. Fix the Monaco event and binding (F1, F2), and make `handleExecute` read from the bound ref.
8. Correct the test mock to match the real component contract (F3), and add a regression test
   asserting Execute reaches the store.
9. Surface errors properly: snackbar + inline results-pane state.
10. Add `Ctrl`/`Cmd`+`Enter` to execute, and execute-selection-if-any.

**Done when:** typing a query and pressing Execute against a live database returns rows, and a
failing query shows the server's message.

### Phase 2 — SQL Server connectivity (2–3 days)

The user-reported problem. Order matters: S6 first, so the rest is diagnosable.

11. **S6 first** — replace the error-flattening serializer with a structured
    `{ kind, message, detail }` payload, and render `detail` in the UI.
12. Emit `server=tcp:{host},{port}` from the frontend and build the string via `join(';')` (S1, S7).
13. Enable the `rustls` feature for tiberius; delete the manual encryption sniffing; let the
    connection string drive encryption (S2, S4).
14. Use `TcpStream::connect_named` so named instances resolve via SQL Browser (S3).
15. Add connect and query timeouts across all three drivers (S5).
16. Add health-check-on-borrow with single-retry reconnect, and emit connection-state events (S8).
17. De-duplicate and target-gate the auth features; verify Windows Integrated Security (S9).
18. Expose TLS mode and trust-certificate as real form fields, defaulting to *verify* rather
    than the current hardcoded `TrustServerCertificate=true` (Sec2).

**Done when:** connections succeed against a non-default port, a named instance, and an
encryption-required server (Azure SQL is the useful test target), and every failure produces an
actionable message.

### Phase 3 — Correctness and stability (3–4 days)

19. Rewrite `mssql.rs::row_to_json` on `try_get` with full type coverage (C1).
20. Rewrite `postgres.rs::row_to_json` with `Option<T>` targets and full type coverage (C2).
21. Fix the invalid `list_schemas` SQL (C3).
22. Fix cross-database browsing for MSSQL and Postgres (C4).
23. Move to per-connection `Arc<Mutex<..>>` so one query can't block the app (C5).
24. Make `get_connections` resilient to malformed records (C6).
25. Fix the MSSQL `SELECT TOP` generation (C7).
26. Postgres: `simple_query` path plus `CommandComplete` → `messages` (C8).
27. MySQL: quote/comment-aware statement splitting (C9).
28. Wire `queryParams` through and populate `messages` with row counts (C10).
29. Add driver integration tests behind the existing `*_TEST_DB_URL` env-var pattern, with
    docker-compose fixtures for all three engines.

**Done when:** a table containing `BIGINT`, `DECIMAL`, `DATETIME`, `UUID`, `NULL` and binary
columns round-trips through all three drivers without a panic and with correct JSON.

### Phase 4 — Round out the feature set (5–7 days)

30. Column level in the explorer — call the `list_columns` command that already exists, give
    table nodes a `children` array, add per-node loading state.
31. Query cancellation, with a token per running query.
32. Result windowing: a configurable `maxRows` cap enforced in Rust, a virtualised grid, and a
    clear "truncated at N rows" indicator.
33. Result export: CSV, JSON, and copy-to-clipboard for selection.
34. Query history (persisted) and saved queries.
35. Persist open tabs and their contents across restarts.
36. Schema-aware Monaco autocomplete — register a completion provider fed from the explorer's
    cached metadata.
37. "Test Connection" button, plus the advanced connection options listed in §5.
38. Tree refresh and a filter box.
39. SQLite and DuckDB drivers — cheapest possible validation that the trait generalises.

### Phase 5 — UI and interaction (3–4 days)

40. Define a real Vuetify dark theme; remove every hardcoded hex; bind Monaco's theme to it.
41. Application bar and status bar (connection, elapsed time, row count, health).
42. Replace the magic-number heights with a flex layout; raise the default window to 1280×800
    with a sensible minimum.
43. Fix the context menu to use `:target`.
44. Empty states for "no connections" and "no results".
45. Results grid: sort, resize, `NULL` styling, cell inspection.
46. Keyboard navigation and an accessibility pass.
47. Decide on Vuetify: pin to a stable 3.x, or accept the 4.x beta and track its breaking changes.

### Phase 6 — Platform and security (4–6 days, partly strategic)

48. **Migrate to Tauri v2.** This resolves B5 — v1 cannot be built on any current Linux distro
    because `libsoup-2.4` and `webkit2gtk-4.0` no longer ship, and v1 is EOL so that will not
    improve. The migration also brings a real capability/permission model. This is the single
    highest-leverage structural change and should be scheduled deliberately rather than deferred
    indefinitely.
49. Move passwords into the OS keychain via the already-declared `keyring` dependency; store
    only a reference in `.settings.dat`, and migrate existing plaintext records on first run (Sec1).
50. Set a restrictive CSP (Sec3).
51. Replace `format!`-built identifiers with a proper quoting helper per dialect (Sec4).
52. Bundle and sign for macOS/Windows/Linux; wire release automation.

---

## 8. Suggested sequencing

Phases 0–2 are the critical path and are worth doing as one continuous push (roughly a week):
they take the project from "does not build" to "connects to SQL Server reliably and runs
queries", which is the whole of the user's stated complaint. Phase 3 should follow immediately —
without it, the app will panic on the first table containing a date or a `BIGINT`.

Phases 4–5 are the "round out" work and can be interleaved or reordered by preference; each item
is independent. Phase 6 item 48 (Tauri v2) should be scheduled before Phase 4 if Linux support
matters, because the migration will touch every command signature and doing it after the feature
build-out means reworking more surface area.

## 9. Decisions needed

1. **Tauri v2 migration** — do it before the feature work (recommended, less rework), after, or
   accept that Linux builds require Ubuntu 22.04 indefinitely?
2. **Error detail** — confirm the move from sanitised strings to full error detail in the UI.
   Recommended for a local single-user desktop tool; it is the direct cause of the "flaky"
   diagnosis.
3. **Vuetify 4 beta** — hold on the beta and absorb breaking changes, or pin back to stable 3.x?
4. **Engine scope** — are Redshift/SQLite/DuckDB/BigQuery still in scope, or should the README
   be trimmed to what will actually ship?
5. **TLS default** — recommended default is verify-certificate with an explicit opt-out toggle,
   replacing today's unconditional `TrustServerCertificate=true`.
