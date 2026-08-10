# Future work

This document holds the designs for four known weaknesses that the current
code does not correct. Each section gives the problem, the design of the
correction, the order of the work, and the risks.

## 1. Stream the rows of a result instead of gathering them in memory

### The problem

Every driver returns a `QueryResponse` that holds every row as
`Vec<Vec<serde_json::Value>>`. The whole response then crosses the Tauri
bridge as one JSON body. Three costs follow:

- A large result takes several times its raw size in memory, because each
  cell is a `serde_json::Value`.
- The user sees no row until the last row has arrived.
- The export command runs the statement again and still gathers every row
  in memory before it writes the file (`export_query` in
  `backend/src/commands.rs`).

### The design

Add a sink interface that receives rows as the driver reads them:

```rust
/// Where the rows of one execution go.
pub trait RowSink: Send {
    /// Starts one result set. Called once for each set of the run.
    fn begin_set(&mut self, columns: Vec<ColumnInfo>) -> Result<()>;
    /// Receives one row. The answer tells the driver to go on or to stop
    /// the read, for example because a row limit is reached.
    fn row(&mut self, row: Vec<JsonValue>) -> Result<SinkControl>;
    /// Ends one result set.
    fn end_set(&mut self, truncated: bool) -> Result<()>;
    /// Receives one message of the server.
    fn message(&mut self, message: Message);
}

pub enum SinkControl {
    Continue,
    Stop,
}
```

Add one method to `DatabaseDriver`:

```rust
async fn execute_stream(
    &mut self,
    query: &str,
    params: Option<&QueryParams>,
    options: &ExecOptions,
    sink: &mut dyn RowSink,
) -> Result<RunSummary>;
```

`RunSummary` carries `rows_affected`, `elapsed_ms` and `stats`. The present
`execute_query` becomes a default method: it runs `execute_stream` into a
`BufferSink` that builds the present `QueryResponse`. One code path then
serves both forms.

Two sinks:

- `BufferSink` keeps the rows up to `max_rows` and reports `truncated`.
  It reproduces the behaviour of today.
- `FileSink` writes each row to a CSV or JSON file as it arrives. The
  export command uses it, and a large export then holds one row at a time
  in memory.

### The drivers

- **SQLite**: the read runs inside `spawn_blocking`, so the sink must cross
  a thread. Send the rows through a bounded channel in blocks of about one
  thousand, and drive the sink on the async side.
- **PostgreSQL**: use `query_raw`, which gives a `RowStream`. The simple
  query path streams per statement in the same way.
- **MySQL**: use `query_iter` and forward each row.
- **MS SQL Server**: the driver already walks a `QueryStream` in
  `collect_sets`; forward each item to the sink instead of a vector.
- **Athena**: the page loop in `read_results` forwards each page to the
  sink instead of a vector.

### Paging for the grid

A second step bounds the memory of the interface as well. The backend keeps
the buffered result of the view in a cache keyed by the request identifier.
A new command `fetch_rows(request_id, offset, count)` returns one window of
rows, and the virtual scroller of the grid asks for windows as the user
scrolls. The cache entry goes when the tab runs again, when the tab closes
and when the connection closes. The Excel export builds its workbook in the
interface, so it reads the windows through the same command.

### The order of the work

1. Add `RowSink`, `SinkControl`, `RunSummary`, `BufferSink` and the default
   `execute_query`, with unit tests against a stub driver.
2. Convert the SQLite driver and the PostgreSQL driver, whose libraries
   stream naturally. The end-to-end tests run against SQLite in memory.
3. Convert the MS SQL Server driver, the MySQL driver and the Athena
   driver.
4. Switch the export command to `FileSink` and delete the buffered export
   path.
5. Add the row cache and `fetch_rows`, and convert the grid to windowed
   reads.

### The risks

- The sink crosses `await` points, so it needs `Send`, and the SQLite
  bridge adds a channel whose backpressure must be bounded.
