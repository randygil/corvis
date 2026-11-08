---
status: unavailable
---

# Serverless

Corvis has no adapters for *serverless* platforms (AWS Lambda, Google Cloud Functions, Azure
Functions). The application is designed to run as a process that listens on a port with
`app.listen(...)`.

What does work today:

- **Container platforms that scale to zero** (Cloud Run, Azure Container Apps, Fly.io and
  similar). They run your binary as a normal HTTP server; you only need to listen on the port
  the platform indicates, for example by reading `PORT` with `ConfigService`:

  ```rust
  let port: u16 = app.get::<ConfigService>()?.get_or("PORT", 8080);
  app.listen(format!("0.0.0.0:{port}")).await
  ```

  A Rust binary starts in milliseconds and uses little memory (see the measurements in
  [Corvis vs Fastify, Express and Hono](/faq/corvis-vs-fastify-express-hono)), which helps with
  cold starts.

- **Processing requests in memory.** `CorvisApplication::handle(request)` takes a
  `corvis::Request` (built with `Request::new(method, uri, headers, body)`) and returns the
  `Response` without opening any socket; this is what the testing utilities use. It is the piece
  on which an adapter of your own could be written, but Corvis does not provide the conversion from the events of
  any platform.

Keep in mind that, in an environment that freezes the process between invocations, background
tasks (`ScheduleModule`, intervals, WebSockets) do not run reliably.
