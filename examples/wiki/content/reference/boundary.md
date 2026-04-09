# boundary

File-level isolation between server, client, and shared code.

## Syntax

```forge
#! boundary: server
#! boundary: client
#! boundary: shared
```

The boundary directive must be the first line in a `.forge` file.

## Description

Boundaries enforce architectural isolation at compile time. Server-only code (endpoints, database access) cannot be referenced from client code, and vice versa. Shared code is accessible from both sides. The compiler checks cross-file references and rejects illegal access.

## Example

**server.forge**
```forge
#! boundary: server

endpoint admin_panel() -> Html
  give "<h1>Admin</h1>"
```

**shared.forge**
```forge
#! boundary: shared

type User
  name: Text
  email: Text
```

**client.forge**
```forge
#! boundary: client

# Can use User from shared.forge
# Cannot access admin_panel from server.forge — compile error
```

## Multi-File Projects

Run with multiple source files:

```bash
forge serve server.forge -s shared.forge
```

The boundary checker validates cross-file references before merging.

## Key Properties

- `server` — endpoints, data.store/get, agent declarations, wardens
- `client` — client-side rendering (future web compilation target)
- `shared` — types, events, state machines accessible from both sides
- Cross-boundary violations are compile-time errors, not runtime
- Each file has exactly one boundary (or none, defaulting to server)

## See Also

- [pure](/docs?slug=pure) — determinism boundary (orthogonal to file boundary)
- [system](/docs?slug=system) — server-side agent orchestration
- [event](/docs?slug=event) — shared events can cross boundaries
