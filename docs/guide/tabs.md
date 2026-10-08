---
title: Tabs and saved statements
description: Work in several editor tabs, each with its own server session, and save statements to files or the library.
order: 4
---

# Tabs and saved statements

To open a tab, click **+** in the tab row or press `Ctrl`/`Cmd` + `T` or
`Ctrl`/`Cmd` + `N`. Each tab keeps its own statement, connection and parameter
values, and its own server session with its temporary tables and `SET` options.
A session that stays idle for 60 minutes closes. When a connection reaches its
session limit, a new tab can also close a session that has been idle for at
least 5 minutes. The **Connections** page of this guide describes both cases.

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

Click **Save** or press `Ctrl`/`Cmd` + `S` (**Save to file** in the palette and
the **File** menu) to write the tab's statement to a file. A tab that came from a file saves back to the same file. Saving a tab that
has no file opens your operating system's save dialog, which starts in the files
panel's first folder, and the tab then takes the file's name and keeps it.

Opening or saving a file through a dialog gives the application access to that
file only, so its folder doesn't join the files panel. To see the files beside
it, open the folder from the **File** menu. The panel doesn't show hidden
entries, and the application neither reads nor writes them inside an open
folder.

If you open a file from a folder and then close the folder, the application
loses write access to that file, but the tab still shows it after a restart.
**Save** then opens the save dialog at that file's name and folder. Confirm it once, and the following
saves write the file directly.

The files panel doesn't watch the disk. A folder reads its entries again each
time you expand it, and the **Refresh** button at the top of the panel reads each
top folder again. If another program writes a file, the tab keeps its old
text, and saving from the tab overwrites the whole file. When a folder can't be
read, the panel shows the reason with a **Retry** button.

### Encodings

A file keeps its encoding when you save it. The application reads UTF-8, UTF-8
with a byte order mark, UTF-16 with a byte order mark, and Windows-1252, which
is how many older Windows tools save scripts. A file that isn't valid UTF-8 and
has no byte order mark opens as Windows-1252. If you then type a character that
Windows-1252 can't store, such as an emoji or a Greek letter, the save writes
UTF-8 with a byte order mark instead, so no character is lost, and a warning
tells you.

The application doesn't open a file that looks binary, such as an image. A file
counts as binary when its first 8 KB contain a zero byte, except UTF-16 text
with a byte order mark.

## Damaged settings files

The application keeps its connections, history, open tabs and
files-panel folders in JSON files. When one of those files can't be read, the application renames it to
`<name>.corrupt-<number>` beside the original, starts with an empty file in
its place, and shows a notice that names the kept copy. You can open the copy
in a text editor to recover what it contains.
