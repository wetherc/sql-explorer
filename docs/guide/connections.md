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

A stored password belongs to one server and login. If you change the engine,
host, port, user, named instance or connection string of a saved record and
leave the password box empty, the application drops the stored password when
you save, so the password never goes to another server. The form asks you to
type the password again in that case. An Athena record drops its stored
secret access key when its AWS region or access key ID changes, and also when
its credentials stop coming from an access key. **Duplicate** copies a record
without its password, so you enter the password again for the copy.

For an option that the form doesn't show, enter a **Connection string** in the
form's **Advanced** section. The string replaces the host, the port and the
database. The form's user, password and connection time limit still apply when
the string doesn't give them, and on PostgreSQL and MySQL the form's transport
mode also applies when the string names no mode. On MS SQL Server, the form's
transport mode applies when the string has no `Encrypt` key, the form's
certificate setting applies when the string has no `TrustServerCertificate`
key, and the form's application name applies when the string names none. The
form's authentication method, such as Windows Authentication or the Azure CLI,
applies when the string gives no user, password or integrated security key. The settings file keeps the
string as plain text, so the application refuses to save a string that contains
a password. Type the password in the **Password** box instead.

## Transport

A connection encrypts its traffic in one of four modes:

- **Verify certificate**, the default. Use it outside a trusted network.
- **Encrypt, accept any certificate**.
- **Encrypt if the server supports it**.
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
instance name in **Named instance**, in the form's **Advanced** section. While a
named instance is set, the form disables the **Port** box and the connection
ignores any port in it, because the SQL Browser service gives the port.

You can authenticate in four ways:

- **SQL login**, with a user and a password.
- **Windows Authentication**, with your own account. Windows uses SSPI. On macOS and Linux the connection
  uses your Kerberos ticket, so run `kinit` first and name the server by its
  full host name.
- **Microsoft Entra ID (Azure CLI)**. Run `az login` first. Each
  connection reads a fresh token.
- **Microsoft Entra ID (access token)**, with a token that you paste. A token is valid
  for about one hour, and the application can't renew it, so the form asks for a
  new token once the server refuses the old one.

Windows Authentication and the Azure CLI need no secret. When you save a connection
with either method, the application removes any password or token that the
keychain kept for it.

If a Windows Authentication connect fails with "Couldn't reach the Kerberos
server. Check your VPN or network connection.", the login timed out while it
waited for your Kerberos server (the KDC), or Kerberos reported that it couldn't
find or contact a KDC for your realm. The SQL Server itself may be fine. This
usually means the VPN is off or the network can't see your domain's KDC.
Connect to the VPN and try again. The error's detail keeps the driver's own
text, such as the time limit that passed or the GSSAPI message.

## AWS Athena

An Athena connection needs a region. It can also reuse an earlier run's result,
up to an age that you choose. A reused result costs nothing, because Athena
scans no data for it.

## Sessions

Each tab has its own server session, so statements in two tabs run at the same
time. A tab's temporary tables, `SET` options and transactions stay with that
tab's session. The connection's **Max sessions** option caps the number of
sessions on one server, and it defaults to six.

A tab's session stays open while the tab is idle. After 60 minutes without a
statement, the application closes the session, unless the session is inside an
open transaction. When a new tab needs a session and the connection is already
at its cap, the application closes the session that went unused the longest,
but only if that session has been idle for at least 5 minutes. It skips a
session that is running a statement, waiting to run one, or inside a
transaction. A closed session loses its temporary tables and `SET` options, and
the tab opens a fresh session on its next run. If no session can close, the new
tab's run fails with a message that suggests closing a tab or raising
**Max sessions**.

When a connection stops answering, the application opens it again. A session
that sat idle for 30 seconds gets a quick check before its next use, and a
session that gives no answer within 5 seconds counts as one that stopped
answering. This catches a connection that a firewall or a sleeping laptop
dropped without notice. On MS SQL Server, PostgreSQL and MySQL, the operating
system also sends a keepalive probe after 60 seconds of silence on the socket,
which keeps a firewall from dropping an idle connection, and closes a connection
whose server stops answering, even in the middle of a long statement. A
connection string that sets its own keepalive values keeps them. The colour beside each
connection shows its state.

A read for the explorer tree, the Properties dialog or completion ends with a
timeout error after 60 seconds. The time counts from the click, so it includes
the check and the opening of a new connection. On Athena, each request to AWS
also gives up after 30 seconds and is retried.

On MS SQL Server, PostgreSQL and MySQL, these reads also stop waiting for a
lock after 5 seconds. If another session has an uncommitted change to a table,
such as an `ALTER TABLE` in an open transaction, the read of that table fails
with a message about the lock, and the rest of the tree keeps loading. The
limit applies only to the explorer's own connection. Statements in your tabs
wait for locks as the server's settings decide.
