---
status: documentado
---

# Cookies

Las cookies que envía el cliente se leen de la `Request`, sin registrar ningún middleware, y las que
envía el servidor se escriben en la respuesta. El parseo sigue al paquete `cookie-parser` y la
escritura a `res.cookie` de Express (paquete `cookie` 0.7).

## Leer cookies

`req.cookies()` devuelve un `HashMap` con todas las cookies y `req.cookie("nombre")` una sola:

```rust
#[get("me")]
fn me(&self, #[req] req: Request) -> HttpResult<Value> {
    let sid = req.cookie("sid").ok_or_else(|| UnauthorizedException::new("No session"))?;
    Ok(json!({ "sid": sid }))
}
```

Desde un guard, un interceptor o un middleware se leen igual, a partir de la request
(`ctx.request().cookies()`).

Reglas del parseo:

- Si un nombre se repite, gana el primero.
- Se quitan las comillas dobles que envuelven el valor.
- Solo se decodifican los valores que contienen `%`; si el escape no es válido se conserva el valor
  original.

No hay cookies firmadas (`signedCookies`): si necesitas que el cliente no pueda alterar el valor,
guarda un JWT firmado con `JwtService` o un identificador opaco.

## Escribir cookies con `#[res(passthrough)]`

El parámetro `#[res(passthrough)] res: Res` da acceso al objeto de respuesta sin quitarle a Corvis la
serialización del valor de retorno:

```rust
use corvis::{CookieOptions, Res};

#[post("login")]
#[http_code(204)]
async fn login(&self, #[body] dto: LoginDto, #[res(passthrough)] res: Res) -> HttpResult<()> {
    let sid = self.auth.create(&dto).await?;
    res.cookie(
        "sid",
        &sid,
        CookieOptions::new()
            .http_only(true)
            .secure(true)
            .same_site("lax")
            .max_age(7 * 24 * 60 * 60 * 1000),
    );
    Ok(())
}

#[post("logout")]
#[http_code(204)]
fn logout(&self, #[res(passthrough)] res: Res) {
    res.clear_cookie("sid", CookieOptions::new());
}
```

Opciones de `CookieOptions`:

| Método | Atributo |
|---|---|
| `.max_age(ms)` | `Max-Age` y `Expires`. **En milisegundos**, como en Express |
| `.expires(fecha)` | `Expires` (`chrono::DateTime` en UTC) |
| `.domain(..)`, `.path(..)` | `Domain`, `Path` (`/` por defecto) |
| `.http_only(true)`, `.secure(true)` | `HttpOnly`, `Secure` |
| `.same_site("lax" \| "strict" \| "none")` | `SameSite` |
| `.partitioned(true)`, `.priority("high")` | `Partitioned`, `Priority` |
| `.raw(true)` | No aplica `encodeURIComponent` al valor |

Además de `cookie`, `Res` tiene `cookie_json` (serializa un `Value` con el prefijo `j:`, como
Express) y `clear_cookie` (envía la cookie con `Expires` en 1970).

## Escribir cookies en una `Response`

Si el handler construye su propia `Response`, añade la cookie con `Cookie`:

```rust
#[post("theme")]
fn theme(&self) -> Response {
    Response::json(json!({ "ok": true }))
        .with_cookie(Cookie::new("theme", "dark").http_only(true).max_age(3600))
}
```

Ojo: `Cookie::max_age` recibe **segundos**, mientras que `CookieOptions::max_age` recibe
milisegundos.

## Validación

Tanto `res.cookie` como `Cookie` validan el nombre, el valor, `path`, `domain` y `same_site` igual que
el paquete `cookie` 0.7 (corrige CVE-2024-47764): un valor tomado de la petición no puede inyectar
atributos. Si algo no es válido, la llamada entra en `panic` con el `TypeError` del paquete y el
cliente recibe un `500` genérico sin la cookie. Para comprobarlo sin `panic`, usa
`Cookie::try_to_string()`.

## Pruebas

Las utilidades de testing permiten enviar cabeceras arbitrarias, incluida `Cookie`. Ver
[Testing](/fundamentals/testing).
