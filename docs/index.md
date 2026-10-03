---
title: An SSMS alternative for Mac
description: "SQL Explorer is an open source SQL client for macOS, Windows and Linux that connects to MS SQL Server, AWS Athena, PostgreSQL, MySQL, MariaDB and SQLite."
---

SQL Explorer is an open source desktop client for SQL databases that runs on
macOS, Windows and Linux. If you work on a Mac and miss SQL Server Management
Studio (SSMS) or Azure Data Studio, SQL Explorer gives you an alternative to
both.

It connects to:

- MS SQL Server, with a SQL login, your own account (SSPI on Windows, Kerberos
  on macOS and Linux), Microsoft Entra ID through the Azure CLI, or an access
  token
- AWS Athena, with each statement's scan cost in the status bar
- PostgreSQL
- MySQL and MariaDB
- SQLite

![The object tree, the editor and a query result](screenshots/overview.png)

## Browse a server's objects

The tree lists each connection's databases, schemas, tables, views and
columns. An object's context menu scripts its CREATE, SELECT, INSERT and
UPDATE statements, and the Properties dialog shows a table's columns, indexes
and constraints.

![A table's context menu in the tree](screenshots/explorer-menu.png)

## Write and run queries

The editor completes table and column names, including the columns behind an
alias. Every tab gets its own server session, so two tabs can run queries at
the same time. When a query uses a named parameter such as `:country`, the
editor asks for its value before the query runs.

![Completion after an alias](screenshots/completion.png)

## Read and export results

The result grid draws only the rows on screen, so large results stay fast. You
can sort and filter the rows and see how many you have selected. Export them to
CSV, JSON, Markdown, INSERT statements or Excel, or copy them to the clipboard.

![A result with a filter, a sort and a selection](screenshots/grid.png)

## Read a statement's plan

A plan tab shows a statement's estimated or actual execution plan.

![A statement's execution plan](screenshots/plan.png)

## Query AWS Athena

The status bar shows how much data each statement scanned, what that scan
cost, and what the session has cost so far. A connection can also reuse an
earlier result up to an age you choose, and a reused result scans no data.

![An Athena query and the data it scanned](screenshots/athena.png)

## Run it on Linux

The Linux build comes as a `.deb` package, an `.rpm` package and an AppImage.
Passwords go to your desktop's Secret Service, such as GNOME Keyring or
KWallet. The screenshot below has five sample servers open at once:
PostgreSQL, MS SQL Server, MySQL, MariaDB and SQLite.

![Five open connections on Linux, with a MySQL result](screenshots/linux/overview.png)

![A filtered SQLite result in the light theme](screenshots/linux/sqlite.png)

## Keep your secrets safe

Passwords and AWS secret keys live in your operating system's keychain, and the
settings file never contains a password. Every connection verifies the
server's certificate by default.

## Get it

Download a build from the
[releases page](https://github.com/wetherc/sql-explorer/releases), or build it
from source by following the
[README](https://github.com/wetherc/sql-explorer#setup). SQL Explorer is free
and open source under the MIT license.
