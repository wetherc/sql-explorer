# Limitations

This file records what the application cannot do, and why. Each entry names
the cause and the state of any fix.

## One dependency is held as a copy

`backend/vendor/tiberius` holds a copy of `tiberius` 0.12.3, which is the
newest release. Cargo is pointed at the copy through `[patch.crates-io]` in
`backend/Cargo.toml`. The copy carries two changes that the release does not.
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

The driver reports what it can reach: the severity, the line number, the
procedure name and the number of every server error.

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

## The partitions of Athena come from a statement

Athena keeps the partitions in the catalog of Glue, and the driver reads them
with `SHOW PARTITIONS`. The service refuses that statement for a relation
that holds no partition and for a view, and the driver answers with an empty
list when the refusal names that cause. Another refusal reaches the user as
an error.

## Athena takes no bound parameters

The driver refuses a statement that carries a parameter, because the client
gives no way to bind one. Put the values into the statement.
