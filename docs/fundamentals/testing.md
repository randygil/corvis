---
status: documentado
---

# Testing

Corvis trae utilidades para probar a tres niveles: **unitario** (un servicio con sus dependencias simuladas), **de integración** (un módulo completo con peticiones HTTP en memoria) y **end-to-end** (un servidor real en un puerto). Todo se ejecuta con el `cargo test` de siempre; no hay un runner aparte.

```rust
use corvis::prelude::*;
use corvis::testing::{Test, request};
```

## `#[corvis::test]`

Los tests asíncronos se marcan con `#[corvis::test]`. Genera un `#[test]` normal que ejecuta el cuerpo en un runtime multihilo de Tokio (dos workers):

```rust
#[corvis::test]
async fn find_all_returns_empty_list() {
    // ...
}
```

## Módulo de pruebas

`Test::create_testing_module()` arma un módulo raíz sintético con lo que le pidas y lo compila con el mismo contenedor que usa la aplicación real:

```rust
#[corvis::test]
async fn should_return_hello_world() {
    let module = Test::create_testing_module()
        .controller::<AppController>()
        .provider::<AppService>()
        .compile()
        .await
        .unwrap();

    let controller = module.get::<AppController>().unwrap();
    assert_eq!(controller.get_hello(), "Hello World!");
}
```

(Es el test que genera `corvis new` en `app_controller.rs`.)

### Construcción

| Método | Uso |
|---|---|
| `.import::<M>()` | Importa un módulo, como en `imports` |
| `.import_dynamic(m)` | Importa un módulo dinámico (`ConfigModule::for_root(...)`, ...) |
| `.imports(vec![...])` | Varios imports a la vez (`ModuleImport`) |
| `.provider::<T>()` | Registra un provider de clase |
| `.providers(vec![...])` | Providers arbitrarios (`Provider::use_value(...)`, ...) |
| `.controller::<C>()`, `.controllers(vec![...])` | Registra controladores |
| `.with_logger(true)` | Muestra los logs del framework (por defecto solo errores) |
| `.compile().await` | Crea el contenedor y todas las instancias |

`compile` devuelve un error si falta una dependencia o hay un ciclo, con el mismo mensaje que la aplicación:

```text
Corvis can't resolve dependencies of the NeedsCats (?). Please make sure that the argument CatsService at index [0] is available in the BrokenModule context.
```

### `TestingModule`

| Método | Uso |
|---|---|
| `get::<T>()` | Instancia de un provider o controlador |
| `get_token::<T>("TOKEN")` | Instancia registrada con un token |
| `resolve::<T>().await` | Resuelve un provider; con scope `Transient` crea una instancia nueva |
| `init().await` | Ejecuta los hooks de inicialización |
| `close().await` | Ejecuta los hooks de apagado |
| `create_corvis_application()` | Convierte el módulo en una `CorvisApplication` para hacer peticiones |

## Reemplazar dependencias

Lo habitual en un test es sustituir lo que sale del proceso (base de datos, servicios externos) por dobles de prueba.

### Providers

```rust
let module = Test::create_testing_module()
    .import::<CatsModule>()
    .override_provider::<CatsService>()
    .use_value(CatsService::mock())
    .compile()
    .await
    .unwrap();
```

`override_provider::<T>()` reemplaza el provider de clase `T` y `override_provider_token("TOKEN")`, uno registrado con un token. Los dos terminan en:

| Método | Reemplazo |
|---|---|
| `.use_value(v)` | Un valor; debe ser del tipo que piden los consumidores |
| `.use_value_arc(arc)` | Un `Arc` ya construido (por ejemplo, `Arc<dyn Repo>`) |
| `.use_class::<Mock>()` | Otra clase inyectable |
| `.use_factory(\|ctx\| async move { ... })` | Una factory, igual que `Provider::use_factory` |

El reemplazo se aplica en cualquier módulo del grafo donde esté registrado ese token.

### Módulos

`.override_module::<ProdModule, MockModule>()` sustituye un módulo estático completo por otro.

### Guards, interceptors, pipes y filtros

```rust
struct AllowAll;

#[async_trait]
impl CanActivate for AllowAll {
    async fn can_activate(&self, _ctx: &ExecutionContext) -> Result<bool, HttpException> {
        Ok(true)
    }
}

let module = Test::create_testing_module()
    .import::<AppModule>()
    .override_provider_token("PREFIX")
    .use_value("Lord ".to_string())
    .override_guard::<RolesGuard>(AllowAll)
    .compile()
    .await
    .unwrap();
```

(`crates/corvis/tests/app.rs`)

`override_guard::<G>(mock)`, `override_interceptor::<I>(mock)`, `override_pipe::<P>(mock)` y `override_filter::<F>(mock)` reemplazan los enhancers referenciados **por tipo** en `#[use_guards(G)]`, `#[use_interceptors(I)]`, `#[use_pipes(P)]`, `#[use_filters(F)]` o en un parámetro.

