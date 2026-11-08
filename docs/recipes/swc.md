---
status: unavailable
---

# SWC (fast compiler)

This recipe does not apply to Corvis. In TypeScript frameworks, SWC replaces the TypeScript compiler to speed up the step from TypeScript to JavaScript. A Corvis application is Rust: `rustc` compiles it through Cargo and there is no transpilation step to replace.

What does affect compile time:

- **Debug mode in development.** `corvis start` and `cargo run` compile without optimizations, much faster than `--release`.
- **Incremental compilation.** Cargo enables it by default in debug: it only recompiles what changed.
- **Splitting into crates.** In a [Cargo workspace](/cli/monorepo), changing one crate does not force recompiling the others.
- **Automatic restart.** `corvis start --watch` recompiles and restarts on save; see [Hot reload](/recipes/hot-reload).
