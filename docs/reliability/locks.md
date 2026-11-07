---
status: unavailable
---

# Distributed locks

Corvis does not include distributed locks (mutual exclusion across several instances of the application).

## Alternative

- Within a single process, a `tokio::sync::Mutex` kept in a provider is enough.
- Across instances that share a PostgreSQL database, you can use Postgres advisory locks with `corvis-orm` raw SQL, inside a transaction so the lock is released when it ends:

```rust
use corvis_orm::{OrmError, sql};

db._transaction(|tx| async move {
    let rows = tx._query_raw(sql!("SELECT pg_try_advisory_xact_lock({key}) AS locked")).await?;
    // If `locked` is false, another instance holds the lock.
    Ok::<_, OrmError>(rows)
}).await?;
```

This is useful, for example, to make a `#[cron]` task run on a single instance (see [Task scheduling](/application/task-scheduling)).
