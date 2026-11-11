---
status: unavailable
---

# Protección CSRF

Corvis no incluye un middleware anti-CSRF (tokens sincronizados o *double submit cookie*). Sí tiene
[sesiones de servidor](/es/http/session), donde podrían guardarse esos tokens, pero nada los genera ni los comprueba por
ti: el `SameSite=Lax` por defecto de la cookie de sesión es una capa, no un reemplazo.

Un ataque CSRF necesita que el navegador adjunte credenciales automáticamente, es decir, cookies.
Según cómo autentiques, estas son las alternativas reales:

## Si autenticas con `Authorization: Bearer`

Es el flujo de [Autenticación](/es/security/authentication) con la estrategia `jwt`. El navegador nunca añade
esa cabecera por su cuenta, así que un sitio ajeno no puede forjar una petición autenticada. No
necesitas protección CSRF adicional, siempre que no guardes además el token en una cookie.

## Si autenticas con cookies

1. **`SameSite`**: emite la cookie con `same_site("lax")` o `same_site("strict")`. La cookie de sesión de
   [`SessionModule`](/es/http/session) es `SameSite=Lax` por defecto. En los navegadores actuales no se envía en
   peticiones `POST`, `PUT` y `DELETE` entre sitios ni en peticiones `fetch` y XHR.

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

   Lo que `SameSite=Lax` **no** cubre:

   - Envía la cookie en las navegaciones `GET` de nivel superior entre sitios (un enlace, una redirección). Una ruta
     `GET` nunca debe cambiar estado.
   - No detiene peticiones de subdominios hermanos ni de otros orígenes del mismo sitio: `https://a.example.com` y
     `https://b.example.com` son el mismo sitio.
   - No se aplica cuando configuras `SameSite=None`.
   - Depende de que el navegador lo aplique.
   - No detiene el CSRF de inicio de sesión (un atacante inicia la sesión de la víctima en la cuenta del atacante).

   **`SameSite=Lax` no detiene el CSRF de inicio de sesión.** Un `POST` de formulario entre sitios a la ruta de login es
   una navegación de nivel superior: el servidor responde `200` con su `Set-Cookie`, el navegador la guarda y la víctima
   queda con la sesión iniciada como el atacante. Rechaza esa petición antes de comprobar las credenciales, con una lista de
   permitidos: una petición que cambia estado pasa solo cuando `Sec-Fetch-Site` es `same-origin` (o `none`, una
   navegación que escribió el usuario) o, si esa cabecera no está, cuando `Origin` es el host al que se envió la
   petición; una lista explícita de orígenes permitidos también pasa. Todo lo demás se rechaza: `cross-site`,
   **`same-site`** (un subdominio hermano puede servir páginas que controla un atacante), un valor que nadie definió, una
   cabecera repetida. Una petición que no trae ninguna de las dos cabeceras se acepta (un cliente que no es un
   navegador, o un navegador muy viejo), así que esta comprobación es una capa sobre `SameSite`, no un reemplazo. El
   ejemplo `cats-app` lo hace con un guard, `SameOriginGuard`, delante del guard de login:

   ```rust
   #[post("login")]
   #[use_guards(SameOriginGuard::new().allow_origin("https://app.example.com"), AuthGuard::new("local").log_in())]
   fn login(&self, user: User<Account>) -> Account { user.0 }
   ```

   ```rust
   // La comprobación completa (examples/cats-app/src/auth/same_origin.rs)
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

   La comparación del host usa la cabecera `Host` que recibió la aplicación. Detrás de un proxy que reescribe `Host`, el
   `Origin` del navegador ya no coincide: enumera los orígenes públicos con `allow_origin`.

   Mantén `Secure` y `HttpOnly`, y usa el prefijo de nombre `__Host-` (`.name("__Host-sid")` en `SessionOptions`), que
   impide que un subdominio hermano establezca la cookie.

2. **Comprobar `Origin`** (o `Referer`, o exigir una cabecera propia como `X-Requested-With`, o un token) en las
   peticiones que cambian estado. Un middleware global basta:

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
