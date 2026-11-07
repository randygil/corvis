---
status: unavailable
---

# TypeORM

TypeORM is a Node.js ORM, and Corvis, which is a Rust framework, has no integration with it: there is no equivalent module and no injectable per-entity repositories.

## Alternative

Use [corvis-orm](/data/corvis-orm), the workspace's built-in ORM (PostgreSQL and SQLite, with injectable `OrmModule` and `OrmService`), or register an `sqlx` pool or another crate as a factory provider (see [Overview](/data/overview)).
