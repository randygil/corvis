---
status: documented
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
| `corvis start` | | Ejecuta `cargo run` (con `--watch`, `watchexec`) |
| `corvis build` | | Ejecuta `cargo build` (debug); con `--release`, modo release |
| `corvis posture` | | Imprime el informe de postura de rutas (rutas con guard, públicas, sin protección) |
| `corvis info` | `i` | Muestra el sistema operativo y las versiones de Rust, Cargo y la CLI |
| `corvis --version` | `-v` | Versión de la CLI |

Todas las opciones se describen en [Uso](/es/cli/usages).

## Dependencia del framework

Por defecto el `Cargo.toml` generado depende de la versión publicada del framework que corresponde a la CLI (la versión de la propia CLI, como requisito con caret):

```toml
[dependencies]
corvis = { version = "0.1.0" }
serde = { version = "1", features = ["derive"] }
```

Para usar una copia local del framework u otra versión:

- `--corvis-path <ruta>` o la variable de entorno `CORVIS_PATH` escribe `path = "<ruta>"`;
- `--corvis-version <requisito>` escribe `version = "<requisito>"`.

Una opción de la línea de comandos gana sobre `CORVIS_PATH`, y un `CORVIS_PATH` vacío cuenta como no definido. Pasar `--corvis-path` y `--corvis-version` a la vez es un error. Cuando la ruta viene de `CORVIS_PATH`, el comando imprime una línea `NOTE` que lo indica.

```toml
[dependencies]
corvis = { path = "C:/Users/tu-usuario/src/corvis/crates/corvis" }
serde = { version = "1", features = ["derive"] }
```

::: warning Hasta el primer lanzamiento
Corvis todavía no está en crates.io, así que un proyecto generado con los valores por defecto no puede resolver `corvis`. Hasta el primer lanzamiento, define `CORVIS_PATH` con la carpeta `crates/corvis` de tu clon (Bash: `export CORVIS_PATH=~/src/corvis/crates/corvis`; PowerShell: `$env:CORVIS_PATH = "C:/src/corvis/crates/corvis"`) o pasa `--corvis-path`. Si la CLI se compiló desde un clon que todavía existe, imprime una línea `NOTE` con esa carpeta.
:::

## Extensión para VS Code

En `editors/vscode` hay una extensión con snippets, los comandos de la CLI en la paleta y un explorador de rutas. Su README explica cómo instalarla.

**Corvis: New Project** ejecuta `corvis new`, así que la advertencia "Hasta el primer lanzamiento" de arriba también le aplica. Configura el ajuste `corvis.new.corvisPath` de la extensión con la carpeta `crates/corvis` de tu clon (también se acepta la raíz del repositorio) y el comando se ejecuta con `CORVIS_PATH` apuntando a ella, de modo que el proyecto generado tiene `corvis = { path = "..." }`. El valor debe ser absoluto (se expande `~`) y contener un `Cargo.toml`; si no, la extensión muestra un error y no ejecuta la CLI. Solo **New Project** recibe la variable. Con el ajuste vacío la extensión no cambia nada, y un `CORVIS_PATH` exportado en el shell que abrió VS Code sigue aplicando.
