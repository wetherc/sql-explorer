# Future work

This document holds the design for one known weakness that the current
code does not correct. The section gives the problem, the design of the
correction, the order of the work, and the risks. The claims about the
code were examined against the source on 2026-08-10.

Two related notes stand in `docs/LIMITATIONS.md` instead of here: the
Excel export builds its sheet in memory, and a PostgreSQL script without
parameters is gathered by the library. Each note names the condition
under which a fix becomes worth its cost. A backend cache with windowed
reads for the grid is also set aside: the grid filters, sorts, selects
and exports over the full set of rows on the client, the row limit
already bounds what the grid holds, and such a cache needs backend sort
and filter to keep those functions. Design it as its own effort when a
measured case shows that the bound of `max_rows` is not enough.

## 1. Report an Entra access token that has expired

### The problem

The `EntraAccessToken` method stores the pasted token in the keychain as if
it were a password. Such a token lives for about one hour. The silent
reconnection path (`ensure_healthy`) reuses the stored token, so a
reconnection after the hour fails with a message that does not name the
cause. `describe_login` in `backend/src/db/drivers/mssql.rs` returns the
raw error for every method except `Integrated`, so the user sees the text
of the server.

### The design

- Read the `exp` claim of the token when a connection opens. The claim sits
  in the middle part of the JWT, base64url encoded without padding, so the
  decode uses `URL_SAFE_NO_PAD`. No signature check is needed to read a
  date the client itself acts on.
- Refuse a token whose `exp` lies more than 60 seconds in the past, before
  a socket opens, with the message: "The access token has expired. Paste a
  new one, or use the Azure CLI method, which reads a fresh token on each
  connection." The 60 seconds absorb a clock that runs early. The
  reconnection path calls `connect`, so this one check covers both paths,
  and no stored timestamp is needed.
- Add an `EntraAccessToken` arm to `describe_login` that maps a login
  refusal to the same message.
- In the interface, an authentication failure on a connection with this
  method opens the connection form with the token field cleared and
  focused. Two facts about the current code shape this step:
  - No path in `frontend/src/stores/connections.ts` examines the kind of
    an error today; every failure goes to `ui.reportError`. The store gains
    a branch for `ErrorKind.Authentication` on this method.
  - For a saved connection, an empty secret field means "keep the stored
    secret" (`ConnectionForm.vue`). For an expired token that meaning is
    wrong. The form entered through this path must require a new token and
    must not fall back to the stored one.
- Add a section to `LIMITATIONS.md`, next to the Windows Authentication
  section: the pasted token is not refreshed, and the Azure CLI method is
  the durable choice.

### The order of the work

1. A pure function `token_expiry(token: &str) -> Option<SystemTime>` with
   tests for a well formed token, a token without `exp`, an `exp` that is
   not a number, text with fewer than three parts, and text that is not a
   JWT.
2. The check in the `EntraAccessToken` arm of `auth_method`, and the new
   arm of `describe_login`, with tests.
3. The form behaviour in the interface and the documentation section.

### The risks

Small. The parse must not reject a token it cannot read; an unreadable
token goes to the server unchanged, and the server stays the judge.
