---
status: unavailable
---

# CI/CD integration

There is no Corvis integration with CI/CD systems (for example, to publish or compare the application graph on every change).

## Alternative

A Corvis project's pipeline is a Cargo pipeline:

```bash
cargo build --release
cargo test --workspace
```

The Corvis repository also uses `cargo deny check` (configured in `deny.toml`: security advisories, allowed licenses and sources) and `cargo audit` (`.cargo/audit.toml`, which ignores RUSTSEC-2023-0071 in `rsa` because `rsa` only reaches `Cargo.lock` through `sqlx-mysql`, which is never compiled); you can reuse that configuration. The workspace libraries declare `#![forbid(unsafe_code)]`. End-to-end tests are written with the [Testing](/fundamentals/testing) utilities and do not need to open a port.
