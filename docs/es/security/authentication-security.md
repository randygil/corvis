---
status: documented
---

# Proteger la autenticación

El módulo de autenticación hace mucho por ti, y hay cosas que no puede hacer. Esta página es para quien despliega una
aplicación que lo usa: qué hace ya el framework, qué debes hacer antes de salir a producción, los errores que permiten las
piezas al combinarse y qué sigue abierto. No repite el [modelo de amenazas](/es/security/authentication-threat-model), que
está escrito para quien evalúa el módulo y tiene una tabla por superficie; cada punto de aquí que se apoya en una
mitigación enlaza a la fila que la explica. La mecánica de cada estrategia está en [Passport](/es/recipes/passport), y la
guía es [Autenticación](/es/security/authentication). Los ejemplos de código salen de `examples/auth-app`.

## Qué hace el framework por ti

- Las credenciales se comparan en tiempo constante, y los secretos, los tokens y los hashes nunca llegan a un log, a una
  respuesta ni a una salida de `Debug` ([API keys](/es/security/authentication-threat-model#api-keys), [JWT](/es/security/authentication-threat-model#jwt)).
- Un token se acepta solo con un algoritmo de la familia de la clave y con una expiración, `none` no se puede escribir, y
  una clave demasiado corta detiene el arranque ([JWT](/es/security/authentication-threat-model#jwt)).
- Las contraseñas se hashean con un algoritmo que exige memoria, un usuario desconocido cuesta lo mismo que uno conocido, y
  cada hash se acota antes de verificarse ([contraseñas y login local](/es/security/authentication-threat-model#contrasenas-y-login-local)).
- Un identificador de sesión cambia en el login y en el logout, la cookie está firmada y es `HttpOnly`, y una cookie que
  llega dos veces no es una credencial ([sesiones](/es/security/authentication-threat-model#cookies-firmadas-y-sesiones)).
- OAuth2 siempre envía `state` y PKCE, nunca construye un redirect URI a partir de la petición, y liga un flujo a la sesión
  y al usuario que lo inició ([OAuth2](/es/security/authentication-threat-model#oauth2)).
- Todo fallo de una estrategia responde el mismo `401`, un panic o un error termina la cadena en lugar de dejar pasar la
  petición, y un usuario vacío nunca se autentica ([núcleo de estrategias](/es/security/authentication-threat-model#nucleo-de-estrategias)).
- El nombre de una estrategia que nadie registró detiene el arranque, y se advierte de una ruta que ejecuta el throttler
  después de un `AuthGuard` ([orden de los guards](/es/security/authentication-threat-model#orden-de-los-guards-y-rutas-publicas)).
- Todo callback que escribes (un validador, un proveedor de claves) tiene un plazo ([núcleo de estrategias](/es/security/authentication-threat-model#nucleo-de-estrategias)).
- Los valores de los parámetros de credenciales del query que declara una estrategia se ocultan en el log de peticiones
  ([valores por defecto](/es/security/authentication-threat-model#defaults)).
- Las rutas de login se pueden limitar por cliente y por cuenta antes de que se hashee ninguna contraseña
  ([control de intentos de login](/es/security/authentication-threat-model#control-de-intentos-de-login)).

## Lista de comprobación antes de salir a producción

**HTTPS y flags de las cookies.** Sirve la aplicación por HTTPS (o detrás de un proxy que termine TLS) y deja `Secure`
activado. Asigna a la cookie de sesión, a la cookie de state de OAuth2 y a una cookie que guarde un JWT el prefijo `__Host-`:
un navegador se niega entonces a guardar una puesta por un subdominio hermano (cookie tossing), que de otro modo dejaría a
ese subdominio cerrar sesiones de las víctimas o iniciarlas como otra persona. `SessionOptions::name("__Host-sid")` lo hace
para la sesión, y la cookie de state de OAuth2 lo tiene por defecto. Solo el desarrollo con HTTP plano lo desactiva, y el
framework registra una advertencia cuando está desactivado.

**Secretos.** Cada secreto tiene al menos la longitud mínima ([valores por defecto](/es/security/authentication-threat-model#defaults)),
viene del entorno o de un almacén de secretos, es distinto para cada propósito (clave de tokens, secreto de sesión, secreto
de la cookie de state, clave de servicio) y nunca se sube al repositorio. Genéralos con `openssl rand -hex 32`. Para rotar
un secreto de cookies firmadas o de sesión, pon el nuevo primero y conserva el anterior hasta que pase la vida más larga de
una cookie ([Sesión](/es/http/session#secretos-y-rotacion)); una clave de tokens se rota emitiendo con la clave nueva y
verificando con un proveedor de claves que conozca ambas hasta que expiren los tokens antiguos. Nunca firmes tus propias
cookies con los secretos de la sesión.

**Vida de los tokens.** Un token es válido hasta que expira, y no hay revocación: un token robado funciona hasta entonces.
Haz que los tokens de acceso duren poco y renuévalos volviendo a iniciar sesión o con un endpoint de refresco propio. Usa una
clave o una audiencia distinta para cada tipo de token (acceso, refresco, restablecimiento de contraseña): la estrategia no
comprueba el tipo de un token, así que un tipo se aceptaría como otro.

**Límite de intentos de login.** Limita las rutas de login por dirección de cliente y por cuenta, y registra el throttler
**antes** de cualquier `AuthGuard`, de modo que una petición que supera el límite no cueste ningún hash. Limita las dos
rutas de un login OAuth2. Un límite por cuenta significa que cualquiera puede bloquear una cuenta durante su duración: es un
compromiso que aceptas o que reemplazas con tu propia política. Ver
[Proteger una ruta de login](/es/security/rate-limiting#proteger-una-ruta-de-login) y
[el modelo de amenazas](/es/security/authentication-threat-model#control-de-intentos-de-login).

**Credenciales en URLs y logs.** Envía las credenciales en una cabecera o en un body. Una clave, un token o una contraseña
en el query string es opcional en toda estrategia, termina en los logs de acceso de cada proxy entre el cliente y tú, en el
historial del navegador y en `Referer`, y solo se oculta del log de peticiones de Corvis para los parámetros que declara una
estrategia: un parámetro que lees en tu propio código se imprime salvo que lo registres (`Request::redact_query_param`).
Usa `Request::redacted_url()` cuando registres una URL.

**CSRF cuando una credencial viaja en una cookie.** El navegador envía una cookie por su cuenta. Rechaza las peticiones que
cambian estado y vienen de otro sitio, **login** incluido (`SameSite=Lax` no lo detiene): el ejemplo pone una comprobación
de origen, el `SameOriginGuard` de `src/same_origin.rs`, antes de comprobar las credenciales. Ver
[Protección CSRF](/es/security/csrf).

**Un almacén de sesiones real.** El almacén en memoria pertenece a un proceso: las sesiones se pierden al reiniciar y no se
comparten entre instancias. Para más de una instancia, escribe un `SessionStore` sobre una base de datos compartida (los
almacenes de Redis y de ORM [aún no se incluyen](/es/security/authentication#que-no-se-incluye)).

**Destinos de redirección.** Una redirección construida con datos de la petición es un open redirect. El `return_to` de
OAuth2 acepta solo una ruta relativa, y el redirect URI se configura, nunca se construye a partir de la petición; cualquier
redirección que escribas tú mismo necesita el mismo cuidado ([Escribir una estrategia](/es/recipes/passport#escribir-una-estrategia)).

**Cuentas de proveedores.** Identifica al usuario de un proveedor por `(provider, id)`. Vincula una cuenta existente solo
mediante `verified_email()`. No leas ningún claim del `id_token`: no está verificado.

**API keys.** Genera claves largas y aleatorias, muestra cada una una sola vez, guarda un digest, compara con los helpers y
asigna a cada cliente su propia clave para poder revocar una.

**Contraseñas.** Hashea con `PasswordHasher`; verifica con `verify_or_dummy`; reemplaza los hashes viejos con
`needs_rehash` tras un login correcto; mientras la mayoría de los hashes guardados sigan siendo bcrypt, define
`dummy_algorithm` con el algoritmo guardado. Los métodos bloqueantes son para código fuera del runtime de Tokio. Aplica tu
propia política de contraseñas: el framework no tiene ninguna y no comprueba ninguna lista de contraseñas filtradas. Ver
[Cifrado y hashing](/es/security/encryption-and-hashing).

**Direcciones de clientes.** Configura `trust proxy` con los proxies que controlas, o todo cliente detrás de tu balanceador
de carga comparte un solo límite de tasa ([el modelo de amenazas](/es/security/authentication-threat-model#alcance-y-supuestos)).

**Callbacks que esperan.** Un validador que bloquea su hilo en lugar de esperar con `await` no puede cancelarse por su
plazo. Espera con `await` las llamadas a la base de datos y a la red; nunca bloquees. Un validador que se cancela se detiene
en un `await`, así que no debe dejar estado a medio escribir: haz que las escrituras que deben ir juntas sean una sola
llamada (el ejemplo reemplaza un hash de contraseña en un solo paso).

**Caché.** No registres `CacheInterceptor` para toda la aplicación cuando tiene rutas `GET` autenticadas
(ver [Limitaciones conocidas](#limitaciones-conocidas)).

## Por estrategia: tu parte

| Estrategia | El trabajo del framework | Tu trabajo |
|---|---|---|
| JWT | verificar la firma, la familia de algoritmos y la expiración; rechazar una clave corta y un token sobredimensionado; acotar el proveedor de claves y el validador | elegir la vida del token; mantener los tokens fuera de las URLs; claves separadas por tipo de token; no hay revocación, así que hazlos cortos; aplicar las mismas reglas en un handshake de WebSocket |
| Local | leer y acotar las credenciales; una verificación por intento; el `401` uniforme | guardar hashes; devolver `None` cuando no hay usuario; volver a hashear los hashes viejos; limitar por cliente y por cuenta; una política de contraseñas; bloquear cuentas si lo quieres |
| API key | leer solo de los lugares configurados; comparación en tiempo constante; rechazar claves en conflicto o sobredimensionadas | generar claves largas; guardar digests; una clave por cliente; rotar y revocar; mantener la clave fuera de las URLs |
| Sesión | firmar la cookie; cambiar el identificador en el login y en el logout; acotar el almacén | HTTPS; prefijo `__Host-`; una comprobación de origen en las peticiones con cookie, login incluido; un almacén que encaje con tu despliegue; cerrar las sesiones que debas cerrar |
| OAuth2 | `state`, PKCE, el flujo ligado a su sesión y a su usuario, el cliente del proveedor con la verificación de TLS siempre activa | registrar el redirect URI; identificar por `(provider, id)`; vincular solo por e-mail verificado; limitar ambas rutas; guardar cifrados los tokens del proveedor si los conservas |
| Estrategia propia | el fallo uniforme, los panics capturados, los usuarios vacíos rechazados | fallar, nunca pasar; acotar lo que lees; comparación en tiempo constante; ningún secreto en errores ni en mensajes de panic |

## Errores comunes

**1. Un guard opcional como único guard de una ruta que necesita un usuario.** Un guard opcional deja pasar la petición
cuando todas las estrategias fallaron, así que una petición anónima ejecuta el handler.

```rust
// Cualquiera puede llamar a esto: el guard es opcional y nada más comprueba quién llama.
#[post("close")]
#[use_guards(AuthGuard::new("jwt").optional())]
fn close_account(&self, req: Request) -> HttpResult<()> { /* ... */ }
```

Usa un guard obligatorio, o pide `User<T>`, que responde `401` cuando no hay ninguno. Un guard opcional es para rutas que
cualquiera puede llamar y que personalizan su respuesta, y esas se marcan con `#[public]`:

```rust
// examples/auth-app/src/controllers.rs
    #[get("me")]
    #[use_guards(AuthGuard::new("jwt"))]
    fn me(&self, user: User<Account>) -> Account {
        user.0
    }
```

**2. Un guard que lee el usuario sin un `AuthGuard` antes.** El middleware de sesión restaura el usuario desde la cookie en
cada ruta, así que un guard de roles en una ruta sin `AuthGuard` se autentica solo con la cookie, y un `POST` de formulario
entre sitios lleva la cookie ([el usuario de sesión](/es/security/authentication-threat-model#el-usuario-de-sesion-en-rutas-sin-authguard)).

```rust
// Autenticada por lo que restauró el middleware de sesión, en una ruta sin AuthGuard.
#[use_guards(RolesGuard)]
```

Lista primero un `AuthGuard` que nombre las credenciales que acepta la ruta:

```rust
// examples/auth-app/src/controllers.rs
    #[get("admin/users")]
    #[set_metadata(ROLES_KEY, [Role::Admin])]
    #[use_guards(AuthGuard::chain(["jwt", "session"]), RolesGuard)]
    fn admin_users(&self) -> Vec<Account> {
        self.users.accounts()
    }
```

**3. Un guard global sin `.skip_public()`, con rutas marcadas `#[public]`.** Las rutas públicas se autentican como
cualquier otra salvo que el guard diga que respeta la marca, así que la ruta de login responde `401` a todos.

```rust
app.use_global_guards(AuthGuard::new("jwt")); // `#[public]` se ignora: la ruta de login también está protegida
```

```rust
app.use_global_guards(AuthGuard::new("jwt").skip_public());
```

Un guard global que omite las rutas públicas confía en cada marca `#[public]`, así que revísalas, y activa la
[postura de rutas](/es/security/route-posture) para que un guard olvidado detenga el arranque.

**4. El throttler después de la autenticación.** Un límite de tasa que se ejecuta después del `AuthGuard` no limita nada: la
contraseña ya se hasheó. El arranque advierte cuando ve este orden; registra el throttler como guard global mediante el
contenedor para que se ejecute primero:

```rust
#[use_guards(AuthGuard::new("local"), ThrottlerGuard)] // la contraseña se hashea antes de comprobar el límite
```

```rust
// examples/auth-app/src/app_module.rs
    providers = [
        // Un guard global registrado mediante el contenedor se ejecuta antes de todo `AuthGuard`.
        Provider::app_guard::<ThrottlerGuard>(),
        // ...
    ],
```

**5. Un guard envoltorio que oculta el guard que lleva dentro.** Un guard que envuelve un `AuthGuard` debe listarlo en
`inner_guards`, o el arranque no puede comprobar los nombres de sus estrategias ni su orden respecto al throttler. No
devuelvas el guard interno desde `as_any`: eso es para el propio guard.

**6. `Pass` desde una estrategia propia.** `StrategyOutcome::Pass` deja pasar la petición sin usuario. Devuelve `Fail`
cuando la credencial falta o es incorrecta, y `Error` cuando falló una dependencia.

**7. Devolver `auth_info()` al cliente.** Contiene el motivo de un fallo, y el framework lo mantiene fuera de la respuesta a
propósito: una respuesta que dice *por qué* falló un login le dice a un atacante qué probar a continuación. Regístralo; no
lo devuelvas.

**8. Comparar secretos con `==`.** Se detiene en el primer byte distinto. Compara con los helpers, que hashean ambos lados y
comparan los digests en tiempo constante, y rechaza un secreto esperado vacío antes de comparar:

```rust
if key == expected { /* ... */ } // filtra tiempos, y dos cadenas vacías son iguales
```

```rust
// examples/auth-app/src/auth/api_key.rs
        Ok(self.keys.expected.matches(key).then_some(ServiceCaller { service: "reports" }))
```

**9. Vincular cuentas por e-mail.** Una dirección es una afirmación del proveedor. Compararla sin comprobar que el proveedor
la verificó permite que cualquiera que registre la dirección de otra persona en un proveedor tome esa cuenta:

```rust
let user = users.find_by_email(&profile.emails[0].value); // sin verificar, y se ignora el id
```

```rust
// examples/auth-app/src/auth/provider.rs
        let name = profile.display_name.as_deref().unwrap_or("user");
        Ok(Some(self.users.for_provider_user(&profile.provider, &profile.id, profile.verified_email(), name)))
```

**10. Un validador que devuelve un usuario falso.** `Ok(Some(expected.matches(key)))` dice "encontrado" para toda clave, y el
usuario es `false` cuando la clave es incorrecta. Un usuario así nunca se autentica, y una advertencia te dice qué validador
corregir; di "ningún usuario" con `None`.

**11. `.log_in()` en el primer paso de un login de dos pasos.** Un login hecho por el guard se conserva cuando el handler
retorna con normalidad, sea cual sea el estado que fije. Un handler que implementa un segundo paso se autentica sin
`.log_in()` y llama a `Authenticator::log_in` después del último paso.

**12. Un lector de cookie colocado antes del lector de cabecera.** `ExtractJwt::from_extractors` prueba solo el primer token
no vacío, así que con un extractor de cookie primero, cualquiera que pueda plantar una cookie anula la cabecera
`Authorization` del usuario. Lee solo los lugares que necesitas, el más confiable primero.

**13. Completar la vinculación de una cuenta bajo otro usuario.** Una vinculación iniciada por un usuario y completada por
otro en un navegador compartido la rechaza el framework cuando ambas rutas ejecutan los mismos guards. Ejecuta los mismos
guards en la ruta que inicia el flujo y en el callback, y vincula la cuenta del proveedor a `Request::user()` solo después
de que hayan autenticado la petición.

## Limitaciones conocidas

Estos hallazgos de [la revisión](/es/security/authentication-threat-model#hallazgos) siguen abiertos. Hasta que se corrija
cada uno, usa la alternativa.

| Issue | Qué está abierto | Qué hacer hasta que se corrija |
|---|---|---|
| #173 | las estrategias no se ejecutan en el handshake de WebSocket, y `AuthGuard` deniega los mensajes de socket | autentica en `handle_connection` y aplica allí las reglas de la estrategia JWT; ver [WebSockets](/es/security/authentication#websockets) |
| #174 | `JwtService` acepta un secreto corto y un token sin `exp`, que la estrategia JWT rechaza | autentica las peticiones con la estrategia; si verificas un token con `JwtService`, comprueba tú mismo la expiración y la longitud del secreto |
| #172 | no hay lista de permitidos para el redirect URI de OAuth2 | registra un redirect URI exacto por entorno en el proveedor, y ejecuta una estrategia por dominio |
| #170, #171 | no hay almacén de sesiones en Redis ni en un ORM | el almacén en memoria es por proceso: escribe un `SessionStore` sobre una base de datos compartida |
| #177 | `CacheInterceptor` usa la URL como clave, así que un registro global repite la respuesta `GET` de un usuario a otro | no lo registres para toda la aplicación si tiene rutas `GET` autenticadas; usa la caché en rutas que son iguales para todos |
| #178 | en Socket.IO un middleware de conexión en el namespace principal no cubre los demás, y `Debug` de un `Handshake` imprime el payload `auth` | registra el middleware en cada namespace que tenga handlers; nunca formatees un handshake con `{:?}` |

Una más es un límite y no un bug: un almacén de sesiones caído hace fallar las rutas que llevan una cookie de sesión, las
públicas y los health checks incluidos, porque el middleware de sesión se ejecuta antes de todo guard. Mantén el almacén tan
fiable como la aplicación, y pon un limitador delante de `SessionModule` si pueden llegar a él peticiones con la cookie de
otra persona.
