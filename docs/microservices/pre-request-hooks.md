---
status: no-disponible
---

# Hooks previos a la petición

Corvis no tiene una capa de microservicios: no hay transportes, `ClientProxy`, patrones de mensaje (`@MessagePattern`/`@EventPattern`) ni aplicaciones híbridas que escuchen HTTP y un broker a la vez.

Como no hay manejadores de mensajes, tampoco hay hooks previos a la petición para ellos.

## Alternativa

En HTTP, el [middleware](/middleware) se ejecuta antes que el resto del pipeline.
