# The explorer

The explorer shows the objects of each open connection: the databases, the
schemas, the tables, the views and the columns. A key column carries an icon
of its own, and each column shows its type.

Folders hold the tables, the views, the routines, the events, the indexes, the
constraints, the triggers and the partitions of a schema or a relation.

## Folders of a schema

A schema shows these folders, in this order:

1. **Tables**. On PostgreSQL, a partitioned table has an icon of its own, and
   its **Partitions** folder lists its partitions.
2. **Views**.
3. **Materialized Views**, on PostgreSQL only. A materialized view has a
   **Columns** folder and an **Indexes** folder.
4. **Foreign Tables**, on PostgreSQL only. A foreign table has a **Columns**
   folder and a **Keys** folder.
5. **Synonyms**, on MS SQL Server only. A synonym does not open. The tree
   shows the name of the object that it points at beside the synonym.
6. **Procedures** and **Functions**, on the engines that have routines.
   SQLite has no stored routines, so it shows neither folder.
7. **Events**, on MySQL and MariaDB only. Each event shows its schedule, such
   as `EVERY 1 DAY` or `AT 2026-01-01 00:00:00`.

## Triggers

A table has a **Triggers** folder on MS SQL Server, PostgreSQL, MySQL,
MariaDB and SQLite. A view has one too on MS SQL Server, PostgreSQL and
SQLite, because a trigger on a view can run in place of the change. On
PostgreSQL, a foreign table also has a **Triggers** folder.

Each trigger shows when it runs and the changes that fire it, such as
`AFTER INSERT, UPDATE` or `INSTEAD OF DELETE`. A PostgreSQL trigger can also
fire on `TRUNCATE`. A trigger or an event that the engine keeps but does not
run shows in a paler text, and its hint ends with `disabled`. A trigger of
MySQL, MariaDB and SQLite is always enabled. On MySQL and MariaDB, the
triggers show in the order that they fire: `BEFORE` before `AFTER`, then
`INSERT`, `UPDATE` and `DELETE`. Triggers with the same timing and event show
in the order that `FOLLOWS` and `PRECEDES` set.

### Trigger order in a MySQL CREATE text

The `CREATE` text of a MySQL or MariaDB trigger does not keep the place of
the trigger in the firing order. The server gives the text without a
`FOLLOWS` or `PRECEDES` clause. This limit applies only to a table with two
or more triggers that have the same timing and event.

If you drop one of these triggers and run its text, the trigger fires after
the other triggers of its group. When the triggers change or read the same
values, a change to the data can then give a different result. The server
gives no error or warning.

To keep the order of one trigger, edit its text before you run it. After
`FOR EACH ROW`, add `FOLLOWS` and the name of the trigger above it in the
list with the same timing and event. For the first trigger of the group, add
`PRECEDES` and the name of the trigger below it. To make all the triggers of
a table again, run their texts in the order of the list. Each trigger then
goes after the triggers before it, and the order stays the same.

A PostgreSQL replica trigger also shows in a paler text, and its hint ends
with `replica`. PostgreSQL runs a replica trigger only in a session whose
`session_replication_role` is `replica`, so a normal session does not run
it. A trigger with `ENABLE ALWAYS TRIGGER` runs in all sessions, and it shows
as enabled.

The list leaves out the triggers that the engine makes for its own use. On
PostgreSQL, these are the triggers of a foreign key. A PostgreSQL trigger
that `CREATE CONSTRAINT TRIGGER` makes shows in the **Keys** folder alone, as
a constraint of the type `trigger`. When the trigger has the words
`DEFERRABLE` and `INITIALLY DEFERRED`, its hint gives them. On MS SQL Server, a
trigger of the database, which fires on a change to the schema, does not
show below a table. On SQLite, the folder of a table also shows a temporary
trigger on that table.

An index shows its key columns in key order. On MS SQL Server, the `INCLUDE`
columns of an index follow the key, as in `a, b include (c)`.

The filter box keeps the path down to each match, so a name deep in the tree
stays reachable. A match keeps all of its children, so an open match shows
what it holds. A name that is wider than the panel scrolls across.

## The connection of the tree

The tree, the completion and the drafts of the menu read the catalog on a
second connection of the record. A read of the catalog thus does not wait
behind a statement of a tab. The temporary tables of a tab and the databases
that a tab attaches stay in the session of that tab. The tree does not show
them, and a refresh does not show them.

A SQLite database in memory exists in one session alone. A second connection
to it opens a separate empty database. Thus the tree reads that database on
the session that all the tabs share. A read of the tree then waits while a
tab runs a statement. Use the refresh button to read the tables that a tab
made.

## What the menu of an object gives

The context menu of an object builds statements in the backend, so every name
carries the quotes of its engine:

- A preview of the rows.
- A `SELECT`, an `INSERT` and an `UPDATE` draft.
- A `CREATE` draft of a table. The draft holds no index, no default and no
  constraint.

A trigger and an event get their `CREATE` text alone, which the engine
reads from its catalog. On MySQL and MariaDB, a body of more than one
statement comes between `DELIMITER $$` and `DELIMITER ;`. If the body
contains `$$`, the terminator is a different one that the body does not
contain, such as `$$$` or `//`. The editor then runs the text as one
statement. The MySQL and MariaDB text of a trigger does not keep its place in
the firing order. Refer to **Trigger order in a MySQL CREATE text** in the
Triggers section.
On PostgreSQL, the `CREATE` text of a view, a materialized view and a trigger names each object of a user schema with
its schema, so the text runs under any search path. The `CREATE` statement of a
materialized view without data ends with `WITH NO DATA`, so the text makes an
empty view. After this statement, the text has a `CREATE INDEX` statement for
each index of the view, in the order of the index names. The text of a PostgreSQL
trigger that is disabled, a replica trigger or an always trigger has a second
line. This line is an `ALTER TABLE` statement with `DISABLE TRIGGER`,
`ENABLE REPLICA TRIGGER` or `ENABLE ALWAYS TRIGGER`. A trigger that you make
again from the text thus gets the same state. A materialized view and a synonym get their `CREATE`
text and a `SELECT` alone, and a synonym has no **Properties** item. A foreign table gets a
`SELECT`, an `INSERT` and an `UPDATE`, because a `CREATE` draft of its columns
makes a plain table.

The **Properties** dialog holds the facts of a relation, its columns, its
indexes and its constraints, and it reads them in one call.

## Completion

The editor completes the names that the tree has opened on the connection
of the tab. The names of the other open connections stay out. A full stop after an
alias offers the columns of the table that the alias names. The editor
reads an alias in a `FROM`, a `JOIN`, an `UPDATE` and an `INSERT INTO`
clause.

When you open a database in the tree, the application reads every relation
of that database in the background. The completion then knows a name that
the tree has not opened. The **Columns the editor learns** setting sets how
much of a large schema the read keeps.

On MS SQL Server, the read also includes the synonyms. A synonym gets the
columns of its target when the target is a table or a view in the same
database. The completion offers the name alone for any other synonym.
