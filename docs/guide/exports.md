---
title: Exports and row limits
description: Export rows to CSV, JSON, Markdown, INSERT statements or Excel, copy them, run a statement straight to a file, save full results on this computer, pause a query at the row limit, and set the grid and export row limits.
order: 8
---

# Exports and row limits

You can export a result to a CSV, JSON, Markdown, INSERT or Excel file, or copy
it to the clipboard. The grid's menu offers two types of export:

- **Export rows** writes the rows that the grid has, passing them through
  the interface. If you select rows first, it writes only those rows.
- **Export all rows** runs the statement again in the backend and writes each
  row to the file as it arrives, so the rows never reach the interface. These
  entries appear only when the row limit stopped the read, and they offer CSV,
  JSON and Excel files.

**Export all rows** runs the statement that produced the result, on that run's
connection. A pinned result from an older run therefore writes its own rows,
even after you change the tab's text or connection. Each entry says where its
rows come from. An entry that runs the query again says **Runs the query again**
and how long the last run took, for example **Runs the query again (last run
took 4 min 12 s)**, so you can judge the cost before you click.

On Athena, the statement doesn't run again. Athena saves the full result of
every query in S3, so the export reads the rest of the rows from that saved
result, and you don't pay for a second scan. The export entries then say
**From the saved result (doesn't run again)**, the warning above the grid says
how old the saved result is, for example **Saved on Athena, 3 h ago**, and the
tab stays free for other statements while the export runs. The app forgets a saved
result when you run the tab again (unless you pinned the result), close the
result or the tab, or disconnect. It also forgets a result after 12 hours,
and forgets the oldest one when it has more than 64. If Athena no
longer has the result, the export fails and asks you to run the query again.

When it runs the statement again, **Export all rows** accepts only statements
that read. The backend refuses a statement that contains a word such as
`INSERT`, `UPDATE`, `DELETE`, `INTO` or `EXEC` outside a string or a comment,
although it accepts a `FOR UPDATE` clause on PostgreSQL and MySQL.

While **Export all rows** runs, a bar above the grid reads **Exporting all
rows…** and has a **Stop** button. The export menu stays disabled until the
export ends, and a tab runs one such export at a time. The rows go to a
temporary file beside the one you chose, and the file takes its name only when
the export finishes, so a stopped or failed export leaves no file behind.

## Run to file

**Run to file…** runs the statement under the cursor once and sends its rows to
two places at the same time. Every row, up to the export row limit, goes to a
file, and the grid shows the first rows, up to the grid's row limit, like a
normal run. You get the preview and the whole file from one execution, so the
server doesn't do the work twice. The button sits beside **Run all**, and the
command palette and the **File** menu list it too.

The save dialog opens before the statement runs. The file's extension sets its
format: `.json` writes JSON, `.xlsx` writes an Excel file, and any other name
writes CSV. If you close the dialog, nothing runs and the tab keeps its
results.

A note above the grid says how many rows it shows and where the rest went, for
example "Showing the first 10,000 rows. All 52,310 rows were saved to
/Users/me/orders.csv." A notice in the corner reports the file and its row
count, as **Export all rows** does.

Because the statement runs only once, **Run to file…** accepts any statement,
including ones that change data, and it runs a whole script when you select
one. The grid shows every result set of the script, each up to the grid's row
limit, and **Messages** lists what the server sent. A statement that returns no
result set writes no file, and the run reports an error after the statement has
run.

When the text can return more than one result set, for example two `SELECT`
statements or a procedure call, a second dialog opens after the save dialog and
asks where the results go:

- **First result only** writes the first result set to the file, and the other
  results appear in the grid alone.
- **One file per result set** (CSV and JSON) writes the first result to the
  file you chose and each later result to a file beside it: `orders.csv`,
  `orders-2.csv`, `orders-3.csv` and so on. When a name is already taken, the
  app adds a number, as in `orders-2 (2).csv`, so it never replaces a file you
  didn't choose.
- **One sheet per result set** (Excel) writes each result to a sheet of its own
  in the file you chose, named **Result 1**, **Result 2** and so on.

The dialog selects the choice you made last time, and the export row limit
applies to each result separately. The note above each grid names the file or
sheet of its result. The app can't always tell from the text how many results a
procedure returns, so when a run with **First result only** returns more than
one, a notice says "Only the first result went to the file."

An Excel sheet has room for 1,048,575 rows below its header. When you choose an
`.xlsx` file and the export row limit in Settings is higher than that, a dialog
after the save dialog warns you before the run starts, and **Save as CSV instead…**
opens the save dialog again with a `.csv` name. If you run anyway and a result
fills its sheet, the rest of that result isn't saved, and the note above the
grid and the notice in the corner both say so. For a single query that only
reads, the app also stops reading from the server at that point, once the grid
has its rows. A script keeps running to its end, so every statement in it still
runs.

**Stop** and the connection's time limit work as they do for a normal run. The
rows go to a temporary file beside the one you chose, so a stopped or failed run
leaves no file behind.

If the run fails before the statement starts, for example because the server
can't be reached, a notice says "Run to file didn't start, so nothing was
saved." Its **Try again** button runs the statement again with the file you
already chose, so the save dialog doesn't open a second time. The app keeps
your choice of file for an hour. After that, or once a run has started, run to
file again to choose the file.

## Saved full results

Turn on **Save full results on this computer** in the **Results** group of
Settings, and **Export all rows** stops running the query a second time. The
option is off by default.

With the option on, a run doesn't stop reading at the grid's row limit. It keeps
reading up to the export row limit, shows the first rows in the grid as usual,
and writes every row of the result to a temporary file on your computer. The
warning above the grid then gives the row count and the file's size, for example
**Saved on this computer, 52,310 rows, 412 MB**, and the **Export all rows**
entries say **From the rows saved on this computer**. The export reads the
file, so the server does no more work and the export finishes much sooner.

Runs take longer with the option on, because the statement keeps running until
it reaches the end of its result or the export row limit. The tab shows the
statement as running for that time, and **Stop** works as usual.

The app saves the result of a single statement on any database except Athena.
A script with more than one statement reads only up to the grid's row limit.
Athena doesn't need the option, because it already keeps the full result of
each query in S3. **Run to file…** and query plans never save a result this way.

**Disk space for saved results** sets the space that all saved results can use
together, from 1 to 1,024 gigabytes. The default is 2 GB. When a new result
needs more space, the app deletes the oldest saved result first. A result is
not saved in these cases, and **Export all rows** runs the query again for it:

- The result has more rows than the export row limit.
- The result doesn't fit in the disk space, even after the app deletes every
  older saved result. The read then stops at the grid's row limit.
- The disk refuses the write, for example because it's full.
- You stop the run, the run reaches the connection's time limit, or a statement
  fails.

When the export row limit, the disk space or the disk keeps a result from being
saved, **Messages** says why, and a notice in the corner points you there.

The app deletes a saved file when you run the tab again (unless you pinned the
result), close the result or the tab, or disconnect. It also deletes a file
after 12 hours, and deletes the oldest one when it has more than 64 kept
results. The files live in a folder under the app's cache folder. If the app
quits or crashes, it deletes the files that are left over the next time it
starts. Each running copy of the app has a folder of its own, so a second copy
never deletes the files of the first.

## Paused queries

Turn on **Pause queries at the row limit** in the **Results** group of Settings,
and a query that reaches the grid's row limit pauses there instead of ending.
The grid shows the first rows, and the statement stays open on the server. When
you choose **Export all rows**, the export continues the same read into the
file, so the statement doesn't run a second time and the server doesn't repeat
its work. The option is off by default.

A bar above a paused result says how long the pause has left and has two
buttons. **Export all rows** writes every row to a CSV, JSON or Excel file:
first the rows that the grid shows, then the rest of the read. **Release** ends
the statement and frees the tab. An export takes the paused read once, so after
the export, or after an export that fails, the result is no longer paused and
a second export runs the query again. While the query is paused, the **Export
all rows** entries of the grid's menu say **Continues the paused read**.

A paused query costs the server something for as long as it waits:

- The statement stays open, with its connection and the memory that it uses on
  the server.
- The server can keep locks on the rows that the statement read, so another
  session that wants to change those rows can wait for them.
- On PostgreSQL, the read waits inside a transaction. A server with
  `idle_in_transaction_session_timeout` set below the pause limit ends the
  session, and the export then fails with a message that says so.
- On MySQL and MariaDB, the app raises `net_write_timeout` for the session while
  the statement runs, so the server doesn't close the connection while nothing
  reads from it, and sets the old value again afterwards.

**Pause time limit** sets how long a query can stay paused, from 1 to 60
minutes. The default is 10 minutes. When the time runs out, the app releases
the query on its own. The app also releases a paused query when you run the tab
again (also when you pinned the result), close the tab, or disconnect.

While its query is paused, the tab can't run anything else on its session. A
new run, a query plan, or **Run to file…** in that tab releases the paused
query first. Other tabs aren't affected, because each tab has a session of its
own.

Only some queries pause. The text must be a single statement that only reads,
it must run in a tab, and the database must be PostgreSQL, MySQL, MariaDB or
SQL Server. A script, a statement that changes data, and a run on SQLite or
Athena end at the row limit as before. While the paused query waits, the app
keeps a copy of the grid's rows for the export. A result whose first rows need
more than 128 MB for that copy ends at the row limit instead, and **Messages**
says why.

**Save full results on this computer** already gives the export every row, so a
run doesn't pause while that option is on.

## Excel files

An Excel sheet fits 1,048,576 rows, including the row of column names. An
export with more rows stops there, and a warning tells you, so you can export to
CSV to get every row.

A number becomes an Excel number when it has at most 15 significant digits,
because Excel keeps no more, and a longer number goes in as text. Text that
looks like a number becomes a number only in a column of a numeric type, so a
code such as `007` in a text column keeps its leading zeros.

An Excel cell takes at most 32,767 characters. Longer text is cut at that
length, and a warning after the export says how many cells were cut.

## INSERT files

An INSERT file writes booleans as `TRUE` or `FALSE` on PostgreSQL and Athena,
and as `1` or `0` on the other engines. Arrays become a PostgreSQL array literal
such as `'{1,2}'` on PostgreSQL and an `ARRAY[...]` constructor on Athena. On
the other engines, arrays and objects become their JSON text.

## Clipboard

A copy to the clipboard separates the cells with tabs. A cell that contains a
tab or a line break, or that begins with a quote, goes in quotes the way Excel
writes it, so a spreadsheet keeps the value in one cell.

## Row limits

The settings have two separate limits by design:

| Setting          | What it limits                                               | Default   |
| ---------------- | ------------------------------------------------------------ | --------- |
| Row limit        | The rows that the grid keeps                                 | 10,000    |
| Export row limit | The rows that **Export all rows** and **Run to file…** write | 1,000,000 |

The grid limit keeps the interface fast, because every row in the grid lives in
the interface's memory. Each connection also has a **Row limit** in its form's
**Advanced** section, and a run uses the smaller of the two. The export limit is
much higher, because those rows go straight to the file.

When a result reaches its limit, a warning above the grid reports the stop. The
rows you see are the first rows of the answer, in the order the server sent
them.

On PostgreSQL, a `SELECT`, `WITH`, `VALUES` or `TABLE` statement that runs
outside a transaction reads its rows through a cursor, and the read is committed
when the grid has enough rows. Work that the statement does on the server is
therefore kept. For example, a function called by the `SELECT` that inserts
rows keeps those rows, even when the grid shows only the first part of the
answer. A function that runs once for each row keeps the work only for the rows
that were read.
