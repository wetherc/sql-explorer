# The explorer

The explorer shows the objects of each open connection: the databases, the
schemas, the tables, the views and the columns. A key column carries an icon
of its own, and each column shows its type.

Folders hold the tables, the views, the routines, the indexes, the constraints
and the partitions of a schema or a relation.

The filter box keeps the path down to each match, so a name deep in the tree
stays reachable. A name that is wider than the panel scrolls across.

## What the menu of an object gives

The context menu of an object builds statements in the backend, so every name
carries the quotes of its engine:

- A preview of the rows.
- A `SELECT`, an `INSERT`, an `UPDATE` and a `DELETE` draft.
- A `CREATE` draft of a table. The draft holds no index, no default and no
  constraint.

The **Properties** dialog holds the facts of a relation, its columns, its
indexes and its constraints, and it reads them in one call.

## Completion

The editor completes the names that the tree has opened. A full stop after an
alias offers the columns of the table that the alias names.

One command reads every relation of a database at once. The completion then
knows a name that the tree has never opened.
