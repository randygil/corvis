---
status: unavailable
---

# Compression

Corvis does not compress responses (there is no equivalent to the `compression` middleware with gzip,
deflate or brotli) and it does not decompress requests either: a body with
`Content-Encoding: gzip`, `deflate` or `br` is rejected with `415 unsupported content encoding`.

The recommended alternative is to compress in the reverse proxy in front of the application
(nginx, Caddy, Traefik, a load balancer or a CDN). There, compression does not use CPU from the threads that
serve requests, and you configure it once for all services. With nginx, for example:

```nginx
gzip on;
gzip_types application/json text/plain text/css application/javascript;
gzip_min_length 1024;
```

If you compress in the proxy, keep in mind that the `ETag` values Corvis generates are weak (`W/"..."`),
so they remain valid for the compressed version. Do not compress
[Server-Sent Events](/http/server-sent-events) responses: the proxy could accumulate events in its buffer.
