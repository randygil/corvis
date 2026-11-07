---
status: unavailable
---

# Cabeceras de seguridad

Corvis no trae un middleware que fije cabeceras de seguridad (al estilo del paquete `helmet` de
Node.js). Lo que sí hace por defecto:

- Envía `X-Powered-By: corvis`. Quítala con `app.disable("x-powered-by")`.
- Los cuerpos de respuesta llevan `Content-Type` con `charset` y `Content-Length`.

El resto de cabeceras se añade con un middleware global de pocas líneas.

## Un middleware propio

`app.use_fn` registra un middleware global a partir de un closure. El closure recibe la `Request` y
un `Next`; llama a `next.run(req)` y modifica la respuesta que devuelve:

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

- `set_default_header` solo escribe la cabecera si nadie la fijó antes de enviar la respuesta: un
  `#[header(...)]` en una ruta, un interceptor o un filtro siguen teniendo prioridad. Usa
  `set_header` si quieres sobrescribir siempre.
- Las respuestas de error de una ruta (guards, pipes, handler) llegan al middleware ya convertidas
  en `Response` por los filtros, así que también reciben las cabeceras. Un `404` de una ruta
  inexistente, en cambio, llega como `Err` y lo convierte después el filtro global: con el `?` del
  ejemplo esa respuesta sale sin estas cabeceras.
- Registra este middleware **antes** que `corvis::serve_static(...)` si quieres que también cubra los
  archivos estáticos: los middleware globales se ejecutan en el orden en que se registran.

Para reutilizarlo, conviértelo en un tipo que implemente `CorvisMiddleware` y regístralo con
`app.use_middleware(...)` o con `consumer.apply::<M>()` en un módulo. Ver [Middleware](/es/middleware).

## HSTS y TLS

`Strict-Transport-Security` solo tiene sentido sobre HTTPS, y Corvis no termina TLS (ver
[HTTPS y varios servidores](/es/faq/multiple-servers)). Fíjala en el proxy inverso que termina TLS, o
en el middleware anterior si sabes que toda la aplicación se sirve por HTTPS.
