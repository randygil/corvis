---
status: unavailable
---

# Workspaces

The Corvis CLI **does not have a monorepo mode yet**: there is no configuration file that groups several applications and no commands to generate or build a specific application inside a repository that has several.

## Alternative: a Cargo workspace

In Rust, that role is played by a [Cargo workspace](https://doc.rust-lang.org/cargo/reference/workspaces.html), and it is exactly what the Corvis repository itself uses: the framework crates and the examples (`examples/tertulia`, `examples/cats-app`, `examples/blog-orm`) are members of the same workspace.

```toml
# Cargo.toml at the root
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

Each application is created with `corvis new` inside `apps/` and run with Cargo from the root:

```bash
cargo run -p api
cargo test --workspace
```

The `corvis generate` commands work inside each application: run them from its folder (the CLI searches upward for the first `Cargo.toml` with `src/`). To share code between applications, see [Libraries](/cli/libraries).
