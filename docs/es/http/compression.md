---
status: unavailable
---

# Compresión

Corvis no comprime las respuestas (no hay un equivalente al middleware `compression` con gzip,
deflate o brotli) y tampoco descomprime las peticiones: un cuerpo con
`Content-Encoding: gzip`, `deflate` o `br` se rechaza con `415 unsupported content encoding`.

La alternativa recomendada es comprimir en el proxy inverso que tienes delante de la aplicación
(nginx, Caddy, Traefik, un balanceador o una CDN). Allí la compresión no consume CPU de los hilos que
atienden peticiones y se configura una sola vez para todos los servicios. Con nginx, por ejemplo:

```nginx
gzip on;
gzip_types application/json text/plain text/css application/javascript;
gzip_min_length 1024;
```

Si comprimes en el proxy, ten en cuenta que los `ETag` que genera Corvis son débiles (`W/"..."`),
así que siguen siendo válidos para la versión comprimida. No comprimas las respuestas
[Server-Sent Events](/es/http/server-sent-events): el proxy podría acumular eventos en su búfer.
