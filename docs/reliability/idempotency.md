---
status: unavailable
---

# Idempotency keys

Corvis has no built-in support for idempotency keys (storing the response of a request identified by an `Idempotency-Key` header and replaying it if it arrives again).

## Alternative

It can be built with existing pieces:

- An [interceptor](/interceptors) reads the header with `ctx.request()`, looks up a stored response and, if there is none, runs `next.handle()` and stores the result.
- To store responses within a single process, `CacheManager` works (see [Caching](/data/caching)). If you have several instances, store the key in the database with a unique constraint (`#[unique]` on a [corvis-orm](/data/corvis-orm) model), so two simultaneous requests are not both executed.
