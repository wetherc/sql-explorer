# Limitations

This file records what the application cannot do, and why. Each entry names
the cause and the state of any fix.

## One dependency is held as a copy

`backend/vendor/tiberius` holds a copy of `tiberius` 0.12.3, which is the
newest release. Cargo is pointed at the copy through `[patch.crates-io]` in
`backend/Cargo.toml`. The copy carries six changes that the release does not.
Read this list before an upgrade, because an upgrade drops the copy and brings
each defect back.

### The TLS handshake is not sent

The crate holds each write of the TLS handshake in a buffer and sends the
buffer only when the caller flushes it. A TLS library that the crate drives
itself flushes at the end of each round. The library of the operating system
does not: it writes its messages and then waits for an answer. The first
message therefore never left the machine, the server sent nothing, and the
connection waited until its time limit ran out.

The copy sends the buffer at the moment the wrapper begins to read. One whole
round of the handshake then travels in one packet, which is what the server
expects.

The change is in `src/client/tls.rs`, in `poll_read` of
`TlsPreloginWrapper`. The source of `tiberius` does not hold this change.

### The client can send an attention packet

The release gives no way to stop a statement that runs. The copy adds
`Client::attention_handle`, which returns a handle that another task can
hold while the driver reads the results. A call to `signal` on the handle
makes the connection send an `Attention` packet. The server then ends the
statement, the stream of the statement ends with `Error::Canceled`, and
the connection stays open for the next statement. The next request reads
past the tokens of the stopped request up to the acknowledgement, which is
the last token of its message. The change is in
`src/client.rs`, in `src/client/attention.rs` and in
`src/client/connection.rs`.

### The Kerberos library needs a newer `libgssapi`

`libgssapi` 0.4.6 builds a slice from the address of a GSSAPI buffer that
holds nothing. The Kerberos library of macOS returns such a buffer. That is
undefined behaviour, and the current release of Rust answers it with a panic
that cannot unwind, which stops the whole application.

Release 0.8.1 of `libgssapi` carries a guard at each such buffer, and the
release of `tiberius` asks for 0.4. The copy asks for 0.8.1 and calls the two
methods of `ClientCtx` that the newer release changed. The change is in
`Cargo.toml` and in `src/client/connection.rs`. The source of `tiberius` holds
the same change, so this part goes away with the next release.

### The client can read a `sql_variant` column

The release holds `todo!()` where it decodes the metadata and the values of
a `sql_variant` column, so one such column caused a panic in the task that
runs the query. The copy reads the type of the column and the value of each
cell.

A `sql_variant` value carries its own base type. The reader takes the base
type out of the value and gives the data of that type, so a whole number, a
decimal, a date, a text and a binary value each keep their form in the grid.
A value of no length is a null value. A base type that the reader cannot
take gives a protocol error, which the application shows as a query error.
The change is in `src/tds/codec/column_data/variant.rs`, in
`src/tds/codec/type_info.rs` and in
`src/tds/codec/token/token_col_metadata.rs`.

### The client can read its authentication method

The release gives no way to read the user and the password out of a
configuration that a connection string built. The copy adds
`Config::get_authentication`, `AuthMethod::user` and `AuthMethod::password`.
The driver uses them to find a password in a connection string, and to add
the password of the keychain to a string that gives none. The change is in
`src/client/config.rs` and in `src/client/auth.rs`.

### The client reads a money value as a decimal

The release reads a `money` and a `smallmoney` value into a double. A money
value holds up to 19 digits and a double holds 15 or 16, so
922337203685477.5807 became 922337203685477.6. The copy reads the value as a
`Numeric` with a scale of four, and the grid shows each digit. The change is
in `src/tds/codec/column_data/money.rs`.

## A saved connection string can still hold a password

The application refuses to save a connection string that holds a password,
because the settings file keeps the string as plain text. A record that the
settings file already holds is not changed when the application starts. Such
a record still opens, and its password stays in the file until the user
edits the record. The next save then refuses the string until the user moves
the password to the Password box.

## PRINT text and statement counts of MS SQL Server

