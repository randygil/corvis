---
status: documented
---

# Testing

Corvis ships utilities for testing at three levels: **unit** (a service with its dependencies mocked), **integration** (a complete module with in-memory HTTP requests), and **end-to-end** (a real server on a port). Everything runs with the usual `cargo test`; there is no separate runner.

```rust
use corvis::prelude::*;
use corvis::testing::{Test, request};
```

## `#[corvis::test]`

Async tests are marked with `#[corvis::test]`. It generates a regular `#[test]` that runs the body on a multi-threaded Tokio runtime (two workers):

```rust
#[corvis::test]
async fn find_all_returns_empty_list() {
    // ...
}
```

## Testing module

`Test::create_testing_module()` assembles a synthetic root module with whatever you ask for and compiles it with the same container the real application uses:

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

(This is the test that `corvis new` generates in `app_controller.rs`.)

### Construction

| Method | Use |
|---|---|
| `.import::<M>()` | Imports a module, as in `imports` |
| `.import_dynamic(m)` | Imports a dynamic module (`ConfigModule::for_root(...)`, ...) |
| `.imports(vec![...])` | Several imports at once (`ModuleImport`) |
| `.provider::<T>()` | Registers a class provider |
| `.providers(vec![...])` | Arbitrary providers (`Provider::use_value(...)`, ...) |
| `.controller::<C>()`, `.controllers(vec![...])` | Registers controllers |
| `.with_logger(true)` | Shows the framework logs (errors only by default) |
| `.compile().await` | Creates the container and all the instances |

`compile` returns an error if a dependency is missing or there is a cycle, with the same message as the application:

```text
Corvis can't resolve dependencies of the NeedsCats (?). Please make sure that the argument CatsService at index [0] is available in the BrokenModule context.
```

### `TestingModule`

| Method | Use |
|---|---|
| `get::<T>()` | Instance of a provider or controller |
| `get_token::<T>("TOKEN")` | Instance registered with a token |
| `resolve::<T>().await` | Resolves a provider; with `Transient` scope it creates a new instance |
| `init().await` | Runs the initialization hooks |
| `close().await` | Runs the shutdown hooks |
| `create_corvis_application()` | Converts the module into a `CorvisApplication` to make requests |

## Replacing dependencies

The usual thing in a test is to replace whatever leaves the process (database, external services) with test doubles.

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

`override_provider::<T>()` replaces the class provider `T` and `override_provider_token("TOKEN")`, one registered with a token. Both end in:

| Method | Replacement |
|---|---|
| `.use_value(v)` | A value; it must be of the type the consumers ask for |
| `.use_value_arc(arc)` | An already built `Arc` (for example, `Arc<dyn Repo>`) |
| `.use_class::<Mock>()` | Another injectable class |
| `.use_factory(\|ctx\| async move { ... })` | A factory, like `Provider::use_factory` |

The replacement applies in any module of the graph where that token is registered.

### Modules

`.override_module::<ProdModule, MockModule>()` replaces an entire static module with another.

### Guards, interceptors, pipes, and filters

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

`override_guard::<G>(mock)`, `override_interceptor::<I>(mock)`, `override_pipe::<P>(mock)`, and `override_filter::<F>(mock)` replace the enhancers referenced **by type** in `#[use_guards(G)]`, `#[use_interceptors(I)]`, `#[use_pipes(P)]`, `#[use_filters(F)]`, or in a parameter.

::: warning
Global enhancers registered with `Provider::app_guard` (and its equivalents) or with `use_global_*`, and those passed as an expression (`#[use_guards(RolesGuard::new("admin"))]`), are not replaced with these methods. For those from `use_global_*`, simply don't register them in the test application; for those from `Provider::app_*`, replace their dependencies with `override_provider`.
:::

## In-memory HTTP requests

To test routes, convert the module into an application, initialize it, and use `request(&app)`. Requests go through the complete pipeline (middleware, guards, interceptors, pipes, filters, body parser, CORS) without opening sockets:

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

Configure the application (global pipes, prefix, CORS, versioning) **before** `app.init()`, just as you would in `main`. `request(&app)` panics if the application wasn't initialized.

### `TestClient` and `TestRequest`

`request(&app)` returns a `TestClient` with `get`, `post`, `put`, `patch`, `delete`, `options`, `head`, and `method(Method, path)`. Each one creates a `TestRequest`:

| Method | Use |
|---|---|
| `.header(name, value)` / `.set(name, value)` | Adds a header |
| `.bearer(token)` | `Authorization: Bearer <token>` |
| `.query(key, value)` | Adds a parameter to the query string |
| `.json(&value)` | JSON body |
| `.form(&[(k, v)])` | `application/x-www-form-urlencoded` body |
| `.body(bytes)` | Raw body |
| `.field(name, value)`, `.attach(field, bytes, file)`, `.attach_with_type(..)` | Builds a `multipart/form-data` |
| `.send().await` | Executes the request |

### `TestResponse`

| Method | Use |
|---|---|
| `status()`, `header(name)`, `text()`, `json::<T>()`, `json_value()` | Read the response |
| `expect_status(n)`, `expect_json(value)`, `expect_text(s)`, `expect_header(name, value)` | Chainable assertions (`assert_eq!`); `expect_status` includes the body in the failure message |

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

## End-to-end tests with a real server

When you need a real socket (for example, for WebSockets), `listen_in_background` starts the server in a task and returns the address:

```rust
let mut app = tertulia::create_app().await.unwrap();
let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();   // free port
let url = format!("ws://{addr}/chat");
```

The same `app` can still be used with `request(&app)`, so a test can mix the in-memory client with real clients. `examples/tertulia/tests/chat.rs` does exactly that with `tokio-tungstenite` WebSocket clients.

To test the startup of the complete application (for example, that it fails without a required environment variable), create the application as in `main` and check the error:

```rust
#[corvis::test]
async fn refuses_to_start_without_jwt_secret() {
    // ... without JWT_SECRET in the environment
    let err = tertulia::create_app().await.err().expect("the application started without JWT_SECRET");
    assert!(format!("{err:#}").contains("JWT_SECRET"), "{err:#}");
}
```

(`examples/tertulia/tests/startup.rs`)

## Organization

- Unit tests usually live next to the code, in a `#[cfg(test)] mod tests`. The CLI's schematics (`corvis g service users`) generate one by default; `--no-spec` omits it.
- Integration tests go in the crate's `tests/*.rs`. If the binary has its logic in `main.rs`, extract a `create_app()` function into `lib.rs` so you can use it from tests, as `examples/tertulia` does.
