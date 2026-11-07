---
status: no-disponible
---

# Outbox transaccional

Corvis no implementa el patrón outbox (escribir los mensajes a publicar en una tabla dentro de la misma transacción que los datos, y publicarlos después desde un proceso aparte).

## Alternativa

El patrón se puede montar con lo que ya existe:

- Escribe el registro de negocio y el mensaje pendiente en la misma transacción interactiva de `corvis-orm` (`db._transaction(|tx| async move { ... })`, ver [corvis-orm](/data/corvis-orm)).
- Publica los mensajes pendientes desde una tarea `#[interval]` o `#[cron]` (ver [Tareas programadas](/application/task-scheduling)) y márcalos como enviados.

Ten en cuenta que `EventEmitter` es solo en memoria: un evento emitido justo antes de que el proceso se detenga se pierde.
