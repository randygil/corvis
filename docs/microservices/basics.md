---
status: no-disponible
---

# Visión general

Corvis no tiene una capa de microservicios: no hay transportes, `ClientProxy`, patrones de mensaje (`@MessagePattern`/`@EventPattern`) ni aplicaciones híbridas que escuchen HTTP y un broker a la vez.

## Alternativa

Puedes consumir o publicar mensajes con el cliente Rust de tu broker dentro de un provider normal:

- Crea el cliente con una fábrica asíncrona (`Provider::use_factory`), leyendo la conexión de `ConfigService`.
- Empieza a consumir en `OnApplicationBootstrap` (lanzando una tarea con `corvis::tokio::spawn`) y detén el consumo en `BeforeApplicationShutdown` u `OnApplicationShutdown`; llama a `app.enable_shutdown_hooks()` para que se ejecuten con SIGINT/SIGTERM.
- Para un proceso que solo consume, sin escuchar en un puerto, consulta [Aplicaciones standalone](/standalone-applications): hoy `create_application_context` no registra los handlers `#[on_event]`, `#[cron]`, `#[interval]` ni `#[timeout]`, así que el consumo debe arrancar desde un hook del ciclo de vida.
- Dentro del mismo proceso, `EventEmitter` y `#[on_event]` desacoplan productores y consumidores (ver [Eventos](/application/events)).

Ver [Eventos del ciclo de vida](/fundamentals/lifecycle-events) y [Providers asíncronos](/fundamentals/async-providers).
