---
status: unavailable
---

# Security headers

Corvis does not ship a middleware that sets security headers (in the style of Node.js's `helmet`
package). What it does by default:

- Sends `X-Powered-By: corvis`. Remove it with `app.disable("x-powered-by")`.
- Response bodies carry `Content-Type` with `charset` and `Content-Length`.

The remaining headers are added with a short global middleware.

## A middleware of your own

`app.use_fn` registers a global middleware from a closure. The closure receives the `Request` and
a `Next`; it calls `next.run(req)` and modifies the response it returns:

```rust
use corvis::prelude::*;

#[corvis::main]
async fn main() -> corvis::Result<()> {
    let mut app = CorvisFactory::create::<AppModule>().await?;
    app.disable("x-powered-by");
    app.use_fn(|req, next| async move {
        let mut res = next.run(req).await?;
        res.set_default_header("X-Content-Type-Options", "nosniff");
        res.set_default_header("X-Frame-Options", "SAMEORIGIN");
        res.set_default_header("Referrer-Policy", "no-referrer");
        res.set_default_header("Cross-Origin-Opener-Policy", "same-origin");
        res.set_default_header("Content-Security-Policy", "default-src 'self'");
        Ok(res)
    });
    app.listen(3000).await
}
```

- `set_default_header` only writes the header if nobody set it before the response is sent: a
  `#[header(...)]` on a route, an interceptor or a filter still take priority. Use
  `set_header` if you want to always overwrite.
- Error responses from a route (guards, pipes, handler) reach the middleware already converted
  into a `Response` by the filters, so they also receive the headers. A `404` for a nonexistent
  route, on the other hand, arrives as an `Err` and is converted afterwards by the global filter: with the `?` in the
  example that response goes out without these headers.
- Register this middleware **before** `corvis::serve_static(...)` if you want it to also cover
  static files: global middleware runs in the order in which it is registered.

To reuse it, turn it into a type that implements `CorvisMiddleware` and register it with
`app.use_middleware(...)` or with `consumer.apply::<M>()` in a module. See [Middleware](/middleware).

## HSTS and TLS

`Strict-Transport-Security` only makes sense over HTTPS, and Corvis does not terminate TLS (see
[HTTPS and multiple servers](/faq/multiple-servers)). Set it in the reverse proxy that terminates TLS, or
in the middleware above if you know the whole application is served over HTTPS.
