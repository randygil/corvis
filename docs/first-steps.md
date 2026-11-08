---
status: documented
---

# First steps

In this section you will create a minimal Corvis application, understand which files the CLI generates, and get it running. The concepts that appear here (modules, controllers, providers) are explained in detail in the following pages.

## Requirements

- A stable Rust installation with support for the **2024 edition** (Rust 1.85 or later). The workspace crates and the projects the CLI generates use `edition = "2024"`.
- `git`, if you want `corvis new` to initialize the project repository (it can be skipped with `--skip-git`).

Corvis runs on [Tokio](https://tokio.rs) and [hyper](https://hyper.rs). You don't need to declare them as dependencies: the `corvis` crate re-exports `tokio`, `futures`, `serde_json`, `anyhow`, `bytes`, and `async_trait`.

## Installing the CLI

The CLI lives in the repository itself, in `crates/corvis-cli`. From a clone of the repository:

```bash
cargo install --path crates/corvis-cli
```

This leaves the `corvis` binary on your `PATH` (usually in `~/.cargo/bin`).

## Creating a project

```bash
corvis new my-app
cd my-app
cargo run
```

`corvis new` creates the `my-app` folder with a Cargo project ready to build. The `corvis` dependency it writes in `Cargo.toml` depends on the options:

| Option | Dependency written |
|---|---|
| (none) | `path = "..."` pointing to `crates/corvis` in the repository the CLI was built from |
| `--corvis-path <dir>` or the `CORVIS_PATH` variable | `path = "<dir>"` |
| `--corvis-version <ver>` | `version = "<ver>"` (a Cargo version requirement, to use a version published in a registry) |

::: tip
The simplest option today is the path dependency: install the CLI from your clone of the repository and let `corvis new` point to it. If you move the clone, update the path in `Cargo.toml`.
:::

The project name must start with a letter and contain only letters, digits, `_`, or `-`.

## Generated files

```text
my-app/
├── .env                 # PORT=3000
├── .gitignore
├── Cargo.toml
└── src/
    ├── main.rs
    ├── app_module.rs
    ├── app_controller.rs   # includes a test
    └── app_service.rs
```

| File | Contents |
|---|---|
| `app_module.rs` | The application's **root module** (`AppModule`). It imports `ConfigModule` as a global module and registers the controller and the service. |
| `app_controller.rs` | A controller with a single `GET /` route and a test that exercises it in memory. |
| `app_service.rs` | An injectable service with the route's logic. |
| `main.rs` | The entry point: it creates the application with `CorvisFactory` and starts listening. |

### `main.rs`

```rust
mod app_controller;
mod app_module;
mod app_service;

use corvis::prelude::*;

use app_module::AppModule;

#[corvis::main]
async fn main() -> corvis::Result<()> {
    let mut app = CorvisFactory::create::<AppModule>().await?;
    app.require_route_posture();
    let port: u16 = app.get::<ConfigService>()?.get_or("PORT", 3000);
    app.listen(port).await
}
```

- `#[corvis::main]` turns the `async fn main` function into a `main` that starts a multi-threaded Tokio runtime. It requires the function to be `async`.
- `CorvisFactory::create::<AppModule>()` walks the module graph starting from `AppModule`, creates all providers and controllers, and returns a `CorvisApplication`. If a dependency is missing or there is a cycle, it returns the error (and logs it).
- `app.get::<ConfigService>()` gets an instance from the container. Here it is used to read `PORT` from `.env` or the environment.
- `app.require_route_posture()` turns on the route posture check: the application refuses to start if a route has no guard and is not marked `#[public]`. The generated `get_hello` route is marked `#[public]` so the project starts as generated; a route you add without a guard or `#[public]` is reported at startup, and `corvis posture` lists every route with its posture.
- `app.listen(port)` registers the routes, runs the initialization hooks, and serves requests until the process ends.

`listen` accepts a port (`3000`, which listens on `0.0.0.0`), a string (`"127.0.0.1:3000"` or `"3000"`), a `String`, or a `SocketAddr`.

### `app_module.rs`

```rust
use corvis::prelude::*;

use crate::app_controller::AppController;
use crate::app_service::AppService;

#[module(
    imports = [ConfigModule::for_root(ConfigModuleOptions { is_global: true, ..Default::default() })],
    controllers = [AppController],
    providers = [AppService],
)]
pub struct AppModule;
```

### `app_controller.rs` and `app_service.rs`

```rust
use corvis::prelude::*;

use crate::app_service::AppService;

#[controller]
pub struct AppController {
    app_service: Arc<AppService>,
}

#[routes]
impl AppController {
    // Routes need a guard or #[public]: main enforces route posture (see require_route_posture).
    #[get]
    #[public]
    fn get_hello(&self) -> String {
        self.app_service.get_hello()
    }
}
```

```rust
use corvis::prelude::*;

#[injectable]
pub struct AppService;

impl AppService {
    pub fn get_hello(&self) -> String {
        "Hello World!".to_string()
    }
}
```

The `app_service: Arc<AppService>` field is all that injection needs: `#[controller]` generates the constructor and the container resolves each `Arc<T>` field by its type. You will see it in detail in [Providers](/providers).

`use corvis::prelude::*;` brings in the attributes (`module`, `controller`, `routes`, `injectable`, ...), the pipeline traits, the most used exceptions, `Arc`, `Value`, `json!`, and `async_trait`.

## Running the application

```bash
cargo run
```

On startup you will see the startup log:

```text
[Corvis] 12345  - 07/11/2026, 10:00:00     LOG [CorvisFactory] Starting Corvis application...
[Corvis] 12345  - 07/11/2026, 10:00:00     LOG [InstanceLoader] AppModule dependencies initialized +1ms
[Corvis] 12345  - 07/11/2026, 10:00:00     LOG [RoutesResolver] AppController {/}: +0ms
[Corvis] 12345  - 07/11/2026, 10:00:00     LOG [RouterExplorer] Mapped {/, GET} route +0ms
[Corvis] 12345  - 07/11/2026, 10:00:00     LOG [CorvisApplication] Corvis application successfully started +0ms
```

Open `http://localhost:3000/` and you will see `Hello World!`.

To rebuild and restart when you save files, use `corvis start --watch` (see [CLI](/cli/overview)).

## Running the tests

The generated controller comes with a test that creates a testing module and makes an in-memory request, without opening sockets:

```bash
cargo test
```

The [Testing](/fundamentals/testing) page explains `Test::create_testing_module()` and the `request(&app)` client.

## Next steps

- [Controllers](/controllers): routes, parameters, and responses.
- [Providers](/providers) and [Modules](/modules): dependency injection and organization.
- The CLI also generates complete resources: `corvis generate resource cats` (see [CRUD generator](/recipes/crud-generator)).
- The repository examples (`examples/cats-app`, `examples/tertulia`, `examples/blog-orm`) show complete applications. They run with `cargo run -p cats-app`, `cargo run -p tertulia`, or `cargo run -p blog-orm`.
