---
status: documented
---

# Migrar desde Passport

Esta página es para quien tiene una aplicación Node que autentica con Passport y la está pasando a Corvis, quizá una ruta a la
vez. Relaciona cada clase, opción y llamada de los paquetes de Node con Corvis, lista lo que no se ofrece, lista cada
comportamiento que responde distinto a propósito y por qué, y explica cómo ejecutar ambas aplicaciones lado a lado mientras los
usuarios conservan sus sesiones. Las opciones, valores por defecto y límites están en la referencia, [Passport](/es/recipes/passport); cada fila de abajo
enlaza a ella. Corvis tiene el mismo modelo de estrategias y su comportamiento está grabado contra las bibliotecas originales (`passport`
0.7.0, `passport-jwt` 4.0.1, `passport-local` 1.0.0, `passport-headerapikey` 1.2.2, `passport-custom` 1.2.1, `express-session`
1.19.0, `passport-oauth2` 1.8.0 y los perfiles de Google y GitHub, `bcryptjs`), sin necesitar Node al ejecutar los tests.

La integración de Passport con el framework se llama aquí "la integración original de passport".

## Antes de empezar

**Lo que se conserva sin cambios**

- Los nombres de las estrategias que das al guard: `AuthGuard::new("jwt")`, `"local"`, `"session"`. La estrategia de API key es
  la excepción: Corvis la llama `"api-key"`, y `const NAME: &'static str = "headerapikey";` en el validador conserva el nombre
  anterior.
- La respuesta a una petición que nadie autenticó: `401 {"message":"Unauthorized","statusCode":401}`, sin cabecera
  `WWW-Authenticate`, sea lo que sea que las estrategias adjuntaron a su fallo.
- `req.user` y `req.authInfo`: `Request::user()` (JSON) y el parámetro `User<T>`, y `Request::auth_info()`.
- El formato de la cookie de sesión y de los registros de sesión, de modo que ambas aplicaciones pueden leer las mismas sesiones, y los
  hashes bcrypt `$2a$`/`$2b$`, que se verifican tal cual.

**Lo que debes decidir primero**

