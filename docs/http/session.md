---
status: unavailable
---

# Session

Corvis has no server-side sessions: there is no middleware equivalent to `express-session`, no
session store, and no `req.session` object.

Real alternatives, depending on what you need:

- **Stateless authentication.** A JWT signed with `JwtService` and sent in
  `Authorization: Bearer` covers most APIs. See [Authentication](/security/authentication).
- **Your own cookie-based session.** Issue an `HttpOnly` cookie with a random identifier
  (`res.cookie(...)`, see [Cookies](/http/cookies)), store the data in an injectable provider (an
  in-memory map or your database with [corvis-orm](/data/corvis-orm)) and read it in a guard with
  `req.cookie("sid")`. The guard can attach the data to the request with `req.insert(value)` so that
  the handler reads it with a `FromContext` parameter.
- **Data that only lives during one request.** `req.insert(value)` and `req.get::<T>()` (the
  request extensions) are used to pass data between middleware, guards, interceptors and the
  handler. They do not outlive the request.

If you store sessions in memory, remember that each instance of the application has its own map and
that they are lost on restart.
