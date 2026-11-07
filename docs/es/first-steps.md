---
status: documented
---

# Primeros pasos

En esta sección vas a crear una aplicación mínima de Corvis, entender qué archivos genera la CLI y ponerla a correr. Los conceptos que aparecen aquí (módulos, controladores, providers) se explican con detalle en las páginas siguientes.

## Requisitos

- Una instalación estable de Rust con soporte para la **edición 2024** (Rust 1.85 o posterior). Los crates del workspace y los proyectos que genera la CLI usan `edition = "2024"`.
- `git`, si quieres que `corvis new` inicialice el repositorio del proyecto (se puede omitir con `--skip-git`).

Corvis se ejecuta sobre [Tokio](https://tokio.rs) y [hyper](https://hyper.rs). No necesitas declararlos como dependencias: el crate `corvis` reexporta `tokio`, `futures`, `serde_json`, `anyhow`, `bytes` y `async_trait`.

## Instalar la CLI

La CLI vive en el propio repositorio, en `crates/corvis-cli`. Desde un clon del repositorio:

```bash
cargo install --path crates/corvis-cli
```

Esto deja el binario `corvis` en tu `PATH` (normalmente en `~/.cargo/bin`).

## Crear un proyecto

```bash
corvis new mi-app
cd mi-app
cargo run
```

`corvis new` crea la carpeta `mi-app` con un proyecto de Cargo listo para compilar. La dependencia de `corvis` que escribe en `Cargo.toml` depende de las opciones:

| Opción | Dependencia escrita |
|---|---|
| (ninguna) | `path = "..."` apuntando a `crates/corvis` del repositorio desde el que se compiló la CLI |
| `--corvis-path <dir>` o variable `CORVIS_PATH` | `path = "<dir>"` |
| `--corvis-version <ver>` | `version = "<ver>"` (un requisito de versión de Cargo, para usar una versión publicada en un registro) |

::: tip
Lo más simple hoy es la dependencia por ruta: instala la CLI desde tu clon del repositorio y deja que `corvis new` apunte a él. Si mueves el clon, actualiza la ruta en `Cargo.toml`.
:::

El nombre del proyecto debe empezar con una letra y contener solo letras, dígitos, `_` o `-`.

## Archivos generados

```text
mi-app/
├── .env                 # PORT=3000
├── .gitignore
├── Cargo.toml
└── src/
    ├── main.rs
    ├── app_module.rs
    ├── app_controller.rs   # incluye un test
    └── app_service.rs
```

| Archivo | Contenido |
|---|---|
| `app_module.rs` | El **módulo raíz** de la aplicación (`AppModule`). Importa `ConfigModule` como módulo global y registra el controlador y el servicio. |
| `app_controller.rs` | Un controlador con una sola ruta `GET /` y un test que la ejercita en memoria. |
| `app_service.rs` | Un servicio inyectable con la lógica de la ruta. |
| `main.rs` | El punto de entrada: crea la aplicación con `CorvisFactory` y la pone a escuchar. |

### `main.rs`

```rust
mod app_controller;
mod app_module;
mod app_service;

use corvis::prelude::*;

use app_module::AppModule;

#[corvis::main]
async fn main() -> corvis::Result<()> {
    let app = CorvisFactory::create::<AppModule>().await?;
    let port: u16 = app.get::<ConfigService>()?.get_or("PORT", 3000);
    app.listen(port).await
}
```

- `#[corvis::main]` convierte la función `async fn main` en un `main` que arranca un runtime multihilo de Tokio. Exige que la función sea `async`.
- `CorvisFactory::create::<AppModule>()` recorre el grafo de módulos desde `AppModule`, crea todos los providers y controladores y devuelve una `CorvisApplication`. Si falta una dependencia o hay un ciclo, devuelve el error (y lo registra en el log).
- `app.get::<ConfigService>()` obtiene una instancia del contenedor. Aquí se usa para leer `PORT` desde `.env` o el entorno.
- `app.listen(port)` registra las rutas, ejecuta los hooks de inicialización y atiende peticiones hasta que el proceso termina.

`listen` acepta un puerto (`3000`, que escucha en `0.0.0.0`), una cadena (`"127.0.0.1:3000"` o `"3000"`), un `String` o un `SocketAddr`.

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

### `app_controller.rs` y `app_service.rs`

```rust
use corvis::prelude::*;

use crate::app_service::AppService;

#[controller]
pub struct AppController {
    app_service: Arc<AppService>,
}

#[routes]
impl AppController {
    #[get]
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

El campo `app_service: Arc<AppService>` es todo lo que hace falta para la inyección: `#[controller]` genera el constructor y el contenedor resuelve cada campo `Arc<T>` por su tipo. Lo verás en detalle en [Providers](/es/providers).

`use corvis::prelude::*;` trae los atributos (`module`, `controller`, `routes`, `injectable`, ...), los traits del pipeline, las excepciones más usadas, `Arc`, `Value`, `json!` y `async_trait`.

## Ejecutar la aplicación

```bash
cargo run
```

Al arrancar verás el log de inicio:

```text
[Corvis] 12345  - 07/11/2026, 10:00:00     LOG [CorvisFactory] Starting Corvis application...
[Corvis] 12345  - 07/11/2026, 10:00:00     LOG [InstanceLoader] AppModule dependencies initialized +1ms
[Corvis] 12345  - 07/11/2026, 10:00:00     LOG [RoutesResolver] AppController {/}: +0ms
[Corvis] 12345  - 07/11/2026, 10:00:00     LOG [RouterExplorer] Mapped {/, GET} route +0ms
[Corvis] 12345  - 07/11/2026, 10:00:00     LOG [CorvisApplication] Corvis application successfully started +0ms
```

Abre `http://localhost:3000/` y verás `Hello World!`.

Para recompilar y reiniciar al guardar archivos, usa `corvis start --watch` (ver [CLI](/es/cli/overview)).

## Ejecutar los tests

El controlador generado trae un test que crea un módulo de pruebas y hace una petición en memoria, sin abrir sockets:

```bash
cargo test
```

La página [Testing](/es/fundamentals/testing) explica `Test::create_testing_module()` y el cliente `request(&app)`.

## Siguientes pasos

- [Controladores](/es/controllers): rutas, parámetros y respuestas.
- [Providers](/es/providers) y [Módulos](/es/modules): inyección de dependencias y organización.
- La CLI también genera recursos completos: `corvis generate resource cats` (ver [Generador CRUD](/es/recipes/crud-generator)).
- Los ejemplos del repositorio (`examples/cats-app`, `examples/tertulia`, `examples/blog-orm`) muestran aplicaciones completas. Se ejecutan con `cargo run -p cats-app`, `cargo run -p tertulia` o `cargo run -p blog-orm`.
