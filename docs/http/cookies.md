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

There are no signed cookies (`signedCookies`): if you need the client not to be able to alter the value,
store a JWT signed with `JwtService` or an opaque identifier.

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
        .with_cookie(Cookie::new("theme", "dark").http_only(true).max_age(3600))
}
```

Note: `Cookie::max_age` takes **seconds**, while `CookieOptions::max_age` takes
milliseconds.

## Validation

Both `res.cookie` and `Cookie` validate the name, the value, `path`, `domain` and `same_site` the same way as
the `cookie` 0.7 package (which fixes CVE-2024-47764): a value taken from the request cannot inject
attributes. If something is not valid, the call `panic`s with the package's `TypeError` and the
client receives a generic `500` without the cookie. To check without a `panic`, use
`Cookie::try_to_string()`.

## Testing

The testing utilities let you send arbitrary headers, including `Cookie`. See
[Testing](/fundamentals/testing).
