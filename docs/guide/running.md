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

## Running and failed tabs

While a statement runs, its tab shows a spinner, so you can see which tabs are
busy from any tab. When a tab's last run failed, the tab shows a red dot until
the next run starts.

When a tab's session is inside a transaction that it hasn't committed or rolled
back, the tab shows a lock icon with the tooltip "Open transaction on this tab's
session". Closing that tab asks you first, because closing it rolls the
transaction back. The application checks the transaction after a run that can
start or end one, such as a `BEGIN`, a `COMMIT`, a write or a procedure call. A
run of plain `SELECT` statements skips the check, unless the tab turned on
`IMPLICIT_TRANSACTIONS` or turned off `autocommit`, where a read can start a
transaction too.

## Stopping a statement

While a statement runs, a **Stop** button appears beside **Run**, and
`Ctrl`/`Cmd` + `Shift` + `C` does the same. After you press it, the button reads
**Stopping…** until the run ends. The connection's time limit
also stops a statement that runs too long.

When a result passes the row limit, the application asks the server to end the
statement early, unless ending it would lose work. On MS SQL Server, ending a batch of several statements would skip the statements after the
one that reached the limit. Ending a write that returns rows, such as
`INSERT ... OUTPUT`, would roll the write back, and inside a transaction with
`XACT_ABORT` on, the whole transaction would roll back. On MySQL and MariaDB, a
statement that can write, such as a procedure call, runs to its end for the same
reason.

In these cases the application reads the rest of the result and drops the rows
past the limit. The toolbar shows "Still reading rows past the limit. The server
can't end this batch early." beside **Stop**, and the run lasts as long as the
server takes to send every row. **Stop** still works, but on MS SQL Server it
ends the whole batch, so the statements after the current one don't run.

Changing the tab's connection or closing the tab also stops the statement, and
the application asks you before either one. On some engines a stop opens a new
session, which discards the old session's temporary tables and `SET` options.

When a stop or the time limit closes the tab's session, the tab tells you with
a warning in the corner and in **Messages**: "This tab's session was reset.
Temporary tables, open transactions and SET options are gone." An open
transaction on the old session rolls back when the session closes. You see the
same warning when a session that stopped answering is opened again before a
run.

## Messages

The **Messages** tab lists what the server sent, with each message's severity,
code, line and procedure. Messages appear while the script runs, so you can
follow a long script's `PRINT` or `RAISE NOTICE` output without waiting for the
whole script to finish.

The tab shows the last 500 messages. A line at the top counts the earlier
messages that it leaves out, so a loop that prints for every row stays quick.

To keep messages, use the save button above the list:

- **Save shown messages** writes the messages the tab has in memory to a text
  file. A tab keeps between 2,000 and 4,000 of a run's latest messages, so a
  long run's file starts with a line that counts the messages it doesn't have.
- **Save all messages to file** sends every message of the tab's runs to a
  text file that you choose. Each run adds a line with its start time and then
  its messages, and a run that fails adds its error at the end. The tab keeps
  using the file until you choose **Stop saving messages** or close the tab.

Choose the file before you run the script if you need every message. If you
choose it while a statement runs, the file starts with up to 4,000 of that
run's latest messages, and a line counts the earlier ones that are missing.
The application writes to the file while the run goes on, so you can read it
in another program. If a write fails, for example because the disk is full,
the run's messages end with a warning that names the file.

When a statement fails, the results panel switches to **Messages**, where the
error appears with the server's detail and any advice. The error also appears
in the corner.

## Error locations

When the server names the place of an error, the editor underlines it, and the
error in **Messages** has a **Go to line** button that moves the cursor there.
The line counts from the top of the editor, also when you ran a selection or
the statement under the cursor. The mark goes away as soon as you edit the text
or run again.

## Plans

**Plan** reads one statement's plan. An estimated plan doesn't run the
statement. An actual plan does, so the application asks first, because a
statement that writes rows will write them and a statement on Athena will scan
data. In a script, the plan covers only the statement under the cursor.
