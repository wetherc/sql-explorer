# Connections

Open the connections panel from the rail. The **New connection** button opens
the form. The form shows the fields that the engine you chose uses, and hides
the rest.

The application connects to five engines: MS SQL Server, AWS Athena,
PostgreSQL, MySQL and MariaDB, and SQLite.

## Before you save

The **Test** button opens the connection, confirms that it answers, and closes
it again. Use it before you save a record that you are not sure of.

A password goes into the keychain of the operating system. The settings file
holds no password. Leave the password box empty when you edit a record, and
the stored password stays as it is.

## Transport

A connection encrypts its traffic in one of four ways:

- Verify the certificate. This is the default. Use it outside a trusted
  network.
- Encrypt, and accept any certificate.
- Encrypt when the server offers encryption.
- No encryption. The credentials and the results then cross the network in
  clear text.

## MS SQL Server

A named instance finds its port through the SQL Browser service. Give the name
of the instance in the advanced part of the form.

Four methods authenticate against the server:

- A SQL login, with a user and a password.
- The account of the user. Windows uses SSPI. macOS and Linux use the Kerberos
  ticket of the user, so run `kinit` first and name the server by its full host
  name.
- Microsoft Entra ID through the Azure CLI. Run `az login` first. This method
  reads a fresh token for each connection.
- Microsoft Entra ID with an access token that you paste. Such a token is
  valid for about one hour, and the application cannot get another one for it.
  The form asks for a new token once the server refuses the old one.

## AWS Athena

An Athena connection needs a region. It can also reuse the result of an
earlier run, up to an age that you give. A reused result costs nothing,
because the engine scans no data for it.

## Sessions

Each tab holds one server session, so the statements of two tabs run at the
same time. The temporary tables, the `SET` options and the transactions of a
tab stay with the session of that tab. The session limit in the options of the
connection bounds the sessions of one server, and it starts at six.

A connection that stops answering is opened again. The colour beside each
connection reports its state.
