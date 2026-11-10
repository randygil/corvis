---
status: documented
---

# Modelo de amenazas de autenticación

Esta página indica qué protege el módulo de autenticación, de quién, con qué mecanismo y qué prueba lo demuestra, y qué
deja a cargo de la aplicación. Cubre `corvis::auth` (el núcleo de estrategias y las estrategias JWT, local, API key,
sesión y OAuth2), las cookies firmadas que usan y el throttler delante de las rutas de login. Léela cuando decidas si
puedes confiar en el módulo para una ruta dada, y antes de escribir un guard de autorización, una ruta de login o un
handshake de WebSocket.

Cada estrategia se revisó por separado cuando se escribió; los hallazgos de esas revisiones se enlazan desde el
[registro](#hallazgos), no se repiten. Esta página añade lo que solo existe **entre** los módulos: una cookie de sesión
junto a un token, las cadenas, el orden de los guards, los lugares de credenciales compartidos y los sockets. Cada fila
nombra el archivo de pruebas o el escenario de la especificación que falla cuando se quita la mitigación. La guía para
quien usa el módulo está en [Autenticación](/es/security/authentication), la [receta de Passport](/es/recipes/passport),
[Sesión](/es/http/session) y [Rate limiting](/es/security/rate-limiting).

## Alcance y supuestos

Dentro del alcance: el código de `crates/corvis/src/auth/`, `crates/corvis/src/jwt.rs`,
`crates/corvis/src/http/signed_cookie.rs`, el bucle de guards del router, los espacios de usuario de `Request` y los
hooks del throttler que usa el módulo.

Supuestos en los que se apoya el módulo:

- **TLS termina en tu servidor o en un proxy en el que confías.** Las cookies son `Secure` por defecto; con HTTP plano en
  desarrollo lo desactivas y el módulo registra una advertencia.
- **El generador aleatorio del sistema operativo funciona.** Los identificadores, los valores de state, los verificadores
  PKCE y las sales de contraseñas salen de él, y un fallo es un error, nunca un respaldo más débil.
- **Los secretos son secretos.** El módulo rechaza los cortos, nunca los imprime, y no puede proteger una aplicación cuyo
  `JWT_SECRET` o secreto de sesión está en el repositorio.
- **`trust proxy` está bien configurado.** El throttler y `Request::ip()` creen en `X-Forwarded-For` solo de los proxies
  que listas.
- **El almacén de usuarios es de la aplicación.** El módulo autentica una petición contra un valor de usuario que
  devuelve tu validador; no decide quién puede existir, qué puede hacer un usuario ni cuándo se bloquea una cuenta.

Fuera del alcance, con el motivo: la autenticación multifactor, el CAPTCHA, la comprobación de contraseñas filtradas y el
bloqueo de cuentas (política de la aplicación; la [página de rate limiting](/es/security/rate-limiting#proteger-una-ruta-de-login)
muestra lo que puede hacer el throttler), los tokens CSRF (ver [Protección CSRF](/es/security/csrf)), una prueba de
penetración externa o una auditoría del interior de las crates criptográficas, y un fuzzer guiado por cobertura (el
repositorio hace fuzzing con pruebas de propiedades, ver [Hallazgos](#hallazgos)).

## Activos

| Activo | Dónde vive | Qué gana un atacante |
|---|---|---|
| Contraseñas | cuerpos de las peticiones, tu almacén de usuarios (como hashes) | toma de la cuenta en este y en otros sitios |
| Secretos de firma (secreto HMAC del JWT, `CookieKeys`, claves de state de OAuth2) | tu configuración | falsificar tokens, cookies o states de cualquier usuario |
| Identificadores de sesión | la cookie de sesión firmada, el almacén de sesiones | tomar una sesión |
| Tokens (JWT, API keys, tokens de acceso y de refresco de OAuth2) | cabeceras de las peticiones, tus almacenes | actuar como el usuario o el servicio hasta que expiren |
| Client secret de OAuth2 | tu configuración | hacerse pasar por tu aplicación ante el proveedor |
| La identidad de usuario de una petición (`Request::user()`, `User<T>`) | la petición | cada decisión de autorización posterior |
| Logs | tu destino de logs | cada credencial que se imprimió |

## Fronteras de confianza

| Frontera | Se confía en ella para | No se confía en ella para |
|---|---|---|
| Navegador | guardar sus propias cookies | nada de lo que envía: cada cabecera, cookie, parámetro de query y body lo controla el atacante |
| Proxy inverso | la dirección del cliente (cuando `trust proxy` lo lista), la terminación de TLS | nada más; puede añadir cabeceras, pero debería quitar aquellas de las que lees credenciales |
| La aplicación y su configuración | todo | un error del desarrollador es el principal riesgo al que apuntan los valores por defecto y las comprobaciones de arranque del módulo |
| Almacén de sesiones | guardar los registros que escribió la aplicación | la disponibilidad (una caída es un error en cada petición que lleva una cookie de sesión, rutas públicas incluidas), la confidencialidad de los registros |
| Proveedor de OAuth2 | los tokens y el perfil que devuelve tras un código válido | el texto de sus errores: nada de lo que dice llega a una respuesta, un log o un error |
| Logs | que un operador los lea | estar libres de credenciales: el módulo oculta lo que sabe que son credenciales |

**¿Está dentro del alcance un almacén de sesiones comprometido?** En parte. Un atacante que puede **leer** el almacén
obtiene los identificadores de las sesiones vivas, pero no la clave de firma, así que con el almacén solo no puede
construir la cookie firmada que envía un navegador: la cookie es `s:<id>.<firma>` y el módulo verifica la firma antes de
cualquier acceso al almacén. Los registros también contienen el valor serializado del usuario (normalmente un id) y lo
que tu aplicación haya puesto en la sesión, así que no guardes secretos en ella. Un atacante que puede **escribir** en el
almacén puede crear una sesión para cualquier usuario: el almacén es tan confiable como la base de datos de usuarios. Usa
un almacén con los mismos controles de acceso y el almacén de Redis o de base de datos que prefieras; el almacén en
memoria es por proceso.

## Atacantes

- **Atacante anónimo de red**: envía cualquier petición, en cualquier cantidad, con cualquier credencial: adivinación,
  credential stuffing, repetición, entrada malformada, entrada sobredimensionada, peticiones entre sitios a través del
  navegador de una víctima.
- **Usuario autenticado**: intenta actuar como otro usuario, conservar una sesión tras el logout o llegar a rutas
  pensadas para otro rol.
- **Página web maliciosa**: hace que el navegador de la víctima envíe peticiones con sus cookies (CSRF, CSRF de login,
  cookie tossing desde un subdominio hermano).
- **Endpoint de proveedor OAuth2 malicioso o comprometido**: responde con documentos sobredimensionados, lentos u
  hostiles.
- **Atacante con acceso de lectura a los logs, a los reportes de errores o al almacén de sesiones**: busca credenciales.

No se considera: un atacante que controla el proceso del servidor, el generador del sistema operativo o la compilación.

## Amenazas por superficie

Cada tabla lista la amenaza, lo que hace el módulo al respecto, la prueba o el escenario de la especificación que lo
demuestra y lo que queda. "Spec" significa el escenario de la capacidad de OpenSpec indicada, que se conserva en
`openspec/specs/` una vez archivado el cambio.

### Núcleo de estrategias

| Amenaza | Mitigación | Verificado por | Riesgo residual |
|---|---|---|---|
| Una estrategia que falla abierta deja pasar una petición | `Fail` pasa a la siguiente estrategia, `Error` termina la cadena, un panic es un error; ninguna estrategia incluida devuelve `Pass` | `crates/corvis/tests/auth_guard.rs`, `crates/corvis/tests/auth_hardening.rs`, `crates/corvis/tests/auth_invariants.rs` (`built_in_strategies_never_pass`) | una estrategia propia que devuelve `Pass` o tiene éxito con un usuario que no comprobó |
| Un usuario que se serializa como `null`, `false`, `0` o `""` se autentica | un `Success` así es un fallo, en toda estrategia | `crates/corvis/tests/auth_empty_user.rs` | ninguno |
| El motivo del fallo llega al cliente | la respuesta es el `401 {"message":"Unauthorized","statusCode":401}` simple; el motivo solo está en `Request::auth_info()` | `crates/corvis/tests/auth_invariants.rs` (`reasons_are_indistinguishable`, `chain_position_is_not_revealed`) | un closure `on_failure` que devuelve el fallo es decisión de la aplicación |
| Una respuesta de guard se usa fuera de la fase de guards | `respond_with` solo funciona dentro de `can_activate` | `crates/corvis/tests/auth_hardening.rs` | ninguno |
| El nombre de una estrategia está mal escrito o queda oculto tras un guard envoltorio | los nombres se comprueban al arrancar, también a través de `CanActivate::inner_guards` | `crates/corvis/tests/auth_guard.rs` | un envoltorio que no expone `as_any` ni `inner_guards` no se comprueba hasta su primera petición |
| Credenciales en un query string llegan a los logs | las estrategias declaran sus parámetros de credenciales; el log de peticiones, el `Debug` de una petición y el mensaje 404 del router los ocultan | `crates/corvis/tests/auth_api_key_log.rs`, `crates/corvis/tests/auth_oauth2_log.rs`, `crates/corvis/tests/fuzz_auth.rs` (`canary_secrets`) | el framework no conoce los parámetros que lee tu propio código |

### JWT

| Amenaza | Mitigación | Verificado por | Riesgo residual |
|---|---|---|---|
| Confusión de algoritmo, `none`, una clave de otra familia | una lista de algoritmos permitidos por familia de claves; `crit` y las cabeceras malformadas se rechazan antes del proveedor de claves | `crates/corvis/tests/auth_jwt.rs`, `crates/corvis/tests/auth_jwt_header.rs` | ninguno conocido |
| Secreto HMAC débil | un secreto de menos de 32 bytes detiene la aplicación al arrancar | `crates/corvis/tests/auth_jwt_options.rs`, `crates/corvis/tests/auth_defaults.rs` (`secret_minimum`) | un secreto largo que se puede adivinar |
| Token sin expiración, expirado o aún no válido | `exp` es obligatorio, `nbf` se respeta, `clock_tolerance` es cero por defecto y está limitado a 300 s | `crates/corvis/tests/auth_jwt.rs` | un token sigue siendo válido hasta `exp`: no hay revocación |
| Entrada sin verificar sobredimensionada o con anidamiento profundo | `max_token_length` 8192; la cabecera y el payload sin verificar se parsean sin `serde_json::Value` | `crates/corvis/tests/auth_jwt_header.rs` | ninguno |
| El token en una cookie es una credencial ambiental | los extractores son explícitos; un extractor de cookie está documentado como superficie de CSRF | `crates/corvis/tests/auth_jwt.rs` | `SameSite` y la protección CSRF son de la aplicación |
| Credencial en la URL | el extractor de query es opcional y registra una advertencia al arrancar | `crates/corvis/tests/auth_jwt_log.rs` | la URL sigue estando en los logs de los proxies |
| `JwtService` (el módulo anterior) es más laxo que la estrategia | `verify` aplica `iss` y `aud` de forma estricta cuando están configurados y rechaza `crit`; allí todavía se aceptan secretos de menos de 32 bytes y la falta de `exp` | `crates/corvis/tests/auth_jwt.rs` | en seguimiento en #174; usa la estrategia JWT para autenticar |

### Contraseñas y login local

| Amenaza | Mitigación | Verificado por | Riesgo residual |
|---|---|---|---|
| Descifrado sin conexión de hashes robados | argon2id (19456 KiB, 2 iteraciones, 1 lane) para hashes nuevos, verificación de bcrypt y argon2 | `crates/corvis/tests/auth_password.rs` | contraseñas débiles |
| Un hash plantado encarece la verificación | límites de verificación (memoria de argon2, iteraciones, lanes, coste de bcrypt, longitud del hash) comprobados antes de cualquier derivación | `crates/corvis/tests/auth_password.rs` | ninguno |
| Enumeración de usuarios por tiempo o por respuesta | `verify_or_dummy` verifica contra un hash falso para un usuario desconocido; un solo `401` para cualquier motivo | `crates/corvis/tests/auth_local.rs`, `crates/corvis/tests/auth_invariants.rs` (`reasons_are_indistinguishable`) | un validador que hace trabajo extra para los usuarios que existen |
| Agotamiento de recursos mediante hashing | concurrencia y espera acotadas; las credenciales de más de 1024 bytes nunca llegan al validador | `crates/corvis/tests/auth_password.rs`, `crates/corvis/tests/auth_local.rs` | un atacante aún puede ocupar los huecos: limita la tasa de la ruta |
| Credenciales en una URL | desactivado por defecto, advertencia al arrancar cuando se activa | `crates/corvis/tests/auth_local.rs` | ninguno |
| Adivinación de contraseñas | ver [Control de intentos de login](#control-de-intentos-de-login) | `crates/corvis/tests/auth_login_throttle.rs` | dimensionar el límite es tarea de la aplicación |

### API keys

| Amenaza | Mitigación | Verificado por | Riesgo residual |
|---|---|---|---|
| Tiempo de la comparación | comparación de digests en tiempo constante con `subtle`; `ExpectedApiKey` necesita al menos 32 bytes | `crates/corvis/tests/auth_api_key.rs`, `crates/corvis/tests/auth_defaults.rs` (`secret_minimum`) | la búsqueda propia del validador |
| Dos fuentes con claves distintas, o un valor inutilizable, dejan que decida otra fuente | los valores en conflicto o inválidos hacen fallar el intento | `crates/corvis/tests/auth_api_key.rs` | ninguno |
| La clave llega a los logs por la URL o por `Debug` | las fuentes de query son opcionales y se ocultan en todas partes; `Debug` no imprime ningún valor | `crates/corvis/tests/auth_api_key_log.rs`, `crates/corvis/tests/auth_invariants.rs` (`debug_of_every_public_type`) | ninguno |
| Una petición sin autenticar cuesta una búsqueda | el validador se ejecuta para cualquier clave bien formada y lo acota `validator_timeout` | `crates/corvis/tests/auth_invariants.rs` (`unauthenticated_cost_of_each_route_shape`) | la búsqueda de la aplicación es alcanzable sin credenciales: limita su tasa |

### Cookies firmadas y sesiones

| Amenaza | Mitigación | Verificado por | Riesgo residual |
|---|---|---|---|
| Cookie falsificada o alterada | HMAC-SHA256 con claves de al menos 32 bytes, verificación en tiempo constante, hasta 8 claves para rotación | `crates/corvis/tests/signed_cookies.rs`, `crates/corvis/tests/auth_session.rs` | el nombre de la cookie no forma parte de la firma: usa claves separadas por cookie |
| Fijación de sesión | un identificador nuevo en el login y en el logout; el registro anterior se destruye | `crates/corvis/tests/auth_session.rs`, `crates/corvis/tests/auth_invariants.rs` (`callback_that_logs_in`) | ninguno |
| Una petición fallida crea o renueva una sesión | un login fallido no escribe nada ni pone cookie; se permite borrar cookies y la renovación con `rolling` | `crates/corvis/tests/auth_invariants.rs` (`failed_login_with_an_anonymous_session`) | ninguno |
| Un login que un guard posterior rechaza sigue con la sesión iniciada | un login hecho por un guard se conserva solo cuando la petición no se rechaza | `crates/corvis/tests/auth_session_review.rs`, `crates/corvis/tests/auth_session_review2.rs` | decide un handler que inicia sesión por sí mismo |
| Sesión robada del almacén o de la cookie | `HttpOnly`, `Secure`, `SameSite=Lax`; timeouts de inactividad de 24 h y absoluto de 7 días | `crates/corvis/tests/auth_defaults.rs` (`cookies_set_by_the_module`) | la cookie es una credencial ambiental: la protección CSRF es tuya |
| Caída del almacén o un registro hostil | cada llamada está acotada por `store_timeout`; los registros tienen límites de tamaño y profundidad; una carga fallida es un `500` | `crates/corvis/tests/auth_session_http.rs`, `crates/corvis/tests/auth_session_contention.rs` | con el almacén caído, falla toda petición que lleva una cookie de sesión, rutas públicas y health checks incluidos |
| Inundación del almacén en memoria | `max_sessions`, límites por usuario y un presupuesto total de bytes | `crates/corvis/tests/auth_session_memory_heap.rs` | una inundación anónima puede expulsar sesiones anónimas; usa un almacén compartido en producción |
| CSRF de login | `SameSite=Lax` no lo detiene | `docs/security/csrf.md` | la aplicación añade una comprobación de mismo origen o un token a las rutas de login |

### OAuth2

| Amenaza | Mitigación | Verificado por | Riesgo residual |
|---|---|---|---|
| CSRF de login y un callback falsificado | `state` ligado al navegador (cookie `__Host-` firmada o la sesión), comprobado antes de cualquier petición al proveedor, de un solo uso | `crates/corvis/tests/auth_oauth2.rs`, `crates/corvis/tests/auth_invariants.rs` (`failed_oauth2_callback`, `forged_callback_on_an_authenticated_session`) | ninguno |
| Intercepción del código | PKCE `S256` siempre activo | `crates/corvis/tests/auth_oauth2.rs` | ninguno |
| Repetición de un callback | un state ya canjeado se rechaza, también mientras la primera petición está en curso y tras el logout | `crates/corvis/tests/auth_invariants.rs` (`callback_in_flight_twice_reaches_the_provider_once`, `callback_replayed_after_logout`) | ninguno |
| Open redirect mediante `return_to` o `failure_redirect` | solo una ruta relativa bajo una regla fija, o una URL `https` absoluta configurada | `crates/corvis/tests/auth_oauth2.rs`, `crates/corvis/tests/fuzz_auth.rs` (`redirect_targets`) | la lista de redirect URI permitidos está en seguimiento en #172 |
| Un proveedor hostil responde con documentos enormes, lentos o con redirecciones | sin redirecciones, sin variables de proxy, plazos y límites de tamaño, verificación de TLS | `crates/corvis/tests/auth_oauth2.rs`, `crates/corvis/tests/auth_oauth2_review.rs` | ninguno |
| Tokens en logs, errores o `Debug` | el `Debug` de grants y tokens imprime solo nombres; el texto del proveedor nunca llega a una respuesta | `crates/corvis/tests/auth_oauth2_log.rs`, `crates/corvis/tests/auth_invariants.rs` (`debug_of_every_public_type`) | el validador recibe los tokens: guárdalos cifrados |
| Iniciar logins cuesta recursos al servidor | un inicio de login con el state en la sesión escribe un registro; el número de peticiones en curso está acotado | `crates/corvis/tests/auth_login_throttle.rs` (`oauth2_login_start_over_the_limit`) | limita la tasa de la ruta de login |
| El state y la regeneración de la sesión no coinciden | el registro del state se elimina antes de contactar al proveedor, el identificador cambia en el login y no sobrevive ninguna clave `oauth2:` | `crates/corvis/tests/auth_invariants.rs` (`callback_that_logs_in`) | ninguno |

### Combinaciones de estrategias

Son las interacciones que ninguna estrategia ve por sí sola.

| Amenaza | Mitigación | Verificado por | Riesgo residual |
|---|---|---|---|
| Una cookie de sesión da una identidad que el guard de la ruta no estableció | después de ejecutarse un `AuthGuard`, el usuario es el que ese guard fijó, o ninguno: un guard de token opcional borra el usuario de la sesión, un guard de token lo reemplaza, una ruta solo de token responde `401` a una cookie sola | `crates/corvis/tests/auth_invariants.rs` (`session_of_one_user_and_token_of_another`, `optional_token_guard_with_a_session_cookie`, `token_only_route_with_only_a_session_cookie`, `session_strategy_in_the_chain`) | ninguno |
| El usuario de sesión en rutas sin `AuthGuard` | ver [el usuario de sesión](#el-usuario-de-sesion-en-rutas-sin-authguard) | `crates/corvis/tests/auth_invariants.rs` (`roles_guard_without_an_auth_guard`) | un guard escrito a mano que confía en `Request::user()` admite por sí solo una cookie de sesión |
| Caída a una estrategia más débil tras un fallo de una dependencia | un `Error` termina la cadena, así que una caída del proveedor o del almacén nunca pasa a la siguiente | `crates/corvis/tests/auth_invariants.rs` (`failing_dependency_before_a_weaker_strategy`) | ninguno |
| Una estrategia OAuth2 en una cadena | inicia un login para cualquier petición que no sea un callback, así que termina la cadena con una redirección | `crates/corvis/tests/auth_invariants.rs` (`an_oauth2_strategy_in_a_chain_redirects_instead_of_falling_through`) | reserva dos rutas para OAuth2 y mantenla fuera de las cadenas; en el último lugar convierte cada petición anónima en un inicio de login |
| Dos estrategias leen el mismo lugar de forma distinta | el lector bearer del JWT y la fuente `bearer()` de la API key parsean `Authorization` igual; las cabeceras repetidas se unen como lo hace Node | `crates/corvis/tests/auth_invariants.rs` (`bearer_value_shared_by_two_strategies`), `crates/corvis/tests/fuzz_auth.rs` (`gate_matches_model`) | el lector de API key también se rinde ante una cabecera con más de 640 bytes de espacio inicial, lo que solo lo hace más estricto |
| Una credencial enviada en el lugar equivocado autentica | una credencial autentica solo mediante la estrategia configurada para su lugar | `crates/corvis/tests/auth_invariants.rs` (`token_in_a_place_that_is_not_configured`) | ninguno |
| Un registro de sesión dejado por el state de OAuth2 autentica | el state nunca se convierte en usuario; la sesión anterior se destruye en el login | `crates/corvis/tests/auth_invariants.rs` (`callback_that_logs_in`) | ninguno |
| Cookies puestas en una petición fallida | una petición fallida no recibe ninguna cookie de sesión o de state que otorgue autoridad | `crates/corvis/tests/auth_invariants.rs` (`failed_login_with_an_anonymous_session`, `failed_oauth2_callback`) | la cookie de state se devuelve tal como estaba en un callback falsificado, así que el navegador conserva su flujo genuino |
| Una petición aleatoria llega a un handler al que no debería | un modelo de toda la cuadrícula de formas de guards frente a credenciales aleatorias | `crates/corvis/tests/fuzz_auth.rs` (`gate_matches_model`) | el modelo y el código podrían equivocarse de la misma manera: los escenarios anteriores están escritos a mano |
| Un secreto aparece en la salida | marcadores en cada lugar de credenciales de cada ruta, con los secretos reales de la aplicación | `crates/corvis/tests/fuzz_auth.rs` (`canary_secrets`) | credenciales que registra tu propio código |

### Orden de los guards y rutas públicas

| Amenaza | Mitigación | Verificado por | Riesgo residual |
|---|---|---|---|
| Un throttler que se ejecuta después de un `AuthGuard` no limita nada | una petición limitada no ejecuta ninguna estrategia, ninguna derivación de claves, ninguna escritura en el almacén ni ninguna petición al proveedor cuando el throttler va primero; el arranque registra una advertencia para un `AuthGuard` que se ejecuta antes de un `ThrottlerGuard` en una ruta | `crates/corvis/tests/auth_login_throttle.rs`, `crates/corvis/tests/auth_guard_order_log.rs` | un guard envoltorio que oculta ambos guards oculta la advertencia |
| `#[public]` abre una ruta que necesitaba autenticación | `#[public]` solo lo respeta un guard construido con `.skip_public()`; un guard construido sin él autentica también las rutas públicas | `crates/corvis/tests/auth_invariants.rs` (`invalid_credentials_on_a_public_route`, `controller_marked_public_handler_not_marked`) | un guard global con `.skip_public()` confía en cada marca `#[public]` |
| Una ruta con solo un guard opcional cuenta como protegida | un guard opcional no cuenta como control de acceso para el posture de rutas | `crates/corvis/tests/auth_invariants.rs` (`posture_with_only_an_optional_guard`), `crates/corvis/tests/route_posture.rs` | un guard escrito a mano cuenta salvo que sobrescriba `controls_access` |
| Las peticiones anónimas son caras | un token que no se verifica o una cookie que no se verifica no cuestan ninguna llamada al validador, ninguna búsqueda, ningún acceso al almacén ni ninguna petición al proveedor | `crates/corvis/tests/auth_invariants.rs` (`unauthenticated_cost_of_each_route_shape`) | el validador de API key, la derivación de contraseñas y un inicio de login con el state en la sesión cuestan algo por diseño |

### Handshakes de WebSocket y Socket.IO

| Amenaza | Mitigación | Verificado por | Riesgo residual |
|---|---|---|---|
| Se ejecutan estrategias por un mensaje de socket | `AuthGuard` deniega cada mensaje sin ejecutar una estrategia, y la aplicación avisa una vez por gateway | `crates/corvis/tests/auth_ws.rs`, `crates/corvis/tests/auth_invariants.rs` (`global_auth_guard_and_a_gateway`) | ninguno |
| Una cookie de sesión autentica un upgrade | las peticiones de upgrade se responden antes del router: sin middleware, sin sesión, sin usuario en el handshake | `crates/corvis/tests/auth_invariants.rs` (`session_cookie_on_an_upgrade_request`) | autentica en `handle_connection` |
| El handshake acepta un token que el guard HTTP rechaza | el patrón documentado verifica el token con `JwtService`, cuyas reglas son más laxas que las de la estrategia JWT | no mitigado: el patrón está en `docs/security/authentication.md`, la brecha es #173 | en seguimiento en #173 (estrategias en el handshake de WebSocket) y #174; hasta entonces aplica las reglas de la estrategia JWT en `handle_connection` |
| Secuestro de WebSocket entre sitios | `GatewayOptions::allowed_origins` | `crates/corvis/tests/ws_security.rs` | opcional |
| El token en el query del handshake llega a los logs | el log de la línea de petición lo escribe `LoggingInterceptor`, que se ejecuta en las rutas HTTP: un handshake no lo registra | por construcción (`crates/corvis/src/common/interceptor.rs`) | los proxies y tu propio código del handshake aún pueden registrar la URL: prefiere una cabecera o el primer mensaje |
| Handshake de Engine.IO | las peticiones de Engine.IO se despachan antes del router, así que no se ejecuta ningún middleware, sesión o guard para ellas | por construcción (`crates/corvis/src/server.rs`); `crates/corvis/tests/engine_io.rs` muestra la petición de handshake que recibe el callback | autentica en el handler de conexión |
| Handshake de Socket.IO | pendiente: la capa de Socket.IO todavía no está integrada (#33) | ninguna todavía | se revisará cuando llegue; las estrategias en el handshake son #173 |

### Control de intentos de login

| Amenaza | Mitigación | Verificado por | Riesgo residual |
|---|---|---|---|
| Adivinación de contraseñas desde un cliente | el throttler `default` cuenta por dirección del cliente, con el `ttl` más corto | `crates/corvis/tests/auth_login_throttle.rs` (`password_guessing_by_one_client_over_the_client_limit`) | las direcciones detrás de un NAT compartido comparten un límite |
| Adivinación de contraseñas de una cuenta desde muchas direcciones | `login_tracker` cuenta por cuenta en la ruta de login | `crates/corvis/tests/auth_login_throttle.rs` (`same_account_from_many_addresses`) | cualquier cliente puede bloquear una cuenta durante la duración del bloqueo |
| El usuario movido a otro lugar esquiva el contador | el tracker lee el usuario que leerá la estrategia local, con las mismas opciones | `crates/corvis/tests/auth_login_throttle.rs` (`username_moved_to_the_query_string`, `nested_field_name`) | ninguno |
| El contador revela si una cuenta existe | contar no depende de la cuenta; el valor del tracker es un digest | `crates/corvis/tests/auth_login_throttle.rs` (`existing_and_unknown_accounts`, `storage_keys`) | ninguno |
| Intentos sin usuario | se cuentan por la dirección del cliente | `crates/corvis/tests/auth_login_throttle.rs` (`missing_username`) | ninguno |
| Una petición limitada aún cuesta una derivación | el throttler se ejecuta primero | `crates/corvis/tests/auth_login_throttle.rs` (`password_guessing_over_the_limit`) | ninguno |

## El usuario de sesión en rutas sin AuthGuard

El middleware de sesión restaura el usuario en cada ruta que cubre, antes de cualquier guard, igual que `passport.session()`
fija `req.user` en el original. La revisión decidió **conservarlo**, y fijarlo con una prueba, por estos motivos:

- **No añade autoridad.** El usuario restaurado es el que la misma cookie autentica mediante `AuthGuard::new("session")`;
  no se lee nada que una petición con esa cookie no pudiera usar ya, y todo `AuthGuard` de la ruta lo reemplaza o lo
  borra, de modo que el guard de la ruta decide la identidad (las primeras filas de la tabla de combinaciones).
- **La paridad es el objetivo del módulo.** Las aplicaciones migradas leen `req.user` en rutas sin guard (la
  personalización opcional es el caso común). Restaurar solo cuando se ejecuta la estrategia `"session"` las rompería en
  silencio.
- **La regla más estricta no cierra el riesgo real**, que es un guard de autorización escrito a mano que confía en
  `Request::user()` y no tiene un `AuthGuard` delante. Con la regla más estricta no vería ningún usuario y denegaría, lo
  cual es más seguro, pero haría que la identidad dependiera del orden de los guards. La regla a seguir es la misma en
  ambos casos: pon un `AuthGuard` primero, y un `AuthGuard::new("session")` a nivel de ruta antes de cualquier guard que
  lea el usuario.

La consecuencia, dicha con claridad: un guard que lee `Request::user()` en una ruta sin `AuthGuard` autentica solo por la
cookie de sesión, sin comprobar nada de la petición (así que un envío de formulario entre sitios la lleva). La prueba
`roles_guard_without_an_auth_guard` en `crates/corvis/tests/auth_invariants.rs` fija ese comportamiento. El posture de
rutas trata un guard así como control de acceso salvo que devuelva `false` en `controls_access`, así que el posture por
sí solo no te dice que la ruta sea segura. Ver [Sesión](/es/http/session#guards-y-el-usuario-de-la-sesion).

## Defaults

Una regla por fila, la misma en cada estrategia. `crates/corvis/tests/auth_defaults.rs` lee el bloque que sigue a la
tabla y compara cada línea con el código, así que un valor por defecto que cambia en un solo lugar hace fallar
`cargo test`.

| Ajuste | Valor por defecto | Se define en | Valor por defecto del original |
|---|---|---|---|
| Mínimo de secreto (HMAC del JWT, claves de cookies, API key esperada) | 32 bytes, error al arrancar | `jwt`, `http::signed_cookie`, `api_key` | ninguno |
| Longitud del token JWT | 8192 bytes | `JwtStrategyOptions::max_token_length` | ninguno |
| Tolerancia de reloj del JWT | 0 s (como máximo 300 s) | `JwtStrategyOptions::clock_tolerance` | 0 |
| `exp` del JWT | obligatorio | `JwtStrategyOptions::allow_missing_exp` | opcional |
| Longitud de la API key | 512 bytes (como máximo 4096) | `ApiKeyStrategyOptions::max_key_bytes` | ninguna |
| Timeout del validador de API key | 5 s | `ApiKeyStrategyOptions::validator_timeout` | ninguno |
| Fuente de la API key | la cabecera `X-Api-Key` | `ApiKeyStrategyOptions` | la cabecera |
| Longitud de usuario y contraseña | 1024 bytes cada uno | `LocalStrategyOptions` | ninguna |
| Hashes de contraseña nuevos | argon2id, 19456 KiB, 2 iteraciones, 1 lane; coste de bcrypt de al menos 10 | `PasswordHasherOptions` | bcrypt, sin mínimo |
| Límites de verificación | coste de bcrypt 14, argon2 65536 KiB, 8 iteraciones, 16 lanes, 512 bytes de hash | `VerifyLimits` | ninguno |
| Espera por un hueco de hashing | 10 s | `PasswordHasherOptions` | ninguna |
| Cookie de sesión | `connect.sid`, `Path=/`, `HttpOnly`, `Secure`, `SameSite=Lax` | `SessionCookie` | sin `Secure`, sin `SameSite` |
| Timeouts de sesión | inactividad 24 h, absoluto 7 días | `SessionOptions` | ninguno |
| Tamaño del registro de sesión y timeout del almacén | 16384 bytes, 5 s | `SessionOptions` | ninguno |
| Renovación de la sesión y datos en el login | `rolling` desactivado; los datos se descartan en el login | `SessionOptions` | `rolling` desactivado |
| State de OAuth2 | duración 600 s, PKCE `S256` | `OAuth2Options` | ninguno, opcional |
| Cookie de state de OAuth2 | `__Host-oauth2.<name>`: `HttpOnly`, `Secure`, `SameSite=Lax`, `Path=/` | `OAuth2Options` | ninguna |
| Límites de OAuth2 | código 2048 bytes, conexión 5 s, petición 10 s, documento de token 65536 bytes, perfil 262144 bytes, 32 peticiones en curso, 100000 states canjeados | `OAuth2Options`, `OAuth2HttpOptions` | ninguno |
| Credenciales en una URL | desactivado para toda estrategia | cada estrategia | activado en algunas |

```ini
jwt.min_secret_bytes = 32
jwt.max_token_length = 8192
jwt.clock_tolerance_seconds = 0
jwt.max_clock_tolerance_seconds = 300
jwt.require_exp = true
api_key.min_secret_bytes = 32
api_key.max_key_bytes = 512
api_key.validator_timeout_seconds = 5
api_key.default_source = X-Api-Key
cookie_keys.min_secret_bytes = 32
cookie_keys.max_secrets = 8
local.max_credential_bytes = 1024
local.argon2_memory_kib = 19456
local.argon2_iterations = 2
local.argon2_parallelism = 1
local.bcrypt_min_cost = 10
local.verify_max_bcrypt_cost = 14
local.verify_max_argon2_memory_kib = 65536
local.verify_max_argon2_iterations = 8
local.verify_max_argon2_parallelism = 16
local.verify_max_hash_bytes = 512
local.acquire_timeout_seconds = 10
session.cookie_name = connect.sid
session.cookie_path = /
session.cookie_http_only = true
session.cookie_secure = true
session.cookie_same_site = Lax
session.idle_timeout_seconds = 86400
session.absolute_timeout_seconds = 604800
session.max_session_bytes = 16384
session.store_timeout_seconds = 5
session.rolling = false
session.keep_session_info_on_login = false
oauth2.state_ttl_seconds = 600
oauth2.pkce = true
oauth2.state_cookie = HttpOnly; Secure; SameSite=Lax; Path=/
oauth2.max_code_bytes = 2048
oauth2.connect_timeout_seconds = 5
oauth2.timeout_seconds = 10
oauth2.max_token_bytes = 65536
oauth2.max_profile_bytes = 262144
oauth2.max_in_flight = 32
oauth2.redeemed_states_max = 100000
url_credentials.local = off
url_credentials.api_key = off
url_credentials.jwt = off
```

Diferencias conocidas entre estrategias, cada una con su motivo: el client secret de OAuth2 no tiene longitud mínima (lo
emite el proveedor); el extractor de cookie del JWT no tiene advertencia al arrancar mientras que el de query sí la tiene
(una cookie no está en una URL, pero ver la fila de CSRF de la tabla de JWT); `JwtService` acepta lo que la estrategia
JWT rechaza (#174).

## Dependencies

Las crates criptográficas que incorpora cada feature de autenticación, con la versión de `Cargo.lock`.
`crates/corvis/tests/auth_source_checks.rs` falla cuando cambia la versión mayor de una crate listada sin actualizar esta
tabla, y `deny.toml` prohíbe una segunda versión de cada crate marcada como "one generation" (`cargo deny check`).
Ningún aviso de la base de datos de RustSec se aplica a ninguna de ellas en el momento de la revisión (`cargo audit` y
`cargo deny check`, 2026-11-10); ninguna de las listas de excepciones (`deny.toml`, `.cargo/audit.toml`) nombra a alguna.

| Feature | Crate | Version | Features activadas | Función | Política |
|---|---|---|---|---|---|
| `jwt` | `jsonwebtoken` | 9.3.1 | `default`, `pem`, `simple_asn1`, `use_pem` | firma y decodificación de tokens | one generation |
| `jwt` | `ring` | 0.17.14 | `alloc`, `dev_urandom_fallback`, `std` | primitivas de firma bajo `jsonwebtoken` y rustls | one generation |
| `auth-local` | `argon2` | 0.5.3 | `alloc`, `default`, `password-hash`, `rand` | argon2id | one generation |
| `auth-local` | `blake2` | 0.10.6 | ninguna | hash dentro de argon2 | one generation |
| `auth-local` | `password-hash` | 0.5.0 | `alloc`, `rand_core` | formato de cadena PHC | one generation |
| `auth-local` | `bcrypt` | 0.17.1 | `getrandom`, `std`, `zeroize` | hashing y verificación con bcrypt | one generation |
| `auth-local` | `blowfish` | 0.9.1 | `bcrypt` | cifrado dentro de bcrypt | one generation |
| `auth-local` | `cipher` | 0.4.4 | ninguna | traits de cifrado por bloques | one generation |
| `auth-api-key` | `subtle` | 2.6.1 | `i128`, `std` | comparación en tiempo constante | one generation |
| `auth-api-key` | `sha2` | 0.10.9 | `std` | digests de claves | one generation |
| `auth-session` | `hmac` | 0.12.1 | ninguna | firmas de cookies | one generation |
| `auth-session` | `digest` | 0.10.7 | `alloc`, `mac`, `std`, `subtle` | traits bajo `hmac` y `sha2` | one generation |
| `auth-session` | `rand` | 0.8.8 | `getrandom`, `std`, `std_rng` | `OsRng` para identificadores y states | one generation |
| `auth-oauth2` | `tokio-rustls` | 0.26.6 | `ring`, `tls12` | cliente TLS para el proveedor | one generation |
| `auth-oauth2` | `rustls` | 0.23.46 | `ring`, `std`, `tls12` | TLS | one generation |
| `auth-oauth2` | `rustls-webpki` | 0.103.16 | `alloc`, `ring`, `std` | verificación de certificados | one generation |
| `auth-oauth2` | `webpki-roots` | 0.26.11 | ninguna | anclas de confianza; una versión que solo reexporta 1.0.11 | shim of 1.0.11, not in the list |

`openssl`, `openssl-sys` y `native-tls` están prohibidas, así que el cliente del proveedor, la terminación de TLS y la
verificación de tokens conservan una única pila revisada. Dos crates tienen más de una generación en el grafo y no están
en la lista de una sola generación, por estos motivos: `getrandom` (tres generaciones: el generador del sistema
operativo entra a través de `rand` 0.8, `bcrypt` y dependencias que no son crates de autenticación) y `rand_core` (0.6
para `rand` 0.8, 0.10 para otras crates); `Cargo.lock` también lista `rand` 0.10, que viene de `proptest` (solo
pruebas), y `webpki-roots` 1.0.11, que es lo que reexporta `webpki-roots` 0.26.11. Los secretos se generan con `OsRng` en
`crates/corvis/src/auth/session/session.rs`, `crates/corvis/src/auth/oauth2/state.rs` y
`crates/corvis/src/auth/local/hasher.rs`; la comprobación del código fuente falla si un archivo de los módulos de
autenticación usa otro generador.

## Hallazgos

Los hallazgos de las revisiones anteriores, hechas por cambio, con sus correcciones y pruebas de regresión, están en las
notas de cada cambio archivado (las secciones "Security review" de `design.md`): el núcleo de estrategias
(`openspec/changes/archive/2026-11-10-e3-strategy-core-and-guard-responses/`), JWT
(`openspec/changes/archive/2026-11-10-e3-jwt-strategy/`), local y hashing de contraseñas
(`openspec/changes/archive/2026-11-10-e3-local-strategy-and-password-hashing/`), API keys
(`openspec/changes/archive/2026-11-10-e3-api-key-strategy-and-custom-strategies/`), sesiones y cookies firmadas
(`openspec/changes/archive/2026-11-10-e3-sessions-and-signed-cookies/`) y OAuth2
(`openspec/changes/archive/2026-11-10-e3-oauth2-strategies/`).

Esta revisión miró solo las interacciones (identidad, cadenas, lugares compartidos, orden de guards y rutas públicas,
OAuth2 con sesiones, cookies en las rutas de fallo, postura frente a CSRF, sockets, valores por defecto, secretos en la
salida, uso de recursos, combinaciones de features). Un hallazgo se corrige en el cambio cuando la corrección queda
dentro de los módulos de autenticación y no cambia ningún requisito de otra capacidad; todo lo demás se registra como
issue. No se difirió ningún hallazgo de severidad alta o crítica.

| Id | Severidad | Componentes | Resultado |
|---|---|---|---|
| R-1 | Media | `crates/corvis/src/jwt.rs`: el `Debug` de `JwtModuleOptions` y `JwtSignOptions` imprimía el secreto HMAC y la clave privada PEM | Corregido: `Debug` muestra si hay un secreto o una clave definidos, nunca su valor. Prueba: `debug_of_every_public_type` en `crates/corvis/tests/auth_invariants.rs` falla sin la corrección |
| R-2 | Baja | la comprobación del orden de guards: un `ThrottlerGuard` después de un `AuthGuard` no limita nada, y nada lo advertía | Corregido: advertencia al arrancar, `ThrottlerGuard::as_any`. Pruebas: `crates/corvis/tests/auth_guard_order_log.rs` |
| R-3 | Baja | control de intentos de login: la receta por cuenta se podía esquivar moviendo el usuario al query string o a un campo anidado | Corregido: `login_tracker` lee lo que lee la estrategia. Pruebas: `crates/corvis/tests/auth_login_throttle.rs` |
| R-4 | Info | una estrategia OAuth2 en una cadena la termina con una redirección | Documentado aquí y en la [receta de Passport](/es/recipes/passport); fijado por `an_oauth2_strategy_in_a_chain_redirects_instead_of_falling_through` |
| R-5 | Info | el lector `bearer()` de la API key revisa como máximo `max_key_bytes + 128` bytes de espacio inicial, el lector de tokens cualquier cantidad | Nada que corregir: la diferencia hace al lector de API key más estricto, nunca más permisivo; fijado en `bearer_value_shared_by_two_strategies` |
| R-6 | Info | un callback de OAuth2 falsificado recibe de vuelta la cookie de state del propio navegador sin cambios (el flujo genuino sigue siendo utilizable) | Intencional; fijado en `failed_oauth2_callback` |
| #173 | Media | estrategias en el handshake de WebSocket: `AuthGuard` deniega los mensajes de socket y los handshakes autentican con `JwtService` | Issue registrado; usa `handle_connection` y las reglas de la estrategia JWT hasta que llegue |
| #174 | Baja | valores por defecto de `JwtService`: se aceptan secretos de menos de 32 bytes y tokens sin `exp` | Issue registrado; usa la estrategia JWT para autenticar |
| #172 | Baja | lista de redirect URI permitidos para OAuth2 | Issue registrado |
| #170, #171 | Info | almacenes de sesión en Redis y en un ORM | Issues registrados; el almacén en memoria es por proceso |

Issues propuestos por esta revisión, para que el mantenedor los registre: una caída del almacén de sesiones hace fallar las
rutas públicas y los health checks para los navegadores que llevan una cookie de sesión (baja, disponibilidad: restaurar
el usuario de sesión de forma perezosa u omitir la carga en las rutas `#[public]`); fuzzing guiado por cobertura de los
parsers de autenticación (`cargo fuzz`, informativo); atestaciones de la cadena de suministro para las crates
criptográficas (`cargo vet`, informativo).
