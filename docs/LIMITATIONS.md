# Limitations

This file records what the application cannot do, and why. Each entry names
the cause and the state of any fix.

## One dependency is held as a copy

`backend/vendor/tiberius` holds a copy of `tiberius` 0.12.3, which is the
newest release. Cargo is pointed at the copy through `[patch.crates-io]` in
`backend/Cargo.toml`. The copy carries three changes that the release does not.
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
the connection stays open for the next statement. The change is in
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

## MS SQL Server does not show the text of PRINT

`tiberius` decodes the info tokens that carry `PRINT` and a `RAISERROR` of low
severity, and then drops them. The crate gives no public way to read them, so
the Messages tab cannot show that text.

The driver reports what it can reach. An error of the server carries its
number, its severity, its state, its line and its procedure, and those reach
the user beside the text of the error.

PostgreSQL has no such limit: a `NOTICE`, a `WARNING` and an `INFO` arrive on
the connection and reach the Messages tab with the severity, the code, the
detail and the hint that the server sent.

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
A statement that waits for the same connection while the application replaces
it still runs on the old session and fails. Its tab can run again at once,
because the next statement takes the new connection.

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

## The Excel export builds the whole sheet in memory

The Excel export in `frontend/src/lib/xlsx.ts` builds the sheet as one
string and compresses it on the main thread of the interface, so its
memory cost grows with the number of rows in the grid. The row limit of
the view bounds that number. The CSV and JSON exports run the statement
again in the backend and write one row at a time, so a large export goes
through those forms. A fix moves the Excel export to the backend as a
sink; no case has needed it yet.

## A PostgreSQL script without parameters is gathered by the library

A script without parameters goes to PostgreSQL through the simple
protocol, and `tokio-postgres` gathers the whole answer of a simple query
before it returns it. The library gives no streaming form of the simple
query, so the memory cost of such a script grows with the size of its
answer until the row limit applies. A statement with parameters streams
one row at a time.
