---
status: unavailable
---

# CSRF protection

Corvis does not include an anti-CSRF middleware (synchronized tokens or *double submit cookie*). It does have
[server-side sessions](/http/session), where such tokens could be kept, but nothing generates or checks them for you:
the `SameSite=Lax` default of the session cookie is a layer, not a replacement.

A CSRF attack needs the browser to attach credentials automatically, that is, cookies.
Depending on how you authenticate, these are the real alternatives:

## If you authenticate with `Authorization: Bearer`

This is the flow from [Authentication](/security/authentication) with `JwtModule`. The browser never adds
that header on its own, so a third-party site cannot forge an authenticated request. You
do not need additional CSRF protection, as long as you do not also store the token in a cookie.

## If you authenticate with cookies

1. **`SameSite`**: issue the cookie with `same_site("lax")` or `same_site("strict")`. The session cookie of
   [`SessionModule`](/http/session) is `SameSite=Lax` by default. In current browsers it is not sent on cross-site
   `POST`, `PUT` and `DELETE` requests or on `fetch` and XHR requests.

   ```rust
   #[post("login")]
   #[http_code(204)]
   async fn login(&self, #[body] dto: LoginDto, #[res(passthrough)] res: Res) -> HttpResult<()> {
       let sid = self.auth.create_session(&dto).await?;
       res.cookie(
           "sid",
           &sid,
           CookieOptions::new().http_only(true).secure(true).same_site("lax"),
       );
       Ok(())
   }
   ```

   `Res` and `CookieOptions` are imported from `corvis` (`use corvis::{CookieOptions, Res};`). See
   [Cookies](/http/cookies).

   What `SameSite=Lax` does **not** cover:

   - It sends the cookie on top-level cross-site `GET` navigations (a link, a redirect). A `GET` route must never change
     state.
   - It does not stop requests from sibling subdomains or other origins of the same site: `https://a.example.com` and
     `https://b.example.com` are the same site.
   - It does not apply when you configure `SameSite=None`.
   - It depends on the browser enforcing it.
   - It does not stop login CSRF (an attacker logs the victim in to the attacker's account).

   **`SameSite=Lax` does not stop login CSRF.** A cross-site form `POST` to the login route is a top-level navigation:
   the server answers `200` with its `Set-Cookie`, the browser stores it, and the victim is logged in as the attacker.
   Refuse such a request before the credentials are checked, with an allow-list: a state-changing request passes only
   when `Sec-Fetch-Site` is `same-origin` (or `none`, a navigation the user typed), or, when that header is absent, when
   `Origin` is the host the request was sent to; an explicit list of allowed origins passes too. Anything else is refused:
   `cross-site`, **`same-site`** (a sibling subdomain can run pages an attacker controls), a value nobody defined, a
   repeated header. The `cats-app` example does it with a guard, `SameOriginGuard`, in front of the login guard:

   ```rust
   #[post("login")]
   #[use_guards(SameOriginGuard::new().allow_origin("https://app.example.com"), AuthGuard::new("local").log_in())]
   fn login(&self, user: User<Account>) -> Account { user.0 }
   ```

   ```rust
   // The whole check (examples/cats-app/src/auth/same_origin.rs)
   pub fn refuses(&self, method: &str, sec_fetch_site: &[&str], origins: &[&str], host: Option<&str>) -> bool {
       if matches!(method, "GET" | "HEAD" | "OPTIONS") {
           return false;
       }
       let allowed_origin = match origins {
           [origin] => self.allowed.iter().any(|allowed| allowed == origin),
           _ => false,
       };
       match sec_fetch_site {
           [] => {}
           [site] => return !(site.eq_ignore_ascii_case("same-origin") || site.eq_ignore_ascii_case("none") || allowed_origin),
           _ => return true,
       }
       match origins {
           [] => false,
           [origin] => {
               if allowed_origin {
                   return false;
               }
               let authority = origin.split_once("://").map(|(_, rest)| rest);
               !matches!((authority, host), (Some(authority), Some(host)) if authority.eq_ignore_ascii_case(host))
           }
           _ => true,
       }
   }
   ```

   The host comparison uses the `Host` header the application received. Behind a proxy that rewrites `Host`, the browser's
   `Origin` no longer matches it: list the public origins with `allow_origin`.

   Keep `Secure` and `HttpOnly`, and use the `__Host-` name prefix (`.name("__Host-sid")` in `SessionOptions`), which
   stops a sibling subdomain from setting the cookie.

2. **Check `Origin`** (or `Referer`, or require a custom header such as `X-Requested-With`, or a token) on requests that
   change state. A global middleware is enough:

   ```rust
   app.use_fn(|req, next| async move {
       let safe = matches!(req.method().as_str(), "GET" | "HEAD" | "OPTIONS");
       if !safe {
           let allowed = req.header("origin").is_none_or(|o| o == "https://app.example.com");
           if !allowed {
               return Err(ForbiddenException::new("Invalid origin"));
           }
       }
       next.run(req).await
   });
   ```

   Browsers send `Origin` on cross-origin `POST`, `PUT`, `PATCH` and `DELETE`
   requests. Requests without `Origin` (non-browser clients) pass.

3. **Restrictive CORS**: with `credentials(true)`, use an explicit list of origins. CORS does not prevent
   a form from being submitted, but it does prevent a third-party page from reading the response. See [CORS](/security/cors).

## WebSockets

A gateway that authenticates by cookie is vulnerable to *Cross-Site WebSocket Hijacking*. Limit the
origins with `GatewayOptions::allowed_origins`, which responds `403` on the handshake to an `Origin` that
is not on the list. See [Gateways](/websockets/gateways).
