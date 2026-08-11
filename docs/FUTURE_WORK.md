# Future work

This document holds the designs for known weaknesses and planned features
that the current code does not have. Each section gives the problem, the
design of the correction, the order of the work, and the risks. The claims
about the code were examined against the source on 2026-08-11.

Two notes in `docs/LIMITATIONS.md` now have designs here: the Excel export
that builds its sheet in memory (section 12) and the PostgreSQL script
whose answer is gathered by the library (section 13). A backend cache with
windowed reads for the grid stays set aside: the grid filters, sorts,
selects and exports over the full set of rows on the client, the row limit
already bounds what the grid holds, and such a cache needs backend sort
and filter to keep those functions. Design it as its own effort when a
measured case shows that the bound of `max_rows` is not enough.

## 1. Report an Entra access token that has expired

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

## 2. Collapse the results panel to a bar

### The problem

The results panel below the editor cannot close. The split in
`frontend/src/components/QueryView.vue:122-138` uses the `splitpanes`
package, and both panes carry `:min-size="MIN_EDITOR_SIZE"` (15 percent,
`frontend/src/stores/layout.ts:21`). The editor therefore never holds more
than 85 percent of the height.

### The design

- Add `resultsCollapsed: boolean` to the `Layout` interface in
  `frontend/src/stores/layout.ts`. The default is `false`. `parseLayout`
  gains a branch that reads the field and falls back to the default. The
  field persists with the rest of the layout under the `localStorage` key
  `sql-explorer.layout`. The state is one value for the whole application,
  the same as `editorSize`.
- In `QueryView.vue`, when the state is collapsed, the second pane leaves
  the split and a thin bar takes its place below the editor. The bar shows
  the name of the active results tab, the row count of the last run, and
  one button that expands the panel. The pane content stays mounted with
  `v-show`, so the grid keeps its scroll position and its selection.
- The results tab row (`QueryView.vue:144-199`) gains a collapse button in
  its action group, with the icon `mdi-chevron-down`.
- `editorSize` does not change on collapse. An expansion restores the
  split that the user had.
- Add a command `view.results` with the key `mod+j` to the command array
  in `frontend/src/layouts/AppLayout.vue`, so the palette and the keys
  dialog list it.
- A run that starts while the panel is collapsed expands the panel, so the
  user sees the rows arrive.

### The order of the work

1. The store field, the `parseLayout` branch, and the action, with tests
   in `layout.spec.ts`.
2. The markup, the bar, and the buttons in `QueryView.vue`, with tests.
3. The palette command, with tests in `AppLayout.spec.ts`.

### The risks

Small. The Monaco editor must take the new height; the resize path of the
split already exercises that. The bar must stay one implementation for
both orientations of section 3.

## 3. Turn the results panel to the side

### The problem

The results panel sits below the editor and nowhere else. The `horizontal`
attribute on the `splitpanes` element (`QueryView.vue:122`) fixes the
stacked form.

### The design

- Add `resultsOrientation: 'below' | 'beside'` to the `Layout` interface,
  with the default `'below'`, a `parseLayout` branch, and persistence, the
  same as section 2.
- Bind the attribute: `:horizontal="layout.layout.resultsOrientation ===
  'below'"`. The `splitpanes` package supports both forms on the same
  component.
- The results tab row gains a toggle button next to the collapse button,
  with the icons `mdi-dock-bottom` and `mdi-dock-right`.
- One `editorSize` percent serves both orientations.
- The splitter style in `QueryView.vue:885-888` sets `min-height` alone;
  it gains `min-width: 4px` for the vertical splitter. The drag rules in
  `frontend/src/style.css:64-67` cover only
  `.splitpanes--horizontal.splitpanes--dragging`; they gain the vertical
  class with the cursor `col-resize`.
- When the panel is collapsed in the `beside` orientation, the bar of
  section 2 still sits below the editor. One bar serves both orientations.

### The order of the work

1. The store field with tests.
2. The binding, the button, and the style rules, with tests.

### The risks

Small. The grid computes its column widths from its container; a switch of
orientation changes that width, and the grid must lay its columns out
again. Test with a wide result set.

## 4. One control that opens a new tab

