---
status: documentado
---

# Cuerpo sin procesar

Algunos casos necesitan los bytes exactos que envió el cliente, no el JSON ya parseado: el ejemplo
típico es verificar la firma HMAC de un webhook. En Corvis no hay que activar nada: cada `Request`
conserva su cuerpo original, y `req.raw_body()` lo devuelve como `Bytes`.

```rust
#[post("payments")]
#[http_code(200)]
fn payments(&self, #[req] req: Request, #[body] event: Value) -> HttpResult<Value> {
    let raw: &[u8] = req.raw_body();
    // `verifier` es un provider tuyo que comprueba la firma con los bytes originales.
    self.verifier.check(req.header("x-signature"), raw)?;
    Ok(json!({ "received": event["id"] }))
}
```

`#[body]` y `req.raw_body()` pueden usarse a la vez: el primero recibe el valor parseado y el segundo
los bytes originales. También está disponible en guards, interceptors y middleware a través de la
request (`ctx.request().raw_body()`).

Ver [Webhooks](/es/http/webhooks) para un guard completo de verificación de firma.

## Qué se parsea y qué no

| `Content-Type` | `#[body]` / `req.body()` | `req.raw_body()` |
|---|---|---|
| `application/json` | JSON parseado (estricto: solo objetos o arrays arriba) | Bytes originales |
| `application/x-www-form-urlencoded` | Objeto, con la sintaxis de `qs` (`a[b]=1`) | Bytes originales |
| Cualquier otro (`text/plain`, `application/xml`...) | `null` | Bytes originales |
| `multipart/*` | Campos de texto, después de un interceptor de archivos | Vacío (ver abajo) |

Límites:

- JSON y urlencoded: 100 kb cada uno por defecto, con `413 request entity too large` al superarlos.
  Se cambian con `app.use_body_parser("json", bytes)` y `app.use_body_parser("urlencoded", bytes)`.
- Otros tipos se leen hasta el límite de transporte, 1 MiB por defecto (`app.set_body_limit(bytes)`).

`app.disable_body_parser()` desactiva el parseo de JSON y urlencoded: `#[body]` recibe `null` y
`req.raw_body()` sigue teniendo los bytes. Útil si quieres parsear tú mismo con `serde_json`.

## Cuerpos multipart

Los cuerpos `multipart/*` no los lee el servidor: los lee, en *streaming*, el interceptor de archivos
de la ruta. En esas peticiones `req.raw_body()` está vacío. Si quieres procesar el multipart por tu
cuenta, léelo con `corvis::multer::read_body(&req, limite).await?`. Ver
[Subida de archivos](/es/http/file-upload).

## Cuerpos comprimidos

Un cuerpo con `Content-Encoding` distinto de `identity` se rechaza con
`415 unsupported content encoding` (Express los descomprime). Ver [Compresión](/es/http/compression).
