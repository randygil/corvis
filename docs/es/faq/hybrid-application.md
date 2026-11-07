---
status: unavailable
---

# Aplicación híbrida

Una aplicación híbrida atiende peticiones HTTP y, a la vez, escucha mensajes de uno o más
transportes de microservicios (Redis, NATS, Kafka, etc.). Corvis no tiene microservicios todavía
(ver [Microservicios](/es/microservices/basics)), así que tampoco tiene aplicaciones híbridas ni un
método para conectar transportes a una aplicación HTTP.

Lo que sí puedes combinar en una misma aplicación:

- **HTTP y WebSockets.** Los controladores y los `#[websocket_gateway]` conviven en el mismo
  proceso y el mismo puerto, y comparten providers. En `examples/tertulia`, `RoomsService` se inyecta
  tanto en el gateway como en un controlador REST, y el controlador inyecta el gateway para empujar
  anuncios por WebSocket.
- **HTTP y tareas en segundo plano.** `ScheduleModule` (cron, intervalos, timeouts) y
  `EventEmitterModule` corren dentro de la misma aplicación. Ver
  [Tareas programadas](/es/application/task-scheduling) y [Eventos](/es/application/events).
- **Un consumidor de colas propio.** Puedes arrancar un bucle de consumo (con el cliente del broker
  que prefieras) en `on_application_bootstrap` de un provider, con `corvis::tokio::spawn`, y detenerlo
  en `on_application_shutdown`. Los handlers que llame no pasan por guards, pipes ni interceptors.
- **Un contexto sin HTTP.** Para un proceso *worker* separado,
  `CorvisFactory::create_application_context::<M>()` crea el contenedor de DI sin servidor. Ver
  [Aplicaciones standalone](/es/standalone-applications).
