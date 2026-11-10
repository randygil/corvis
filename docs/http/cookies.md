---
status: documented
---

# Cookies

Cookies sent by the client are read from the `Request`, without registering any middleware, and cookies
sent by the server are written to the response. Parsing follows the `cookie-parser` package and
writing follows Express's `res.cookie` (the `cookie` 0.7 package).

## Reading cookies

`req.cookies()` returns a `HashMap` with all the cookies, and `req.cookie("name")` returns a single one:

```rust
#[get("me")]
fn me(&self, #[req] req: Request) -> HttpResult<Value> {
    let sid = req.cookie("sid").ok_or_else(|| UnauthorizedException::new("No session"))?;
    Ok(json!({ "sid": sid }))
}
```

From a guard, an interceptor or a middleware they are read the same way, from the request
(`ctx.request().cookies()`).

Parsing rules:

- If a name is repeated, the first one wins.
- The double quotes wrapping the value are removed.
- Only values that contain `%` are decoded; if the escape is not valid, the original value is kept. An escape needs two hexadecimal digits (`%+A` is not valid).

Signed cookies (the `signedCookies` of `cookie-parser`) are covered in [Signed cookies](#signed-cookies).

## Writing cookies with `#[res(passthrough)]`

The `#[res(passthrough)] res: Res` parameter gives access to the response object without taking away from Corvis the
serialization of the return value:

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

`CookieOptions` options:

| Method | Attribute |
|---|---|
| `.max_age(ms)` | `Max-Age` and `Expires`. **In milliseconds**, as in Express |
| `.expires(date)` | `Expires` (`chrono::DateTime` in UTC) |
| `.domain(..)`, `.path(..)` | `Domain`, `Path` (`/` by default) |
| `.http_only(true)`, `.secure(true)` | `HttpOnly`, `Secure` |
| `.same_site("lax" \| "strict" \| "none")` | `SameSite` |
| `.partitioned(true)`, `.priority("high")` | `Partitioned`, `Priority` |
| `.raw(true)` | Does not apply `encodeURIComponent` to the value |

Besides `cookie`, `Res` has `cookie_json` (serializes a `Value` with the `j:` prefix, as
Express does) and `clear_cookie` (sends the cookie with `Expires` set to 1970).

## Writing cookies on a `Response`

If the handler builds its own `Response`, add the cookie with `Cookie`:

```rust
#[post("theme")]
fn theme(&self) -> Response {
    Response::json(json!({ "ok": true }))
        .with_cookie(Cookie::new("theme", "dark").http_only(true).max_age(3_600_000))
}
```

Both APIs take `max_age` in **milliseconds**, like Express, and write `Max-Age` as whole seconds
(rounded down). Before this change `Cookie::max_age` took seconds: `max_age(3600)` is now 3.6 seconds, so
write `max_age(3_600_000)` for one hour.

## Validation

Both `res.cookie` and `Cookie` validate the name, the value, `path`, `domain` and `same_site` the same way as
the `cookie` 0.7 package (which fixes CVE-2024-47764): a value taken from the request cannot inject
attributes. If something is not valid, the call `panic`s with the package's `TypeError` and the
client receives a generic `500` without the cookie. To check without a `panic`, use
`Cookie::try_to_string()`.

## Signed cookies

With the `auth-session` cargo feature, `CookieKeys` signs a value so the client cannot alter it (the `signedCookies` of
`cookie-parser`). The signature is an HMAC-SHA256 of the value with a secret, in the `cookie-signature` format:

```rust
use corvis::http::CookieKeys;

let keys = CookieKeys::new([std::env::var("COOKIE_SECRET").unwrap_or_default()])?;

let signed = keys.sign("dark");                    // "s:dark.<43 base64 characters>"
let value: Option<String> = keys.unsign(&signed);  // Some("dark")
let current: bool = keys.verified_with_current(&signed);
```

- `CookieKeys::new([..])` takes 1 to 8 secrets, each at least 32 bytes; anything else is an error. The first one signs and
  all of them verify. The error names the position and the length of the secret, never the secret.
- `keys.unsign(&input)` returns the value, or `None` when the input is not `s:`-prefixed or the signature does not match.
- `keys.verified_with_current(&input)` is true only when the first secret verifies. Use it to tell a cookie that still
  needs to be signed again after a rotation.
- A request reads one with `req.signed_cookie("prefs", &keys)`. It gives `None` when the cookie is absent, unsigned or
  invalid. `j:` JSON cookies are not decoded.
- Write it with the usual options:

```rust
#[post("theme")]
#[http_code(204)]
fn theme(&self, #[res(passthrough)] res: Res) {
    let keys = &self.keys;
    res.cookie("prefs", &keys.sign("dark"), CookieOptions::new().http_only(true));
}
```

An input over 4096 bytes is refused before any HMAC is computed, and the signature is compared in constant time. To
rotate, put the new secret first and keep the old one until the cookies signed with it have expired: `CookieKeys::new([new,
old])`. The `Debug` output of `CookieKeys` is redacted, and no error contains the secret.

For a whole session (an identifier in a signed cookie and the data on the server), use [Session](/http/session).

## Testing

The testing utilities let you send arbitrary headers, including `Cookie`. See
[Testing](/fundamentals/testing).
