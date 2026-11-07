---
status: no-disponible
---

# Protección CSRF

Corvis no incluye un middleware anti-CSRF (tokens sincronizados o *double submit cookie*). Tampoco
tiene sesiones de servidor, que es donde esos tokens suelen guardarse.

Un ataque CSRF necesita que el navegador adjunte credenciales automáticamente, es decir, cookies.
Según cómo autentiques, estas son las alternativas reales:

## Si autenticas con `Authorization: Bearer`

Es el flujo de [Autenticación](/es/security/authentication) con `JwtModule`. El navegador nunca añade
esa cabecera por su cuenta, así que un sitio ajeno no puede forjar una petición autenticada. No
necesitas protección CSRF adicional, siempre que no guardes además el token en una cookie.

## Si autenticas con cookies

1. **`SameSite`**: emite la cookie con `same_site("lax")` o `same_site("strict")`. El navegador no la
   envía en peticiones `POST` desde otros sitios.

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

   `Res` y `CookieOptions` se importan de `corvis` (`use corvis::{CookieOptions, Res};`). Ver
   [Cookies](/es/http/cookies).

2. **Comprobar `Origin`** en las peticiones que cambian estado. Un middleware global basta:

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

   Los navegadores envían `Origin` en las peticiones `POST`, `PUT`, `PATCH` y `DELETE` entre
   orígenes. Las peticiones sin `Origin` (clientes que no son navegadores) pasan.

3. **CORS restrictivo**: con `credentials(true)`, usa una lista explícita de orígenes. CORS no impide
   que se envíe un formulario, pero sí que una página ajena lea la respuesta. Ver [CORS](/es/security/cors).

## WebSockets

Un gateway que autentica por cookie es vulnerable a *Cross-Site WebSocket Hijacking*. Limita los
orígenes con `GatewayOptions::allowed_origins`, que responde `403` en el handshake a un `Origin` que
no está en la lista. Ver [Gateways](/es/websockets/gateways).
