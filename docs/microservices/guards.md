---
status: no-disponible
---

# Guards

Corvis no tiene una capa de microservicios: no hay transportes, `ClientProxy`, patrones de mensaje (`@MessagePattern`/`@EventPattern`) ni aplicaciones híbridas que escuchen HTTP y un broker a la vez.

Como no hay manejadores de mensajes, tampoco hay guards para ellos.

## Alternativa

Comprueba la autorización en el código del consumidor. Los [guards](/guards) existen para HTTP y [WebSockets](/websockets/guards).
