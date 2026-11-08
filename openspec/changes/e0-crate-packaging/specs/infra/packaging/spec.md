# Spec Delta

## ADDED Requirements

### Requirement: Every published crate packages cleanly

Every workspace crate without `publish = false` (`corvis`, `corvis-macros`, `corvis-orm`, `corvis-orm-macros`,
`corvis-swagger`, `corvis-swagger-macros`, `corvis-cli`) SHALL package with `cargo package` (naming the seven crates with `-p`; `--workspace` also tries the examples with Cargo 1.99 and fails on them) without errors
and without manifest warnings. Crates with `publish = false` (the examples and `bench/corvis-app`) MUST NOT be
packaged.

#### Scenario: Packaging the workspace succeeds
- **WHEN** a contributor runs `cargo package --allow-dirty -p <each of the seven crates>` on a clean checkout of `main`
- **THEN** seven `.crate` files are produced under `target/package/`, each crate's verification build succeeds, and
  cargo prints no `warning: manifest has no ...` line

#### Scenario: Examples are never packaged
- **WHEN** `cargo package` runs for the seven crates
- **THEN** no `.crate` file is produced for `blog-orm`, `cats-app`, `tertulia` or `corvis-app`

### Requirement: Intra-workspace dependencies declare a version

Every dependency of a published crate on another workspace crate SHALL declare both `path` and `version`, through
`[workspace.dependencies]`. A dependency on a proc-macro crate of the workspace MUST use an exact requirement
(`=x.y.z`) equal to the version of both crates, because the generated code calls the hidden `__private` API of the
runtime crate. `cargo deny check bans` SHALL reject wildcard requirements (`wildcards = "deny"`), while path
dependencies of `publish = false` crates stay allowed.

#### Scenario: A path dependency without version is rejected
- **WHEN** a contributor adds `foo = { path = "../foo" }` to a published crate
- **THEN** `cargo deny check bans` fails with a wildcard error and the check script names the crate and dependency

#### Scenario: Macro crates are pinned
- **WHEN** the packaged `corvis` manifest (`target/package/corvis-0.1.0/Cargo.toml`) is inspected
- **THEN** its `corvis-macros` dependency is `version = "=0.1.0"` and has no `path`

### Requirement: License and notices travel with every crate

The `.crate` file of every published crate SHALL contain `LICENSE` and `THIRD_PARTY_NOTICES.md` at its root, with the
same bytes as the files at the repository root. The repository-root files are the only ones edited by hand; a check
MUST fail when a crate copy differs from its source.

#### Scenario: Package contains the license and notices
- **WHEN** `cargo package --list -p corvis-swagger` runs
- **THEN** the list includes `LICENSE` and `THIRD_PARTY_NOTICES.md`

#### Scenario: A stale copy is caught
- **WHEN** a contributor edits the root `THIRD_PARTY_NOTICES.md` without updating the crate copies
- **THEN** `node scripts/check-crate-files.mjs` exits non-zero and names every out-of-date copy

### Requirement: Crate metadata is complete

Every published crate SHALL declare `description`, `license`, `authors`, `repository`, `homepage`, `rust-version`,
`readme`, between one and five `keywords`, and at least one `categories` entry from the crates.io category list.
`license`, `authors`, `repository`, `homepage`, `edition` and `rust-version` MUST be inherited from
`[workspace.package]`, so they are set in one place. `rust-version` MUST be a version on which
`cargo check --workspace` succeeds.

#### Scenario: Missing metadata is caught
- **WHEN** a published crate's manifest has no `categories`
- **THEN** `node scripts/check-crate-files.mjs` exits non-zero naming the crate and the missing field

#### Scenario: Declared minimum Rust version builds
- **WHEN** `cargo +<rust-version> check --workspace` runs with the toolchain named in `[workspace.package]`
- **THEN** it succeeds
