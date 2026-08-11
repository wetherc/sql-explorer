# Tabs and saved statements

The **+** button of the tab row opens a tab, and so do `Ctrl` or `Cmd` with
`T` or with `N`. Each tab holds its own statement, its own connection and its
own values of the parameters.

The **File** menu of the top bar holds the four commands of a file: a new
query, a query from a file, a folder of queries, and the write of the query
that stands open. Each entry names its key.

A double click on the name of a tab changes that name. The Enter key keeps the
new name and the Escape key throws it away.

A tab that holds changes carries a mark. Closing such a tab asks first,
because the text of the statement goes with it.

The open tabs come back after a restart, with their names, their statements
and their values.

## The history

Each statement that runs goes into the history panel, with its connection and
the time of its run. A click on an entry opens that statement in a new tab, on
the connection of the entry when that connection is open.

## The file of a tab

The **Save** button writes the statement of the tab to a file, and so do
`Ctrl` or `Cmd` with `S`. A tab that came from the files panel goes back to
the same file. A tab without a file opens the save dialog of the operating
system, which starts in the first folder of the files panel. The tab then
carries the name of that file and keeps it.

The files panel holds no watch on the disk. A file that another program
writes keeps its old text in the tab, and a save from the tab writes the
whole file over it.

## Saved statements

The button beside **Save** keeps the statement of the tab in the library,
under a name and a folder that you give. The library stands in the history
panel, and a click opens the statement in a new tab.

The history and the library both persist.
