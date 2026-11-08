---
status: partial
---

# Adaptadores HTTP

Corvis tiene un único servidor HTTP, construido sobre [hyper](https://hyper.rs) 1 y el runtime
multihilo de Tokio. No hay adaptadores intercambiables (no existe una variante "sobre Express" y
otra "sobre Fastify"): el servidor integrado reproduce la semántica de una aplicación Express 5 en
lo que el cliente observa (body parser, router, `ETag`, cabeceras por defecto, errores) y es el que
se mide en el benchmark del repositorio.

Esta página reúne lo que se puede configurar de ese servidor.

## Protocolos

- HTTP/1.1 y HTTP/2 sin TLS con *prior knowledge* (h2c), activo por defecto.
  `ServerOptions::new().h2c(false)` acepta solo HTTP/1.x. Si h2c sigue activo por defecto se decide en el issue
  #144; consulta [Análisis de HTTP y compatibilidad](/es/security/http-parsing).
- No hay HTTPS: termina TLS en un proxy inverso. Ver [HTTPS y varios servidores](/es/faq/multiple-servers).
- Upgrades a WebSocket, con gateways o con `corvis::ws`. Ver [Gateways](/es/websockets/gateways).

## Timeouts y conexiones

`app.set_server_options(ServerOptions)` usa por defecto los valores de Node.js:

```rust
use std::time::Duration;
use corvis::ServerOptions;

app.set_server_options(
    ServerOptions::new()
        .headers_timeout(Duration::from_secs(30))    // 60 s por defecto
        .request_timeout(Duration::from_secs(120))   // 300 s por defecto; responde 408
        .keep_alive_timeout(Duration::from_secs(10)) // 5 s por defecto
        .max_connections(10_000)                     // sin tope por defecto
        .max_header_size(16 * 1024),                 // 32 KiB por defecto; responde 431
);
```

Cada timeout acepta `None` para desactivarlo, por ejemplo `.request_timeout(None)`. Al alcanzar
`max_connections`, las conexiones nuevas esperan en la cola de `accept`. Las conexiones WebSocket abiertas cuentan
hasta que se cierran, y también un socket durante hasta 2 s después de una respuesta de error temprana (`413`, `431`,
...), mientras el servidor deja que el cliente la lea antes de cerrar. Con un tope, agrega un heartbeat a tus gateways
para que los clientes caídos no ocupen lugares.

## Cuerpos de las peticiones

| Ajuste | Por defecto | API |
|---|---|---|
| Límite del parser JSON / urlencoded | 100 kb cada uno (`413` al superarlo) | `app.use_body_parser("json", bytes)`, `app.use_body_parser("urlencoded", bytes)` |
| Desactivar el parseo | Activo | `app.disable_body_parser()` |
| Límite de transporte de otros tipos | 1 MiB | `app.set_body_limit(bytes)` |
| Profundidad máxima de un JSON | 256 niveles (`400`) | `app.set_body_parser_max_depth(n)` |
| Parser de `req.query` | `simple` (como Express 5) | `app.set_query_parser(QueryParser::Extended)` |

Solo se parsean `application/json` y `application/x-www-form-urlencoded`; el resto deja el cuerpo
sin parsear y disponible en `req.raw_body()` (ver [Cuerpo sin procesar](/es/faq/raw-body)). Los cuerpos
`multipart/*` los lee un interceptor de archivos (ver [Subida de archivos](/es/http/file-upload)).
`QueryParser` está en `corvis::http::body_parser`.

## Respuestas

- `ETag` débil en los cuerpos JSON, texto y HTML, con `304` para `GET`/`HEAD` condicionales.
  `app.set_etag(EtagMode::Strong)` o `app.disable("etag")` lo cambian (`EtagMode` está en `corvis::http`).
- `X-Powered-By: corvis`. `app.disable("x-powered-by")` la quita y
  `app.set_x_powered_by("...")` cambia el valor.

## IP del cliente y proxies

Por defecto `req.ip()` es la dirección del socket y `X-Forwarded-For` se ignora. Detrás de un proxy
inverso, configura en quién confías con la semántica de `proxy-addr`:

```rust
use corvis::TrustProxy;

app.set_trust_proxy(1);                                          // un salto
app.set_trust_proxy(TrustProxy::parse("loopback, 10.0.0.0/8")?); // direcciones o subredes
app.set_trust_proxy(true);                                       // todos los saltos
```

Con eso, `req.ip()` es la primera dirección de `X-Forwarded-For` (de derecha a izquierda) en la que
no se confía, `req.ips()` devuelve la cadena de confianza y `req.hostname()` usa
`X-Forwarded-Host` solo si se confía en el socket. Afecta también a `#[ip]` y al
[`ThrottlerGuard`](/es/security/rate-limiting).

## Acceso al servidor desde providers

`HttpAdapterHost` es inyectable en cualquier provider y da acceso al servidor, por ejemplo para
registrar listeners del evento `upgrade`. Ver [Adaptador HTTP](/es/faq/http-adapter).

## Rendimiento

El directorio `bench/` del repositorio contiene un benchmark reproducible (`bash bench/run.sh`)
contra una aplicación equivalente de Node.js. Los resultados y el entorno de medición están en
[Corvis frente a Fastify, Express y Hono](/es/faq/corvis-vs-fastify-express-hono). Una aplicación real medida
frente a su original de Node.js está en
[Benchmark: una aplicación migrada frente a su original](/es/faq/port-vs-original-benchmark).
