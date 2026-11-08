---
status: unavailable
---

# Libraries

The CLI **does not generate libraries** yet (there is no `library` schematic and no import paths configured by the CLI).

## Alternative: a library crate in the workspace

A Corvis module is a regular Rust type, so it can be published from any crate. Create a library crate in a [Cargo workspace](/cli/monorepo):

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

And in the application:

```toml
# apps/api/Cargo.toml
[dependencies]
shared = { path = "../../libs/shared" }
```

```rust
#[module(imports = [shared::SharedModule], controllers = [AppController])]
pub struct AppModule;
```

The visibility rules are the usual ones: the application only sees what `SharedModule` exports. See [Modules](/modules).

Corvis itself follows this pattern: `corvis-orm` is a separate crate that exposes `OrmModule` to import in any application.
