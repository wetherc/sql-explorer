---
title: Exports and the two row limits
description: Export rows to CSV, JSON, Markdown, INSERT statements or Excel, and set the two row limits.
order: 8
---

# Exports and the two row limits

A result goes to a CSV, JSON, Markdown, INSERT or Excel file, or to the
clipboard.

The menu of the grid holds two types of export:

- **Export the rows** writes what the grid holds. The rows pass through the
  interface. Mark rows first, and the entries then write those rows alone.
- **Write every row** runs the statement again in the backend and writes each
  row to the file as it arrives, so the rows never reach the interface. These
  entries stand in the menu only when the row limit stopped the read, and they
  offer a CSV file, a JSON file and an Excel file.

**Write every row** runs the statement that made the result, on the
connection of that run. A kept result of an older run thus writes its own
rows, also after a change of the text or of the connection of the tab.

**Write every row** accepts a statement that only reads, because it runs the
statement again. The backend refuses a statement that has a word such as
`INSERT`, `UPDATE`, `DELETE`, `INTO` or `EXEC` outside a string or a comment.
A `FOR UPDATE` clause on PostgreSQL and MySQL is accepted.

An Excel sheet holds 1048576 rows, the row of the column names among them, so
an export of more rows than that stops there and reports the stop.

An Excel cell gets a number when the value has at most 15 significant
digits, because Excel keeps no more. A longer number goes in as text. A text
of more than 32767 characters is cut at that bound, which is the largest
cell that Excel accepts.

An INSERT file writes a boolean as `TRUE` or `FALSE` on PostgreSQL and
Athena, and as `1` or `0` on the other engines. An array becomes the text
form of an array on PostgreSQL, as `'{1,2}'`, and an `ARRAY[...]`
constructor on Athena. On the other engines an array or an object becomes
its JSON text.

A copy to the clipboard separates the cells with tabs. A cell that holds a
tab or a line break, or that begins with a quote, goes in quotes, as Excel
writes it, so a spreadsheet keeps the value in one cell.

## The two limits

The settings hold two separate limits, and they are separate by design:

| Setting          | What it bounds                           | Default |
| ---------------- | ---------------------------------------- | ------- |
| Row limit        | The rows that the grid holds             | 10000   |
| Export row limit | The rows that **Write every row** writes | 1000000 |

The grid limit keeps the interface quick, because every row it holds lives in
the memory of the interface. Each connection also holds a Row limit in the
advanced part of its form, and a run uses the smaller of the two. The export limit is far higher, because those
rows go straight to the file.

A result that meets its limit reports the stop as a warning above the grid.
The rows that you see are the first rows of the answer, in the order that the
server sent them.
