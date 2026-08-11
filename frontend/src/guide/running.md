# Run and stop

Choose a connection at the top of the tab, then write a statement.

- `Ctrl` or `Cmd` with `Enter` runs the statement under the cursor.
- The same keys with `Shift` run the whole script.
- A selection runs in place of the statement under the cursor.

A script runs statement by statement. The splitter respects quotes, comments,
dollar tags and the `DELIMITER` command of MySQL.

The **Format** button lays the statement out with the rules of its dialect.

## While a statement runs

The **Stop** button stands beside **Run** while a statement runs. The time
limit of the connection also stops a statement that runs too long.

A stop opens a new session on some engines, so the temporary tables and the
`SET` options of the old session go with it.

## The messages

The **Messages** tab holds what the server sent, with the severity, the code,
the line and the procedure of each message. A failure of a statement appears
there as well as in the corner.

## Plans

The **Plan** button reads the plan of one statement. The estimated plan needs
no run. The actual plan runs the statement, so the application asks first: a
statement that writes rows writes them, and a statement on Athena scans data.

The plan of a script covers the statement under the cursor alone.