Release 0.12.3 of `tiberius` decodes the info tokens that carry `PRINT` and a
`RAISERROR` of low severity, and then drops them. It also drops the `DONE` and
`DONEINPROC` tokens that end each statement. The vendored copy gives the text
of an info token to the caller as a `QueryItem::Message`. It gives each `DONE`
and `DONEINPROC` token as a `QueryItem::Done`, with the count of rows when the
server sets the count flag. The first read of a request stops at the first of
these items, so a message or a count before the first result set also reaches
the caller. The copy also gives each error token as a `QueryItem::Error`
where it stands in the answer, and the stream still ends with the first
error of the batch. The changes are in `src/tds/stream/query.rs` and
`src/tds/codec/token/token_done.rs`.

The driver sends every batch through the path that keeps rows, so the rows of
an `INSERT ... OUTPUT` and of a `BEGIN ... END` block reach the grid. The
`DONE` token of a statement ends its result set. A count outside a result set,
such as the count of an `UPDATE`, goes to the Messages tab as "N rows
affected.". A statement under `SET NOCOUNT ON`, and a statement such as
`CREATE TABLE`, sends no count and adds no message.

An error of the server carries its number, its severity, its state, its line
and its procedure, and those reach the user beside the text of the error. A
batch can go on after an error, for example after a division by zero with
`XACT_ABORT` off. The first error of the batch is the error of the run. Each
later error goes to the Messages tab with its number, severity, state and
line, and the counts and the result sets of the statements between them
stay.

PostgreSQL is the same: a `NOTICE`, a `WARNING` and an `INFO` arrive on the
connection and reach the Messages tab with the severity, the code, the detail
and the hint that the server sent.

## A server that offers only the older ciphers

The TLS of MS SQL Server runs on the library of the operating system, and not
on `rustls`. `rustls` holds the AEAD ciphers alone and states that it will not
add the older block form. A server that offers `ECDHE_RSA_WITH_AES_256_CBC`
and nothing newer cannot speak to `rustls`, and such servers are common.
PostgreSQL keeps `rustls`, because no such server has appeared for it.

## Windows Authentication needs a ticket and the full host name

On Windows the account of the user reaches the server through SSPI. On macOS
and on Linux it reaches the server through Kerberos, so the user needs a
ticket, which `kinit` gives.

The name of the service is built as `MSSQLSvc/host:port` from the host as the
user typed it. A connection that names the server by an address or by a short
name therefore asks for a ticket that the domain does not hold. Use the full
host name.

`backend/examples/mssql_probe.rs` opens one connection with this method and
prints the trace of each step, for a connection that does not work.

## A pasted access token is not made fresh again

A connection with the method "Microsoft Entra ID with an access token" keeps
the token that the user pasted. Such a token is valid for about one hour, and
the application cannot get another one for it.

The driver reads the date in the token before it opens a socket, and it
refuses a token that is more than 60 seconds past that date. The connection
form then asks for a new token. A connection that must stay open for longer
than one hour uses the method "Microsoft Entra ID with the Azure CLI", which
reads a fresh token on each connection.

## Athena and the catalog of Glue

The metadata API of Athena returns the parameters of a table as a map, and a
catalog of Glue may hold a value in that map that is absent. The parser of the
AWS SDK refuses such a map with "dense map cannot contain null values".

The driver reads the catalog with statements against `information_schema` for
the rest of the session once it meets that answer. Such a statement scans no
data in storage, so it adds no cost, but it is slower than the API call.

## The catalog statements of four engines have no test against a server

The lists of the routines, the indexes and the constraints are read with a
statement against the catalog of the engine. Only SQLite runs against a real
database in the unit tests. For MS SQL Server, MySQL, PostgreSQL and Athena
the tests check the text of the statement alone, so a statement that the
engine refuses shows itself the first time a user opens the folder.

## The partitions of Athena come from a metadata relation

Athena keeps the partitions in the catalog of Glue. Engine version 2 gave them
with `SHOW PARTITIONS`. Engine version 3 follows Trino, which holds no such
statement and answers it with "mismatched input 'PARTITIONS'". The driver
therefore reads the relation whose name is the name of the table with
`$partitions` at the end. That relation holds one column for each partition
key, and the driver joins the columns into the form that `SHOW PARTITIONS`
gave, which is `key=value` with a slash between the keys.

A relation with no partition key, and a view, hold no such relation, so the
service reports a relation that is absent. The driver answers with an empty
list when the refusal names that cause. Another refusal reaches the user as an
error.