### The problem

Two controls do the same work. The application header holds a "New query"
button (`frontend/src/layouts/AppLayout.vue:9-15`, `data-test`
`app-new-query`). The tab row holds a "+" button
(`frontend/src/components/QueryTabs.vue:36-47`, `data-test` `new-tab`).
Both call `tabs.add()` with no arguments.

### The design

Remove the header button. Keep the "+" button in the tab row, the button
in the empty state (`QueryTabs.vue:69-77`), and the command `tab.new` with
the key `mod+t`. The tab row is where the new tab appears, so the control
that creates it belongs there.

### The order of the work

One change: remove the button from `AppLayout.vue` and update the tests in
`AppLayout.spec.ts` that find `app-new-query`.

### The risks

None known.

## 5. Rename a tab

### The problem

A tab takes the name "Query N" (`nextTitle`,
`frontend/src/stores/tabs.ts:67-70`) and the user cannot change it. The
store already holds `rename(id, title)` (`tabs.ts:153-158`), but its only
caller is the save path of the saved-query library. Titles already persist
in the workspace snapshot.

### The design

- A double click on a tab label replaces the label with a text field. The
  Enter key and a blur commit the text through `tabs.rename`. The Escape
  key cancels. An empty text cancels, which `rename` already enforces.
- The text field stops the propagation of its pointer events, so an edit
  does not fight the activation of the tab.
- Add a command `tab.rename` that starts the edit on the active tab, so
  the palette lists the function.
- A tab that holds a file (section 11) takes the base name of the file as
  its title when the file opens. A rename of such a tab changes the title
  alone; it does not rename the file.

### The order of the work

1. The inline edit in `QueryTabs.vue`, with tests for commit, cancel, and
   empty text.
2. The palette command, with tests.

### The risks

Small. The edit lives inside a Vuetify `v-tab`; the component swallows
some pointer events, so the tests must cover the double click and the
commit inside it.

## 6. Horizontal scroll in the explorer tree

### The problem

A long column name is cut with an ellipsis. `.node-label` in
`frontend/src/components/ExplorerTree.vue:386-391` sets `overflow:
hidden`, `text-overflow: ellipsis` and `min-width: 0`. The data type sits
in `.node-hint` with `margin-left: auto` (`ExplorerTree.vue:394-400`), so
the hints align on the right edge of the panel. The scroll container
`.explorer-body` (`frontend/src/components/DbExplorer.vue:335-340`)
already allows both axes with `overflow: auto`, but every row shrinks to
the panel width, so nothing overflows.

### The design

- Give `.explorer-tree` the rules `width: max-content` and `min-width:
  100%`. Each row then takes the width of the widest row. A tree narrower
  than the panel still fills the panel.
- Remove the ellipsis rules from `.node-label` and keep `white-space:
  nowrap`. Remove the `:title` attribute and its comment
  (`ExplorerTree.vue:45-47`), because the whole name is now readable.
- Keep `margin-left: auto` on `.node-hint`. The hints then align on one
  right edge: the panel edge for a narrow tree, the scrolled edge for a
  wide tree. The user scrolls to see them, which is the wanted behaviour.
- The keyboard focus path (`rowElements.get(key)?.focus()`,
  `ExplorerTree.vue:199`) lets the browser scroll the focused row into
  view. Call `focus({ preventScroll: true })` and then
  `scrollIntoView({ block: 'nearest', inline: 'nearest' })`, so a vertical
  key walk does not throw the horizontal position away.

### The order of the work

One change set in `ExplorerTree.vue`: the style rules, the template, and
the focus call, with tests that assert the classes and the focus options.

### The risks

Small. The completion source reads `node.hint`
(`frontend/src/stores/explorer.ts:400`); this design does not touch the
hint text, so that consumer is safe.

## 7. Make the parameter feature visible and safe

### The problem

The feature is hard to find and hard to trust:

- A parameter is written as `:name`, and nothing in the interface teaches
  that. The only entry point is the "Parameters" toolbar button
  (`frontend/src/components/QueryView.vue:63-71`).
- The values dialog appears at run time, as a surprise, when a value is
  missing (`withParams`, `QueryView.vue:504-538`).
