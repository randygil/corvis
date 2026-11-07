---
status: unavailable
---

# Activación por socket

Corvis **no admite activación por socket** (recibir de systemd u otro supervisor un socket ya abierto). `app.listen` siempre abre su propio socket a partir de una dirección: un puerto (`3000`), una cadena (`"0.0.0.0:3000"`) o un `SocketAddr`.

## Alternativas

- **Reinicios ordenados**: con `app.enable_shutdown_hooks()`, al recibir `SIGINT` o `SIGTERM` el servidor deja de aceptar conexiones y ejecuta los lifecycle hooks de cierre. Combínalo con un balanceador o un proxy inverso que reparta el tráfico entre dos instancias mientras una se reinicia. Ver [Despliegue](/es/deployment).
- **Puertos privilegiados**: en lugar de que el proceso abra el puerto 80 o 443, escucha en un puerto alto y deja el puerto público al proxy inverso.
