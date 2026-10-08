---
title: Parameters
description: Use named parameters such as :country in a statement, and give their values when it runs.
order: 6
---

# Parameters

Write `:name` in a statement to create a parameter:

```sql
SELECT * FROM orders WHERE customer_id = :id AND city = :city
```

The editor gives each parameter name its own colour. A name starts with a letter
or an underscore, so the `:30` in `10:30` isn't a parameter, and PostgreSQL's
`::` cast isn't one either. A colon inside a PostgreSQL array's brackets, as in
`a[lo:hi]`, marks a slice rather than a parameter, although the editor still
colours `:hi` as a name.

The bar above the editor lists each of the statement's parameters with its
current value, and a missing value reads `unset`. Click a name, or click
**Parameters**, to open the values dialog.

## Value types

Each value keeps the type that you choose, so a value stays text even when it
looks like a number, and an identifier such as `007` keeps its leading zeros.

| Type          | What it sends                                 |
| ------------- | --------------------------------------------- |
| Text          | The text exactly as you typed it              |
| Number        | A number. Text that isn't a number is refused |
| Boolean       | `true` or `false`                             |
| NULL          | `NULL`                                        |

If a value is missing when you run the statement, the dialog opens first. The
values stay with the tab, so the next run doesn't ask again, and they come back
after a restart.

## Engine differences

Every engine except Athena binds the values, so a value never becomes part of
the statement's text. Athena has no way to bind a value, so its values go into
the text as literals, with the quotes inside a text value doubled.

On MS SQL Server, a statement with a parameter runs inside `sp_executesql`, so a
`USE` or a `SET` inside that batch applies to that batch alone. A script with a
parameter is also sent whole rather than one statement at a time, because the
placeholder numbers belong to the whole text.

A batch with a parameter can still begin or end a transaction. The server
reports error 266 when `sp_executesql` ends with a different transaction count,
but the run shows a warning in its place, and the tab still shows the lock icon
of the open transaction. A batch without parameters goes to the server as a
plain batch, the way SQL Server Management Studio sends it. Its temporary
tables, `SET` options, `USE` and `BEGIN TRANSACTION` stay with the tab's session
after the run.

MS SQL Server receives a text value as `nvarchar`. When you compare it with a
`varchar` column, the server converts the column rather than the value, so it often
can't seek an index on that column and scans the whole table instead. Cast
the parameter to the column's type to keep the index in use:

```sql
SELECT * FROM orders WHERE code = CAST(:code AS varchar(20))
```

On PostgreSQL, a statement with a parameter has to stand alone, because the
server refuses a script of two statements when either one contains a parameter.
MySQL and SQLite run such a script one statement at a time, and each statement
gets the values of its own parameters.
