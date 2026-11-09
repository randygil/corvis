---
status: documented
---

# Hot reload

Rust compiles to a native binary, so there is no module replacement in a running process. What you can have is **recompiling and restarting** the application every time you save a file:

```bash
cargo install --locked watchexec-cli     # only once
corvis start --watch
```

`corvis start --watch` runs `cargo run` under [watchexec](https://github.com/watchexec/watchexec): it watches the project, recompiles when a file changes and launches the server again. `--release` and the arguments after `--` are included: `corvis start --watch --release -- --port 4000` restarts `cargo run --release -- --port 4000`. The exact command and the install options are in [Usage](/cli/usages#corvis-start). If watchexec is not installed, `corvis start --watch` stops with a message that shows `cargo install --locked watchexec-cli`.

On Windows, watchexec can only force-terminate the running server when it restarts it, so shutdown hooks do not run on each restart during development. On Linux and macOS it sends a termination signal first.

## Compile times

In medium-sized projects, the time of each restart is set by compilation. Some measures that help:

- **Compile in debug mode** during development (what `corvis start` does); leave `--release` for production.
- **Split the project into crates** inside a [workspace](/cli/monorepo): Cargo only recompiles the ones that changed.
- **Keep an eye on the `target` folder**: each configuration (debug, release, tests) generates its own copy of the dependencies and can take up several GB.

## State between restarts

Each restart creates a new process, so in-memory state (caches, sessions, WebSocket connections) is lost. WebSocket clients must reconnect; the `examples/tertulia` client does it on its own.