- The editor gives no mark on a `:name` token, so the user cannot see what
  the application found.
- A value of the Number kind that does not parse becomes `null` in
  silence (`jsonOfParam`, `frontend/src/lib/params.ts:15-27`).
- The Boolean kind takes free text, and every text except `true` means
  `false`.

### The design

- Colour the `:name` token in the editor. The Monaco language definition
  in `frontend/src/plugins/monaco.ts` gains one tokenizer rule for a colon
  followed by name characters, outside strings and comments, with its own
  token colour. The rule follows the scanner in `backend/src/sql.rs:392`
  and treats `::` as a cast.
- Show a parameter bar between the toolbar and the editor when the
  statement holds parameters. The bar holds one chip per name in the form
  `:name = value`, and an unset value shows as `unset`. A click on a chip
  opens the values dialog. The bar reads the names through the existing
  `query_parameters` command, called from a watcher on the editor text
  with a debounce of 300 milliseconds.
- Improve the dialog:
  - One line of help at the top: "Write `:name` in the statement to make a
    parameter."
  - The Number kind validates its text. A text that does not parse shows
    an inline error and blocks the confirm button. Remove the silent
    `null` from `jsonOfParam`.
  - The Boolean kind becomes a select with the two values `true` and
    `false`.
- Write a guide topic for parameters (section 8), with the Athena literal
  substitution and the MS SQL whole-script behaviour from
  `LIMITATIONS.md`.

### The order of the work

1. The validation changes in `params.ts` and the dialog, with tests.
2. The tokenizer rule, with tests.
3. The parameter bar, with tests for the debounce and the chip click.

### The risks

Moderate. The tokenizer rule is a second implementation of the scanner in
`sql.rs`, so the two can drift. The rule colours text and nothing else;
the backend scanner stays the single judge of what runs. A drift then
shows a wrong colour, not a wrong query.

## 8. A guide inside the application

### The problem

The application holds no documentation. The only help is the keys dialog
(`AppLayout.vue:115-138`). The README and the documents under `docs/` are
not bundled, and the content security policy allows no remote origin, so
the application cannot point at a web page.

### The design

- Add a rail icon `mdi-help-circle-outline` with the label "Guide" at the
  bottom of the rail (`AppLayout.vue:47-63`). The icon does not join the
  `Panel` union: it opens a dialog, because the side panel is capped at
  640 pixels and prose needs more width. The dialog is an `AppDialog`, the
  same as the settings dialog, with a topic list on the left and the topic
  content on the right. The open state lives in `stores/ui.ts` as
  `guideOpen`, next to `keyboardHelpOpen`.
- Write the topics as Markdown files under `frontend/src/guide/`, imported
  with the Vite `?raw` suffix. Render them with the `marked` package into
  `v-html`. The content is bundled at build time and no user text enters
  it, so no sanitiser is needed and the content security policy is not
  touched.
- The first topics: connections, the explorer, tabs and files, run and
  stop, parameters, the results grid, exports and the two row limits, and
  the keyboard. Write them from the README and from `LIMITATIONS.md`.
- Add a command `app.guide` to the command array, so the palette and the
  keys dialog list it.

### The order of the work

1. The dialog, the store flag, the rail icon, and the command, with one
   placeholder topic and tests.
2. The `marked` dependency and the topic renderer, with tests.
3. The topic content.

### The risks

Small. The rendered Markdown needs its own scoped styles under both
themes. The content can go stale; a release check reads the topics against
the code.

## 9. The row limit and the reported error

### The problem

A user reports that a query whose result passes the configured limit of
10000 rows ends in an error. The code does not hold an error path for the
limit itself. `BufferSink::row` (`backend/src/db/sink.rs:92-101`) marks
the set `truncated` and answers `Stop`. Every driver caps at
`options.max_rows` and reports the warning "N rows returned. The row limit
stopped the read." (`backend/src/db/drivers.rs:501-510`), and the
interface shows a warning notice and an alert.

The paths near the limit that can produce an error:

