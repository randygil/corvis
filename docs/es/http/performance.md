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

- El servidor acepta solo HTTP/1.1, como el módulo `http` de Node. `ServerOptions::new().h2c(true)` acepta también
  HTTP/2 sin TLS por conocimiento previo (h2c), para un proxy o plataforma configurado para enviarlo (Caddy `h2c://`,
  Traefik `scheme=h2c`, nginx `grpc_pass`, Cloud Run con un puerto `h2c`). Con h2c activo, una conexión lleva hasta
  100 peticiones concurrentes (`http2_max_concurrent_streams`), `max_header_size` limita también la lista de cabeceras
  de HTTP/2 (`431`) y `max_connections` limita conexiones y no peticiones. Con h2c desactivado, una conexión con conocimiento previo se
  cierra sin respuesta. Consulta [Análisis de HTTP y compatibilidad](/es/security/http-parsing).
- Las peticiones HTTP/1 con encuadre ambiguo (LF sin CR como fin de línea, espacios después de un tamaño de chunk,
  `Content-Length` con `Transfer-Encoding`) se responden con `400` y se cierra la conexión.
  `ServerOptions::new().insecure_http_parser(true)` las vuelve a aceptar para clientes antiguos y registra un aviso al
  arrancar; consulta [Análisis de HTTP y compatibilidad](/es/security/http-parsing).
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
        .max_header_size(16 * 1024)                  // 32 KiB por defecto; responde 431 (HTTP/1 y HTTP/2)
        .http2_max_concurrent_streams(50)            // 100 por defecto, por conexión HTTP/2
        .http2_keep_alive_interval(Duration::from_secs(30)) // sin PING de HTTP/2 por defecto
        .http2_keep_alive_timeout(Duration::from_secs(10)), // 20 s por defecto
);
```

Cada timeout acepta `None` para desactivarlo, por ejemplo `.request_timeout(None)`. Al alcanzar
`max_connections`, las conexiones nuevas esperan en la cola de `accept`. Las conexiones WebSocket abiertas cuentan
hasta que se cierran, y también un socket durante hasta 2 s después de una respuesta de error temprana (`413`, `431`,
...), mientras el servidor deja que el cliente la lea antes de cerrar. Con un tope, agrega un heartbeat a tus gateways
para que los clientes caídos no ocupen lugares.

### Conexiones HTTP/2

Estas opciones aplican a toda conexión HTTP/2 (hoy h2c) y nunca a las de HTTP/1.

- `max_header_size` limita la lista de cabeceras de HTTP/2, contada como en el RFC 9113 (longitud del nombre y del
  valor más 32 bytes por campo), de modo que el mismo número es comparable con HTTP/1, no idéntico byte a byte. Una
  petición que lo supera se responde `431` en su stream y los demás streams siguen funcionando.
- `http2_max_concurrent_streams` (100 por defecto, mínimo 1) es el número de streams que un cliente puede tener
  abiertos a la vez en una conexión.
- `max_connections` cuenta conexiones, no streams, como `server.maxConnections` de Node. Las peticiones en curso de
  clientes HTTP/2 quedan acotadas por `max_connections` x `http2_max_concurrent_streams`.
- `headers_timeout` y `keep_alive_timeout` aplican a la conexión mientras ninguno de sus streams está en curso, así que
  un stream largo mantiene viva la conexión. `request_timeout` aplica al cuerpo de cada stream por separado: un `POST`
  detenido se responde `408` sin afectar a los demás streams.
- `http2_keep_alive_interval` (desactivado por defecto) envía un PING cada intervalo, y `http2_keep_alive_timeout`
  cierra la conexión cuando un PING no se confirma a tiempo (mínimo 1 s; un intervalo cero significa sin PING).
  Actívalo con SSE u otros streams largos sobre HTTP/2, donde un cliente que dejó de responder de otro modo nunca se
  detectaría.
- Un cliente que reinicia streams y abre otros nuevos más rápido de lo que sus handlers se detienen nunca consigue más
  handlers a la vez que `http2_max_concurrent_streams`, y una conexión que cancela más de 500 peticiones antes de su
  respuesta en 10 s se cierra (rapid reset).
- Limitación conocida ([#157](https://github.com/randygil/corvis/issues/157)): un cliente vivo que deja de leer retiene
  su conexión. Mientras la escritura hacia el cliente está bloqueada, ni el timeout del PING ni los de inactividad se
  activan; el keep-alive con PING detecta clientes que dejaron de responder, no que dejaron de leer.

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
