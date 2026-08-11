# Exports and the two row limits

A result goes to a CSV, JSON, Markdown, INSERT or Excel file, or to the
clipboard.

The menu of the grid holds two kinds of export:

- **Export the rows** writes what the grid holds. The rows pass through the
  interface. Mark rows first, and the entries then write those rows alone.
- **Write every row** runs the statement again in the backend and writes each
  row to the file as it arrives, so the rows never reach the interface. These
  entries stand in the menu only when the row limit stopped the read, and they
  offer a CSV file, a JSON file and an Excel file.

An Excel sheet holds 1048576 rows, the row of the column names among them, so
an export of more rows than that stops there and reports the stop.

## The two limits

The settings hold two separate limits, and they are separate by design:

| Setting          | What it bounds                           | Default |
| ---------------- | ---------------------------------------- | ------- |
| Row limit        | The rows that the grid holds             | 10000   |
| Export row limit | The rows that **Write every row** writes | 1000000 |

The grid limit keeps the interface quick, because every row it holds lives in
the memory of the interface. The export limit is far higher, because those
rows go straight to the file.

A result that meets its limit reports the stop as a warning above the grid.
The rows that you see are the first rows of the answer, in the order that the
server sent them.
