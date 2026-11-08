# corvis-orm-macros

Procedural macros (`#[derive(Model)]`) for [`corvis-orm`](https://crates.io/crates/corvis-orm), the ORM of the [Corvis](https://corvis.randygil.com) backend framework.

This crate is an implementation detail of `corvis-orm`: it is released together with it, pinned to the exact same
version, and generates code that calls hidden items of `corvis-orm`. Do not depend on it directly; depend on
`corvis-orm`, which re-exports the macros.

- Documentation: <https://corvis.randygil.com>
- Repository: <https://github.com/randygil/corvis>

Licensed under the MIT license. Third-party notices are in `THIRD_PARTY_NOTICES.md`.
