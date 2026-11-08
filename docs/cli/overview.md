---
status: documented
---

# Overview

`corvis` is the framework's command-line tool. It creates new projects with the recommended structure, generates the usual pieces (modules, controllers, services, guards...) and registers them in the right module, and wraps the Cargo commands you use every day.

## Installation

The CLI lives in the workspace, in `crates/corvis-cli`. From the root of the Corvis repository:

```bash
cargo install --path crates/corvis-cli
```

That installs the `corvis` binary in `~/.cargo/bin`. Check the installation:

```bash
corvis --version
corvis info
```

`corvis` without arguments (or with an unknown command) shows the help.

## Basic workflow

```bash
corvis new my-api            # creates the my-api/ project
cd my-api
corvis g resource cats       # full CRUD in src/cats/
corvis start --watch         # cargo run, restarting on save
```

`corvis new` leaves a project that compiles and responds `Hello World!` on `GET /`:

```text
my-api/
├── .env                 # PORT=3000
├── .gitignore
├── Cargo.toml
└── src/
    ├── main.rs          # CorvisFactory::create::<AppModule>() and listen
    ├── app_module.rs    # global ConfigModule + AppController + AppService
    ├── app_controller.rs  # GET / and a test with corvis::testing
    └── app_service.rs
```

The generated `main.rs` reads the port from `PORT` through `ConfigService` (3000 by default) and listens on all interfaces.

## Commands

| Command | Alias | What it does |
|---|---|---|
| `corvis new <name>` | `n` | Creates a new project |
| `corvis generate <schematic> <name>` | `g` | Generates an element and registers it |
| `corvis start` | | Runs `cargo run` (with `--watch`, `cargo watch -x run`) |
| `corvis build` | | Runs `cargo build` (debug); with `--release`, release mode |
| `corvis info` | `i` | Shows the operating system and the Rust, Cargo and CLI versions |
| `corvis --version` | `-v` | CLI version |

All options are described in [Usage](/cli/usages).

## Framework dependency

Corvis is not published on crates.io yet, so the generated `Cargo.toml` points to a local copy:

- by default, to the `crates/corvis` folder the CLI was built from;
- with `--corvis-path <path>` or the `CORVIS_PATH` environment variable, to another path;
- with `--corvis-version <requirement>`, to a registry version (for when it is published).

```toml
[dependencies]
corvis = { path = "C:/Users/your-user/src/corvis/crates/corvis" }
serde = { version = "1", features = ["derive"] }
```

## VS Code extension

In `editors/vscode` there is an extension with snippets, the CLI commands in the palette and a route explorer. Its README explains how to install it.
