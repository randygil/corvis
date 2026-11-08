---
status: unavailable
---

# CSRF protection

Corvis does not include an anti-CSRF middleware (synchronized tokens or *double submit cookie*). It also does not
have server-side sessions, which is where those tokens are usually stored.

A CSRF attack needs the browser to attach credentials automatically, that is, cookies.
Depending on how you authenticate, these are the real alternatives:

## If you authenticate with `Authorization: Bearer`

This is the flow from [Authentication](/security/authentication) with `JwtModule`. The browser never adds
that header on its own, so a third-party site cannot forge an authenticated request. You
do not need additional CSRF protection, as long as you do not also store the token in a cookie.

## If you authenticate with cookies

1. **`SameSite`**: issue the cookie with `same_site("lax")` or `same_site("strict")`. The browser does not
   send it on `POST` requests from other sites.

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

2. **Check `Origin`** on requests that change state. A global middleware is enough:

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
