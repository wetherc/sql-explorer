---
title: The results grid
description: Sort, filter and select a result's rows, keep a result beside the next one, and read the status bar.
order: 7
---

# The results grid

The grid draws only the rows on screen, so a large result stays fast. You can
sort and filter the rows, the grid marks missing values, and a wide value opens
in its own box.

Each statement in a script that returns rows gets its own tab beside the
**Messages** tab. Each column header shows the column's type. On PostgreSQL, a
column of a custom array type shows the type's internal name, such as `_mytype`.
A statement that writes, such as `INSERT ... RETURNING`, isn't prepared first,
so its columns of a custom type show the type's numeric OID instead of a name.

## Pinned results

Click the pin beside the tabs to keep a result. The next statement then leaves
the pinned result's rows and run time in place, so you can compare two results
side by side.

## Panel layout

The buttons beside the tabs move the results panel. One hides the panel and
leaves a bar below the editor, which `Ctrl`/`Cmd` + `J` also does. The other
moves the panel between below the editor and beside it. Running a statement
brings the panel back.

## Status bar

The status bar shows the last run's row count and time. While a statement runs,
the same spot shows the time so far. For Athena, the bar also shows how much
data the statement scanned and what that cost at the rate in your settings.
