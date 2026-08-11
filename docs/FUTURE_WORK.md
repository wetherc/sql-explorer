# Future work

This document holds the designs for two known weaknesses that the current
code does not correct. Each section gives the problem, the design of the
correction, the order of the work, and the risks. The claims about the code
were examined against the source on 2026-08-10, and the errors of the first
version are corrected here.

The work runs in the order of the sections. Section 1 adds new commands,
and those commands follow the request convention that
`frontend/src/lib/api.ts` states above its `call` helper. Section 2 comes
last.

## 1. Stream the rows of a result instead of gathering them in memory

### The problem

Every driver returns a `QueryResponse` that holds every row as
`Vec<Vec<serde_json::Value>>`. The whole response then crosses the Tauri
bridge as one JSON body. Two costs follow:

- A large result takes several times its raw size in memory, because each
  cell is a `serde_json::Value`. The grid then holds about four more copies
  of the rows (`sourceRows`, `rowTexts`, `filteredRows`, `sortedRows` in
  `frontend/src/components/ResultsGrid.vue`).
- The export command runs the statement again and still gathers every row
  in memory before it writes the file (`export_query` in
  `backend/src/commands.rs`).

The first version of this document gave a third cost: the user sees no row
until the last row arrives. The design below does not correct that cost.
`execute_query` still returns one response when the run ends. A correction
needs Tauri events that push blocks of rows to the interface during the run.
That is a different design, and this document does not give it.

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

`RunSummary` carries `rows_affected`, `elapsed_ms` and `stats`. When every
driver has `execute_stream`, the present `execute_query` becomes a default
method: it runs `execute_stream` into a `BufferSink` that builds the present
`QueryResponse`. One code path then serves both forms. Until that point,
each driver keeps its own `execute_query`, and `execute_stream` has a
default body that returns `Unsupported`.

Two sinks:

- `BufferSink` keeps the rows up to `max_rows` and reports `truncated`.
  It reproduces the behaviour of today, with one known change: the
  PostgreSQL driver splices the notices that arrive before the run at the
  front of the messages (`postgres.rs`, `execute_query`). The sink receives
  messages in arrival order and has no way to put one at the front. The
  Messages tab then shows the notices in arrival order.
- `FileSink` writes each row to a CSV or JSON file as it arrives. The
  export command uses it, and a large export then holds one row at a time
  in memory. It writes the first result set and answers `Stop` at the end
  of that set, because the export writes one file.

### The drivers

The first version of this document misstated three of the five drivers.
The corrected state of each driver is:

- **SQLite**: the read runs inside `spawn_blocking` through
  `with_connection`, and the whole response builds inside the blocking
  closure. Restructure `with_connection`: the closure sends events
  (`BeginSet`, a block of about one thousand rows, `EndSet`, `Message`)
  through a bounded channel with a capacity of a few blocks, and the async
  side drives the sink. A `Stop` travels back through a shared flag that
  the closure examines on each row.
- **PostgreSQL**: the driver does not use `query_raw` today. The path with
  parameters uses `client.query`, which gathers every row; convert it to
  `query_raw`, which gives a `RowStream`. The path without parameters uses
  `simple_query`, which returns a vector that the library already
  gathered. The library gives no public streaming form of the simple
  query, so that path feeds the sink from the vector and keeps its memory
  cost. Record this limit in the module comment.
- **MySQL**: the driver uses `exec_iter` and then `result.collect()`,
  which gathers each set before the row limit applies. Replace the collect
  with a read of one row at a time through `next().await`, and forward
  each row to the sink.
- **MS SQL Server**: the driver already walks a `QueryStream` in
  `collect_sets`; forward each item to the sink instead of a vector. The
  function already drains the rows past `max_rows` to keep the connection
  fit for use. A `Stop` from the sink must drain in the same way, and the
  existing timeout bounds that drain.
- **Athena**: the page loop in `read_results` forwards each page to the
  sink instead of a vector. A `Stop` returns without a fetch of the pages
  that remain.

### Paging for the grid

The first version of this document gave a second step: a backend cache of
the buffered rows, a command `fetch_rows(request_id, offset, count)`, and
windowed reads for the grid. That step is not part of this design, for two
reasons:

- The grid filters, sorts, selects and exports over the full set of rows
  on the client. Windowed reads remove those functions unless the backend
  takes the sort and the filter as parameters, which doubles the scope of
  the step.
