---
status: documented
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

Detalles del parser:

- `application/json` y `application/x-www-form-urlencoded` se parsean con o sin parámetros como `charset`. Una petición sin cuerpo deja `#[body]` en `null`.
- El JSON es estricto (solo `{` o `[` al nivel superior), un cuerpo vacío es `{}`, el BOM se ignora y los errores de sintaxis usan los mismos mensajes que `JSON.parse` de V8 (`Expected property name or '}' in JSON at position 1 (line 1 column 2)`, `Unexpected token '"', ""x"" is not valid JSON`...). La respuesta es `400 { "message": ..., "error": "Bad Request", "statusCode": 400 }`.
- A diferencia de Express, la profundidad de anidamiento del JSON está limitada a 256 niveles (`400 Maximum nesting depth of 256 exceeded in JSON at position 256 (line 1 column 257)`). El `serde_json::Value` recursivo desbordaría la pila con unos pocos kB de `[[[...]]]`. `app.set_body_parser_max_depth(n)` lo cambia y también acota la profundidad de los cuerpos urlencoded. `corvis::http::json_parse::parse` aplica el mismo límite; para otro valor usa `parse_utf16_with_depth`.
- urlencoded usa `qs` con las opciones de `body-parser` (objetos y arrays anidados). Más de 32 niveles responde `400 The input exceeded the depth` y más de 1000 parámetros, `413 too many parameters`. Los escapes `%XX` exigen dos dígitos hexadecimales (`%+A` no es válido).
- El límite de 100 kb responde `413 { "statusCode": 413, "message": "request entity too large" }`.
- El cuerpo se lee solo hasta el límite del parser que lo va a procesar: un JSON o urlencoded que anuncia un `Content-Length` mayor responde `413` sin leerlo. Los demás tipos se leen hasta el límite de transporte para que `req.raw_body()` siga disponible.
- Un cuerpo que no se puede leer (el cliente corta, *chunked* mal formado) responde `400 request aborted` y uno que no llega a tiempo, `408 Request Timeout`.
- Un charset no soportado responde `415 unsupported charset "LATIN1"`. `Content-Type` y `Content-Encoding` se leen como latin1, igual que Node.
- Los errores del parser pasan por CORS y por los filtros de excepciones globales, como un `next(err)` de Express.

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
