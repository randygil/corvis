---
status: documentado
---

# Visión general

`corvis` es la herramienta de línea de comandos del framework. Crea proyectos nuevos con la estructura recomendada, genera las piezas habituales (módulos, controladores, servicios, guards...) y las registra en el módulo que corresponde, y envuelve los comandos de Cargo que usas a diario.

## Instalación

La CLI vive en el workspace, en `crates/corvis-cli`. Desde la raíz del repositorio de Corvis:

```bash
cargo install --path crates/corvis-cli
```

Eso instala el binario `corvis` en `~/.cargo/bin`. Comprueba la instalación:

```bash
corvis --version
corvis info
```

`corvis` sin argumentos (o con un comando desconocido) muestra la ayuda.

## Flujo básico

```bash
corvis new mi-api            # crea el proyecto mi-api/
cd mi-api
corvis g resource cats       # CRUD completo en src/cats/
corvis start --watch         # cargo run, reiniciando al guardar
```

`corvis new` deja un proyecto que compila y responde `Hello World!` en `GET /`:

```text
mi-api/
├── .env                 # PORT=3000
├── .gitignore
├── Cargo.toml
└── src/
    ├── main.rs          # CorvisFactory::create::<AppModule>() y listen
    ├── app_module.rs    # ConfigModule global + AppController + AppService
    ├── app_controller.rs  # GET / y un test con corvis::testing
    └── app_service.rs
```

El `main.rs` generado lee el puerto de `PORT` a través de `ConfigService` (por defecto 3000) y escucha en todas las interfaces.

## Comandos

| Comando | Alias | Qué hace |
|---|---|---|
| `corvis new <nombre>` | `n` | Crea un proyecto nuevo |
| `corvis generate <schematic> <nombre>` | `g` | Genera un elemento y lo registra |
| `corvis start` | | Ejecuta `cargo run` (con `--watch`, `cargo watch -x run`) |
| `corvis build` | | Ejecuta `cargo build` (con `--release`, en modo release) |
| `corvis info` | `i` | Muestra el sistema operativo y las versiones de Rust, Cargo y la CLI |
| `corvis --version` | `-v` | Versión de la CLI |

Todas las opciones se describen en [Uso](/es/cli/usages).

## Dependencia del framework

Corvis todavía no está publicado en crates.io, así que el `Cargo.toml` generado apunta a una copia local:

- por defecto, a la carpeta `crates/corvis` desde la que se compiló la CLI;
- con `--corvis-path <ruta>` o la variable de entorno `CORVIS_PATH`, a otra ruta;
- con `--corvis-version <requisito>`, a una versión del registro (para cuando se publique).

```toml
[dependencies]
corvis = { path = "C:/Users/tu-usuario/src/corvis/crates/corvis" }
serde = { version = "1", features = ["derive"] }
```

## Extensión para VS Code

En `editors/vscode` hay una extensión con snippets, los comandos de la CLI en la paleta y un explorador de rutas. Su README explica cómo instalarla.
