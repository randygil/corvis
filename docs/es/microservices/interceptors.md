---
status: unavailable
---

# Interceptors

Corvis no tiene una capa de microservicios: no hay transportes, `ClientProxy`, patrones de mensaje (`@MessagePattern`/`@EventPattern`) ni aplicaciones híbridas que escuchen HTTP y un broker a la vez.

Como no hay manejadores de mensajes, tampoco hay interceptors para ellos.

## Alternativa

Envuelve el procesamiento de cada mensaje en una función propia que mida tiempos o registre resultados. Los [interceptors](/es/interceptors) existen para HTTP y [WebSockets](/es/websockets/interceptors).
