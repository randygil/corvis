---
status: no-disponible
---

# Colas

Corvis todavía no incluye un módulo de colas de trabajo (productores, consumidores, reintentos y trabajos persistidos en un broker como Redis). No hay atributos para declarar procesadores ni un registro de colas.

## Alternativa

- Para trabajo en segundo plano dentro del mismo proceso, emite un evento con `EventEmitter::emit`: cada listener corre en su propia tarea de Tokio y la petición HTTP no espera (ver [Eventos](/es/application/events)).
- Para trabajo periódico, usa `#[interval]` o `#[cron]` de `ScheduleModule` (ver [Tareas programadas](/es/application/task-scheduling)).
- Para una cola persistente, usa directamente el cliente de Rust del broker que prefieras y regístralo como provider con `Provider::use_factory` (ver [Providers asíncronos](/es/fundamentals/async-providers)). Corvis no aporta reintentos ni visibilidad de trabajos en ese caso.
