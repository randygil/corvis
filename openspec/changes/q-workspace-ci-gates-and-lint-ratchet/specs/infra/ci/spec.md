# Spec Delta

## ADDED Requirements

### Requirement: Rustdoc builds without warnings
CI SHALL build the documentation of every workspace crate with `cargo doc --no-deps --all-features` and
`RUSTDOCFLAGS=-D warnings` in a job named `doc`, so a broken or private intra-doc link, an invalid HTML tag or a bare
URL in a doc comment fails the pull request. `corvis-cli`, whose binary has the name of the `corvis` library, MAY be
documented in a step of its own so the two never write the same output folder at once.

#### Scenario: A broken intra-doc link
- **WHEN** a pull request adds a doc comment that links to an item that does not exist
- **THEN** the `doc` job fails and names the file and line of the link

#### Scenario: A link to a private item
- **WHEN** a public item's doc comment links to a private function
- **THEN** the `doc` job fails

### Requirement: The declared minimum Rust version builds
CI SHALL install the toolchain named by `rust-version` in the workspace `Cargo.toml` and run `cargo check --locked` on
every published crate with default features and with all features, in a job named `msrv`. The toolchain MUST be read
from `Cargo.toml`, so the declaration and the check cannot disagree.

#### Scenario: Code that needs a newer toolchain
- **WHEN** a pull request uses a standard library function stabilized after the declared `rust-version`
- **THEN** the `msrv` job fails

#### Scenario: Raising the minimum version
- **WHEN** a pull request raises `rust-version`
- **THEN** the `msrv` job installs and checks with the new version, with no other change to the workflow

### Requirement: Each feature of every published crate builds on its own
CI SHALL build `corvis-orm` and `corvis-swagger` with each of their features alone, with no default features and with
all features (`cargo hack check --each-feature --no-dev-deps`), in a job named `features`. The `corvis` features keep
their clippy loop.

#### Scenario: A feature that only builds with another
- **WHEN** a pull request makes `corvis-orm` fail to build with `--no-default-features --features corvis`
- **THEN** the `features` job fails and names the feature set

### Requirement: Lint findings only go down
CI SHALL count, per published crate and per lint, the findings of the ratchet lints listed in
`scripts/lint-ratchet.mjs` and the unexplained `ignore` doctest fences, and compare them with `lint-baseline.json` in a
job named `lint-ratchet`. A count above the baseline MUST fail and list the new findings; a count below it MUST fail
until the baseline is lowered in the same pull request. Tests MUST NOT be counted for `unwrap_used`, `expect_used` and
`panic`.

#### Scenario: A new unwrap in library code
- **WHEN** a pull request adds `.unwrap()` to non-test code of `corvis-swagger`
- **THEN** the `lint-ratchet` job fails and names the file, the line and `clippy::unwrap_used`

#### Scenario: Findings fixed without lowering the baseline
- **WHEN** a pull request removes three `missing_docs` findings from `corvis` and leaves `lint-baseline.json` as it was
- **THEN** the job fails with the instruction to run `node scripts/lint-ratchet.mjs --update`

#### Scenario: An unwrap in a test
- **WHEN** a pull request adds `.unwrap()` inside a `#[cfg(test)]` module
- **THEN** the counts do not change and the job passes

#### Scenario: An ignored doctest with a reason
- **WHEN** a pull request adds a doc example fenced as ```` ```ignore (needs a database) ````
- **THEN** the fence is not counted

### Requirement: Public API changes are reported
CI SHALL run `cargo semver-checks` on every pull request against the merge base with `main`, in a job named `semver`,
and write its result to the job summary. Until the first release to crates.io the job MUST NOT be a required check.

#### Scenario: A removed public function
- **WHEN** a pull request removes a public function of `corvis-swagger`
- **THEN** the `semver` job's summary lists the removal, and the pull request can still be merged if the required
  checks pass

## MODIFIED Requirements

### Requirement: Every pull request runs the definition-of-done checks

The repository SHALL run a CI workflow on every pull request targeting `main`, and a maintainer SHALL be able to start
it by hand. A pull request MUST NOT be considered mergeable unless all of its required jobs pass.

The required jobs are `fmt` (`cargo fmt --all --check`), `clippy` (`-D warnings`), `test` (build with warnings
denied, then `cargo test --workspace`), `fuzz`, `deny` (`cargo deny check`), `audit` (`cargo audit`), `docs`
(`npm run docs:check` in `docs/`), `doc` (rustdoc with warnings denied), `msrv`, `features` and `lint-ratchet`.

#### Scenario: A clippy warning fails the pull request
- **WHEN** a pull request introduces code that produces a clippy warning
- **THEN** the `clippy` job fails and the pull request shows a failed required check

#### Scenario: A compiler warning fails the pull request
- **WHEN** a pull request introduces an unused import in any workspace crate, example or test
- **THEN** the build step of the `test` job fails because warnings are denied

#### Scenario: A broken docs link fails the pull request
- **WHEN** a pull request adds an English page without its Spanish counterpart under `docs/es/`
- **THEN** the `docs` job fails with the `[pages]` message of `docs/scripts/check.mjs`

#### Scenario: A manual run on main
- **WHEN** a maintainer starts the CI workflow by hand on `main`
- **THEN** every job runs as on a pull request

#### Scenario: A rustdoc warning fails the pull request
- **WHEN** a pull request adds a doc comment with a broken intra-doc link
- **THEN** the `doc` job fails and the pull request shows a failed required check

### Requirement: Changes reach main only through checked pull requests

Work SHALL land on `dev`, the integration branch, and reach `main` through a pull request. The `main` branch SHALL be
protected so that the CI jobs (`fmt`, `clippy` on Linux and Windows, `test` on Linux and Windows, `fuzz`, `deny`,
`audit`, `docs`, `packaging`, `doc`, `msrv`, `features`, `lint-ratchet`) are required status checks, enforced for
administrators too, with force pushes and deletion blocked.

A direct push to `main` MUST be rejected. `dev` SHALL block force pushes and deletion. The `semver` job MUST NOT be a
required check until the first release to crates.io.

#### Scenario: A direct push
- **WHEN** a maintainer pushes a commit straight to `main`
- **THEN** GitHub rejects the push because the required status checks have not passed on it

#### Scenario: A pull request with a failing check
- **WHEN** a required job fails on a pull request
- **THEN** the pull request cannot be merged until the job passes

#### Scenario: The lint ratchet is required
- **WHEN** the `lint-ratchet` job fails on a pull request to `main`
- **THEN** the pull request cannot be merged