- A sink that stops the read early must leave the connection in a clean
  state; on a wire protocol the driver must drain the rest of the stream.
- The cache of step 5 holds rows in the backend; its size needs a bound and
  the entries need a clear owner, or the leak moves from the interface to
  the backend.

## 2. Report an Entra access token that has expired

### The problem

The `EntraAccessToken` method stores the pasted token in the keychain as if
it were a password. Such a token lives for about one hour. The silent
reconnection path (`ensure_healthy`) reuses the stored token, so a
reconnection after the hour fails with a message that does not name the
cause.

### The design

- Read the `exp` claim of the token when a connection opens. The claim sits
  in the middle part of the JWT, base64 encoded; no signature check is
  needed to read a date the client itself acts on.
- Refuse a token that is already expired before a socket opens, with the
  message: "The access token has expired. Paste a new one, or use the Azure
  CLI method, which reads a fresh token on each connection."
- Map a login refusal under this method to the same message when the token
  age passes one hour, in the style of `describe_login` in
  `backend/src/db/drivers/mssql.rs`.
- In the interface, an authentication failure on a connection with this
  method opens the connection form with the token field focused.
- Add one line to `LIMITATIONS.md`: the pasted token is not refreshed, and
  the Azure CLI method is the durable choice.

### The order of the work

1. A pure function `token_expiry(token: &str) -> Option<SystemTime>` with
   tests for a well formed token, a token without `exp`, and text that is
   not a JWT.
2. The check in `MssqlDriver::connect` and the message mapping.
3. The form behaviour in the interface and the documentation line.

### The risks

Small. The parse must not reject a token it cannot read; an unreadable
token goes to the server unchanged, and the server stays the judge.

## 3. Give every command one request shape

### The problem

The invoke layer in `frontend/src/lib/api.ts` sends some commands flat
(`execute_query` spreads its fields) and some nested under `request`
(`explain_query`, `script_object`, the file commands). Null against
undefined is normalised by hand in some methods and not in others. Every
new command decides its shape again, and a mismatch only fails at run time.

### The design

- Convention: a command with more than two fields takes one `request`
  struct, named after the command, with `serde(rename_all = "camelCase")`
  and `serde(default)` on the optional fields. A command with one or two
  plain fields stays flat.
- Backend: convert `execute_query`, `cancel_query`, `schema_snapshot`,
  `preview_query`, `script_object` (already nested), and the `list_*`
  family to request structs.
- Frontend: one helper `call<T>(command, request?)` that walks the request
  and turns `undefined` into `null`, so the normalisation lives in one
  place. Each api method becomes one line.
- The application is a desktop binary whose two halves ship together, so
  both sides change in one commit and no compatibility shim is needed.

### The order of the work

1. The helper and the conversion of the `list_*` family, with the tests.
2. The conversion of the execution commands.
3. A lint note in `api.ts` that names the convention for the next command.

### The risks

Small and mechanical. The danger is a silently renamed field, which the
round-trip tests in `api.spec.ts` and the backend serde tests catch.

## 4. Measure the schema index before touching it

### The finding, corrected

A review claimed that `schemaIndex` in `frontend/src/stores/explorer.ts`
rebuilds on every tree click because `node.loading = true` invalidates it.
Verification shows that Vue tracks dependencies for each property: the
index never reads `loading`, so that write does not invalidate it. The
index rebuilds only when `children` or the snapshots change, and those
changes carry new names, so the rebuild is necessary work.

### What remains worth doing

Only a measurement, and work after it only if the measurement says so:

- Wrap the computed body with `performance.mark` and load a snapshot of
  about twenty thousand columns. If the rebuild stays under roughly ten
  milliseconds, stop here.
- If it does not: split the computed in two, one over the snapshots and one
  over the tree, and merge the two in a third. A change in the tree then
  leaves the snapshot part cached, which holds most of the names.

### The risks

None until the measurement runs. The split doubles the bookkeeping for
duplicate names, so it should not land without the numbers.
