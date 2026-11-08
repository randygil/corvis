# Design

## Context

Verified on 2026-11-08 by reading the manifests and running `cargo package --list -p <crate> --allow-dirty` (which
does not compile) and `cargo package --no-verify -p corvis --allow-dirty` (which stops before any build).

**Workspace** (`Cargo.toml`). Members: the seven crates, `examples/*` and `bench/corvis-app`. `[workspace.package]`
sets `version = "0.1.0"`, `edition = "2024"`, `license = "MIT"`, `authors` and `repository`; no `rust-version`,
`homepage` or `documentation`. `[workspace.dependencies]` declares `corvis = { path = "crates/corvis" }` and
`corvis-macros = { path = "crates/corvis-macros" }`, both without `version`. The examples (`blog-orm`, `cats-app`,
`tertulia`) and `bench/corvis-app` have `publish = false`.

**Per crate** (manifest fields; `readme`, `keywords`, `categories`; files in the package list besides `src/**/*.rs`):

| Crate | Metadata gaps | Intra-workspace deps | Package list |
|---|---|---|---|
| `corvis` | one category (`web-programming::http-server`); `readme = "../../README.md"` (Spanish today, #3) | `corvis-macros` (workspace, no version) | 122 entries: `README.md` (copied from the root), `proptest-regressions/multer/fuzz.txt`, 20 `tests/*.rs`, `tests/fixtures/` (JSON, `.mjs` oracle scripts, `jwt_rsa_*.pem`) |
| `corvis-macros` | no `keywords`, `categories`, `readme` | none | 12 entries, sources only |
| `corvis-cli` | only `name`, `version`, `edition`, `license`: no `description`, `authors`, `repository`, `keywords`, `categories`, `readme`. `cargo package --list` prints `warning: manifest has no description, documentation, homepage or repository` | none | 12 entries, sources plus `tests/cli_input.rs` |
| `corvis-orm` | no `categories` | `corvis` (workspace, optional, no version), `corvis-orm-macros = { path = "../corvis-orm-macros" }` (no version) | 49 entries: `README.md` (Spanish), `proptest-regressions/fuzz_filters.txt`, `tests/` with `fixtures/oracle/{migration.sql,results.json,schema.prisma}` and `generated/*.rs` |
| `corvis-orm-macros` | no `keywords`, `categories`, `readme` | none | 5 entries |
| `corvis-swagger` | no `readme` | `corvis` (workspace, no version), `corvis-swagger-macros = { path = "../corvis-swagger-macros", version = "0.1.0" }` | 23 entries, including the four `src/assets/*` embedded with `include_str!` |
| `corvis-swagger-macros` | no `keywords`, `categories`, `readme` | none | 7 entries |

**No package contains `LICENSE` or `THIRD_PARTY_NOTICES.md`.** Both exist only at the repository root. The license is
MIT (`LICENSE`, "Copyright (c) 2026 Randy Gil"). `THIRD_PARTY_NOTICES.md` (434 lines, Spanish) lists the MIT,
BSD-3-Clause and Apache-2.0 projects whose API, messages, templates and data Corvis replicates, with their license
texts; for example, the Swagger UI templates in `crates/corvis-swagger/src/assets/swagger-ui-*.tpl` and the Scalar
theme (`scalar-theme.css`) come from Swagger UI (Apache-2.0), the original framework's swagger module (MIT) and Scalar (MIT), per lines 15, 50 and 51 of the notices, and the zod regexes and messages used by `corvis::z` are MIT.

**Packaging errors.** `cargo package --no-verify -p corvis --allow-dirty` fails with:

```
all dependencies must have a version requirement specified when packaging.
dependency `corvis-macros` does not specify a version
```

`corvis-orm` (`corvis`, `corvis-orm-macros`) and `corvis-swagger` (`corvis`) have the same problem.
`corvis-macros` packages (12 files, 89.3 KiB). `deny.toml:29-34` already notes that these versionless path
dependencies are the only source of `wildcards` warnings and asks to switch to `"deny"` once versions are added;
`allow-wildcard-paths = true` keeps path dependencies of `publish = false` crates allowed.

**Minimum Rust version.** Edition 2024 needs Rust 1.85. The code uses let chains (`&& let` in
`crates/corvis/src/modules/schedule/cron_job.rs:426,446` and `crates/corvis-macros/src/handlers.rs:43`), stable since
1.88. Dependencies may need a newer version; that was not measured (it needs a build). The local toolchain is
1.99.0.

**Paths outside the package.** Tests and doc fixtures that read files use paths inside the crate
(`include_str!("fixtures/...")`, `env!("CARGO_MANIFEST_DIR")/tests/fixtures/...`), except
`crates/corvis-orm/tests/common/pg.rs:36` (the external `corvis-tests` folder, skipped when missing) and
`crates/corvis-cli/src/main.rs:145` (the CLI's default `corvis` path, see Out of scope).
`crates/corvis/src/jwt.rs:468-469` embeds the test PEM keys only under `#[cfg(test)]`.

**crates.io names.** `cargo search corvis` returned no results on 2026-11-08; reserving them is #11.

## Goals / Non-Goals

**Goals:**
- `cargo package --workspace` succeeds for the seven crates with no manifest warnings.
- Every `.crate` contains the license and the notices, byte-identical to the root.
- Metadata complete and inherited from one place where possible.
- A cheap check (no compilation) that keeps all of the above from drifting.

**Non-Goals:**
- Publishing (#11), changing versions, or deciding a release process.
- Changing any Rust source or behavior.

## Decisions

1. **Committed copies of `LICENSE` and `THIRD_PARTY_NOTICES.md` in each crate, checked against the root.**
   Alternatives: `license-file = "../../LICENSE"` (cargo copies it into the package, but it replaces the SPDX
   `license = "MIT"` that crates.io and `cargo deny` read, and there is no equivalent for the notices); symlinks
   (unreliable on Windows checkouts, where git writes them as text files unless `core.symlinks` is on); a `build.rs`
   that copies the files (runs at build time, not at package time, and adds a build script to every crate). Copies
   are simple and visible; the check script prevents drift.
2. **Exact pins for proc-macro crates, caret for runtime crates.** `corvis-macros`, `corvis-orm-macros` and
   `corvis-swagger-macros` generate code that calls `::corvis::__private::*` and other hidden items, so a macro crate
   from another release could produce code that does not compile. `=0.1.0` keeps each runtime crate with the macro
   version it was built with (the approach used by serde with `serde_derive`). `corvis-orm` and `corvis-swagger`
   depend on `corvis` with a normal `0.1.0` requirement. Alternative: caret everywhere (lets cargo mix macro and
   runtime versions inside one 0.1.x series).
3. **All intra-workspace dependencies through `[workspace.dependencies]`.** One place holds the path and version, so
   a version bump touches the root manifest only. `corvis-swagger` stops spelling its own `version = "0.1.0"`.
4. **`rust-version` measured, not guessed.** Start at 1.88 (let chains) and raise it until
   `cargo +<version> check --workspace` passes; record the result. Alternative: declare the current stable (1.99),
   which shuts out users with no reason.
5. **A Node check script, not a Rust test.** `scripts/check-crate-files.mjs` reads the manifests with a small TOML
   subset parser or `cargo metadata --no-deps --format-version 1` (JSON) and compares files. It needs no build, so it
   runs in seconds locally and in CI, like `docs/scripts/check.mjs`. Alternative: a `#[test]` in some crate (needs a
   build and belongs to no crate in particular).
6. **Keep tests and fixtures in the packages.** They are small (largest package list: 122 entries for `corvis`) and
   let downstream packagers run the tests. `proptest-regressions/` stays too (proptest replays its seeds when the
   tests run). Alternative: an `include` allow-list of `src/`, `Cargo.toml`, README and license files (smaller
   tarballs, but every new asset under `src/` or fixture must be remembered). Re-evaluate if a package nears the
   crates.io 10 MB limit.
7. **Short English READMEs for crates that have none.** crates.io shows the readme as the crate page; the macro
   crates say they are implementation details of their parent crate and should not be depended on directly.
   `corvis` keeps the root README and `corvis-orm` its own README (both Spanish today; #3 and a follow-up translate
   them).
8. **Categories and keywords.**
   - `corvis`: `web-programming::http-server`, `asynchronous`; keywords unchanged.
   - `corvis-macros`, `corvis-orm-macros`, `corvis-swagger-macros`: `development-tools::procedural-macro-helpers`;
     keywords `corvis`, `macros`, plus `orm` / `openapi` where it applies.
   - `corvis-orm`: `database`, `asynchronous`; keywords unchanged (five already).
   - `corvis-swagger`: add `web-programming`; keywords unchanged.
   - `corvis-cli`: `command-line-utilities`, `development-tools`; keywords `corvis`, `cli`, `scaffolding`,
     `generator`.

## Risks / Trade-offs

- **Notices duplicated eight times.** Whoever edits the root file must run the check script; the script names the
  copies to refresh. With CI (#6) the check becomes automatic.
- **Exact macro pins** force publishing all crates of a release together. That is the intended release unit.
- **`cargo package --workspace` builds every crate under `target/package/`**, adding disk use in the shared target
  directory; the task deletes it after the run. It requires a cargo that packages workspace members against each
  other before they are published (the local 1.99 toolchain does).
- **Test PEM keys inside the `corvis` package** (`tests/fixtures/jwt_rsa_private.pem`) may trip secret scanners of
  mirrors. They are test-only keys; the README of the fixtures folder (if added) can say so. Excluding them would
  break `cargo test` on the packaged source.
- **The CLI package works but its default output does not** outside the repository (Out of scope in the proposal).

## Open questions

- Is pinning the proc-macro crates with `=` acceptable, or does the maintainer prefer caret requirements?
- MSRV policy: is the measured `rust-version` a promise (with an MSRV CI job later), or only informative?
- Should `THIRD_PARTY_NOTICES.md` be translated to English before #11, given that it ships with every crate?
- Should the crates set `documentation` to the docs site instead of the docs.rs default? This change leaves it unset
  (docs.rs) and uses the site as `homepage`.

## Review notes (2026-11-08)

- Macro crates pinned with `=0.1.0` (they are released together with their parent crate).
- `rust-version` is informative for now; an MSRV job in CI is a later issue, not part of #6 or this change.
- `THIRD_PARTY_NOTICES.md` is translated to English in this change before it is copied into every crate (public
  repository text is English); add it as a task before the copy step.
- `documentation` stays on docs.rs (the default); `homepage` points to https://corvis.randygil.com.
- The `corvis new` path dependency problem for a CLI installed from crates.io is filed as a separate issue.
