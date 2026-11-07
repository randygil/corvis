---
status: no-disponible
---

# Workspaces

La CLI de Corvis **no tiene todavía un modo monorepo**: no hay un archivo de configuración que agrupe varias aplicaciones ni comandos para generar o compilar una aplicación concreta dentro de un repositorio con varias.

## Alternativa: un workspace de Cargo

En Rust ese papel lo cumple un [workspace de Cargo](https://doc.rust-lang.org/cargo/reference/workspaces.html), y es exactamente lo que usa el propio repositorio de Corvis: los crates del framework y los ejemplos (`examples/tertulia`, `examples/cats-app`, `examples/blog-orm`) son miembros de un mismo workspace.

```toml
# Cargo.toml en la raíz
[workspace]
resolver = "3"
members = ["apps/*", "libs/*"]

[workspace.dependencies]
corvis = { path = "../corvis/crates/corvis" }
serde = { version = "1", features = ["derive"] }
```

```toml
# apps/api/Cargo.toml
[dependencies]
corvis = { workspace = true }
serde = { workspace = true }
```

Cada aplicación se crea con `corvis new` dentro de `apps/` y se ejecuta con Cargo desde la raíz:

```bash
cargo run -p api
cargo test --workspace
```

Los comandos `corvis generate` funcionan dentro de cada aplicación: ejecútalos desde su carpeta (la CLI busca hacia arriba el primer `Cargo.toml` con `src/`). Para compartir código entre aplicaciones, ver [Librerías](/es/cli/libraries).