A table of Iceberg holds a different set of columns there, which includes the
counts of the records and of the files and a group of the values of the keys.
The tree shows those columns as they come, so the text of a partition of an
Iceberg table is longer than the text of a partition of a table of Hive.

## A plan covers one statement

The keyword that asks for a plan stands in front of one statement, so the
application refuses a request that holds two statements. Select the statement
first, or put the cursor in it and read the plan of that statement.

The actual plan runs the statement. A statement that writes rows writes them,
and a statement on Athena scans data and costs money, so the interface asks
before it reads an actual plan.

MS SQL Server holds the plan switch for the whole session, so the driver turns
the switch off again after each plan. A switch that cannot be turned off leaves
the session in the plan state, and the driver then reports the fault and closes
the connection. `EXPLAIN ANALYZE` of MySQL needs version 8.0.18, and of MariaDB
version 10.1. SQLite reports one plan, which it builds without running the
statement, so a request for the actual plan gives that plan with a message.

## A stop opens a new session on some engines

Stop asks the server to end the statement on a channel of its own, and the
application then waits up to five seconds for the driver to report the failure
that the server sends back through the connection. A driver that reports in
that time leaves the connection in a known state, and the connection stays
open. PostgreSQL and MySQL work this way, and SQLite and Athena end a statement
without touching the connection at all.

MS SQL Server works this way as well. The copy of `tiberius` that this
application holds sends an attention packet, the server ends the statement,
and the connection stays open with its session. A server that does not
answer the packet in five seconds leaves the connection in no known
state, and the application then opens a new connection in its place. The
new session is empty: a temporary table, an open transaction and any `SET`
of the old session are gone with it.

The time limit works the same way on every engine, because a statement that
passes the limit is dropped in the middle of the exchange whatever the engine.
A replacement touches one session alone: the tab that lost its session takes
a new one at once, and the sessions of the other tabs run on.

## Catalog reads have a fixed time limit and no Stop

A read of the catalog for the explorer, the completions, the properties
dialog or a script of an object has a limit of 60 seconds. The wait for the
driver counts against that limit. The time limit of the connection does not
apply to these reads, and no Stop button ends one. A catalog read that waits
behind a lock of the server, for example an MS SQL Server Sch-M lock of a
change that another tab did not commit, thus fails after 60 seconds.

A read that passes the limit is dropped in the middle of the exchange. The
application asks the server to stop the statement and closes the session of
the read, except on SQLite and Athena. The next catalog read opens a new
session. A read that ran on the default session, because no second
connection could open, closes the default session, and the next command
opens a new one. A read that waited for the driver during the whole limit
fails and closes no session, because the exchange that keeps the driver has
a limit of its own.

## MS SQL Server and Athena have no read-only session

MS SQL Server has no setting that makes one session refuse writes. The
read-only switch sets `ApplicationIntent=ReadOnly` on the login. An
availability group sends such a login to a readable secondary, and that
secondary refuses writes. A primary or a standalone server ignores the
intent and accepts writes. The form names this in the hint of the switch.
The only server-side stop is a login without write permissions.

Athena has no read-only mode for a query, so the form hides the switch.
The permissions of the IAM identity and of the workgroup decide what a
statement can change.

On PostgreSQL and MySQL the switch sets the default of the session. A
statement such as `SET default_transaction_read_only = off` or
`SET SESSION TRANSACTION READ WRITE` turns it off again, so the switch
stops an accidental write only.

## The sessions of the tabs

Each tab holds one server session, up to the limit in the options of the
connection. A tab past the limit gets a message at once and runs after
another tab closes, or after the user raises the limit. A local temporary
table belongs to one session, so a `#table` of one tab is not visible in
another tab; a global `##table` of MS SQL Server is. SQLite holds one writer
at a time, so the statements of two tabs that both write contend for the
file. A SQLite database that lives in memory allows one session, because a
second connection to it opens a separate empty database; every tab of such a
connection shares one session. Athena holds no session state at all, so its
sessions are plain request channels.

The application closes the session of a tab after ten minutes with no answer
from the server. A sweep does this once each minute, and a new tab at the
limit starts a sweep at once. Before the close, the application asks the
server whether the session is inside a transaction. A session inside a
transaction stays open until its tab closes, because the close would roll
back the work of the transaction. A session that does not answer this
question in five seconds closes. A closed session loses its temporary tables
and its `SET` options, and the tab gets no message about it. The next
statement of the tab opens a new, empty session.

