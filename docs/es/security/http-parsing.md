---
status: documented
---

# Análisis de HTTP y compatibilidad

Corvis no tiene un modo global "compatible" o "estricto". Cada comportamiento relevante para la seguridad tiene su
propio tratamiento, con la opción segura como valor por defecto. Esta página lista lo que Corvis acepta y rechaza en el
protocolo y en los cuerpos multipart, cómo se compara con el módulo `http` de Node y qué está previsto cambiar.

El formato HTTP/1 lo analiza hyper (y `httparse` para la cabecera de la petición), no Corvis. Las cifras de Node de
abajo salen de una sonda con sockets crudos contra `http.createServer` en Node 24.13 (`llhttp` 9.3.0).

## Qué hace Corvis

| Entrada | Corvis | `http` de Node | Estado |
|---|---|---|---|
| Línea de petición o cabeceras terminadas en LF sin CR | se acepta | `400` | se rechazará |
| Bloque de cabeceras terminado en `CRLF LF` | se acepta | `400` | se rechazará |
| CR suelto dentro de una cabecera | se rechaza | `400` | se mantiene |
| Tamaño de chunk con ceros a la izquierda (`0002`) | se acepta | se acepta | se mantiene |
| Tamaño de chunk que no cabe en 64 bits | se rechaza | `400` | se mantiene |
| Espacio después del tamaño de chunk (`2 \r\n`) | se acepta | `400` | se rechazará |
| LF sin CR en el encuadre chunked (`2\nhi\n0\n\n`) | se rechaza | `400` | se mantiene |
| Extensión de chunk con LF | se rechaza | `400` | se mantiene |
| `Content-Length` y `Transfer-Encoding: chunked` juntos | se procesa como chunked y se cierra la conexión después | `400` | se rechazará |
| `field_nesting_depth` superado en multipart | `400 Field name nesting too deep - <field>` | n/a | cambiado |
| Errores de configuración de busboy (`Malformed content type`, `Unsupported content type: ...`) | `500` | `500` | se mantiene por paridad |

"Se mantiene" significa que el comportamiento está cubierto por un test de regresión. "Se rechazará" significa que un
cambio posterior de la épica Foundations (#1) lo rechazará por defecto y añadirá una opción para desactivarlo llamada
`insecure_http_parser`, por la opción `insecureHTTPParser` de Node. Esa opción todavía no existe.

## Hasta que llegue el analizador más estricto

Corvis acepta un LF solo como fin de línea en la cabecera de la petición. El RFC 9112 permite al receptor hacerlo, pero
es un riesgo si un proxy delante de Corvis lee los mismos bytes de otra forma. Si Corvis está detrás de un proxy o un
balanceador, deja que el proxy normalice las peticiones (la mayoría lo hace y rechaza el LF solo por su cuenta). No se
encontró contrabando de peticiones con los proxies habituales.

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
- HTTP/2 sin TLS (h2c) está activo por defecto, y el módulo `http` de Node no lo habla. Si debe seguir activo por
  defecto se decide en el issue #144; hoy `ServerOptions::new().h2c(false)` lo desactiva. Consulta
  [Rendimiento](/es/http/performance).
- La cabecera de la petición está limitada a 32 KiB por defecto (Node: 16 KiB); `ServerOptions` tiene
  `max_header_size`.
