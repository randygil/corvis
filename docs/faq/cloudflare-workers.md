---
status: no-disponible
---

# Cloudflare Workers

Corvis no se puede ejecutar en Cloudflare Workers. Workers ejecuta JavaScript o WebAssembly en un
entorno sin sockets TCP de servidor ni hilos, y Corvis depende de Tokio (runtime multihilo) y de
hyper (servidor sobre TCP), que no compilan para ese destino.

Alternativas:

- Despliega la aplicación como contenedor o binario en un servidor o en una plataforma de
  contenedores, y usa Cloudflare delante como proxy y CDN.
- Mueve a un Worker solo la lógica de borde (redirecciones, caché, reescritura de cabeceras) y deja
  la API en Corvis.

Ver también [Despliegue](/deployment) y [Serverless](/faq/serverless).
