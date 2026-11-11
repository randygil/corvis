# Spec Delta

## ADDED Requirements

### Requirement: Shared dependencies are declared once
A crates.io dependency that two or more published crates of the workspace use SHALL be declared once, in
`[workspace.dependencies]` of the root `Cargo.toml`, and the crates MUST refer to it with `workspace = true`, adding
features at the use site when they need more. `scripts/check-crate-files.mjs` MUST report a violation with the crates
and the dependency it names.

#### Scenario: The same dependency declared in two crates
- **WHEN** `crates/corvis/Cargo.toml` and `crates/corvis-swagger/Cargo.toml` both declare `regex = "1"`
- **THEN** `node scripts/check-crate-files.mjs` exits with status 1 and names `regex`, `corvis` and `corvis-swagger`

#### Scenario: A crate needs an extra feature
- **WHEN** `corvis-orm` needs `chrono` with the `serde` feature and `corvis` does not
- **THEN** `corvis-orm` declares `chrono = { workspace = true, features = ["serde"] }` and the check passes
