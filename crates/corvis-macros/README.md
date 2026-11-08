# corvis-macros

Procedural macros (decorators) for [`corvis`](https://crates.io/crates/corvis), a modular backend framework for Rust.

This crate is an implementation detail of `corvis`: it is released together with it, pinned to the exact same
version, and generates code that calls hidden items of `corvis`. Do not depend on it directly; depend on
`corvis`, which re-exports every macro.

- Documentation: <https://corvis.randygil.com>
- Repository: <https://github.com/randygil/corvis>

Licensed under the MIT license. Third-party notices are in `THIRD_PARTY_NOTICES.md`.
