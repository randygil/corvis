# Spec Delta

## ADDED Requirements

### Requirement: Every corvis feature builds on its own

CI SHALL check the `corvis` library with clippy and `-D warnings` with no default features and with each of its
features enabled alone, on Linux and Windows.

#### Scenario: Library without default features

- **WHEN** CI runs `cargo clippy -p corvis --no-default-features -- -D warnings`
- **THEN** it passes

#### Scenario: One feature alone

- **WHEN** CI runs the same check with `--features jwt`, `--features websockets` or `--features health-tls`
- **THEN** each passes
