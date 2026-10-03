---
title: Run and stop
description: Run a statement or a whole script, read its messages and plan, and stop a statement that takes too long.
order: 5
---

# Run and stop

Pick a connection at the top of the tab, then write a statement.

- `Ctrl`/`Cmd` + `Enter` runs the statement under the cursor.
- `Ctrl`/`Cmd` + `Shift` + `Enter` runs the whole script.
- If you select text, the selection runs instead of the statement under the
  cursor.

A script runs one statement at a time. The splitter understands quotes,
comments, dollar tags and MySQL's `DELIMITER` command. Each statement commits on
its own unless the script opens a transaction, and the first error ends the
script, so the statements after it don't run.

On MS SQL Server the unit is the batch rather than the statement. A line that
contains only `GO` ends a batch, and the server receives each batch whole, so
"the statement under the cursor" means the whole batch around the cursor,
semicolons and all.

**Format** lays the statement out by its dialect's rules.

## Stopping a statement

While a statement runs, a **Stop** button appears beside **Run**. The
connection's time limit also stops a statement that runs too long.

Changing the tab's connection or closing the tab also stops the statement, and
the application asks you before either one. On some engines a stop opens a new
session, which discards the old session's temporary tables and `SET` options.

## Messages

The **Messages** tab lists what the server sent, with each message's severity,
code, line and procedure. A failed statement's error appears there as well as in
the corner.

## Plans

**Plan** reads one statement's plan. An estimated plan doesn't run the
statement. An actual plan does, so the application asks first, because a
statement that writes rows will write them and a statement on Athena will scan
data. In a script, the plan covers only the statement under the cursor.
