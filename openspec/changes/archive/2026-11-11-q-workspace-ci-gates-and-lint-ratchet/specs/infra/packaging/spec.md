# Spec Delta

## ADDED Requirements

### Requirement: Shared dependencies are declared once
A crates.io dependency declared more than once, by two published crates or in two dependency sections of one, SHALL be
declared in `[workspace.dependencies]` of the root `Cargo.toml` and referred to with `workspace = true`, with extra
features at the use site. `scripts/check-crate-files.mjs` MUST report each violation with the dependency and where it
is declared.

#### Scenario: The same dependency declared in two crates
- **WHEN** `crates/corvis/Cargo.toml` and `crates/corvis-swagger/Cargo.toml` both declare `regex = "1"`
- **THEN** `node scripts/check-crate-files.mjs` exits with status 1 and names `regex`, `corvis` and `corvis-swagger`

#### Scenario: The same dependency declared twice in one crate
- **WHEN** `crates/corvis/Cargo.toml` declares `brotli = { version = "9", optional = true }` and the dev-dependency
  `brotli = "9"`
- **THEN** `node scripts/check-crate-files.mjs` exits with status 1 and names `brotli`, `corvis`, `[dependencies]` and
  `[dev-dependencies]`

#### Scenario: A crate needs an extra feature
- **WHEN** `corvis-orm` needs `chrono` with the `serde` feature and `corvis` does not
- **THEN** `corvis-orm` declares `chrono = { workspace = true, features = ["serde"] }` and the check passes
