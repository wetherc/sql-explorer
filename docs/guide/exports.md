---
title: Exports and row limits
description: Export rows to CSV, JSON, Markdown, INSERT statements or Excel, or copy them, and set the grid and export row limits.
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
even after you change the tab's text or connection.

Because it runs the statement again, **Export all rows** accepts only statements
that read. The backend refuses a statement that contains a word such as
`INSERT`, `UPDATE`, `DELETE`, `INTO` or `EXEC` outside a string or a comment,
although it accepts a `FOR UPDATE` clause on PostgreSQL and MySQL.

While **Export all rows** runs, a bar above the grid reads **Exporting all
rows…** and has a **Stop** button. The export menu stays disabled until the
export ends, and a tab runs one such export at a time. The rows go to a
temporary file beside the one you chose, and the file takes its name only when
the export finishes, so a stopped or failed export leaves no file behind.

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

| Setting          | What it limits                           | Default   |
| ---------------- | ---------------------------------------- | --------- |
| Row limit        | The rows that the grid keeps             | 10,000    |
| Export row limit | The rows that **Export all rows** writes | 1,000,000 |

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
