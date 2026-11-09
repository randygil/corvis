# Spec Delta

## MODIFIED Requirements

### Requirement: Every corvis feature builds on its own

CI SHALL check the `corvis` library with clippy and `-D warnings` with no default features and with each of its
features enabled alone, on Linux and Windows.

#### Scenario: Library without default features

- **WHEN** CI runs `cargo clippy -p corvis --no-default-features -- -D warnings`
- **THEN** it passes

#### Scenario: One feature alone

- **WHEN** CI runs the same check with `--features jwt`, `--features websockets`, `--features health-tls` or
  `--features tls`
- **THEN** each passes

## ADDED Requirements

### Requirement: TLS tests run in CI

The Linux and Windows test jobs SHALL run the TLS integration tests with the `tls` feature enabled
(`cargo test -p corvis --features tls --test tls`), in addition to `cargo test --workspace`.

#### Scenario: A TLS regression

- **WHEN** a pull request breaks HTTPS serving
- **THEN** the TLS test step fails on both Linux and Windows
