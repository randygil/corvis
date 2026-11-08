---
status: unavailable
---

# Pipes

Corvis no tiene una capa de microservicios: no hay transportes, `ClientProxy`, patrones de mensaje (`@MessagePattern`/`@EventPattern`) ni aplicaciones híbridas que escuchen HTTP y un broker a la vez.

Como no hay manejadores de mensajes, tampoco hay pipes para ellos.

## Alternativa

Valida cada mensaje con un schema de `corvis::z` (`schema.parse(&valor)` o `parse_as::<T>(&valor)`) antes de procesarlo (ver [Validación](/es/application/validation)). Los [pipes](/es/pipes) existen para HTTP y WebSockets.