- **Longitud de los secretos.** Corvis rechaza al arrancar un secreto de menos de 32 bytes para una clave de token HMAC, una cookie firmada y una API key,
  con un error que nombra el ajuste y nunca el valor. Si una aplicación Node usa secretos más cortos, la migración es también el
  momento de rotarlos (ver [Ejecutar ambos lado a lado](#ejecutar-ambos-lado-a-lado)).
- **Vencimiento de los tokens.** Los tokens necesitan un claim `exp` por defecto. Haz que el lado Node emita uno antes de mover la verificación.
- **Flags de la cookie.** La cookie de sesión de Corvis es `Secure`, `HttpOnly` y `SameSite=Lax` salvo que indiques otra cosa; el valor
  por defecto de Node no es ninguno de ellos.

## Tablas de equivalencias

### La integración original de passport

| Original | Corvis |
|---|---|
| `class JwtStrategy extends PassportStrategy(Strategy)` con `validate(payload)` | un validador (`JwtValidator`, `LocalValidator`, ...) registrado con `Provider::strategy::<JwtStrategy<TuValidador>>()` |
| `@UseGuards(AuthGuard('jwt'))` | `#[use_guards(AuthGuard::new("jwt"))]` |
| `AuthGuard(['jwt', 'apikey'])` | `AuthGuard::chain(["jwt", "api-key"])` |
| `AuthGuard()` con `PassportModule.register({ defaultStrategy })` | `AuthGuard::default()` con `PassportModule::register(PassportModuleOptions::new().default_strategy("jwt"))` |
| `handleRequest(err, user)` devolviendo `user \|\| null` | `.optional()` |
| `handleRequest` lanzando otra excepción | `.on_failure(\|failures\| UnauthorizedException::new("Token required"))` |
| `getRequest` (para leer la request desde un contexto que no es HTTP) | no hace falta: el guard lee la request HTTP por sí mismo, y deniega los demás contextos |
| `if (isPublic) return true` antes de `super.canActivate` | `.skip_public()`, con `#[public]` en las rutas |
| `req.user` | `Request::user()` o el parámetro `User<T>` |
| `req.authInfo` | `Request::auth_info()` |
| un decorador de parámetro `@User()` | el parámetro `User<T>` |

La clase de la estrategia se convierte en un **validador**: el mismo `validate`, con las opciones en `options()` en lugar del constructor.
Para JWT:

```js
export class JwtStrategy extends PassportStrategy(Strategy) {
  constructor(config) {
    super({
      jwtFromRequest: ExtractJwt.fromAuthHeaderAsBearerToken(),
      secretOrKey: config.get('JWT_SECRET'),
    })
  }
  async validate(payload) {
    return { id: payload.sub, username: payload.username }
  }
}
```

```rust
// examples/auth-app/src/auth/jwt.rs
#[async_trait]
impl JwtValidator for JwtAuth {
    type User = Account;

    fn options(&self) -> JwtStrategyOptions {
        JwtStrategyOptions::new()
            .jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token())
            .secret_or_key(JwtKey::secret(self.settings.jwt_secret.clone()))
            .algorithms([JwtAlgorithm::HS256])
    }

    async fn validate(&self, _req: &Request, payload: Value) -> Result<Option<Account>, HttpException> {
        // ...
        let claims: Option<Claims> = corvis::serde_json::from_value(payload).ok();
        Ok(claims.map(|c| Account { id: c.sub, username: c.username, roles: c.roles }))
    }
}
```

`validate` devuelve el usuario (`Some`), ningún usuario (`None`: el `401`) o una excepción (que se pasa tal cual, como `done(err)`). **Devuelve
`None`, no un valor que se serialice a `null`, `false`, `0` o `""`**: el original trata un usuario falsy como un fallo, y
Corvis también, con una advertencia que nombra el validador. Para local:

```js
export class LocalStrategy extends PassportStrategy(Strategy) {
  constructor(users) { super(); this.users = users }
  async validate(username, password) {
    const user = await this.users.findOne(username)
    if (!user || !(await bcrypt.compare(password, user.hash))) throw new UnauthorizedException()
    return user
  }
}
```

```rust
// examples/auth-app/src/auth/local.rs
#[async_trait]
impl LocalValidator for LocalAuth {
    type User = Account;

    async fn validate(&self, _req: &Request, username: &str, password: &str) -> Result<Option<Account>, HttpException> {
        let found = self.users.credentials(username);
        let stored = found.as_ref().map(|(_, hash)| hash.as_str());
        // ...
        let ok = self.hasher.verify_or_dummy(password, stored).await.map_err(PasswordError::into_http_exception)?;
        let Some((account, hash)) = found.filter(|_| ok) else { return Ok(None) };
        // ...
        Ok(Some(account))
    }
}
```

Una contraseña incorrecta es `Ok(None)`, no una excepción: ambas terminan en el mismo `401`, pero una excepción que construyes tú mismo puede
diferir según el usuario.

### `passport-jwt`

| `passport-jwt` | Corvis |
|---|---|
| `jwtFromRequest: ExtractJwt.fromAuthHeaderAsBearerToken()` y los demás extractores | `.jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token())`; no hay un lugar por defecto |
| `secretOrKey`, `secretOrKeyProvider` | `.secret_or_key(JwtKey::secret(..))`, `.secret_or_key_provider(..)` |
| `issuer`, `audience`, `algorithms` | `.issuer(..)`, `.audience(..)`, `.algorithms([..])` |
| `ignoreExpiration: true` | `.ignore_expiration(true)`, con una advertencia al arrancar |
| `passReqToCallback` | siempre activa: `validate(req, payload)` |
| `jwt.sign(payload, { expiresIn })` (`jsonwebtoken`) | `JwtService::sign` con `JwtModuleOptions::expires_in(..)`, o `sign_with(.., JwtSignOptions { expires_in, .. })` |

Cada opción, los extractores y las reglas de algoritmos están en [la referencia de JWT](/es/recipes/passport#la-estrategia-jwt).

### `passport-local` y `bcryptjs`

| Original | Corvis |
|---|---|
| `usernameField`, `passwordField` | `LocalStrategyOptions::default().username_field("email").password_field("pass")` |
| `passReqToCallback` | siempre activa: `validate(req, username, password)` |
| `done(null, false)` | `Ok(None)` |
| `bcrypt.hash(password, 10)` | `hasher.hash(password).await` (argon2id por defecto; `PasswordHasherOptions::bcrypt(10)` conserva bcrypt) |
| `bcrypt.compare(password, hash)` | `hasher.verify(password, hash).await`, o `verify_or_dummy(password, Option<&str>)` para que un usuario desconocido cueste lo mismo |
| `bcrypt.compareSync`, `hashSync` | `verify_blocking`, `hash_blocking`, **solo para código fuera del runtime de Tokio** (dentro de él devuelven un error) |
| una comprobación de que el hash sigue vigente | `hasher.needs_rehash(&stored)`, luego `hash` y guardarlo tras un login exitoso |

El hasher, los algoritmos y los límites están en [Cifrado y hashing](/es/security/encryption-and-hashing); los lugares de
credenciales y los límites de la estrategia están en [la referencia de local](/es/recipes/passport#estrategia-local-usuario-y-contrasena).

### `passport-headerapikey` y `passport-custom`

| Original | Corvis |
|---|---|
| `new HeaderAPIKeyStrategy({ header: 'X-Api-Key', prefix: '' }, false, verify)` | `ApiKeyStrategyOptions::default()` (la cabecera `X-Api-Key`), o `.source(ApiKeySource::header("Authorization").prefix("Api-Key "))` |
| el nombre de estrategia `headerapikey` | `"api-key"`, o `const NAME: &'static str = "headerapikey";` para conservarlo |
| `verify(apiKey, done)` buscando la clave | `validate(req, &ApiKey)`, comparando un digest (`ExpectedApiKey`, `ApiKeySet`), nunca con `==` |
| `passport-custom`: `new CustomStrategy((req, done) => ...)` | un tipo que implementa `Strategy` (`authenticate` devuelve un `StrategyOutcome`); ver [Si vienes de `passport-custom`](/es/recipes/passport#si-vienes-de-passport-custom-y-passport-strategy) |
| una clave leída del query string | `ApiKeySource::query("api_key")`, opcional, con una advertencia al arrancar |

### `express-session` con `passport.session()`

| Original | Corvis |
|---|---|
| `app.use(session({ secret, name, cookie, resave, saveUninitialized, rolling, store }))` | `SessionModule::for_root(SessionOptions::with_secrets([..])?.name(..).cookie(SessionCookie::..).resave(..).save_uninitialized(..).rolling(..).store(..))` |
| `app.use(passport.session())` | el middleware de `SessionModule` y `SessionStrategy<TuSerializador>`, la estrategia llamada `"session"` |
| `passport.serializeUser(fn)` | `SessionSerializer::serialize_user` |
| `passport.deserializeUser(fn)` | `SessionSerializer::deserialize_user` (`Ok(None)` cuando el usuario ya no existe) |
| `req.logIn(user)` | `AuthGuard::new("local").log_in()`, o `Authenticator::log_in(&req, &AuthenticatedUser::new(user)?).await?` |
| `req.logout()` | `Authenticator::log_out(&req).await?` |
| `req.isAuthenticated()` | `AuthGuard::new("session")` responde `401` si es anónimo (la estrategia de sesión del original siempre pasa) |
| `keepSessionInfo` | `SessionOptions::keep_session_info_on_login(true)` |
| `connect-redis` y otros almacenes | `SessionOptions::store(..)` con un tipo que implementa `SessionStore`; los almacenes de Redis y de ORM todavía no se ofrecen |

Ver [Sesiones](/es/recipes/passport#sesiones) y [Sesión](/es/http/session) para el resto.

### `passport-oauth2`, `passport-google-oauth20`, `passport-github2` y `passport-oauth2-refresh`

| Original | Corvis |
|---|---|
| `new GoogleStrategy({ clientID, clientSecret, callbackURL, scope }, verify)` | `OAuth2Options::google(client_id, client_secret, redirect_uri).scope([..])`; `OAuth2Options::github(..)` para GitHub; `OAuth2Options::new(OAuth2Endpoints::new(authorization_url, token_url).userinfo_url(url), ..)` para cualquier otro proveedor |
| `verify(accessToken, refreshToken, profile, done)` | `validate(req, grant: OAuth2Grant)`: `grant.access_token`, `grant.refresh_token`, `grant.params`, `grant.profile` |
| `profile.id`, `profile.emails[0].value` | `profile.id` (compáralo con el nombre del proveedor) y `profile.verified_email()` |
| `state: true`, `pkce: true` con una sesión | siempre activos; tú eliges dónde vive el state: `.state_cookie_secrets([..])` o `.state_in_session()` |
| `callbackURL` como ruta relativa | una URL absoluta, configurada: nada se construye a partir de la cabecera `Host` |
| `failureRedirect` en `authenticate('google', ...)` | `.failure_redirect("/login?error=oauth")` en las opciones de OAuth2 (solo esta estrategia) |
| sobrescribir `OAuth2Strategy.userProfile` para otro proveedor | `userinfo_url` en los endpoints (el perfil es el documento `userinfo` de OpenID) |
| `requestNewAccessToken` de `passport-oauth2-refresh` | `OAuth2Client::new(options)?.refresh(refresh_token)`; no se guarda nada y nada se refresca por sí solo |

Las dos rutas (el login que redirige y el callback) llevan el mismo `AuthGuard::new("google")`. Ver
[OAuth2](/es/recipes/passport#oauth2-google-github-y-otros).

## No disponible

| En el original | En Corvis |
|---|---|
| mensajes flash (`failureFlash`, `successFlash`) | no se ofrecen. Un login fallido es el `401` simple; el motivo está en `Request::auth_info()` para que lo lea un handler o `.on_failure(..)`, nunca en la respuesta |
| `failureRedirect` y `successRedirect` en el guard | no se ofrecen en `AuthGuard`. Redirige desde el handler tras un login; la estrategia OAuth2 tiene `.failure_redirect(url)` |
| `passReqToCallback` | no hace falta: la request siempre se pasa al validador |
| la opción `property`, `assignProperty` | no se ofrecen: el usuario siempre se guarda para `User<T>` y `Request::user()` |
| `session: false` en un guard | es al revés por defecto: un guard nunca escribe una sesión salvo que tenga `.log_in()` |
| `authInfo` con la forma de un error serializado | `Request::auth_info()` es JSON con un motivo fijo; nunca repite nada de la petición |
| callbacks de `req.login`, `req.logout(cb)` | `async`: `Authenticator::log_in(..).await?` y `log_out(..).await?` |

## Qué responde distinto, a propósito

Cada fila es un caso que los tests diferenciales graban del original y marcan como divergencia, más las comprobaciones al arrancar, que
no tienen una petición que grabar. Son las mismas diferencias que la referencia lista junto a cada estrategia, con el motivo.

### Núcleo de estrategias

| El original | Corvis | Por qué |
|---|---|---|
| un guard que nombra una estrategia desconocida responde `500` en cada petición; `AuthGuard()` sin valor por defecto solo registra un log | `init()` falla con un único error que lista cada ruta y el nombre que falta; dos estrategias con un mismo nombre, una cadena vacía y valores por defecto en conflicto también fallan | un error de escritura debe detener el despliegue, no aparecer como un `500` en producción |
| `AuthGuard` se ejecuta en un contexto WebSocket, donde tampoco funciona | deniega el mensaje con `Forbidden resource` y la aplicación avisa una vez por gateway | ninguna estrategia puede leer un mensaje de socket; fallar cerrado |
| `req.user` es una propiedad cualquiera de la request | el usuario tipado vive en su propio espacio; un guard posterior que fija su propio usuario hace que `User<T>` responda `401` | un solo guard decide la identidad de una petición |

### JWT

| El original (`passport-jwt`) | Corvis | Por qué |
|---|---|---|
| un token sin `exp` se acepta y nunca vence | se rechaza por defecto; `.allow_missing_exp()` lo acepta y registra una advertencia al arrancar | un token que no puede vencer no se puede revocar esperando |
| un `iat` no numérico pasa (solo se comprueba con `maxAge`) | falla | no se confía en un claim mal formado |
| un payload que es una cadena se pasa a `validate` | el payload debe ser un objeto JSON | un validador siempre lee claims |
| `fromHeader` solo encuentra nombres escritos en minúsculas | los nombres de cabecera se comparan sin distinguir mayúsculas | las cabeceras HTTP no distinguen mayúsculas |
| sin límite en la longitud del token | 8192 bytes por defecto, comprobados antes de decodificar | trabajo acotado para una petición sin autenticar |
| gana la primera cookie de un nombre | una cookie enviada dos veces no es un token | una cookie plantada desde un subdominio hermano no debe autenticar |
| el provider de claves y `validate` esperan para siempre | ambos tienen un plazo y responden un `503` igual para todo token | una dependencia lenta no debe retener todas las peticiones |
| cualquier longitud de secreto, `ES512` | un secreto HMAC de menos de 32 bytes detiene el arranque; `ES512` no está disponible | RFC 7518 sección 3.2; `jsonwebtoken` 9 no tiene `ES512` |
| las cabeceras `crit` y los segmentos mal formados llegan al provider de claves | se rechazan antes de él | el provider se ejecuta para cualquier token bien formado |

### Local

| El original (`passport-local`) | Corvis | Por qué |
|---|---|---|
| el usuario y la contraseña también se leen del query string | solo con `.allow_query_credentials()`, con una advertencia al arrancar; por defecto la credencial está ausente | una contraseña en una URL se escribe en los logs |
| una credencial numérica o booleana llega a `validate` | una credencial que no es una cadena está ausente | confusión de tipos en la búsqueda de un usuario |
| sin más límite que el tamaño del body | un usuario o una contraseña de más de 1024 bytes falla antes del validador | trabajo acotado antes de un hash |
| lo que hace `validate` decide la respuesta: un throw para un usuario desconocido y `false` para una contraseña incorrecta son dos respuestas | credenciales ausentes, una contraseña incorrecta y un usuario desconocido dan el mismo `401`, y el validador gasta una verificación de hash en ambos casos | sin enumeración de usuarios, ni por estado ni por tiempo |

### API keys

| El original (`passport-headerapikey`) | Corvis | Por qué |
|---|---|---|
| la estrategia se llama `headerapikey` | `"api-key"` (conserva el nombre anterior con `NAME`) | nombres consistentes |
| un prefijo se convierte en una expresión regular sin escapar: `Key+` quita solo `Key` y entrega `+<key>` a `verify` | el prefijo se quita literalmente | sin sorpresas por una expresión regular |
| una clave de cualquier longitud, con un byte NUL, llega a `verify` (fuente de cabecera o bearer) | de más de 512 bytes, o con un carácter de control, falla antes del validador | trabajo acotado, y ningún byte que un proxy y la aplicación lean distinto |
| una sola fuente, o gana la primera | se lee cada fuente configurada, y dos fuentes que llevan claves distintas fallan | un proxy que autentica por un lugar no se puede eludir por otro |
| solo una cabecera | por defecto solo la cabecera `X-Api-Key`; el query string es una fuente solo cuando se configura, con una advertencia | las claves en URLs se registran en logs |

### Sesiones y cookies firmadas

| El original (`express-session`, `cookie-signature`) | Corvis | Por qué |
|---|---|---|
| `destroy()` no envía `Set-Cookie` | se borra la cookie (el navegador la descarta) y se destruye el registro | una cookie obsoleta no debe quedar |
| una sesión sin modificar no envía `Set-Cookie` tras una rotación de secretos | se envía firmada con el secreto actual | la rotación se completa sin esperar a una escritura |
| se llama a `store.destroy` con el identificador de una sesión que nunca se guardó | no se llama | ninguna llamada al almacén para nada |
| al hacer logout `passport` guarda la sesión sin el usuario y luego la regenera | el registro anterior se destruye y se crea el nuevo, sin guardado intermedio | una ventana menos con un registro a medio cerrar |
| sin `SameSite`, sin `Secure`, sin timeouts, sin límite de tamaño | `SameSite=Lax`, `Secure`, timeouts por inactividad y absolutos, un límite de tamaño por registro (ver [Diferencias con el original](/es/http/session#diferencias-con-el-original)) | valores por defecto seguros |
| un login a través del guard permanece cuando un guard posterior o el handler rechazan la petición | se descarta cuando la petición se rechaza | ningún login a medio completar |
| se verifica un valor firmado de cualquier longitud | una entrada de más de 4096 bytes se rechaza antes de calcular ningún HMAC | trabajo acotado |

### OAuth2

| El original (`passport-oauth2` y sus perfiles) | Corvis | Por qué |
|---|---|---|
| `state` y PKCE solo con `state: true`, `pkce: true` y una sesión | siempre se envían: `state`, `code_challenge` y `code_challenge_method=S256`; no hay forma de desactivar `state` | CSRF de login e intercepción del código |
| un `callbackURL` relativo se resuelve contra la cabecera `Host` | el redirect URI es absoluto y está configurado | una cabecera `Host` falsificada no debe elegir adónde envía el proveedor el código |
| la petición de token lleva las cabeceras del cliente HTTP del original | además envía `Accept: application/json` y su propio `User-Agent` | las partes que importan (método, ruta, content type, campos del formulario en orden) son las mismas |
| el access token de Google va en la URL de la petición del perfil | una cabecera bearer para todo proveedor, nunca un token en una URL | las URL se registran en logs |
| GitHub: el e-mail principal como `{ value }`, o todos con `allRawEmails` | cada dirección como `{ value, primary, verified }`, y una marca `verified` en cada e-mail; `verified_email()` | vincula cuentas solo por una dirección verificada |
| un `error` del proveedor distinto de `access_denied` responde `500` | `401` | lo causó la petición, no el servidor |
| el endpoint del token que rechaza el código (400 o 401) responde `500` | `401` | un código reutilizado o falsificado es culpa de la petición |
| un callback con un state que no es de este navegador responde `403` a través de `passport` simple | `401` a través de `AuthGuard` | el fallo uniforme |

**Mantener esta lista al día.** Las filas salen de las marcas `divergence` de `crates/corvis/tests/fixtures/auth/*.json`. Para
listarlas (desde la raíz del repositorio), cuando un cambio posterior añada un caso:

```sh
node -e "const fs=require('fs');const w=(o,f)=>{if(o&&typeof o==='object'){if(o.divergence)console.log(f,'::',o.name||'','::',typeof o.divergence==='string'?o.divergence:o.divergence.reason||JSON.stringify(o.divergence));for(const k in o)w(o[k],f)}};for(const f of fs.readdirSync('crates/corvis/tests/fixtures/auth').filter(f=>f.endsWith('.json')&&!f.startsWith('package')))w(JSON.parse(fs.readFileSync('crates/corvis/tests/fixtures/auth/'+f,'utf8')),f)"
```

## Ejecutar ambos lado a lado

Mover una ruta a la vez funciona cuando ambas aplicaciones aceptan las mismas credenciales. Pon un reverse proxy delante, envía un
prefijo de ruta a Corvis y el resto a Node, y mueve un prefijo por release.

**Sesiones.** Conserva el nombre de cookie `connect.sid` mientras ambas aplicaciones se ejecutan: una cookie de sesión llamada `__Host-sid` en el
lado de Corvis no se enviaría al lado Node. Cada lado lee la cookie cuando tiene:

- el mismo nombre y los mismos secretos: cada uno de al menos 32 bytes en Corvis, el primero firma y todos verifican. Un secreto de
  Node que es más corto hay que reemplazarlo primero. Pon el secreto nuevo primero en el lado Node y espera a que venzan las cookies firmadas
  con el anterior (el timeout absoluto); entonces entrega a Corvis la lista sin el secreto corto; los usuarios cuyas cookies se firmaron
  con él inician sesión de nuevo;
- un almacén al que ambos puedan llegar, con el formato de registro de `express-session`: la clave `passport.user` contiene lo que devolvió `serializeUser`,
  y la clave extra `__corvis` es un dato más para Node. Corvis todavía no tiene un almacén de Redis (#170), así que escribe un `SessionStore` sobre el
  Redis o la base de datos que tu aplicación Node ya usa, con el mismo prefijo de claves y las mismas reglas de vencimiento;
- la misma forma guardada para el usuario: si Node guarda el id suelto (`7`), `serialize_user` devuelve `json!(7)` y
  `deserialize_user` lee un número. Envuelve en un objeto, en ambos lados, un id que pueda ser `0`, o Corvis no lo restaurará
  (un usuario que se serializa a `0`, `""`, `false` o `null` nunca se autentica).

Cuando se haya movido la última ruta, cambia el nombre de la cookie a `__Host-sid` y haz que los usuarios inicien sesión una vez más. Ver
[Lectura cruzada con una app Node](/es/http/session#lectura-cruzada-con-una-app-node).

**Contraseñas.** Los hashes hechos por `bcryptjs` se verifican en Corvis tal cual. Verifica con `verify_or_dummy` y llama a `needs_rehash`
tras un login exitoso: el hash guardado pasa a ser argon2id, y **Node ya no puede leerlo**. Mientras Node todavía tenga que leer
hashes nuevos, configura `PasswordHasherOptions::bcrypt(10)`: escribe hashes `$2b$` que `bcryptjs.compareSync` acepta, y
`needs_rehash` deja de pedir un cambio. Define `dummy_algorithm` con el algoritmo y el costo de los hashes que realmente están
guardados, para que un usuario desconocido tarde lo mismo que uno conocido. Ver [Cifrado y hashing](/es/security/encryption-and-hashing).

**Tokens.** Corvis verifica lo que firmó `jsonwebtoken` cuando tiene el mismo secreto y algoritmo (`HS256` por defecto en el
ejemplo) y un claim `exp`, y cuando las listas de issuer y audience que configuras coinciden con lo que Node pone en el token. Un secreto
de menos de 32 bytes se rechaza. Corvis también puede firmar tokens que Node verifica, así que los access tokens pueden venir de cualquiera de los lados.
Los tokens sin `exp` necesitan `.allow_missing_exp()` mientras estén en circulación, y la advertencia al arrancar lo dirá.

**API keys.** La cabecera por defecto es la misma (`X-Api-Key`); el nombre de la estrategia cambió a `"api-key"`. Si tus guards en Node
se refieren a `headerapikey`, conserva ese nombre con `NAME`. Corvis compara digests, así que guarda un SHA-256 de cada clave (`ApiKey::sha256_hex`)
en lugar de la clave.

**OAuth2.** El client id y el secret siguen igual, pero el redirect URI es la URL de la ruta de callback de Corvis, así que regístrala
con el proveedor (el proveedor puede tener ambas) antes de que se mueva una ruta que inicia sesión. El `state` de un login vive en una cookie del
lado de Corvis o en su sesión, nunca compartido con Node: un login iniciado en un lado termina en ese lado.

## Lista de comprobación

- Todo secreto tiene al menos 32 bytes y viene del entorno, no del repositorio.
- Los tokens llevan `exp`, y las listas de issuer y audience coinciden.
- El nombre de la cookie de sesión, los secretos, el almacén y la forma de `passport.user` se comparten mientras ambos stacks se ejecutan.
- Los hashes de contraseña se verifican con `verify_or_dummy` y se actualizan con `needs_rehash`, o la opción bcrypt mantiene a Node leyendo.
- Las API keys se guardan como digests, y el nombre de la estrategia es el que esperan tus clientes.
- Los redirect URI están registrados para el nuevo host.
- Toda ruta de login tiene el throttler **antes** del `AuthGuard` ([Limitación de tasa](/es/security/rate-limiting#proteger-una-ruta-de-login)).
- Las cookies tienen `Secure`, `HttpOnly`, `SameSite=Lax` y, una vez terminada la migración, el prefijo `__Host-`.
- Las rutas que Node protegía globalmente están protegidas aquí también: un `AuthGuard` global con `.skip_public()`, o
  el [route posture](/es/security/route-posture) exigido.
- Has leído [Asegurar la autenticación](/es/security/authentication-security).