- The row limit already bounds what the grid holds. The cache moves the
  memory from the interface to the backend and adds an owner problem; it
  does not remove the memory.

Do this step only when a measured case shows that the bound of `max_rows`
is not enough, and then design it as its own effort with backend sort and
filter.

The Excel export builds the whole sheet as one string in
`frontend/src/lib/xlsx.ts` and compresses it on the main thread. Windowed
reads do not reduce that cost. When the Excel export becomes a problem,
move it to the backend as an `XlsxSink`. Until then, record the memory
bound in `LIMITATIONS.md`.

### The order of the work

1. Add `RowSink`, `SinkControl`, `RunSummary` and `BufferSink` in a new
   module `backend/src/db/sink.rs`, with unit tests for a run with more
   than one set, a truncation through `Stop`, the message order, and an
   empty run.
2. Convert the SQLite driver. The end-to-end tests run against SQLite in
   memory through the existing `open_memory` helper.
3. Convert the PostgreSQL driver.
4. Convert the MS SQL Server driver, the MySQL driver and the Athena
   driver. Then make `execute_query` the default trait method and delete
   the five copies in the drivers.
5. Switch the export command to `FileSink` and delete the buffered export
   path. Test that a stop in the middle of an export leaves no file, and
   that the formula-mark escape of `csv_field` stays in place.

### The risks

- The sink crosses `await` points, so it needs `Send`, and the SQLite
  bridge adds a channel whose backpressure the bounded capacity holds.
- `run_bounded` in `backend/src/commands.rs` drops the driver future in
  the middle of a message when the user stops the run or the time limit
  ends it. The drop can land between `begin_set` and `end_set`.
  `FileSink` must write to a temporary path, rename the file at a
  successful end, and delete the temporary file when it drops without one.
- A sink that stops the read early must leave the connection in a clean
  state; on a wire protocol the driver must drain the rest of the stream.

## 2. Report an Entra access token that has expired

### The problem

The `EntraAccessToken` method stores the pasted token in the keychain as if
it were a password. Such a token lives for about one hour. The silent
reconnection path (`ensure_healthy`) reuses the stored token, so a
reconnection after the hour fails with a message that does not name the
cause. `describe_login` in `backend/src/db/drivers/mssql.rs` returns the
raw error for every method except `Integrated`, so the user sees the text
of the server.

### The design

- Read the `exp` claim of the token when a connection opens. The claim sits
  in the middle part of the JWT, base64url encoded without padding, so the
  decode uses `URL_SAFE_NO_PAD`. No signature check is needed to read a
  date the client itself acts on.
- Refuse a token whose `exp` lies more than 60 seconds in the past, before
  a socket opens, with the message: "The access token has expired. Paste a
  new one, or use the Azure CLI method, which reads a fresh token on each
  connection." The 60 seconds absorb a clock that runs early. The
  reconnection path calls `connect`, so this one check covers both paths,
  and no stored timestamp is needed.
- Add an `EntraAccessToken` arm to `describe_login` that maps a login
  refusal to the same message.
- In the interface, an authentication failure on a connection with this
  method opens the connection form with the token field cleared and
  focused. Two facts about the current code shape this step:
  - No path in `frontend/src/stores/connections.ts` examines the kind of
    an error today; every failure goes to `ui.reportError`. The store gains
    a branch for `ErrorKind.Authentication` on this method.
  - For a saved connection, an empty secret field means "keep the stored
    secret" (`ConnectionForm.vue`). For an expired token that meaning is
    wrong. The form entered through this path must require a new token and
    must not fall back to the stored one.
- Add a section to `LIMITATIONS.md`, next to the Windows Authentication
  section: the pasted token is not refreshed, and the Azure CLI method is
  the durable choice.

### The order of the work

1. A pure function `token_expiry(token: &str) -> Option<SystemTime>` with
   tests for a well formed token, a token without `exp`, an `exp` that is
   not a number, text with fewer than three parts, and text that is not a
   JWT.
2. The check in the `EntraAccessToken` arm of `auth_method`, and the new
   arm of `describe_login`, with tests.
3. The form behaviour in the interface and the documentation section.

### The risks

Small. The parse must not reject a token it cannot read; an unreadable
token goes to the server unchanged, and the server stays the judge.
