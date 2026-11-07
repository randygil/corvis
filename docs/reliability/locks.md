---
status: no-disponible
---

# Bloqueos distribuidos

Corvis no incluye bloqueos distribuidos (exclusión mutua entre varias instancias de la aplicación).

## Alternativa

- Dentro de un solo proceso, un `tokio::sync::Mutex` guardado en un provider basta.
- Entre instancias que comparten una base PostgreSQL, puedes usar los bloqueos de asesoramiento de Postgres con SQL crudo de `corvis-orm`, dentro de una transacción para que el bloqueo se libere al terminar:

```rust
use corvis_orm::{OrmError, sql};

db._transaction(|tx| async move {
    let rows = tx._query_raw(sql!("SELECT pg_try_advisory_xact_lock({key}) AS locked")).await?;
    // Si `locked` es false, otra instancia tiene el bloqueo.
    Ok::<_, OrmError>(rows)
}).await?;
```

Esto es útil, por ejemplo, para que una tarea `#[cron]` se ejecute en una sola instancia (ver [Tareas programadas](/application/task-scheduling)).
