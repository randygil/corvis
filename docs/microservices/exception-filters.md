---
status: no-disponible
---

# Filtros de excepción

Corvis no tiene una capa de microservicios: no hay transportes, `ClientProxy`, patrones de mensaje (`@MessagePattern`/`@EventPattern`) ni aplicaciones híbridas que escuchen HTTP y un broker a la vez.

Como no hay manejadores de mensajes, tampoco hay filtros de excepción para ellos.

## Alternativa

En un consumidor propio, maneja los errores con `Result` dentro de la tarea que procesa cada mensaje y regístralos con `Logger` (ver [Logging](/application/logger)). Los [filtros de excepción](/exception-filters) existen para HTTP.