A session that stops answering, for example after a network fault, gets a new
session in its place. The server rolls back the transaction of the old session,
and the tab again gets no message.

## A large number parameter goes to the server as digits

The interface holds a number of the parameter dialog as a double, which keeps
17 digits and no more. A value of the number form goes out as a number only if
its digits come back from the double unchanged. If they do not, as with
`90071992547409931`, the digits go out as text and the server converts the text
to the type that the statement asks for. Each engine makes that conversion for
a number column. A text with an exponent, such as `1e3`, stays a number,
because the server does not read that form in each type.

## A JSON value with a large number stays text

The grid reads an array, an object and a `jsonb` value as JSON. A JavaScript
number keeps about 17 digits, so a number such as `9007199254740993` would
change on the read. A value that holds such a number stays the text that the
server sent. The grid shows its digits, and a copy and an export write the
text. A JSON export writes that value as a string, not as an array or an
object.

## Athena takes no bound parameters

The client gives no way to bind a value, so the values of the named parameters
of a statement of Athena go into the text of the statement as literals. A text
value keeps its quotes doubled. Every other engine binds the values, so a value
never becomes part of the statement there.

## A statement with a parameter behaves differently on MS SQL Server

`tiberius` sends a parameterised batch inside `sp_executesql`. Inside that
wrapper a `USE` and a `SET` hold for that batch alone, so they do not reach the
statements that follow. A script that carries a parameter is also sent whole
and not one statement at a time, because the numbers of the placeholders belong
to the whole text. The same script without a parameter is split and each part
holds its own effect.

## A statement with a parameter reads the values of PostgreSQL in binary

A statement without a parameter goes to PostgreSQL through the simple
protocol, which sends every value as text. A statement with a parameter goes
through the extended protocol, which sends every value in the binary form of
its type. The driver holds a reader for the types that the engine ships,
among them the arrays, the ranges, the multiranges, the composites, and the
domains of any element type. Bytes that no reader understands show as text
if the bytes are text, and as base64 if they are not. A geometric type, a
string of bits, and `tsvector` reach the grid this way.

The binary form of a `timestamptz` value holds the moment in UTC. When the
statement returns such a value, the driver reads `SHOW TimeZone` before the
rows and writes each value in that zone, in the form of the `ISO` date
style, such as `2024-01-01 09:00:00-05`. A session with another date style
shows other text on the simple protocol, so the two paths then differ. A
zone name that the zone database does not know gives UTC.

The values that go out take the other direction. The driver writes each
bound value in the text format, so the server converts the text to the type
that the statement asks for. A whole number binds against `int2`, `int4`,
`int8` and `numeric` alike. A value that goes to a `bytea` column must
carry the text form of that type, which is `\x` and then the bytes in
hexadecimal.

A `money` value shows two digits of the fraction. The count of the digits
belongs to the `lc_monetary` setting of the server, and two digits hold for
every locale that PostgreSQL ships.

## The Excel export of the grid builds the whole sheet in memory

The export of the rows that the grid shows goes through
`frontend/src/lib/xlsx.ts`, which builds the sheet as one string and
compresses it on the main thread of the interface. Its memory cost
therefore grows with the number of rows in the grid, which the row limit of
the view bounds. The export of every row writes from the backend one row at
a time, so a large export goes through that entry of the menu.

An Excel sheet holds 1048576 rows, the row of the column names among them.
An export of more rows than that stops at the bound and reports the result
as truncated. The CSV and the JSON forms have no such bound.

Excel keeps 15 significant digits in a number. A number, or a text that
holds only a number, goes into the sheet as a number when it has at most 15
significant digits. A longer number, such as a `bigint` of 19 digits or a
`DECIMAL(38,10)` value, goes in as text, so that no digit changes. `SUM`
does not read such a cell.

Excel accepts at most 32767 characters in one cell. Both writers cut a
longer text at that bound, and the file does not mark the cut. The CSV and
the JSON forms keep the whole text.

## The export of every row reads the text of the statement

