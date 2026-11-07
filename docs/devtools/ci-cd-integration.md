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

The Corvis repository also uses `cargo deny check` (configured in `deny.toml`: security advisories, allowed licenses and sources) and `cargo audit` (`.cargo/audit.toml`); you can reuse that configuration. End-to-end tests are written with the [Testing](/fundamentals/testing) utilities and do not need to open a port.
