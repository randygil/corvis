---
status: unavailable
---

# Drizzle

Drizzle is a TypeScript ORM and has no Corvis integration. There is no module that registers its client in the container.

## Alternative

For typed, SQL-like queries in Rust, register an `sqlx` pool as a provider with `Provider::use_factory` (see [Overview](/data/overview)). If you prefer an ORM with models and relations, use [corvis-orm](/data/corvis-orm).
