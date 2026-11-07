---
status: unavailable
---

# Serverless

Corvis no tiene adaptadores para plataformas *serverless* (AWS Lambda, Google Cloud Functions, Azure
Functions). La aplicación está pensada para correr como un proceso que escucha en un puerto con
`app.listen(...)`.

Lo que sí funciona hoy:

- **Plataformas de contenedores que escalan a cero** (Cloud Run, Azure Container Apps, Fly.io y
  similares). Ejecutan tu binario como un servidor HTTP normal; solo necesitas escuchar en el puerto
  que indique la plataforma, por ejemplo leyendo `PORT` con `ConfigService`:

  ```rust
  let port: u16 = app.get::<ConfigService>()?.get_or("PORT", 8080);
  app.listen(format!("0.0.0.0:{port}")).await
  ```

  Un binario de Rust arranca en milisegundos y usa poca memoria (ver las mediciones en
  [Corvis frente a Fastify, Express y Hono](/es/faq/corvis-vs-fastify-express-hono)), lo que ayuda con
  los arranques en frío.

- **Procesar peticiones en memoria.** `CorvisApplication::handle(request)` recibe una
  `corvis::Request` (construida con `Request::new(method, uri, headers, body)`) y devuelve la
  `Response` sin abrir ningún socket; es lo que usan las utilidades de testing. Es la pieza sobre la
  que se podría escribir un adaptador propio, pero Corvis no ofrece la conversión desde los eventos de
  ninguna plataforma.

Ten en cuenta que, en un entorno que congela el proceso entre invocaciones, las tareas en segundo
plano (`ScheduleModule`, intervalos, WebSockets) no se ejecutan de forma fiable.
