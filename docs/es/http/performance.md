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
        .send_timeout(Duration::from_secs(30))       // 60 s por defecto; cierra la conexión
        .max_connections(10_000)                     // sin tope por defecto
        .max_pending_response_bytes(4 * 1024 * 1024) // 8 MiB por defecto, por conexión; ver abajo
        .max_pending_response_wait(Duration::from_secs(2)) // 5 s por defecto; ver abajo
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

### Clientes que dejan de leer (`send_timeout`)

`send_timeout` (60 s por defecto, `None` lo desactiva) es lo máximo que el servidor espera a que un cliente acepte
bytes de la respuesta. Un cliente que dejara de leer retendría para siempre su conexión, su lugar de
`max_connections` y la memoria de sus respuestas, porque ningún otro timer corre mientras hay una respuesta en curso.
Funciona igual en HTTP/1 y en HTTP/2, de dos maneras:

- **Socket bloqueado.** Si la escritura al socket de la conexión sigue bloqueada durante `send_timeout` sin que el
  socket acepte un solo byte, la conexión se cierra. Así se detecta también a un cliente HTTP/2 que inunda de PING y
  nunca lee las respuestas, y a uno HTTP/1 que encadena peticiones y nunca lee las respuestas.
- **Ventanas de control de flujo cerradas.** Cuando las respuestas de una conexión HTTP/2 esperan a que el cliente
  abra sus ventanas y ninguna se ha movido durante `send_timeout`, la conexión se cierra, aunque el cliente siga
  contestando PING o enviando SETTINGS y WINDOW_UPDATE. Un stream detenido junto a otros que avanzan no cierra la
  conexión: en un enlace lento con muchos streams, siempre hay algunos esperando ventana.

La conexión se descarta de inmediato: sin GOAWAY, sin el resto de la respuesta y sin el drenaje de cierre.

Mide la falta de progreso, no la duración de la respuesta: el progreso lo reinicia, así que una descarga lenta no se
corta. Nunca corre mientras el servidor no tiene nada que enviar: un handler lento, un stream de Server-Sent Events
inactivo y una conexión keep-alive ociosa no se ven afectados, ni tampoco una conexión WebSocket ya establecida (su
cola de envío está acotada por `max_buffered_amount`). Un valor demasiado largo para sumarlo a una lectura del reloj
(`Duration::MAX`) significa sin límite.

**Lo que no acota.** No limita lo poco que un cliente tiene que leer para conservar su conexión:

- En HTTP/2 el cliente tiene que tomar unos 16 KiB (un frame) de algún stream cada `send_timeout`.
- En HTTP/1 el servidor nota el progreso cuando el sistema operativo avisa que el socket admite escritura. En Linux
  configura `TCP_NOTSENT_LOWAT` a 64 KiB en sus sockets mientras `send_timeout` está activo, así que un cliente que lee
  de forma constante se ve cuando toma unos 32 KiB por periodo (sin eso, Linux avisa solo cuando está libre cerca de la
  mitad de un buffer de envío que el autoajuste hace crecer a varios MiB, y un lector lento se cortaría mientras lee).
  En Windows hicieron falta unos 64 KiB por periodo en una conexión de loopback. En macOS y los BSD la opción no se
  configura y la granularidad es la del buffer autoajustado, que puede ser de varios MiB: usa un `send_timeout` mayor
  si sirves clientes lentos ahí.
- Un cliente que reinicia streams y abre otros nuevos no necesita leer nada: cada respuesta nueva cuenta como
  movimiento. Los límites de rapid reset acotan cuánto puede durar eso.

