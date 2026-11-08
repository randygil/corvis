---
status: unavailable
---

# REPL

Corvis **does not include a REPL** to inspect the providers of a running application or call their methods interactively. Rust is a compiled language and there is no interpreter in which to evaluate expressions against the application.

## Alternatives

**An exploration binary or test.** Create the application without an HTTP server and use the container from code:

```rust
// src/bin/explore.rs
use corvis::prelude::*;
use mi_api::{AppModule, CatsService};

#[corvis::main]
async fn main() -> corvis::Result<()> {
    let mut app = CorvisFactory::create::<AppModule>().await?;
    app.init().await?;
    let cats = app.get::<CatsService>()?;
    println!("{:?}", cats.find_all());
    app.close().await
}
```

```bash
cargo run --bin explore
```

See [Standalone applications](/standalone-applications).

**Tests.** `Test::create_testing_module()` compiles a module, lets you replace providers and get any of them with `module.get::<T>()`. It is the fastest way to try a service in isolation; see [Testing](/fundamentals/testing).