- MS SQL Server (`mssql.rs:281-295`) and MySQL (`mysql.rs:203-219`) walk
  the stream to its end past the limit, because a stream that is dropped
  in the middle breaks the connection. A very large result then spends the
  whole time budget in the walk, and `run_bounded`
  (`backend/src/commands.rs:260-274`) answers `Error::Timeout`: "The
  statement passed the limit of N seconds, so the connection was closed."
- The PostgreSQL path without parameters gathers the whole answer before
  the limit applies (section 13), so a very large result can exhaust
  memory or the time budget.

### The design

1. Reproduce the report on each engine with a statement that returns well
   past 10000 rows, and record the exact message. The design below covers
   the known candidate; a different finding gets its own design.
2. Stop the read at the limit instead of walking the rest:
   - MS SQL Server: when the count reaches the limit, signal the attention
     handle that the vendored `tiberius` already provides. The server ends
     the statement, the stream ends with `Error::Canceled`, and the driver
     maps that end to a clean truncated set. The walk then covers only the
     rows in flight, not the rest of the result.
   - MySQL: examine the cancel handle of `mysql_async` for the same shape
     (`KILL QUERY` runs on a second connection). If the engine offers no
     safe early stop, keep the walk and record the cost in
     `LIMITATIONS.md`.
   - PostgreSQL with parameters already breaks and drops the stream.
     PostgreSQL without parameters is section 13.
3. A regression test per engine: a result past the limit ends as a
   truncated set with the warning, not as an error, within a short time
   budget.

### The order of the work

The three numbered steps above, in that order.

### The risks

Moderate. An early stop must leave the connection fit for the next
statement; the attention path of MS SQL Server is proven by the stop
feature, but the MySQL path needs its own proof. The reproduction comes
first for this reason.

## 10. The export limit against the view limit

### The problem

A question, not a defect: an export writes up to 1000000 rows while the
view holds 10000. Is that intended?

### The answer

Yes. The two limits are separate settings by design. The view limit
`maxRows` (`frontend/src/stores/settings.ts:64`, default 10000) bounds
what the grid holds. The export limit `exportRowLimit` (`settings.ts:67`,
default 1000000) bounds "Export every row", which runs the statement again
in the backend and streams each row to the file through `FileSink`
(`backend/src/commands.rs:1212-1215` replaces the options with the export
limit). The README and the settings hints state this. The guide topic on
exports (section 8) repeats it where the user works.

### The order of the work

No code change. The guide topic covers it.

### The risks

None.

## 11. A file browser in the side panel

### The problem

The application cannot open a file from disk. No command lists a folder or
reads a file; the backend only writes through its save dialog
(`backend/src/commands.rs:1082-1164`). The capabilities grant no `fs:`
permission. A tab holds no file path (`QueryTab`,
`frontend/src/stores/tabs.ts:10-21`).

### The design

The backend gains four commands, in the pattern the code already follows:
the backend opens the dialog itself, so a command never touches a path
that the user did not accept.

- `pick_folder`: opens the folder dialog through `tauri-plugin-dialog`,
  records the accepted path in a set on `AppState`, and returns it.
- `list_folder(path)`: refuses a path outside every accepted root, after
  canonicalisation, so a symbolic link cannot step outside. Returns the
  entries as name and kind, folders first, without hidden files.
- `read_text_file(path)`: the same guard, plus a size cap of 5 MB with the
  message "The file is larger than the editor accepts."
- `write_text_file(path, contents)`: the same guard. Writes through a
  temporary file and a rename, the same as `FileSink`.

The accepted roots persist in `workspace.json` next to the tabs, and the
restore path puts them back into the accepted set. The user accepted them
once; the workspace file records that acceptance.

The frontend:

- A new `files` member of the `Panel` union, a rail item
  `mdi-folder-outline`, and a `FilesPanel` component in the side panel.
  The panel holds a "Open a folder" button and one tree per root. The
  tree loads one folder level per expansion. The tree is a new component;
  `ExplorerTree.vue` is bound to the explorer store, and a shared tree is
  a later extraction if the two stay parallel.
- `QueryTab` gains `filePath?: string`, carried through `snapshot`,
  `parseWorkspace` and the restore. A click on a file opens a tab with the
  file text and the base name as the title; a second click focuses the tab
  that already holds the path.
