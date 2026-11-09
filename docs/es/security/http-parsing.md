---
status: documented
---

# Análisis de HTTP y compatibilidad

Corvis no tiene un modo global "compatible" o "estricto". Cada comportamiento relevante para la seguridad tiene su
propio tratamiento, con la opción segura como valor por defecto. Esta página lista lo que Corvis acepta y rechaza en el
protocolo y en los cuerpos multipart, cómo se compara con el módulo `http` de Node y cómo desactivar las comprobaciones más estrictas.

El formato HTTP/1 lo analiza hyper (y `httparse` para la cabecera de la petición). hyper y `httparse` no tienen ninguna
opción para rechazar las entradas ambiguas de abajo, así que Corvis pone una pequeña comprobación de encuadre delante de
hyper en cada conexión (ver [Cómo funciona la comprobación](#como-funciona-la-comprobacion)). Las cifras de Node de
abajo salen de una sonda con sockets crudos contra `http.createServer` en Node 24.13 (`llhttp` 9.3.0).

## Qué hace Corvis

| Entrada | Corvis | `http` de Node | Estado |
|---|---|---|---|
| Línea de petición o cabeceras terminadas en LF sin CR | `400`, conexión cerrada | `400` | se rechaza (`insecure_http_parser` lo acepta) |
| Bloque de cabeceras terminado en `CRLF LF` (o última cabecera terminada en `LF CRLF`) | `400`, conexión cerrada | `400` | se rechaza (`insecure_http_parser` lo acepta) |
| Línea vacía (CRLF o LF solo) antes de la línea de petición | se ignora | se ignora | se mantiene |
| CR suelto dentro de una cabecera | se rechaza | `400` | se mantiene |
| Plegado de línea obsoleto en una cabecera | se rechaza | `400` | se mantiene |
| Tamaño de chunk con ceros a la izquierda (`0002`) | se acepta | se acepta | se mantiene |
| Tamaño de chunk que no cabe en 64 bits | se rechaza | `400` | se mantiene |
| Espacio después del tamaño de chunk (`2 \r\n`, `2\t\r\n`, `2 ;a=b\r\n`) | se rechaza | `400` | se rechaza (`insecure_http_parser` lo acepta) |
| LF sin CR en el encuadre chunked (`2\nhi\n0\n\n`) | se rechaza | `400` | se mantiene |
| Extensión de chunk con LF | se rechaza | `400` | se mantiene |
| LF sin CR dentro de la sección de tráilers de un cuerpo chunked (`0\r\nX-A: b\n\nGARBAGE\r\n\r\n`) | se rechaza | `400` | se rechaza (`insecure_http_parser` lo acepta) |
| `Content-Length` y `Transfer-Encoding` juntos, en cualquier orden | `400`, conexión cerrada | `400` | se rechaza (`insecure_http_parser` lo procesa como chunked y cierra la conexión después) |
| Dos cabeceras `Content-Length` idénticas | se acepta | `400` | se acepta: una sola lectura posible, así que no es ambiguo; difiere de Node |
| `Transfer-Encoding: chunked, chunked` | se acepta | `400` | se acepta: una sola lectura posible, así que no es ambiguo; difiere de Node |
| Petición `HTTP/1.0` con `Transfer-Encoding` | `400` | se acepta | se mantiene (más estricto que Node) |
| `field_nesting_depth` superado en multipart | `400 Field name nesting too deep - <field>` | n/a | cambiado |
| Errores de configuración de busboy (`Malformed content type`, `Unsupported content type: ...`) | `500` | `500` | se mantiene por paridad |

Cada fila está cubierta por un test de regresión. Las filas que nombran `insecure_http_parser` cambiaron en el issue
#146 (épica Foundations #1): antes, Corvis aceptaba esas entradas. Una petición rechazada recibe un `400` y la conexión
se cierra, y nada de lo que llegue después por esa conexión se procesa, así que un proxy que leyera los mismos bytes de
otra forma no puede colar una segunda petición por Corvis.

Los clientes no se ven afectados salvo que escriban las peticiones a mano: el `http` de Node, los navegadores, curl,
reqwest y los proxies terminan siempre las líneas con CRLF, nunca ponen espacios después de un tamaño de chunk y nunca
envían las dos cabeceras de longitud.

## Cómo funciona la comprobación

Una pequeña máquina de estados lee cada conexión antes que hyper y sigue dónde empieza y termina cada petición: la
cabecera, luego el cuerpo por `Content-Length` o por chunks (tamaños, extensiones, tráilers) y luego la cabecera
siguiente. Rechaza las cuatro entradas de arriba y pasa todo lo demás a hyper sin cambios. La sintaxis de las
cabeceras, los límites y los códigos de estado siguen siendo de hyper. Las entradas que no entiende del todo (un
`Transfer-Encoding` que no termina en `chunked`, valores de `Content-Length` distintos o inválidos, `HTTP/1.0` con
`Transfer-Encoding`, una versión HTTP desconocida, una cabecera por encima del límite de tamaño) se entregan a hyper
tal cual y no se lee nada después de esa petición, así que hyper las responde como antes. Después de una petición con
cabecera `Upgrade`, los bytes siguientes se retienen hasta que la petición se responde, porque tras un `101` ya no son
HTTP. Un test de propiedades pasa flujos encadenados generados y mutados por la comprobación y por hyper y falla si no
coinciden en dónde termina una petición.

## Desactivar las comprobaciones

`ServerOptions::new().insecure_http_parser(true)` desactiva las cuatro comprobaciones y solo esas: LF sin CR como fin de
línea en la cabecera y en los tráilers, espacios después de un tamaño de chunk y `Content-Length` junto con
`Transfer-Encoding`. Se llama como la opción `insecureHTTPParser` de Node y es más estrecha: lo que el propio hyper
rechaza (un CR suelto, un tamaño de chunk de más de 64 bits, un LF sin CR en el encuadre de chunks, un LF en una
extensión de chunk, el plegado de línea obsoleto, un `Transfer-Encoding` inválido) sigue rechazado, y h2c, los timeouts
y los límites de tamaño no cambian. Corvis registra un aviso cuando el servidor empieza a escuchar con ella activa.

```rust
use corvis::ServerOptions;

app.set_server_options(ServerOptions::new().insecure_http_parser(true));
```

Úsala solo para clientes antiguos que no se pueden corregir (dispositivos que envían LF sin CR, por ejemplo). Con ella
activa, un LF solo vuelve a ser un fin de línea, lo que es un riesgo si un proxy delante de Corvis lee los mismos bytes
de otra forma: deja que el proxy normalice las peticiones (la mayoría lo hace y rechaza el LF solo por su cuenta).

## Clientes lentos

Los clientes lentos a propósito se cubren con un timer por dirección. Los que leen despacio la petición (slowloris: una
cabecera o un cuerpo que no termina) los cubren `headers_timeout`, `request_timeout` y `keep_alive_timeout`. Los que
leen despacio la respuesta, un cliente que deja de leer y conserva su conexión, su lugar de `max_connections` y la
memoria de sus respuestas, los cubre `send_timeout` (60 s por defecto, en HTTP/1 y HTTP/2, incluidos streams HTTP/2
cuyas ventanas de control de flujo el cliente nunca abre). Acota cuánto puede pasar una conexión sin leer nada, no lo
poco que debe leer. La memoria que se retiene para un cliente así está acotada por conexión por
`max_pending_response_bytes` (8 MiB por defecto): los handlers esperan sitio antes de correr, y las respuestas por
encima del límite se descartan y sus streams HTTP/2 se resetean con `ENHANCE_YOUR_CALM`; los streams que un cliente
resetea cuando su respuesta ya empezó cuentan contra el límite de rapid reset. En todo el servidor la cota es
`max_connections` por ese límite. Ver [Clientes que dejan de leer](/es/http/performance#clientes-que-dejan-de-leer-send-timeout)
y [Memoria de respuestas por conexión](/es/http/performance#memoria-de-respuestas-por-conexion-max-pending-response-bytes). El `server.timeout` de Node
está desactivado por defecto, así que allí un cliente que deja de leer nunca se desconecta.

## Límites de multipart

`limits.field_nesting_depth(n)` rechaza el nombre de un campo de texto con más de `n` corchetes de apertura. La petición
se responde con un error de cliente:

```json
{ "message": "Field name nesting too deep - a[b][c][d]", "error": "Bad Request", "statusCode": 400 }
```

Sin ese límite, un nombre anidado a más de 32 niveles se guarda como clave literal en lugar de rechazarse. Consulta
[Subida de archivos](/es/http/file-upload).

Un tipo de contenido multipart mal formado o no soportado (`multipart/mixed`, por ejemplo) responde `500` con el
cuerpo genérico `Internal server error`, como la aplicación Node original. No se filtra nada, pero un cliente puede
provocar un 5xx a propósito; tenlo en cuenta en las alertas de tasa de errores. Se revisará con el trabajo sobre la
postura de las rutas.

## Otros valores por defecto que conviene conocer

- Cookies: `CookieOptions::default()` no pone `HttpOnly`, `Secure` ni `SameSite`, igual que `res.cookie` en Express.
  Indícalos en cada llamada.
- CORS: un origen dado como expresión regular no está anclado, como `regex.test` de JavaScript, y `reflect_origin()`
  con `credentials(true)` refleja cualquier origen. Consulta [CORS](/es/security/cors).
- HTTP/2 sobre TLS se negocia con ALPN (`h2`) en los listeners iniciados con `listen_tls`; la comprobación de framing de
  HTTP/1 se aplica a HTTP/1.1 sobre TLS igual que a las conexiones planas, y una conexión TLS que no negoció `h2` rechaza
  el preface de HTTP/2 diga lo que diga `h2c`. Vale también al revés: una conexión que negoció `h2` se sirve solo como
  HTTP/2 y debe empezar con el preface de HTTP/2, así que no sirve para enviar peticiones HTTP/1 (ni un upgrade de
  WebSocket HTTP/1) esquivando la comprobación de framing; se cierra sin respuesta.
- HTTP/2 sin TLS (h2c) está desactivado por defecto, como en el módulo `http` de Node, que no lo habla.
  `ServerOptions::new().h2c(true)` lo activa para un proxy o plataforma configurado para enviarlo; los límites de
  HTTP/2 siguen a `ServerOptions`: como máximo 100 streams concurrentes por conexión (`http2_max_concurrent_streams`),
  keep-alive con PING mediante `http2_keep_alive_interval` (desactivado por defecto) y `max_connections` cuenta
  conexiones, no streams. Consulta [Rendimiento](/es/http/performance).
- La cabecera de la petición está limitada a 32 KiB por defecto (Node: 16 KiB); `ServerOptions` tiene
  `max_header_size`, que limita también la lista de cabeceras de HTTP/2. Una lista de cabeceras HTTP/2 demasiado
  grande se responde `431` en su stream.
