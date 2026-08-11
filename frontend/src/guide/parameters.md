# Parameters

Write `:name` in a statement to make a parameter:

```sql
SELECT * FROM orders WHERE customer_id = :id AND city = :city
```

Each name takes a colour of its own in the editor. Two colons together stay
the cast of PostgreSQL and hold no name.

The bar above the editor names each parameter that the statement holds and the
value it carries. A value that is still missing reads as `unset`. A click on a
name opens the dialog of the values, and so does the **Parameters** button.

## The forms of a value

Each value holds the form that you chose, so a value stays text when it looks
like a number. An identifier such as `007` therefore keeps its zeros.

| Form          | What it sends                                    |
| ------------- | ------------------------------------------------ |
| Text          | The text as you wrote it                         |
| Number        | A number. A text that is not a number is refused |
| True or false | One of the two words                             |
| Empty value   | The empty value of the engine                    |

A run with a value that is still missing opens the dialog first. The values
stay with the tab, so a second run needs no dialog, and they come back after a
restart.

## What each engine does with a value

Every engine but Athena binds the values, so a value never becomes part of the
text of the statement. Athena gives no way to bind a value, so its values go
into the text as literals, and a text value keeps its quotes doubled.

On MS SQL Server a statement with a parameter travels inside `sp_executesql`.
A `USE` or a `SET` inside such a batch holds for that batch alone. A script
that carries a parameter is also sent whole and not one statement at a time,
because the numbers of the placeholders belong to the whole text.
