---
status: documented
---

# Postura de rutas

Olvidar `#[use_guards(...)]` es silencioso: la ruta funciona y cualquiera puede llamarla. La verificación de postura de
rutas convierte ese error en un error de arranque. Cuando la activas, la aplicación se niega a arrancar mientras alguna
ruta no tenga guard y no se haya abierto a propósito, y puede imprimir la postura de cada ruta para revisarla.

La verificación es opcional. Una aplicación que no la activa se comporta exactamente como antes.

## La regla

Cada ruta de controlador (cada método HTTP, alias de ruta y versión de URI), cada stream `#[sse]` y cada manejador
`#[subscribe_message]` de un gateway de WebSocket debe ser una de estas cosas:

- **protegida**: se ejecuta un guard que controla el acceso, registrado en cualquier nivel en que el router ejecuta guards
  (ver [Guards](/es/guards#orden)): `Provider::app_guard`, `use_global_guards`, el controlador o la ruta;
- **pública**: está marcada `#[public]`, en la ruta o en su controlador (o gateway);
- **exenta**: coincide con una exención que configuraste, con una razón.

Cualquier otra es **desprotegida**.

## Activarla

Llama a `require_route_posture()` antes de `init()` o `listen()`; las llamadas posteriores no tienen efecto:

```rust
let mut app = CorvisFactory::create::<AppModule>().await?;
app.require_route_posture();
app.listen("127.0.0.1:3000").await?;
```

`init()` (y por tanto `listen()` y `listen_in_background()`) falla entonces con un `RoutePostureError` antes de abrir el
servidor y antes de que se ejecute cualquier hook de módulo o de gateway. Los proyectos creados con
[`corvis new`](/es/cli/usages#corvis-new) ya la llaman, y `examples/tertulia` y `examples/cats-app` la exigen en
`create_app()`.

Para adoptarla paso a paso, usa el modo de advertencia. Registra la misma lista una vez, con nivel de advertencia, y la aplicación
arranca con normalidad. Las rutas protegidas fuera del framework pueden eximirse, siempre con una razón:

```rust
use corvis::{RequestMethod, RoutePostureOptions};

app.set_route_posture(
    RoutePostureOptions::warn()
        .exempt(RequestMethod::Get, "/metrics", "solo se consulta desde la red privada")
        .exempt(RequestMethod::All, "/internal/**", "el proxy inverso autentica antes de que la petición llegue"),
);
```

`RoutePostureOptions::enforce()` elige el modo estricto de la misma forma, y `off()` el modo por defecto.

- El patrón se compara literalmente con la plantilla completa de la ruta (con el prefijo global y la versión), segmento
  a segmento: un segmento literal coincide con el mismo literal (sin distinguir mayúsculas ASCII), `:id` o `{id}`
  coincide solo con un segmento `:param` (así `/files/:id` no exime `/files/export-all`), `*` como segmento completo es
  exactamente un segmento (`/api/*/internal/*`), `**` es cero o más segmentos (`/metrics/**`), y `*` dentro de un
  segmento (`/health*`) nunca cruza un `/`.
- `RequestMethod::All` coincide con cualquier método. Una ruta declarada con `#[all]` solo coincide con una exención `All`.
- La razón es obligatoria. Una razón en blanco hace fallar `init()`, en todos los modos salvo `off`.
- Una exención que no coincide con ninguna ruta registra una advertencia, y también la que coincide solo con rutas que ya
  tienen guard o son públicas (aceptaría esas rutas en silencio si se quitara el guard).
- La autenticación hecha en un middleware o en un proxy inverso es invisible para la verificación, porque el middleware se
  ejecuta antes del enrutado y no deja ninguna declaración que leer. Exime esas rutas con una razón que diga dónde vive la
  protección, como en el ejemplo `/internal/**`. La razón aparece en el informe, así que quien revise puede cuestionarla.
- Para mensajes de gateway usa `exempt_message(path, event, reason)`; ver [Guards de WebSocket](/es/websockets/guards#route-posture-en-gateways).

## Marcar rutas como públicas

`#[public]` va sobre un método de ruta dentro de `#[routes]`, o debajo de `#[controller]` para abrir todo el
controlador. No recibe argumentos y no cambia cómo se atienden las peticiones: guarda la entrada de metadatos
`PUBLIC_METADATA` (`"isPublic"`) con el valor `true`.

```rust
#[controller("auth")]
pub struct AuthController { /* ... */ }

#[routes]
impl AuthController {
    #[post("login")]
    #[public]
    fn login(&self, #[body] dto: LoginDto) -> HttpResult<LoginResponse> { /* ... */ }
}

// Todo el controlador, incluido su stream SSE:
#[controller("stats")]
#[public]
pub struct StatsController { /* ... */ }
```

El valor de una ruta gana al del controlador, así que `#[set_metadata(PUBLIC_METADATA, false)]` en una ruta la vuelve
privada otra vez dentro de un controlador público. `#[set_metadata(PUBLIC_METADATA, true)]`, la misma clave literal
escrita a mano y un [decorador personalizado](/es/custom-decorators#componer-metadatos-customdecorator) que la
establezca tienen el mismo efecto.

Como la clave es la que las aplicaciones ya usan, un guard global que se salta las rutas cuya metadata `"isPublic"` es
`true` sigue funcionando con `#[public]` sin cambios; ver [Rutas públicas con un guard global](/es/guards#rutas-publicas-con-un-guard-global)
y [Autenticación](/es/security/authentication#activar-la-autenticacion-en-todas-las-rutas). Ahí está también la trampa:
`#[public]` es una declaración para la verificación y para tus guards. Un guard que no lee la metadata seguirá
rechazando la ruta.

## Qué guards cuentan

Un guard cuenta solo cuando declara que controla el acceso. Los guards lo declaran por defecto. `ThrottlerGuard` y
`ZodGuard` limitan o validan peticiones sin decidir quién puede llamar a una ruta, así que no cuentan: una ruta cuyo
único guard es un limitador de peticiones global está desprotegida.

Sobrescribe `controls_access()` para que devuelva `false` en tus guards que dejan pasar las peticiones salvo que algo
esté configurado (un guard de roles que devuelve `Ok(true)` cuando la ruta no declara roles, o guards de tenant, de modo
mantenimiento o de feature flags). Registrado como global sin esa sobrescritura, un guard así hace que todas las rutas
parezcan protegidas. Contar un guard es una declaración, no una prueba de que autentique. Ver
[Guards globales que dejan pasar todo](/es/guards#guards-globales-que-dejan-pasar-todo), que describe también la línea que
se registra en modo de advertencia y estricto con los guards que contaron y cuántas rutas cubre cada uno. `corvis g guard` genera un
guard que deniega todas las peticiones hasta que lo implementes, de modo que un guard recién creado nunca cuenta como
protección mientras deja entrar a todos.

## WebSockets y SSE

Las rutas `#[sse]` siguen la misma regla que cualquier otra ruta y se etiquetan `SSE` en el error y en el informe.

También se comprueba cada manejador `#[subscribe_message]`. Los guards que cuentan para un mensaje son los que se
ejecutan para él: los globales (solo mientras los enhancers globales se apliquen a los gateways), luego los del gateway y
luego los del manejador. `#[public]` va debajo de `#[websocket_gateway]` o en un manejador, y `exempt_message` exime un
manejador por ruta del gateway y evento. Los detalles están en
[Route posture en gateways](/es/websockets/guards#route-posture-en-gateways).

Prefiere `#[public]` en un gateway a un guard global que devuelva `Ok(true)` para contextos WebSocket: ese guard haría que
la verificación aceptara todos los manejadores mientras nada autentica.

## Lo que no se comprueba

La verificación cubre las rutas de controladores y los manejadores de mensajes de gateways. Estas superficies quedan
fuera, así que protégelas por separado (la lista completa está en
[Lo que la verificación no ve](/es/guards#lo-que-la-verificacion-no-ve)):

- los archivos estáticos (`ServeStaticModule` y `serve_static`); un archivo estático con la misma ruta que un endpoint
  protegido se sirve antes que el endpoint;
- los listeners `upgrade` crudos (`on_upgrade`, `corvis::ws`);
- las interfaces de Swagger y Scalar y su documento JSON;
- cualquier middleware que responda sin llamar a `next`;
- el handshake de WebSocket de un gateway, que no ejecuta ningún guard (`verify_client` y `allowed_origins` son las
  comprobaciones que existen ahí; ver [Autenticar al conectar](/es/websockets/guards#autenticar-al-conectar)).

`app.route_posture()` lista bajo "Not checked" los montajes de `ServeStaticModule`, los handshakes de gateways (con si
`verify_client` está configurado) y el número de listeners `upgrade` crudos, para que quien revise los vea. Nunca hacen
fallar `init()`. `serve_static` y el resto de middleware no son visibles para el informe.

## El error

En modo estricto `init()` devuelve un error que lista todas las entradas desprotegidas, no solo la primera, ordenadas por
ruta y método, y explica las tres soluciones. El mismo texto se registra una vez con nivel de error:

```text
Route posture check failed: 2 routes have no guard and are not marked #[public].
  GET   /api/rooms                 RoomsController::list
  POST  /api/rooms/:room/messages  RoomsController::post
Add #[use_guards(...)] to the route or its controller (or register a global guard), mark it #[public] if it is meant to be open, or add an exemption with a reason. Guards that do not control access (ThrottlerGuard, ZodGuard) do not count.
```

Cada línea tiene el método (`SSE` para un stream, `WS` para un mensaje de gateway), la ruta completa y
`Controller::handler`. Corrige cada una añadiendo un guard a la ruta o a su controlador, marcándola `#[public]` si debe
estar abierta, o añadiendo una exención con una razón. Ver
[Route posture check failed](/es/faq/common-errors#route-posture-check-failed) en los errores comunes. El tipo del error
es `RoutePostureError`, que puedes obtener con `downcast` del `anyhow::Error` que devuelve `listen()`. El mensaje nunca
incluye datos de la petición ni valores de configuración.

## Revisar la postura

Después de `init()`, y en cualquier modo (también `off`), `app.route_posture()` devuelve el informe: cada entrada con su
método o evento, ruta completa, controlador y manejador, guards (y si cada uno controla el acceso) y estado (protegida,
pública con `public_source`, exenta con su razón, o desprotegida), las superficies que no se comprueban, los guards que
contaron (`access_guards()`) y las exenciones obsoletas o redundantes. Implementa `Display` (una tabla) y
`serde::Serialize`. Úsalo en un test, para que una ruta nueva sin guard haga fallar la CI aunque la aplicación no exija la
verificación:

```rust
let mut app = CorvisFactory::create::<AppModule>().await?;
app.init().await?;
let report = app.route_posture().expect("the report exists after init()");
assert!(report.unprotected().is_empty(), "{report}");
```

`examples/tertulia/tests/chat.rs` (`route_posture_matches_the_routes`) va más lejos y fija qué rutas son públicas y cuáles
están protegidas.

`corvis posture` imprime el mismo informe desde la línea de comandos, sin escuchar en ningún puerto, sea cual sea el modo de
la aplicación. Termina con `0` cuando nada está desprotegido, con `1` cuando algo lo está, y con `2` cuando no pudo obtener
un informe fiable (falló la compilación o el arranque, o venció el tiempo). Usa `--json` en CI. Sus opciones, el formato de
salida y los límites están en [`corvis posture`](/es/cli/usages#corvis-posture).

::: warning Modo informe
`corvis posture` funciona estableciendo `CORVIS_ROUTE_POSTURE_REPORT`, que hace que cualquier binario que la lea imprima
sus rutas y se detenga en `init()` con el código de salida 40 o 41. No la establezcas tú en un despliegue ni en un trabajo
de CI que ejecute tus tests; ver [Modo informe](/es/guards#modo-informe).
:::

## Migrar una aplicación existente

1. Ejecuta `corvis posture` para ver el estado actual sin cambiar la aplicación. Cada ruta marcada `UNPROTECTED` es una
   decisión que tomar.
2. Llama a `app.set_route_posture(RoutePostureOptions::warn())`. La aplicación arranca como antes y registra la lista una
   vez.
3. Para cada ruta, añade un guard, márcala `#[public]` o, si algo fuera del framework la protege, añade una exención con un
   razón. Si ya marcas las rutas abiertas con `#[set_metadata(IS_PUBLIC_KEY, true)]` y el valor `"isPublic"`, ya cuentan
   como públicas; puedes pasarlas a `#[public]` cuando quieras.
4. Lee la línea "access guards counted" que registra el modo de advertencia. Un guard global que no es tu guard de autenticación y
   cubre todas las rutas es el que hay que revisar: haz que `controls_access()` devuelva `false` si deja pasar las
   peticiones cuando no tiene nada que comprobar.
5. Cambia las opciones de aviso por `app.require_route_posture()` (o conserva tus exenciones con
   `RoutePostureOptions::enforce()`), y añade un test que lea `app.route_posture()` o un paso de `corvis posture` a la CI.
