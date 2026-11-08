# Proposal

Implements [randygil/corvis#10](https://github.com/randygil/corvis/issues/10) (epic [#1](https://github.com/randygil/corvis/issues/1)).

## Why

None of the seven library and CLI crates can be packaged for crates.io today. `cargo package` stops on `corvis`
because its `corvis-macros` dependency is a path with no version, and the same applies to `corvis-orm` and
`corvis-swagger`. The packages that do build ship without the MIT license text or the third-party notices, which the
licenses of the projects Corvis replicates require (MIT and Apache-2.0 both ask that the notices travel with copies).
`corvis-cli` has no description, repository or authors, and no crate declares `rust-version`, `homepage` or a full set
of categories. Publishing the names (#11) and any later release depend on this.

## What Changes

- Every published crate (`corvis`, `corvis-macros`, `corvis-orm`, `corvis-orm-macros`, `corvis-swagger`,
  `corvis-swagger-macros`, `corvis-cli`) gets a copy of `LICENSE` and `THIRD_PARTY_NOTICES.md` in its folder, so both
  are inside its `.crate` file.
- A check script (`scripts/check-crate-files.mjs`, Node, like `docs/scripts/check.mjs`) fails when a crate copy differs
  from the root file, when a published crate lacks a required metadata field, or when an intra-workspace dependency has
  no `version`.
- Cargo metadata:
  - `[workspace.package]` gains `rust-version` and `homepage = "https://corvis.randygil.com"`; every crate inherits
    them.
  - `corvis-cli` gains `description`, `authors`, `repository`, `keywords`, `categories` and a `readme`.
  - Every crate has `keywords` (at most five) and `categories` from the crates.io list.
  - The four proc-macro crates, `corvis-swagger` and `corvis-cli` get a short English `README.md` that points to the
    main crate and the documentation site.
- Intra-workspace dependencies get a `version`: `corvis` and `corvis-macros` in `[workspace.dependencies]`, new
  workspace entries for `corvis-orm-macros` and `corvis-swagger-macros`, and every crate uses `workspace = true` for
  them. Each proc-macro crate is pinned with `=` to the version of the crate that uses it.
- `deny.toml`: `wildcards` goes from `"warn"` to `"deny"`, as its own comment (lines 29-32) asks once versions are
  added.
- `CONTRIBUTING.md`: how to check packaging locally (`cargo package --workspace`, the check script) and the rule that
  the root `LICENSE` / `THIRD_PARTY_NOTICES.md` are the sources of the crate copies.

## Capabilities

### New Capabilities

- `infra/packaging`: what every published crate must contain and declare, how intra-workspace dependencies are
  versioned, and the check that keeps it so.

### Modified Capabilities

None.

## Impact

- Manifests: root `Cargo.toml` and the seven `crates/*/Cargo.toml`. No Rust source changes.
- New files: `crates/*/LICENSE`, `crates/*/THIRD_PARTY_NOTICES.md`, five crate READMEs, `scripts/check-crate-files.mjs`.
- `deny.toml`, `CONTRIBUTING.md`.
- `Cargo.lock` does not change (versions of workspace crates stay `0.1.0`).
- Disk: `cargo package --workspace` verifies each crate by building it under `target/package/`; run it once, in the
  shared target directory, and delete `target/package/` afterwards.
- Unblocks #11 (publishing 0.0.x of the names), which stays a separate, explicitly approved step.

## Out of scope

- Publishing anything to crates.io: that is #11 and needs explicit approval.
- `corvis new` without `--corvis-path`/`--corvis-version` writes a path dependency to the folder the CLI was built from
  (`crates/corvis-cli/src/main.rs:136-155`, default at line 145); a CLI installed from crates.io would point to a folder that does not exist.
  Defaulting to the published version: new issue, needed before a usable CLI release.
- Translating `THIRD_PARTY_NOTICES.md` (Spanish today) to English: new issue or part of #3.
- Making the root `README.md` (the readme of `corvis`, Spanish today) English: #3.
- Docs pages that say Corvis is not on crates.io yet or use path dependencies (`docs/first-steps.md:38`, `docs/cli/overview.md:66`,
  `docs/deployment.md:18`, `docs/openapi/introduction.md:17`, `docs/data/corvis-orm.md:17-18`): they stay true until #11.
- A CI job for `cargo package --workspace` and the check script: added to the CI workflow once #6 is merged (task 5.2
  covers it if #6 is already in `main`).
- Release automation, changelogs, version bump policy, `cargo-semver-checks`.
- Turning `multiple-versions` in `deny.toml` into an error (duplicates come from third-party crates).