::: warning
Los enhancers globales registrados con `Provider::app_guard` (y sus equivalentes) o con `use_global_*`, y los que se pasan como expresión (`#[use_guards(RolesGuard::new("admin"))]`), no se reemplazan con estos métodos. Para los de `use_global_*`, simplemente no los registres en la aplicación de prueba; para los de `Provider::app_*`, sustituye sus dependencias con `override_provider`.
:::

## Peticiones HTTP en memoria

Para probar rutas, convierte el módulo en aplicación, inicialízala y usa `request(&app)`. Las peticiones pasan por el pipeline completo (middleware, guards, interceptors, pipes, filtros, body parser, CORS) sin abrir sockets:

```rust
async fn app() -> CorvisApplication {
    let module = Test::create_testing_module().import::<AppModule>().compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.use_global_pipes(ValidationPipe::new().whitelist(true));
    app.init().await.unwrap();
    app
}

#[corvis::test]
async fn crud_validation_and_errors() {
    let app = app().await;
    let client = request(&app);

    let res = client.post("/cats").json(&json!({ "name": "Tom", "age": 3 })).send().await;
    res.expect_status(201);
    assert_eq!(res.json_value(), json!({ "id": 1, "name": "Sir Tom", "age": 3 }));

    client
        .get("/cats/42")
        .send()
        .await
        .expect_status(404)
        .expect_json(json!({ "message": "Cat #42 not found", "error": "Not Found", "statusCode": 404 }));
}
```

Configura la aplicación (pipes globales, prefijo, CORS, versionado) **antes** de `app.init()`, igual que harías en `main`. `request(&app)` entra en pánico si la aplicación no se inicializó.

### `TestClient` y `TestRequest`

`request(&app)` devuelve un `TestClient` con `get`, `post`, `put`, `patch`, `delete`, `options`, `head` y `method(Method, ruta)`. Cada uno crea un `TestRequest`:

| Método | Uso |
|---|---|
| `.header(nombre, valor)` / `.set(nombre, valor)` | Agrega una cabecera |
| `.bearer(token)` | `Authorization: Bearer <token>` |
| `.query(clave, valor)` | Agrega un parámetro al query string |
| `.json(&valor)` | Cuerpo JSON |
| `.form(&[(k, v)])` | Cuerpo `application/x-www-form-urlencoded` |
| `.body(bytes)` | Cuerpo crudo |
| `.field(nombre, valor)`, `.attach(campo, bytes, archivo)`, `.attach_with_type(..)` | Arma un `multipart/form-data` |
| `.send().await` | Ejecuta la petición |

### `TestResponse`

| Método | Uso |
|---|---|
| `status()`, `header(nombre)`, `text()`, `json::<T>()`, `json_value()` | Leen la respuesta |
| `expect_status(n)`, `expect_json(valor)`, `expect_text(s)`, `expect_header(nombre, valor)` | Aserciones encadenables (`assert_eq!`); `expect_status` incluye el cuerpo en el mensaje de fallo |

```rust
client
    .get("/cats/whoami/role")
    .header("x-role", "dev")
    .header("user-agent", "jest")
    .send()
    .await
    .expect_status(200)
    .expect_text("dev:jest")
    .expect_header("cache-control", "none");
```

## Tests end-to-end con un servidor real

Cuando necesitas un socket de verdad (por ejemplo, para WebSockets), `listen_in_background` arranca el servidor en una tarea y devuelve la dirección:

```rust
let mut app = tertulia::create_app().await.unwrap();
let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();   // puerto libre
let url = format!("ws://{addr}/chat");
```

La misma `app` sigue sirviendo para `request(&app)`, así que un test puede mezclar el cliente en memoria con clientes reales. `examples/tertulia/tests/chat.rs` hace exactamente eso con clientes WebSocket de `tokio-tungstenite`.

Para probar el arranque de la aplicación completa (por ejemplo, que falle sin una variable de entorno obligatoria), crea la aplicación como en `main` y revisa el error:

```rust
#[corvis::test]
async fn refuses_to_start_without_jwt_secret() {
    // ... sin JWT_SECRET en el entorno
    let err = tertulia::create_app().await.err().expect("the application started without JWT_SECRET");
    assert!(format!("{err:#}").contains("JWT_SECRET"), "{err:#}");
}
```

(`examples/tertulia/tests/startup.rs`)

## Organización

- Los tests unitarios suelen vivir junto al código, en un `#[cfg(test)] mod tests`. Los schematics de la CLI (`corvis g service users`) generan uno por defecto; `--no-spec` lo omite.
- Los tests de integración van en `tests/*.rs` del crate. Si el binario tiene la lógica en `main.rs`, extrae una función `create_app()` a `lib.rs` para poder usarla desde los tests, como hace `examples/tertulia`.
