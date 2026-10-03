---
title: The object explorer
description: Browse a server's databases, schemas, tables, views, triggers and other objects, and script them from the context menu.
order: 3
---

# The object explorer

The explorer shows each open connection's databases, schemas, tables, views and
columns. Each column shows its type, and key columns have their own icon.
Folders group a schema's or a relation's tables, views, routines, events,
indexes, constraints, triggers and partitions.

## Schema folders

A schema shows these folders, in this order:

1. **Tables**. On PostgreSQL, a partitioned table has its own icon, and its
   **Partitions** folder lists its partitions.
2. **Views**.
3. **Materialized Views** (PostgreSQL only). Each one has a **Columns** folder
   and an **Indexes** folder.
4. **Foreign Tables** (PostgreSQL only). Each one has a **Columns** folder and a
   **Keys** folder.
5. **Synonyms** (MS SQL Server only). A synonym doesn't expand, and the tree
   shows the name of the object it points at beside it.
6. **Procedures** and **Functions**, on engines that have routines. SQLite has
   no stored routines, so it shows neither folder.
7. **Events** (MySQL and MariaDB only). Each event shows its schedule, such as
   `EVERY 1 DAY` or `AT 2026-01-01 00:00:00`.

## Triggers

Tables have a **Triggers** folder on MS SQL Server, PostgreSQL, MySQL, MariaDB
and SQLite. Views have one too on MS SQL Server, PostgreSQL and SQLite, because
a trigger on a view can run instead of the change, and PostgreSQL also gives
foreign tables a **Triggers** folder.

Each trigger shows when it runs and which changes fire it, such as
`AFTER INSERT, UPDATE` or `INSTEAD OF DELETE`. A PostgreSQL trigger can also
fire on `TRUNCATE`.

A trigger or event that the engine keeps but doesn't run appears in paler text,
with a hint that ends in `disabled`. MySQL, MariaDB and SQLite triggers are
always enabled. A PostgreSQL replica trigger also appears in paler text, with a
hint that ends in `replica`. PostgreSQL runs a replica trigger only in a session
whose `session_replication_role` is `replica`, so an ordinary session never runs
it. A trigger set with `ENABLE ALWAYS TRIGGER` runs in every session, and it
shows as enabled.

On MySQL and MariaDB, the triggers appear in firing order: `BEFORE` ahead of
`AFTER`, then `INSERT`, `UPDATE` and `DELETE`. Triggers with the same timing and
event follow the order that `FOLLOWS` and `PRECEDES` set.

The list leaves out triggers that the engine creates for its own use, which on
PostgreSQL are the triggers behind foreign keys. A PostgreSQL trigger made with
`CREATE CONSTRAINT TRIGGER` appears only in the **Keys** folder, as a constraint
of type `trigger`, and its hint shows `DEFERRABLE` and `INITIALLY DEFERRED` when
the trigger has them. On MS SQL Server, database triggers (which fire on schema
changes) don't appear under any table. On SQLite, a table's folder also lists
the temporary triggers on that table.

### Trigger order in a MySQL CREATE text

A MySQL or MariaDB trigger's `CREATE` text doesn't record the trigger's place in
the firing order, because the server returns the text without a `FOLLOWS` or
`PRECEDES` clause. This only affects a table with two or more triggers that
share the same timing and event.

If you drop one of those triggers and run its text, the trigger fires after the
others in its group. When the triggers change or read the same values, the same
change to the data can then give a different result, and the server gives no
error or warning.

To keep one trigger's place, edit its text before you run it. After
`FOR EACH ROW`, add `FOLLOWS` and the name of the trigger above it in the list
with the same timing and event. For the first trigger in the group, add
`PRECEDES` and the name of the trigger below it instead. To recreate all of a
table's triggers, run their texts in list order, so each trigger goes after the
ones before it and the order stays the same.

## Indexes and the filter box

An index shows its key columns in key order. On MS SQL Server, an index's
`INCLUDE` columns follow the key, as in `a, b include (c)`.

The filter box keeps the path down to each match, so you can still reach a name
deep in the tree. A match keeps all of its children, so expanding a match shows
everything inside it. A name wider than the panel scrolls sideways.

## The tree's connection

The tree, completion and the menu's drafts read the catalog on a second
connection of the same record, so catalog reads don't wait behind a tab's
statement. A tab's temporary tables and the databases it attaches stay in that
tab's session, so the tree doesn't show them, even after a refresh.

A SQLite in-memory database exists in one session only, and a second connection
to it opens a separate, empty database. The tree therefore reads an in-memory
database on the session that all the tabs share, which means a tree read waits
while a tab runs a statement. Click the refresh button to see tables that a tab
created.

## The context menu

An object's context menu builds its statements in the backend, so every name
gets its engine's quotes. It offers:

- A preview of the rows.
- `SELECT`, `INSERT` and `UPDATE` drafts.
- A `CREATE` draft for a table. The draft has no indexes, defaults or
  constraints.

The **Properties** dialog shows a relation's details, columns, indexes and
constraints, and it reads them all in one call.

### Triggers and events

Triggers and events get only their `CREATE` text, which the engine reads from
its catalog. On MySQL and MariaDB, a body of more than one statement sits
between `DELIMITER $$` and `DELIMITER ;`, so the editor runs the text as one
statement. If the body contains `$$`, the terminator is a different one that the
body doesn't contain, such as `$$$` or `//`. The text doesn't keep the trigger's
place in the firing order (see **Trigger order in a MySQL CREATE text** above).

On MS SQL Server, a CLR trigger has no `CREATE` text, because `sys.sql_modules`
keeps no definition for it, so the menu reports that the engine gives no text.

On SQLite, a temporary trigger's text names its table the way the original
statement did. When the `ON` clause names the table without a schema, running
the text puts the trigger on the temporary table of that name if one exists, and
on the `main` table otherwise. The trigger can therefore end up on a table in a
different schema from the original.

### Views, materialized views and triggers on PostgreSQL

The `CREATE` text of a PostgreSQL view, materialized view or trigger names every
object in a user schema together with its schema, so the text runs under any
search path. The `CREATE` statement of a materialized view without data ends
with `WITH NO DATA`, so the text creates an empty view. After it comes one
`CREATE INDEX` statement for each of the view's indexes, ordered by index name.

A PostgreSQL trigger that is disabled, a replica trigger or an always trigger
gets a second line in its text: an `ALTER TABLE` statement with
`DISABLE TRIGGER`, `ENABLE REPLICA TRIGGER` or `ENABLE ALWAYS TRIGGER`. A
trigger that you recreate from the text therefore gets the same state.

### Materialized views, synonyms and foreign tables

Materialized views and synonyms get only their `CREATE` text and a `SELECT`, and
a synonym has no **Properties** item. A synonym's `SELECT` is `SELECT *`,
because the MS SQL Server catalog lists no columns for a synonym. Foreign tables
get a `SELECT`, an `INSERT` and an `UPDATE`, because a `CREATE` draft built from
their columns would create a plain table.

## Completion

The editor completes the names that the tree has loaded on the tab's connection,
and leaves out names from other open connections. Typing a full stop after an
alias offers the columns of the table that the alias names. The editor
recognises aliases in `FROM`, `JOIN`, `UPDATE` and `INSERT INTO` clauses.

When you expand a database in the tree, the application reads all of that
database's relations in the background, so completion also knows names that the
tree hasn't loaded. The **Columns the editor learns** setting controls how much
of a large schema this read keeps.

On MS SQL Server, the read includes synonyms. A synonym gets its target's
columns when the target is a table or a view in the same database, and
completion offers only the name for any other synonym.
