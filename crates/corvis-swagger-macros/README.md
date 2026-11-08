# corvis-swagger-macros

Procedural macros (`Api*` decorators and `#[derive(ApiSchema)]`) for [`corvis-swagger`](https://crates.io/crates/corvis-swagger), the OpenAPI module of the [Corvis](https://corvis.randygil.com) backend framework.

This crate is an implementation detail of `corvis-swagger`: it is released together with it, pinned to the exact same
version, and generates code that calls hidden items of `corvis-swagger`. Do not depend on it directly; depend on
`corvis-swagger`, which re-exports the macros.

- Documentation: <https://corvis.randygil.com>
- Repository: <https://github.com/randygil/corvis>

Licensed under the MIT license. Third-party notices are in `THIRD_PARTY_NOTICES.md`.
