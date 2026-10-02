# The explorer

The explorer shows the objects of each open connection: the databases, the
schemas, the tables, the views and the columns. A key column carries an icon
of its own, and each column shows its type.

Folders hold the tables, the views, the routines, the indexes, the constraints
and the partitions of a schema or a relation.

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

A materialized view and a synonym get their `CREATE` text and a `SELECT`
alone, and a synonym has no **Properties** item. A foreign table gets a
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
