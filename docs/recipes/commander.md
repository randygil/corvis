---
status: unavailable
---

# Commander

Corvis **has no module for writing command-line applications** (commands, options and prompts declared with attributes on providers).

## Alternative: the application without a server + an argument parser

A console tool can reuse your modules and services by creating the application without opening the HTTP server. Argument parsing is done by `std::env::args` or any dedicated Rust crate:

```rust
// src/bin/admin.rs
use corvis::prelude::*;
use mi_api::{AppModule, UsersService};

#[corvis::main]
async fn main() -> corvis::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut app = CorvisFactory::create::<AppModule>().await?;
    app.init().await?;
    let users = app.get::<UsersService>()?;
    match args.first().map(String::as_str) {
        Some("count") => println!("{}", users.count().await?),
        _ => eprintln!("usage: admin count"),
    }
    app.close().await
}
```

```bash
cargo run --bin admin -- count
```

For `src/bin/admin.rs` to import `AppModule`, the project needs a `src/lib.rs` that exposes the modules (as `examples/tertulia` does). Providers, configuration and lifecycle hooks work the same as in the server. See [Standalone applications](/standalone-applications).
