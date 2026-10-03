---
title: Tabs and saved statements
description: Work in several editor tabs, each with its own server session, and save statements to files or the library.
order: 4
---

# Tabs and saved statements

To open a tab, click **+** in the tab row or press `Ctrl`/`Cmd` + `T` or
`Ctrl`/`Cmd` + `N`. Each tab keeps its own statement, connection and parameter
values.

Your operating system's **File** menu has four file commands: new query, open a
query from a file, open a folder of queries, and save the current query. Each
entry shows its shortcut. On macOS the menu sits in the bar at the top of the
screen, and on Windows and Linux it sits in the window.

Double-click a tab's name to rename it. Press `Enter` to keep the new name or
`Escape` to discard it.

A tab with unsaved changes shows a mark, and closing it asks you first, because
the unsaved text goes with the tab. If you undo every change so the text matches
the saved text again, the mark goes away. A tab that had the mark at the last
restart keeps it until the next save.

Open tabs come back after a restart, with their names, statements and values.

## History

Every statement that runs goes into the history panel, with its connection and
the time it ran. Click an entry to open its statement in a new tab, on the
entry's connection if that connection is open.

The history keeps the last 500 statements, up to about 4 million characters of
statement and error text in total. When a new entry goes over that amount, the
history drops the oldest entries, although it always keeps the newest entry,
even when that entry's text alone is larger.

## Files

Click **Save** or press `Ctrl`/`Cmd` + `S` to write the tab's statement to a
file. A tab that came from a file saves back to the same file. Saving a tab that
has no file opens your operating system's save dialog, which starts in the files
panel's first folder, and the tab then takes the file's name and keeps it.

Opening or saving a file through a dialog gives the application access to that
file only, so its folder doesn't join the files panel. To see the files beside
it, open the folder from the **File** menu. The panel doesn't show hidden
entries, and the application neither reads nor writes them inside an open
folder.

The files panel doesn't watch the disk. If another program writes a file, the
tab keeps its old text, and saving from the tab overwrites the whole file.

## Saved statements

The button beside **Save** keeps the tab's statement in the library, under a
name and folder that you choose. The library sits in the history panel, and
clicking an entry opens its statement in a new tab.

Both the history and the library persist across restarts.
