---
status: partial
---

# Scripts

A Corvis project has no CLI-specific scripts file: the project is a Cargo crate and the build and run commands are Cargo's. `corvis start` and `corvis build` are shortcuts over them.

| Task | With the CLI | With Cargo |
|---|---|---|
| Run in development | `corvis start` | `cargo run` |
| Run and restart on save | `corvis start --watch` | `cargo watch -x run` (with `--release` and `--` arguments: see [Usage](/cli/usages#corvis-start)) |
| Build | `corvis build` | `cargo build` |
| Build for production | `corvis build --release` | `cargo build --release` |
| Run the tests | | `cargo test` |

`corvis start --watch` requires `cargo-watch` (`cargo install cargo-watch`). If it is missing, the command fails and explains how to install it. See [Usage](/cli/usages#corvis-start).

## Custom commands

If you want short names for project tasks, use Cargo aliases in `.cargo/config.toml`:

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

For longer tasks (migrations, model generation), the `corvis-orm` binary has its own subcommands; see [corvis-orm](/data/corvis-orm).
