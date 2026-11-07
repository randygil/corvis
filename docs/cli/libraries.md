---
status: no-disponible
---

# Librerías

La CLI **no genera librerías** todavía (no hay un schematic `library` ni rutas de importación configuradas por la CLI).

## Alternativa: un crate de librería en el workspace

Un módulo de Corvis es un tipo de Rust normal, así que se puede publicar desde cualquier crate. Crea un crate de librería en un [workspace de Cargo](/cli/monorepo):

```bash
cargo new --lib libs/shared
```

```toml
# libs/shared/Cargo.toml
[dependencies]
corvis = { workspace = true }
```

```rust
// libs/shared/src/lib.rs
use corvis::prelude::*;

#[injectable]
pub struct ClockService;

impl ClockService {
    pub fn now_millis(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
}

#[module(providers = [ClockService], exports = [ClockService])]
pub struct SharedModule;
```

Y en la aplicación:

```toml
# apps/api/Cargo.toml
[dependencies]
shared = { path = "../../libs/shared" }
```

```rust
#[module(imports = [shared::SharedModule], controllers = [AppController])]
pub struct AppModule;
```

Las reglas de visibilidad son las de siempre: la aplicación solo ve lo que `SharedModule` exporta. Ver [Módulos](/modules).

El propio Corvis sigue este patrón: `corvis-orm` es un crate aparte que expone `OrmModule` para importarlo en cualquier aplicación.