**Write every row** runs the statement a second time. The backend accepts a
statement that starts with `SELECT`, `WITH` or `SHOW` and has no word that
changes data outside a quoted region or a comment. The check reads the text
alone, so it cannot see a function or a procedure of the server that writes.
`SELECT nextval('s')` on PostgreSQL and a `SELECT` that calls a function
with side effects both pass. The check also refuses some statements that
only read, such as a `SELECT` whose bare column name is a writing word.

## The filter of the grid reads the rows before it answers

The filter matches against a copy of the text of each row. The grid builds
that copy in slices and gives the main thread back between them, so the
interface answers a pointer and a key while the build runs. The header
shows the part of the rows the build covered. The rows on screen keep the
filter of the last finished build until the new one ends, so a filter on a
result of many rows takes some seconds to answer. The copy weighs as much
as the text of the result, and the grid drops it when the filter clears or
another result arrives.

## The files panel does not watch the disk

The panel reads a folder when the user opens it and when the user asks for
a refresh. It holds no watch on the disk, so a file that another program
writes keeps its old text in the panel and in a tab that shows it. A save
from a tab writes the whole file, so the last write wins and a change that
came from outside is lost. A refresh of the folder, or a second open of the
file, brings the text of the disk back.

## A save writes UTF-8

The editor reads a file in UTF-8, in UTF-16 with a byte order mark, or in
Windows-1252 when the bytes are not UTF-8. A file in another code page, such
as Windows-1251, opens with the characters of Windows-1252 in place of its
own. A save always writes UTF-8 with no byte order mark. The first save of a
file in UTF-16 or in Windows-1252 thus changes the encoding of the file, and
a tool that reads the file in its old encoding shows the accented
characters wrong.

## MySQL walks the rest of a result that passes the row limit

The MS SQL Server driver sends an attention packet when a reading
statement reaches the row limit, so the server ends the statement and the walk covers the
rows in flight alone. MySQL offers no such packet. Its stop runs `KILL
QUERY` from a second connection, which ends the statement with a fault of
the server instead of a clean end and leaves the session of the pool unfit
for the next statement. The driver therefore reads the rest of a large
result and drops the rows past the limit. The time of that walk counts
against the time budget of the run, so a result of many millions of rows
can still end with the timeout message.

## PostgreSQL walks past the row limit for a statement that can write

The PostgreSQL driver sends each statement of a script to the server in a
text of its own, and stops a statement at the row limit with a cancel
request on a second socket. A cancel rolls back the statement that it ends. Inside a transaction block
it also aborts the block, and the `COMMIT` that follows then acts as a
`ROLLBACK`. The driver therefore sends the cancel only for a statement that
only reads, and only when the session is outside a transaction block. A
probe statement before the run tells the driver which case applies. Every
other statement, such as `INSERT ... RETURNING` or a `SELECT` after `BEGIN`,
runs to its end, and the walk drops the rows past the limit. The check of
the text cannot see a function of the server that writes, so a `SELECT` of
such a function outside a block can still lose its writes at the limit. The
walk past the limit reads the rest of the result, and its time counts
against the time budget of the run. The probe costs one more round trip for
each statement of a script that only reads.

## MS SQL Server walks past the row limit for most batches

The attention packet ends the whole batch, not one result set of it. It
also rolls back the statement that it ends. When the session is inside a
transaction and `XACT_ABORT` is on, it rolls back the whole transaction.
The driver therefore sends the packet only for a batch of one statement
that only reads. A probe statement before the batch reads `@@TRANCOUNT` and
`@@OPTIONS`, and the driver sends no packet when the transaction would roll
back. Every other batch, such as a call of a procedure, an
`INSERT ... OUTPUT` or a batch of several statements, runs to its end, and
the walk drops the rows past the limit. The time of that walk counts
against the time budget of the run. The probe costs one extra round trip
for each run of a batch of one reading statement.

## A run with parameters takes one batch on MS SQL Server

The placeholders of the parameters are numbered over the whole text, and
each batch is a request of its own, so the parameters of a later batch
cannot be named. A run with parameters over a script that holds a `GO`
separator is refused, and the message says so. A script without parameters
holds as many batches as the user wrote.

## A pasted AWS session token is not made fresh again

An Athena connection that takes the keys of the form holds them as they were
typed. A session token lives for a limited time, and the application makes
no new one, so a token that is too old stops the connection until the user
pastes a new one. The credentials of the AWS tools of the machine read a
fresh token on each connection, so a profile is the durable choice for a
credential that expires.