- The key `mod+s` on a tab with a `filePath` writes the file through
  `write_text_file` and marks the tab clean. On a tab without a path it
  keeps its current meaning, the saved-query dialog. The library save
  stays a separate action.
- The panel does not watch the disk. An external change wins or loses by
  the last write. Record that in `LIMITATIONS.md`.

### The order of the work

1. The four backend commands with tests: the guard against a path outside
   the roots, the symbolic link case, the size cap, and the write through
   the temporary file.
2. The tab field and the workspace round trip, with tests.
3. The panel, the tree, and the open path, with tests.
4. The save path and the key, with tests, and the `LIMITATIONS.md`
   section.

### The risks

Moderate. The path guard is the security boundary of the feature; its
tests come first. A folder with very many entries needs the lazy load
from the start. The restore of the accepted roots widens what a stolen
workspace file can point at; the guard still refuses everything outside
those roots, and the roots are folders the user chose.

## 12. Move the Excel export to the backend

### The problem

The Excel export builds the whole sheet as one string on the interface
thread and zips it there (`frontend/src/lib/xlsx.ts`, `zipSync` from
`fflate`), so its memory cost grows with the row count, and the bytes then
cross the bridge as base64. The CSV and JSON exports run the statement
again in the backend and write one row at a time through `FileSink`
(`backend/src/commands.rs:1256-1404`). "Export every row" therefore
offers CSV and JSON alone (`ResultsGrid.vue:51-58`).

### The design

- Add the `zip` crate to the backend. `ZipWriter` writes into the
  `BufWriter<File>` that `FileSink` already holds; the file gives the
  `Seek` that the crate wants.
- Extend `ExportFormat` (`commands.rs:1043-1048`) with `Xlsx`, and give
  `FileSink` an xlsx mode:
  - `begin_set` starts the archive, writes the four static parts, and
    opens the entry `xl/worksheets/sheet1.xml` with the header row. The
    static parts and the cell rules port from `xlsx.ts`: inline strings,
    the five XML escapes, the strip of the forbidden control characters,
    and the base-26 column names. No shared strings and no styles, the
    same one-pass shape as the frontend writer.
  - `row` writes one `<row>` element into the open entry.
  - `finish` closes the sheet element, closes the archive, and renames
    the `.part` file, as today.
- A sheet holds at most 1048576 rows. The xlsx mode caps the data rows at
  1048575 plus the header, marks the set truncated at that point, and the
  existing truncation message reaches the user.
- The frontend: add Excel to the "Export every row" menu
  (`ResultsGrid.vue:51-58` and the emit type), to the format union
  (`frontend/src/types/api.ts:167`), and to `EXPORT_FILES`
  (`QueryView.vue:653-659`). The grid export of the selected rows keeps
  the frontend writer, because it exports what the grid holds.
- Rewrite the Excel section of `LIMITATIONS.md`: the grid export stays in
  memory and is bounded by the view limit; the full export streams.

### The order of the work

1. The xlsx mode of `FileSink` with tests: an empty result, a result with
   every value kind, the escapes, the forbidden characters, and the sheet
   row cap. A test opens the file with an unzip step and checks the sheet
   XML.
2. The format plumbing on both sides, with tests.
3. The documentation change.

### The risks

Small. The port copies a writer that already works; the tests compare its
XML against the frontend writer for one shared fixture. The `zip` crate is
a new dependency; pin it and read its advisory history first.

## 13. Stream the PostgreSQL script without parameters

### The problem

A script without parameters goes through `simple_query`
(`backend/src/db/drivers/postgres.rs:696`), which gathers the whole answer
as a vector before the loop begins. The limit then bounds what the sink
keeps, not what the driver holds. `LIMITATIONS.md` states that the library
gives no streaming form; that statement is out of date. The resolved
`tokio-postgres` 0.7.18 exposes `simple_query_raw`, which returns a
`SimpleQueryStream` of the same `SimpleQueryMessage` values, and
`simple_query` itself is that stream plus a collect.

### The design

- In `stream_simple`, replace the `simple_query` call with
  `simple_query_raw`, pin the stream, and walk it with `try_next`. The
  match arms over `RowDescription`, `Row` and `CommandComplete` stay as
  they are.
