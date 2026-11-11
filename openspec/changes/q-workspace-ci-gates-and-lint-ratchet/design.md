# Design

## Context

Source: finding S8 of the engineering-quality review recorded in the issue (file
`quality/q-workspace-ci-gates-and-lint-ratchet.issue.md`). Read on `dev` at `d46758a` (2026-11-11): `Cargo.toml`,
the seven published crates' manifests, `deny.toml`, `clippy.toml`, `.github/workflows/ci.yml`, `nightly.yml`,
`scripts/check-crate-files.mjs`, `scripts/check-naming.mjs`, and the specs `infra/ci` and `infra/packaging`. Nothing
was compiled: the counts this design depends on (lint findings, rustdoc warnings, the oldest toolchain that builds)
are measured by the first task group, and the decisions say what happens for each outcome.

### The findings, checked against the code

| Finding | Where now | Verdict |
|---|---|---|
| No `[workspace.lints]`, no `missing_docs` / `unreachable_pub` policy | `Cargo.toml` (no `[lints]` anywhere); `crates/corvis/src/__private.rs:7` `#![allow(missing_docs)]`; crates only carry `#![forbid(unsafe_code)]` (`corvis`, `corvis-orm`, the three macro crates, `corvis-swagger`) | Confirmed. `corvis-cli` has no `forbid` because `src/posture/tree.rs` needs `unsafe` (signals, Job Objects): a workspace-wide `forbid(unsafe_code)` is not possible |
| No `cargo doc -D warnings` in CI | `ci.yml` (no doc job); broken link `crates/corvis-swagger/src/explorer.rs:31` (`` [`crate::model`] ``) | Confirmed. Sibling: `crates/corvis-swagger/src/scalar.rs:142` links the private `escape_inline_json` (also recorded in the Implementation notes of the archived `e13-apischema-ignores-validation-constraints`). The full count needs a run (task 1.1) |
| Swagger doc examples never compiled | ```` ```ignore ```` at `crates/corvis-swagger/src/lib.rs:10`, `types.rs:108`, `scalar.rs:6`, `document.rs:94` and one more | Confirmed. Wider than reported: about 67 `ignore` fences in `corvis`, 14 in `corvis-orm`, one in each macro crate. Converting them is per-crate work (the swagger ones in `q-openapi-serving-and-api-hygiene`); this change counts them in the ratchet (Decision 1) |
| MSRV declared, never built | `Cargo.toml:23` `rust-version = "1.89"`; `ci.yml:29` `RUST_TOOLCHAIN: "1.99.0"` | Confirmed. Whether 1.89 builds is unknown (edition 2024 needs 1.85; the code uses `if let … && let` chains, stable for edition 2024 since 1.88; dependencies may declare more). Measured in task 1.3 |
| `tokio` `full` and `serde_json` `preserve_order` forced on applications | `Cargo.toml:31-32` (`[workspace.dependencies]`), inherited by `corvis` and `corvis-orm` | Confirmed as facts. Maintainer decisions (Open questions 1 and 2); `preserve_order` is load-bearing (see there) |
| Dependencies outside `[workspace.dependencies]` | `regex = "1"` in `crates/corvis/Cargo.toml` and `crates/corvis-swagger/Cargo.toml`; `tokio-tungstenite = "0.24"` both as optional dependency and dev-dependency of `corvis` | Confirmed. Siblings: `chrono` (`corvis`: `clock, std`; `corvis-orm`: `clock, std, serde`), `futures = "0.3"` and `sha2 = "0.10"` in `corvis` and `corvis-orm`; `syn`, `quote`, `proc-macro2` in the three macro crates |
| No feature matrix | Only the clippy loop over `corvis` features (`ci.yml:72-76`) | Confirmed. `corvis-orm` (`corvis`, `tls-rustls`, both default) is never built with either off |
| No `cargo-semver-checks` | — | Confirmed. The crates are at 0.1.0 and the `packaging` job packages them; a baseline has to come from git until the first release |
| No coverage | — | Confirmed; out of scope (proposal) |
| Benchmark not in CI | — | Dropped as a CI gate: shared runners give timings that cannot decide a merge, and the house rule forbids comparing timings in tests. `bench/corvis-app` already compiles in `cargo build --workspace --all-targets` |
| deny / audit with reasons, actions pinned by SHA, fuzz, Linux + Windows | `deny.toml`, `.cargo/audit.toml`, `ci.yml` | Confirmed (the review's "what is good") |

Found while confirming: every CI job that compiles uses `-D warnings` (`RUSTFLAGS` in the test jobs, `-- -D warnings`
in clippy). A lint added at `warn` to `[workspace.lints]` is therefore an error in every job the moment it lands. This
is what Decision 1 is built around.

## Goals / Non-Goals

**Goals:**

- Land first, with no finding fixed except the broken doc links, and make every later quality fix stick.
- Each new check is cheap to keep green, says exactly what failed, and is either required or explicitly informational.
- The declared MSRV is true.

**Non-Goals:**

- Fixing the counted lint findings (they go down in the changes that touch each crate).
- Coverage, benchmarks in CI, new toolchain features.
- Changing what `tokio` and `serde_json` features applications get.

## Decisions

### 1. A lint floor plus a ratchet, not lints at `warn`

- **Floor.** `[workspace.lints]` in `Cargo.toml` lists the lints that produce no finding in any published crate today
  (task 1.2 runs every candidate once and keeps those at zero). Every published crate gets `[lints] workspace = true`.
  The floor is enforced by the existing `-D warnings` jobs; nothing new to run.
- **Ratchet.** The candidates with findings are counted, not denied. `scripts/lint-ratchet.mjs` runs
  `cargo clippy --workspace --all-targets --message-format=json` with the ratchet lints raised to `warn` on the command
  line (no `-D warnings`), groups the findings by crate, lint and file kind (library code versus tests, using the
  `src` path and `#[cfg(test)]` spans clippy reports), and compares the counts with `lint-baseline.json`:
  - a count above the baseline fails, listing the new findings (file, line, lint);
  - a count below the baseline also fails, with the message "lower the baseline: `node scripts/lint-ratchet.mjs
    --update`", so the baseline only moves down and the commit that fixes findings records it;
  - a lint at zero in every crate is reported as "ready to move to `[workspace.lints]`" (a reminder, not a failure).
- **Ratchet lints (initial set).** rustc: `missing_docs`, `unreachable_pub`, `missing_debug_implementations`,
  `unsafe_op_in_unsafe_fn`. clippy: `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`, `dbg_macro`,
  `undocumented_unsafe_blocks`, `missing_panics_doc`, `missing_errors_doc`; `print_stdout` and `print_stderr` for the
  library crates only (not `corvis-cli`). `clippy.toml` gains `allow-unwrap-in-tests`, `allow-expect-in-tests`,
  `allow-panic-in-tests` (`true`), so tests do not count. Scope: the seven published crates; examples and `bench/` are
  out (they are not published and print by design).
- **Doctest fences.** The script also counts ```` ```ignore ```` fences in `crates/*/src` per crate (new ones must be
  `no_run` or carry a reason on the fence line: ```` ```ignore (needs a database) ````, which the script does not count).
- **Platform.** The job runs on Linux; `cfg(windows)` code is not counted there. Written in the script's header.

Why this order works for the other changes: `q-openapi-serving-and-api-hygiene` removes panics and `unwrap` from
library paths of `corvis-swagger` and lowers its baseline; a later regression in that crate fails the ratchet.

Alternatives considered: (a) the lints at `warn` in `[workspace.lints]` — fails every job at once; (b) per-crate
`#![warn(...)]` with `--cap-lints warn` in CI — loses `-D warnings` for everything else; (c) `#![deny]` only in crates
already clean — leaves the others unchecked and cannot express "no new `unwrap` in `corvis`". Rejected.

Verified by: the script's self-test (`--self-test`, as `check-naming.mjs` has) on built-in samples: a count above,
below and equal to the baseline, a reason-carrying `ignore` fence, a test-only finding; and in CI by a deliberate
`unwrap()` in a draft pull request (task 3.4).

### 2. Rustdoc with warnings denied

Job `doc`: `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features` on Linux. It denies the default
rustdoc lints (`broken_intra_doc_links`, `private_intra_doc_links`, `invalid_html_tags`, `bare_urls`…). The two known
broken links are fixed in this change (`explorer.rs:31` points to `crate::types::SwaggerType` and the `api_extra_models`
macro, which are what the field holds; `scalar.rs:142` names `escape_inline_json` without a link, since it is private).

If task 1.1 finds more than 30 warnings in total, the job builds the crates that are clean with `-p` and the list of
the others stays in the job with a comment naming the follow-up; at 30 or fewer, they are all fixed here (doc comments
only). Alternative considered: `--document-private-items`. Rejected for now: it documents internals users never see and
multiplies the first count.

Verified by: the job itself on the pull request, and a deliberate broken link in a draft pull request (task 3.4).

### 3. An MSRV job, and a true `rust-version`

Job `msrv`: installs the toolchain named by `rust-version` (read from `Cargo.toml` by the job, so there is one source),
then `cargo check --locked -p corvis -p corvis-macros -p corvis-orm -p corvis-orm-macros -p corvis-swagger
-p corvis-swagger-macros -p corvis-cli` with default features and once with `--all-features`. Library and binary
targets only: the promise is to users of the crates, not to the test suites, which may use newer toolchains.

Task 1.3 checks 1.89. If it fails, the change raises `rust-version` to the oldest stable toolchain that passes (found
by bisecting the minor versions) and records the reason (the dependency or language feature) in "Implementation
notes"; README and the docs page that states requirements are updated. A false declaration is worse than a higher one:
Cargo uses `rust-version` to refuse old toolchains with a clear message and, with `resolver = "3"`, to pick dependency
versions; a wrong value hands users a compile error inside the framework instead.

Alternative considered: pin dependencies to old versions to keep 1.89. Rejected: older versions of security-relevant
crates for a promise nobody asked for.

### 4. `features`: each feature of each published crate on its own

Job `features`: `cargo hack check --each-feature --no-dev-deps -p corvis-orm -p corvis-swagger` (each feature alone,
no default features, all features). `corvis` keeps its clippy loop on Linux and Windows (the `infra/ci` requirement
"Every corvis feature builds on its own"); `cargo hack` adds the crates the loop does not cover. `corvis-swagger` has no
features today; the job already includes it so the features `q-openapi-schemas-for-rust-types-and-versions` adds are
built from the day they land. `cargo-hack` is installed with `taiki-e/install-action` at an exact version.

Alternative considered: `--feature-powerset`. Rejected for now: `corvis` has eleven features (2,048 sets); each-feature
plus the existing combined test steps covers the interactions that have caused bugs so far.

### 5. `semver`: reported, not required, until the first release

Job `semver`: `cargo semver-checks --workspace --baseline-rev <merge base with origin/main>` on pull requests, with
the result in the job summary. It is **not** a required check: before 1.0 the house rule is that public API that must
change changes now, and the other `q-` changes do exactly that. What the job buys now is visibility: a pull request that
changes public API without saying so shows it. It becomes required after the first release to crates.io, with the
published version as baseline (Workflow follow-up). Proc-macro crates are skipped by the tool.

Alternative considered: no job until 1.0. Rejected: the API hygiene changes need a way to see what they changed.

### 6. Shared dependencies declared once

`[workspace.dependencies]` gains `regex`, `chrono` (`default-features = false, features = ["clock", "std"]`; `corvis-orm`
adds `serde` at its use site), `futures`, `sha2`, `syn`, `quote`, `proc-macro2` and `tokio-tungstenite`; the crates use
`workspace = true` (adding features where they need more). The duplicate `tokio-tungstenite` dev-dependency of `corvis`
becomes `tokio-tungstenite = { workspace = true }`. `Cargo.lock` must not change versions (task 4.1 checks
`git diff Cargo.lock` is empty or only reorders).

`scripts/check-crate-files.mjs` (the `packaging` job) gains a rule: a crates.io dependency declared by two or more
published crates must come from `[workspace.dependencies]`; the message names the crates and the dependency.

Coordination: `e2-socket-io-hot-path-and-api-hygiene` upgrades `tokio-tungstenite` (its task 5.4). Whichever lands
second takes the other's line: the version lives in one place after this change.

### 7. Required checks

After this change the required status checks of `main` are the existing ones plus `doc`, `msrv`, `features` and
`lint-ratchet`. `semver` is informational. Branch protection is updated by the maintainer (task 6.4 lists the exact
names); the `infra/ci` spec delta says so.

## Security invariants this change comes near

| Invariant | Source | Pinned by |
|---|---|---|
| CI requests only `contents: read`, uses no secrets in pull-request jobs, pins third-party actions by SHA | `infra/ci` "CI runs with least privilege" | The new jobs use the same `permissions`, the same pinned `dtolnay/rust-toolchain`, `Swatinem/rust-cache` and `taiki-e/install-action` SHAs; task 6.2 greps `ci.yml` for `uses:` lines without a 40-character SHA |
| Dependency policy only in `deny.toml` / `.cargo/audit.toml` | `infra/ci` "Dependency policy and advisories are enforced" | No new crate in `Cargo.lock` (task 4.1); `cargo deny check` in the Definition of done |
| One generation of each authentication crate (`sha2`, `subtle`…) | `deny.toml` `[bans]` | Moving `sha2` to `[workspace.dependencies]` keeps one version; `cargo deny check` |

## Behavior visible to applications

| What | Before | After | Consequence |
|---|---|---|---|
| Minimum toolchain | Declared 1.89, never checked | Declared value built in CI; possibly raised (Decision 3) | Users on a toolchain below the new value get Cargo's `rust-version` message |
| Everything else | — | Unchanged | Manifests, CI and doc comments only |

## Risks / Trade-offs

- The first ratchet baseline is large → it is a number, not a to-do list; it only has to stop growing. The other `q-`
  changes lower it for their crates.
- The ratchet depends on clippy's JSON output and lint names → the toolchain is pinned in one place (`ci.yml` env); a
  toolchain bump that renames a lint fails the script with the unknown lint named, and the bump pull request updates
  the baseline (the same rule as today's "toolchain bump is explicit").
- `msrv` may force a higher `rust-version` → Decision 3; recorded and documented.
- Other open changes edit `ci.yml` (`e2-socket-io-differential-tests`, `e2-socket-io-docs-and-example`,
  `e2-socket-io-redis-adapter`, `e3-authentication-hot-path-and-consolidation`) and `e2-socket-io-differential-tests`
  modifies the same required-checks requirement → the new jobs are separate jobs at the end of the file (merge
  conflicts stay trivial), and the spec delta's job list is merged with whatever was archived meanwhile (task 6.6).
- `cargo semver-checks` on a git baseline needs the merge base checked out → `actions/checkout` with `fetch-depth: 0`
  for that job only.

## Open questions

1. **`tokio` `full` (maintainer decision, not taken here).** `[workspace.dependencies]` declares
   `tokio = { version = "1", features = ["full"] }` (`Cargo.toml:32`), and `corvis` and `corvis-orm` inherit it, so
   every application gets `full` (fs, io-util, io-std, macros, net, parking_lot, process, rt, rt-multi-thread, signal,
   sync, time). A grep of `crates/corvis/src`, `crates/corvis-orm/src` and `crates/corvis-swagger/src` finds uses of
   `tokio::time`, `sync`, `task`, `io`, `net`, `fs` and `signal`, the runtime and `select!`, and no `tokio::process`.
   Narrowing would drop `process`, `io-std` and `parking_lot` from applications that do not ask for them (some compile
   time); an application that uses `tokio::process` without declaring `tokio` itself would stop compiling. The three
   port apps declare `tokio` with `full` themselves; a project from `corvis new` does not depend on `tokio` directly.
   Recommendation: narrow to the features the framework uses, in its own issue, with a changelog line.
2. **`serde_json` `preserve_order` (maintainer decision, not taken here).** It is load-bearing, not incidental:
   - `crates/corvis-swagger/src/js.rs:47` calls `Map::shift_remove`, which only exists in the `preserve_order` build;
     the OpenAPI generator's byte-for-byte key order (the parity check compares documents as text) rests on insertion
     order throughout `js.rs`.
   - `crates/corvis/src/http/exception.rs:264` (`shift_remove("statusCode")`, the error body's key order),
     `crates/corvis/src/http/json_parse.rs:237` (request bodies ordered like `Object.keys`),
     `crates/corvis/src/validation/rules.rs:305`, the zod module (`crates/corvis/src/z/mod.rs:368`,
     `z/json_schema.rs:105,316-324,553,559`, issue key order in `z/error.rs:99-110`), session data
     (`crates/corvis/src/auth/session/session.rs:349`, `middleware.rs:144`, `login.rs:50,63`) and the ORM's result key
     order (`crates/corvis-orm/src/engine/read.rs:5`).
   - `crates/corvis/src/socket_io/value.rs:57-62` already measures both builds, so Socket.IO does not depend on it.
   - Cargo features are additive: any crate in the graph that enables `preserve_order` enables it for the whole
     application; the three port apps enable it themselves.
   Dropping it from what the framework forces would require replacing `serde_json::Map` with an ordered map of the
   framework's own in every place above, or the JSON the framework writes stops matching the original byte for byte.
   Recommendation: keep it, and document why in the docs page on dependencies (a follow-up issue if accepted).
3. **Doc warnings above 30** (Decision 2): fix all here or ratchet per crate? Recommendation: per crate above 30.
4. **Make `semver` required now?** Recommendation: no, until the first release (Decision 5).

## Review notes

These settle the open questions this change can settle and override the design where they differ.

1. Open questions 1 and 2 stay with the maintainer. This change does not touch `tokio` or `serde_json` features; the
   Workflow follow-up asks the maintainer and, for `preserve_order`, the facts above go into the follow-up issue.
2. Doc warnings: at most 30 are fixed here; above that, the clean crates are denied and the rest listed (Decision 2).
3. `semver` is informational until the first crates.io release.
4. `rust-version` is raised if 1.89 does not build; the change does not pin old dependency versions to keep it.
5. The required checks added to `main` are exactly `doc`, `msrv`, `features`, `lint-ratchet`.

## Implementation notes

Measured on `dev` at `858230c` (2026-11-11) with the pinned toolchain 1.99.0, on Windows and, for every count the CI
jobs depend on, on Linux (a `rust:1.99.0-bookworm` container sharing the Cargo target directory).

### Task 1.1: rustdoc warnings

`RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features` on the untouched tree: 8 warnings, so all
are fixed here (Decision 2, at most 30). Both known links are among them.

| Crate | File:line | Lint | Fix |
|---|---|---|---|
| `corvis` | `src/z/regex.rs:58` | `private_intra_doc_links` (`BACKTRACK_LIMIT`, private module) | named without a link |
| `corvis` | `src/posture.rs:298`, `:300` | `redundant_explicit_links` | explicit target removed |
| `corvis` | `src/z/locales.rs:8` | `redundant_explicit_links` | explicit target removed |
| `corvis-macros` | `src/lib.rs:243`, `:247` | `broken_intra_doc_links` (`[subscribe_message]` in the doc of two misplaced-attribute macros) | `` `#[subscribe_message]` `` in backticks, like `` `#[routes]` `` in the same message |
| `corvis-swagger` | `src/explorer.rs:31` | `broken_intra_doc_links` (`crate::model`) | points to `SwaggerType` and the `api_extra_models!` macro |
| `corvis-swagger` | `src/scalar.rs:142` | `private_intra_doc_links` (`escape_inline_json`) | named without a link |

The misplaced-attribute macros use one literal for the doc and for the `compile_error!` text, so the compile errors of
a misplaced `#[public]` and `#[use_filters]` now show `` `#[subscribe_message]` `` in backticks (no test matches that
text).

Found while running it: the binary of `corvis-cli` is named `corvis`, like the library, so one `cargo doc --workspace`
run documents both into `target/doc/corvis` at the same time. Cargo only warns ("output filename collision"), but on
Windows one local run failed with "Access denied" on a file of that folder. The `doc` job therefore runs two steps,
`cargo doc --workspace --exclude corvis-cli --no-deps --all-features` and then `cargo doc -p corvis-cli --no-deps
--all-features`: the same crates and lints as the design's command, without the collision.

### Task 1.2: lint candidates

`cargo clippy --workspace --all-targets` with every candidate of Decision 1 at `warn`, on Linux, before `clippy.toml`
allowed `unwrap`, `expect` and `panic` in tests. Library code is what `--lib --bins` reports; test code is the rest
(`#[cfg(test)]` modules, integration tests). Published crates only, library / test:

| Lint | corvis | corvis-cli | corvis-macros | corvis-orm | corvis-orm-macros | corvis-swagger | corvis-swagger-macros |
|---|---|---|---|---|---|---|---|
| `missing_docs` | 1210 / 505 | 0 / 0 | 0 / 0 | 229 / 51 | 1 / 0 | 76 / 81 | 0 / 0 |
| `unreachable_pub` | 32 / 537 | 59 / 1 | 25 / 0 | 14 / 103 | 0 / 0 | 0 / 0 | 10 / 0 |
| `missing_debug_implementations` | 180 / 470 | 0 / 0 | 0 / 0 | 10 / 3 | 0 / 0 | 7 / 19 | 0 / 0 |
| `unsafe_op_in_unsafe_fn` | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| `clippy::unwrap_used` | 149 / 2607 | 0 / 69 | 2 / 16 | 29 / 796 | 7 / 0 | 9 / 29 | 0 / 2 |
| `clippy::expect_used` | 38 / 103 | 1 / 1 | 1 / 0 | 6 / 6 | 0 / 0 | 1 / 0 | 0 / 0 |
| `clippy::panic` | 21 / 116 | 0 / 1 | 0 / 2 | 2 / 19 | 0 / 0 | 5 / 0 | 0 / 0 |
| `clippy::todo`, `unimplemented`, `dbg_macro` | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| `clippy::undocumented_unsafe_blocks` | 0 / 8 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| `clippy::missing_panics_doc` | 91 / 8 | 0 / 0 | 0 / 0 | 6 / 0 | 0 / 0 | 1 / 0 | 0 / 0 |
| `clippy::missing_errors_doc` | 137 / 2 | 0 / 0 | 0 / 0 | 41 / 2 | 0 / 0 | 3 / 0 | 0 / 0 |
| `clippy::print_stdout` | 0 / 9 | 21 / 6 | 0 / 0 | 7 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| `clippy::print_stderr` | 5 / 13 | 7 / 0 | 0 / 0 | 1 / 1 | 0 / 0 | 0 / 0 | 0 / 0 |

Windows differs by a handful (`corvis`: `expect_used` 37 in library code, `unwrap_used` 2603 in tests), from code under
`cfg(target_os = "linux")`. The floor (zero everywhere, on both platforms) is `unsafe_op_in_unsafe_fn`,
`clippy::dbg_macro`, `clippy::todo` and `clippy::unimplemented`. `undocumented_unsafe_blocks` has findings only in
test code of `corvis` (the counting global allocators of the memory tests, 42 blocks in 9 files with all features);
fixing them means editing test files that open branches also touch, so it stays in the ratchet, counted in test code. Every other candidate goes to the
ratchet.

With the floor in place, `cargo clippy -p <the seven> --all-features --all-targets -- -D warnings` passes on Linux and
`cargo clippy --workspace --all-targets -- -D warnings` on Windows: the floor needed no source change.

The committed baseline (`node scripts/lint-ratchet.mjs --update` on Linux, all features, library code) is larger than
the library column above where features add code: `corvis` `missing_docs` 1262, `missing_errors_doc` 179,
`expect_used` 51, `unreachable_pub` 55, `missing_debug_implementations` 182; `undocumented_unsafe_blocks` in tests
42; 67 unexplained `ignore` fences in `corvis`, 14 in `corvis-orm`, 5 in `corvis-swagger`, 1 in each of
`corvis-macros` and `corvis-orm-macros`. 2,679 findings in total.

### Task 1.3: MSRV

`cargo +1.89 check --locked -p <the seven published crates>`, with default features and with `--all-features`, passes
on Windows and on Linux. `rust-version` stays `1.89`; README and the docs need no change (neither states it).

### Decisions taken while implementing

- **Ratchet scope.** The script runs clippy with `-p` on the seven published crates and `--all-features`, so code
  behind a feature is counted and the counts do not depend on which features the examples enable (with `--workspace`,
  the examples' features changed the counts of `corvis` by about 5%).
- **What counts in test code.** Only `undocumented_unsafe_blocks` (marked `tests: true` in the script). The API lints
  (`missing_docs`, `unreachable_pub`, `missing_debug_implementations`, `missing_*_doc`) mean nothing in tests, the spec
  excludes `unwrap_used`, `expect_used` and `panic` there, and printing is what tests do.
- **Baseline layout.** `lint-baseline.json` holds counts per crate, lint and file. The comparison is on the sum per
  crate and lint, as designed; the files only serve to name the findings of the files whose count went up. One line
  per file also lets two branches that lower different files of one crate merge without a conflict.
- **`--update` does not raise a count** unless `--allow-increase` is passed too (for a toolchain bump), so running it
  by habit cannot hide a regression; a raise shows in the diff of `lint-baseline.json` and needs a reason.
- **Unknown lints** are recognized by rustc's `E0602` (a lint named on the command line that the toolchain does not
  know); `unknown_lints` only covers attributes.
- **`ignore` fences** are counted in doc comments (`///`, `//!`) under each crate's `src/`; a fence whose info string
  carries a parenthesized reason is not counted.
- **`features` denies warnings** (`RUSTFLAGS: -D warnings`), like the clippy loop over the `corvis` features: an import
  unused under one feature fails there.
- **`proptest`** also moved to `[workspace.dependencies]`: it is a dev-dependency of `corvis` and `corvis-orm`, and the
  shared-dependency rule counts every dependency section. `flate2` and `brotli`, declared twice inside `corvis`
  (optional dependency and dev-dependency), are not shared by two crates and stay as they are.
- **Tool versions.** `cargo-hack@0.6.46` and `cargo-semver-checks@0.51.0`, the latest versions in the manifests of the
  pinned `taiki-e/install-action` (v2.90.2).

### Task 3.4: deliberate failures (local; nothing pushed, so no run ids)

On a temporary edit of `corvis-swagger` (an `unwrap()` in `serialize_config_to_js`, an `unwrap()` in a `#[test]` of
`document.rs`, a doc link to a missing item and an ```` ```ignore (needs a server) ```` fence), reverted afterwards:

- `node scripts/lint-ratchet.mjs` (Linux) exited 1 with `corvis-swagger: clippy::unwrap_used: 9 -> 10 (above the
  baseline)` and `crates/corvis-swagger/src/scalar.rs:150:22: clippy::unwrap_used`, plus the new `missing_panics_doc`
  of that function. The `unwrap()` in the test and the fence with a reason were not counted.
- The `doc` command exited 101 with `crates\corvis-swagger\src\scalar.rs:144:57: error: unresolved link to
  missing_item`.

### The new jobs' commands, run locally

`doc` (both steps), `features` (`cargo hack` 0.6.46, six feature sets, warnings denied), `lint-ratchet` (Linux,
2,679 findings as in the baseline), `msrv` (1.89, default and all features, Linux and Windows) and `packaging` (the
seven crates package and build with the shared dependencies) pass. `semver` (`cargo semver-checks` 0.51.0 against the
merge base with `main`, `5948300`) works with the 1.99.0 rustdoc and exits 100 with one finding:
`constructible_struct_adds_private_field` on `BodyParserOptions.redacted_params`
(`crates/corvis/src/http/body_parser.rs:51`), a change already on `dev`, not one of this change. While pull requests
target `dev`, the job reports everything `dev` changed since `main`, not only the pull request's own changes; using
the merge base with the pull request's base branch (`github.base_ref`) would show only the latter. Left as designed
(merge base with `main`) for the maintainer to decide.

### Required checks of `main` (for the maintainer)

Add exactly these job names as required status checks: `doc`, `msrv`, `features`, `lint-ratchet`. `semver` stays
informational. `CONTRIBUTING.md` lists the full set.

### Security review

Not needed: no input parsing, limits, authentication, SQL or file system code changes. The new jobs keep the
workflow's `permissions: contents: read`, `persist-credentials: false`, no secrets and the same SHA-pinned third-party
actions; `actionlint` 1.7.7 (with shellcheck) passes on both workflows.
