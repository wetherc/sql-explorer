---
title: Connections
description: Set up connections to MS SQL Server, AWS Athena, PostgreSQL, MySQL, MariaDB and SQLite.
order: 2
---

# Connections

SQL Explorer connects to MS SQL Server, AWS Athena, PostgreSQL, MySQL and
MariaDB, and SQLite. To add a connection, open the connections panel from the
rail and click **New connection**. The form shows only the fields that your
chosen engine uses.

## Before you save

Click **Test** before you save a record you're unsure of. It opens the
connection, checks that the server answers, and closes it again.

Passwords go into your operating system's keychain, and the settings file never
contains one. When you edit a record, leave the password box empty to keep the
stored password. If the keychain isn't reachable (on Linux without a Secret
Service, for example), the form tells you, and the password stays in memory only
until the application closes.

For an option that the form doesn't show, enter a **Connection string** in the
form's **Advanced** section. The string replaces the host, the port and the
database. The form's user, password and connection time limit still apply when
the string doesn't give them, and on PostgreSQL and MySQL the form's transport
mode also applies when the string names no mode. The settings file keeps the
string as plain text, so the application refuses to save a string that contains
a password. Type the password in the **Password** box instead.

## Transport

A connection encrypts its traffic in one of four modes:

- **Verify the certificate**, the default. Use it outside a trusted network.
- **Encrypt, accept any certificate**.
- **Encrypt when the server offers it**.
- **No encryption**, which sends the credentials and the results across the
  network in clear text.

Certificate checks trust the same authorities as your operating system, such as
the roots in the macOS keychain, and the **Certificate authority file** adds one
more. The application reads the system's roots when it starts, so a root that
you add later takes effect after a restart.

## Read-only sessions

The read-only switch in the form's **Advanced** section does something different
on each engine:

- **PostgreSQL and MySQL**: the server refuses writes in the session, although a
  `SET` statement in the session can turn this off again.
- **SQLite**: the file opens read-only.
- **MS SQL Server**: the login asks for a readable secondary of an availability
  group. A primary or a standalone server still accepts writes, so to stop
  writes there, give the user a login without write permissions.
- **AWS Athena**: the form doesn't show the switch.

## MS SQL Server

A named instance finds its port through the SQL Browser service. Enter the
instance name in **Named instance**, in the form's **Advanced** section.

You can authenticate in four ways:

- **A SQL login**, with a user and a password.
- **Your own account.** Windows uses SSPI. On macOS and Linux the connection
  uses your Kerberos ticket, so run `kinit` first and name the server by its
  full host name.
- **Microsoft Entra ID through the Azure CLI.** Run `az login` first. Each
  connection reads a fresh token.
- **Microsoft Entra ID with an access token** that you paste. A token is valid
  for about one hour, and the application can't renew it, so the form asks for a
  new token once the server refuses the old one.

Your own account and the Azure CLI need no secret. When you save a connection
with either method, the application removes any password or token that the
keychain kept for it.

## AWS Athena

An Athena connection needs a region. It can also reuse an earlier run's result,
up to an age that you choose. A reused result costs nothing, because Athena
scans no data for it.

## Sessions

Each tab has its own server session, so statements in two tabs run at the same
time. A tab's temporary tables, `SET` options and transactions stay with that
tab's session. The connection's **Max sessions** option caps the number of
sessions on one server, and it defaults to six.

When a connection stops answering, the application opens it again. The colour
beside each connection shows its state.
