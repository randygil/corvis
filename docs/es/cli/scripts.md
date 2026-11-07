---
status: parcial
---

# Scripts

En un proyecto de Corvis no hay un archivo de scripts propio de la CLI: el proyecto es un crate de Cargo y los comandos de compilación y ejecución son los de Cargo. `corvis start` y `corvis build` son atajos sobre ellos.

| Tarea | Con la CLI | Con Cargo |
|---|---|---|
| Ejecutar en desarrollo | `corvis start` | `cargo run` |
| Ejecutar y reiniciar al guardar | `corvis start --watch` | `cargo watch -x run` |
| Compilar | `corvis build` | `cargo build` |
| Compilar para producción | `corvis build --release` | `cargo build --release` |
| Ejecutar los tests | | `cargo test` |

`corvis start --watch` requiere `cargo-watch` (`cargo install cargo-watch`). Ver [Uso](/es/cli/usages#corvis-start).

## Comandos propios

Si quieres nombres cortos para tareas del proyecto, usa los alias de Cargo en `.cargo/config.toml`:

```toml
# .cargo/config.toml
[alias]
dev = "watch -x run"
prod = "run --release"
```

```bash
cargo dev
cargo prod
```

Para tareas más largas (migraciones, generación de modelos), el binario `corvis-orm` tiene sus propios subcomandos; ver [corvis-orm](/es/data/corvis-orm).