La memoria que un cliente retiene mientras hace eso la acota
[`max_pending_response_bytes`](#memoria-de-respuestas-por-conexion-max-pending-response-bytes).

La opción más cercana de Node es `server.timeout`: está desactivada por defecto (0) y mide la inactividad en ambas
direcciones, por lo que también corta handlers lentos, y por eso Node la desactivó. Con los valores por defecto de
Node, un cliente que deja de leer nunca se desconecta; Corvis lo cierra a los 60 s (el `send_timeout` de nginx tiene el
mismo valor y la misma semántica). Un reproductor que pausa una descarga progresiva más de un minuto se reconecta (los
navegadores reanudan con `Range`); sube el valor o pasa `None` si necesitas el comportamiento anterior.

### Memoria de respuestas por conexión (`max_pending_response_bytes`)

Un cliente que no lee sus respuestas las mantiene en memoria. `max_pending_response_bytes` (8 MiB por conexión por
defecto, `None` lo desactiva) acota eso, lea lo que lea el cliente: uno que nunca concede ventana HTTP/2, uno que
concede un goteo, uno que resetea y reabre streams.

- **Qué cuenta.** El tamaño completo de una respuesta en memoria (JSON, texto, bytes; también cuando la comprime, que
  cuenta el tamaño sin comprimir) desde que su handler devuelve hasta que su cuerpo termina o se descarta, y, para los
  cuerpos en streaming (archivos, Server-Sent Events), la parte del frame de datos actual que aún no se entregó a la
  capa de protocolo. En HTTP/1 una respuesta cuenta hasta que el servidor la escribió en el socket.
- **Admisión antes del handler.** Cada petición reserva una parte del límite (el límite dividido entre las peticiones
  que la conexión puede ejecutar a la vez: `http2_max_concurrent_streams` en HTTP/2, 1 en HTTP/1, o sea 83.886 bytes
  por handler con los valores por defecto). Su handler arranca solo cuando los bytes pendientes más las reservas dejan
  sitio, o cuando la conexión no retiene nada. Hasta entonces la petición espera sin llegar a un handler; ocupa su
  hueco de `http2_max_concurrent_streams`, y un cliente que la resetea mientras tanto cuenta como si hubiera cancelado
  una petición. La espera está acotada por `max_pending_response_wait` (ver abajo).
- **Por encima del límite, HTTP/2 resetea.** Cuando handlers que corrieron juntos devuelven respuestas que llevarían a
  una conexión que ya retiene bytes pendientes por encima del límite, el servidor descarta las respuestas sobrantes y
  resetea sus streams con `ENHANCE_YOUR_CALM`. El primer reset de una conexión se registra como advertencia. Una
  respuesta que cabe en su parte nunca se resetea, y una respuesta siempre se envía, sea cual sea su tamaño. Esto
  también le pasa a un cliente que lee a toda velocidad: cuando muchas respuestas grandes están listas en los mismos
  instantes (una ráfaga de handlers lentos que devuelven juntos), las que superan el límite se resetean aunque nadie
  esté detenido. Dimensiona el límite para lo que esperas tener en vuelo, al menos `http2_max_concurrent_streams`
  veces tu respuesta habitual más grande, y baja `http2_max_concurrent_streams` si tu API devuelve respuestas de varios
  megabytes; un cliente que recibe `ENHANCE_YOUR_CALM` puede reintentar.
- **HTTP/1.** Las peticiones en pipeline corren un handler a la vez: el siguiente espera a que salga la respuesta
  anterior.

La cota de una conexión es el mayor entre el límite y su respuesta más grande, más unos 64 KiB por stream HTTP/2
abierto que hyper aún retiene después de que un cuerpo terminó (los últimos fragmentos de cada respuesta se copian,
así que nunca mantienen viva la respuesta entera; el buffer de envío por stream de hyper es de 32 KiB), más los
codificadores de compresión (`max_per_connection`). No incluye las cabeceras y cuerpos de las peticiones, ni la
memoria que la aplicación guarda fuera de la respuesta: un canal sin límite que alimenta un stream, una caché
repartida en respuestas. En todo el servidor la cota es `max_connections` x el límite (`max_connections` no tiene valor
por defecto: define uno en un servidor expuesto a internet).

**La espera tiene un límite (`max_pending_response_wait`, 5 s por defecto, `None` espera sin límite).** Una respuesta en
memoria cuenta entera hasta que se envía, así que un cliente que descarga despacio una respuesta muy grande retrasa
las demás peticiones de su conexión mientras tanto. Detrás de un proxy que multiplexa varios usuarios en una misma
conexión HTTP/2 (Envoy, HAProxy con backends HTTP/2), eso dejaría que un usuario lento retuviera para siempre las
peticiones de los demás. Una petición que espera más se rechaza: en HTTP/2 su stream se resetea con `REFUSED_STREAM`
(el handler nunca corrió, así que el proxy o el cliente pueden reintentarla en otra conexión, RFC 9113 sección 8.7) y no
cuenta como petición cancelada para la regla de rapid reset; en HTTP/1 se cierra la conexión. El primer rechazo de
una conexión se registra. Con `None`, un cliente que lee un goteo cada `send_timeout` puede mantener esperando a los
demás streams de su conexión todo lo que quiera: úsalo solo si todos los clientes son de confianza, o envía los
cuerpos grandes en streaming.

Los payloads grandes pertenecen a cuerpos en streaming (los archivos estáticos lo son): un cuerpo en streaming solo
cuenta el frame que retiene. Una ráfaga de respuestas en memoria mayores que su parte puede perder streams por
`ENHANCE_YOUR_CALM`; sube el límite si tu API devuelve varios megabytes por respuesta a muchos streams concurrentes.

El módulo `http2` de Node tiene la misma idea en `maxSessionMemory` (10 MB): cuenta los datos salientes en cola y
rechaza streams nuevos mientras se supera. El módulo `http` de Node no tiene límite, y allí un handler HTTP/1 en
pipeline corre concurrentemente.

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
  un stream largo mantiene viva la conexión. Una conexión ociosa sigue en la fase de keep-alive cuando el cliente envía
  un frame que no inicia ninguna petición (WINDOW_UPDATE, PING, SETTINGS): solo una petición nueva la pasa a la fase de
  cabeceras. `request_timeout` aplica al cuerpo de cada stream por separado: un `POST`
  detenido se responde `408` sin afectar a los demás streams.
- `http2_keep_alive_interval` (desactivado por defecto) envía un PING cada intervalo, y `http2_keep_alive_timeout`
  cierra la conexión cuando un PING no se confirma a tiempo (mínimo 1 s; un intervalo cero significa sin PING).
  Actívalo con SSE u otros streams largos sobre HTTP/2, donde un cliente que dejó de responder de otro modo nunca se
  detectaría.
- Un cliente que reinicia streams y abre otros nuevos más rápido de lo que sus handlers se detienen nunca consigue más
  handlers a la vez que `http2_max_concurrent_streams` (rapid reset). Una conexión también se cierra cuando las
  peticiones que canceló antes de su respuesta (un stream reiniciado, también mientras se leía su cuerpo) superan 500 y
  superan la cuarta parte de las que completó, en los últimos 10 s, o superan `http2_max_cancelled_streams` (2000 por
  defecto, unas 200 por segundo sostenidas) sin importar cuántas completó. Un proxy que multiplexa muchos usuarios
  (Envoy, Cloud Run) y ve un pequeño porcentaje de cancelaciones nunca se corta: tolera cerca de un 20% de
  cancelaciones, y 5.000 peticiones/s con 2% canceladas queda muy por debajo del tope. Un atacante paga cuatro
  peticiones completadas por cada reinicio y queda limitado por el tope. El servidor envía antes un GOAWAY ordenado,
  para que el proxy pueda reintentar en otra conexión las peticiones sin respuesta; desde entonces los streams nuevos
  se reinician sin llegar a un handler, y la conexión se cierra tras 1 s. Sube `http2_max_cancelled_streams` si algún
  proxy tuyo cancela más que eso. Una petición cuenta como completada solo cuando termina el cuerpo de su respuesta: un
  stream reseteado después de que su handler devolvió, con el cuerpo aún en envío (o antes de su último frame DATA,
  posiblemente vacío), cuenta como cancelado, y también una
  respuesta que el servidor descarta por superar `max_pending_response_bytes`. Una petición que el cliente cancela no se
  registra.
- El keep-alive con PING detecta clientes que dejaron de responder, no que dejaron de leer; de esos se ocupa
  `send_timeout` (ver arriba).

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
