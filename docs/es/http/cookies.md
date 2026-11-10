---
status: documented
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
  original. Un escape exige dos dígitos hexadecimales (`%+A` no es válido).

Las cookies firmadas (las `signedCookies` de `cookie-parser`) se explican en [Cookies firmadas](#cookies-firmadas).

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
        .with_cookie(Cookie::new("theme", "dark").http_only(true).max_age(3_600_000))
}
```

Ambas APIs reciben `max_age` en **milisegundos**, como Express, y escriben `Max-Age` en segundos enteros
(redondeado hacia abajo). Antes `Cookie::max_age` recibía segundos: `max_age(3600)` ahora son 3,6 segundos,
así que escribe `max_age(3_600_000)` para una hora.

## Validación

Tanto `res.cookie` como `Cookie` validan el nombre, el valor, `path`, `domain` y `same_site` igual que
el paquete `cookie` 0.7 (corrige CVE-2024-47764): un valor tomado de la petición no puede inyectar
atributos. Si algo no es válido, la llamada entra en `panic` con el `TypeError` del paquete y el
cliente recibe un `500` genérico sin la cookie. Para comprobarlo sin `panic`, usa
`Cookie::try_to_string()`.

## Cookies firmadas

Con la feature de cargo `auth-session`, `CookieKeys` firma un valor para que el cliente no pueda alterarlo (las
`signedCookies` de `cookie-parser`). La firma es un HMAC-SHA256 del valor con un secreto, en el formato de
`cookie-signature`:

```rust
use corvis::http::CookieKeys;

let keys = CookieKeys::new([std::env::var("COOKIE_SECRET").unwrap_or_default()])?;

let signed = keys.sign("dark");                    // "s:dark.<43 caracteres base64>"
let value: Option<String> = keys.unsign(&signed);  // Some("dark")
let current: bool = keys.verified_with_current(&signed);
```

- `CookieKeys::new([..])` recibe de 1 a 8 secretos, cada uno de al menos 32 bytes; cualquier otra cosa es un error. El
  primero firma y todos verifican. El error nombra la posición y la longitud del secreto, nunca el secreto.
- `keys.unsign(&input)` devuelve el valor, o `None` cuando la entrada no empieza por `s:` o la firma no coincide.
- `keys.verified_with_current(&input)` es verdadero solo cuando verifica el primer secreto. Úsalo para saber qué cookie
  todavía debe firmarse de nuevo tras una rotación.
- Una petición lee una con `req.signed_cookie("prefs", &keys)`. Da `None` cuando la cookie está ausente, sin firmar o es
  inválida. Las cookies JSON con `j:` no se decodifican.
- Se escribe con las opciones habituales:

```rust
#[post("theme")]
#[http_code(204)]
fn theme(&self, #[res(passthrough)] res: Res) {
    let keys = &self.keys;
    res.cookie("prefs", &keys.sign("dark"), CookieOptions::new().http_only(true));
}
```

Una entrada de más de 4096 bytes se rechaza antes de calcular ningún HMAC, y la firma se compara en tiempo constante. Para
rotar, pon el secreto nuevo primero y conserva el viejo hasta que expiren las cookies firmadas con él:
`CookieKeys::new([nuevo, viejo])`. La salida `Debug` de `CookieKeys` está censurada y ningún error contiene el secreto.

Para una sesión completa (un identificador en una cookie firmada y los datos en el servidor) usa
[Sesión](/es/http/session).

## Pruebas

Las utilidades de testing permiten enviar cabeceras arbitrarias, incluida `Cookie`. Ver
[Testing](/es/fundamentals/testing).