- Keep the walk past the limit. The simple protocol carries every
  statement of the script in one exchange, and the walk keeps the command
  tags, so `rows_affected` of the later statements stays right. The memory
  cost is the point of the change, and the walk holds one message at a
  time.
- Update the comments at `postgres.rs:342-346` and `postgres.rs:687-689`,
  and remove the section of `LIMITATIONS.md` that states the gathering.

### The order of the work

One change set: the call, the comments, the documentation, and the tests
of the driver that cover a multi-statement script and a truncated set.

### The risks

Small. `Cargo.toml` asks for `tokio-postgres = "0.7"` and the lock file
holds 0.7.18; the build breaks loudly if a downgrade removes the method.
The walk past the limit keeps the time cost of a very large answer; that
cost stands today as well, and section 9 records it.

## 14. AWS keys typed into the connection form

### The problem

An Athena connection can only take its credentials from the machine. The
connection builds the AWS configuration with
`aws_config::defaults(...)` and an optional profile name
(`backend/src/db/drivers/athena.rs:167-173`), so the credentials come from
the default chain: the environment variables, the files `~/.aws/config`
and `~/.aws/credentials`, and the instance metadata. The form offers the
region and the profile alone
(`frontend/src/components/ConnectionForm.vue:132-144`). A user who does
not have the AWS CLI installed and configured therefore cannot connect,
even when that user holds an access key ID, a secret access key, and a
session token.

### The design

- Add `awsCredentialSource: 'chain' | 'keys'` to the connection options,
  with the default `'chain'`, and the fields `awsAccessKeyId` and
  `awsSessionTokenSet` next to it. The access key ID is an identifier and
  goes into the connection record. The secret access key and the session
  token are secrets and go into the keychain.
- The secret store keys one secret per connection
  (`state.secrets.set(&connection.id, password)`,
  `backend/src/commands.rs:1076`). Two more secrets need two more keys.
  Use the forms `{id}:aws-secret-access-key` and
  `{id}:aws-session-token`. The delete path (`commands.rs:1092`) removes
  all three keys when a connection is removed.
- In `AthenaDriver::connect`, when the source is `'keys'`, give the loader
  a static provider:
  `loader.credentials_provider(Credentials::new(access_key_id, secret_access_key, session_token, None, "sql-explorer"))`.
  The crate `aws-credential-types` holds `Credentials`; it is already a
  transitive dependency of `aws-config` and becomes a direct one. The
  source `'chain'` keeps the current behaviour, so an existing connection
  does not change.
- Refuse an incomplete pair before a request opens, with the message: "An
  Athena connection with keys needs an access key ID and a secret access
  key." The session token stays optional, because a permanent key pair
  does not have one.
- In the form, a select chooses the source. The source `'keys'` shows
  three fields: the access key ID, the secret access key, and the session
  token. The two secret fields follow the rule the form already holds for
  a saved connection: an empty field means "keep the stored secret". The
  profile field shows only for the source `'chain'`.
- A session token lives for a limited time, the same as the Entra token of
  section 1. The AWS error for an expired token is `ExpiredToken`. Map
  that code to the message: "The session token has expired. Paste a new
  one, or use a profile, which reads a fresh token on each connection."
  The check that `connect` already makes (`list_databases_inner`,
  `athena.rs:191`) reports it at the moment the user connects.
- Add a section to `LIMITATIONS.md`: the typed session token is not
  refreshed, and a profile is the durable choice. Update the README where
  it gives the Athena fields.

### The order of the work

1. The option fields on both sides, the three keychain keys, and the round
   trip through save, load, and delete, with tests.
2. The provider in `AthenaDriver::connect` and the refusal of an
   incomplete pair, with tests.
3. The form fields and the source select, with tests for the "keep the
   stored secret" behaviour.
4. The `ExpiredToken` message, the `LIMITATIONS.md` section, and the
   README.

### The risks

Moderate. The secret paths carry the risk: a secret must not reach the
connection record, the workspace file, or a log line. The tests must
assert that a saved connection holds no secret text. The change to the
delete path must remove every key, or a removed connection leaves its
secrets in the keychain.
