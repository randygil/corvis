# Tasks

Start after `q-workspace-ci-gates-and-lint-ratchet` is merged into `dev`. Shared Cargo target directory only; the
generated project builds in one fixed folder (`CORVIS_GENERATED_TARGET` or `<workspace target>/generated-project`),
never a folder per run. Tests write only under `std::env::temp_dir()` and clean up on drop. No test sleeps to
synchronize.

## 1. Reconcile

- [ ] 1.1 Re-check every file and line of design.md "The findings, checked against the code" against `dev`; write moved
  lines in a new "Implementation notes" section of design.md; verify `cargo test -p corvis-cli` passes on the untouched
  tree

## 2. Failing tests first

- [ ] 2.1 `crates/corvis-cli/tests/cli_edit.rs`: the corpus of design.md Decision 1 driving the editors through the
  binary in a scaffolded project; verify the rustfmt-split group, the commented `#[module(`, the duplicate import and the
  CRLF cases fail on the untouched tree, and record the failures in Implementation notes
- [ ] 2.2 `crates/corvis-cli/tests/generated_project.rs` (`#[ignore]`): steps 1-6 of Decision 4 against the untouched
  templates; run it once and record what happens (expected: posture lists the generated routes; record whether the
  gateway handler is listed, and any build, clippy or test failure of the templates) in Implementation notes

## 3. Edits located by `syn` (Decision 1)

- [ ] 3.1 `crates/corvis-cli/Cargo.toml`: `syn` (`full`, `parsing`) and `proc-macro2` (`span-locations`) through
  `workspace = true`; a span-to-offset helper with unit tests (multi-byte characters, CRLF)
- [ ] 3.2 `edit.rs`: `add_use`, `add_mod`, `add_to_module` returning `Edit::{Unchanged, Changed, Skip}` as Decision 1
  describes (parse before and after, existing-name checks, newline style); keep the existing unit tests passing
- [ ] 3.3 `main.rs`: callers print `SKIP` with the reason and the line to add by hand; verify `tests/cli_edit.rs` and the
  `cli/generate` registration tests of `tests/cli_input.rs` pass

## 4. Safe writes (Decision 2)

- [ ] 4.1 `crates/corvis-cli/src/write.rs`: `Changes` (create-new files, atomic replace through a temporary file in the
  same folder and `rename`, rollback of created files and folders and of edited files)
- [ ] 4.2 `cmd_new`, every `generate` path and `generate_resource` run through one `Changes`; the rollback test (third
  write of `g res` fails) and the no-overwrite test

## 5. Arguments and colors (Decisions 5 and 6)

- [ ] 5.1 `crates/corvis-cli/src/args.rs`: the flag tables, usage errors (unknown flag, missing value, repeated flag,
  extra positional, unknown command with suggestion), status 2, help only for `corvis` / `help` / `-h` / `--help`;
  `start` and `build` parse only before `--`
- [ ] 5.2 Replace `flag`, `opt`, `positional` in `main.rs`; tests of Decision 5 in `tests/cli_input.rs`; verify every
  `cli/start-build` requirement scenario still passes (`tests/cli_input.rs` start cases and the `main.rs` unit tests)
- [ ] 5.3 Color helper (`IsTerminal`, `NO_COLOR`) used by `main.rs` and `posture.rs`; tests of Decision 6; update
  assertions in existing tests that matched escape codes, if any

## 6. Generated routes and the job (Decisions 3 and 4)

- [ ] 6.1 `templates.rs`: `--public` variants of `CONTROLLER`, `RES_CONTROLLER`, `GATEWAY` (`#[public]` and the `TODO`
  comment on every route and handler); the `NOTE` when the crate root enforces posture, listing the template's routes;
  unit tests for both
- [ ] 6.2 `generated_project.rs`: step 7 (the `--public` project, posture exit 0) and the exact unprotected list of step
  6; verify `cargo test -p corvis-cli --test generated_project -- --ignored` passes
- [ ] 6.3 `.github/workflows/ci.yml`: job `generated-project` (Linux, pinned toolchain with clippy and rustfmt,
  rust-cache including the generated target folder, `CORVIS_GENERATED_TARGET`, `timeout-minutes: 45`, the workflow's
  permissions, `persist-credentials: false`); verify the job passes on the pull request

## 7. Posture and smaller items (Decisions 7 and 8)

- [ ] 7.1 `posture.rs` / `posture/tree.rs`: the `Leader` helper (`waitid` with `WNOWAIT` from `libc`, Unix-only
  dependency), group kill before reaping, Windows unchanged; the order test with a fake `Leader`; verify
  `tests/cli_posture.rs` passes on Linux and Windows
- [ ] 7.2 `names.rs`: the singular rules and exception table with the table-driven test; `--entity <Name>` for `g res`
  (validated like a name); the cwd-writing test of `main.rs` with a drop guard and unique names

## 8. Docs and definition of done

- [ ] 8.1 `docs/cli/overview.md` and `docs/cli/usages.md`: usage errors and status 2, `--public`, `--entity`, the `NOTE`,
  `SKIP` and what to do, colors and `NO_COLOR`; Spanish pages under `docs/es/cli/`; `crates/corvis-cli/README.md`;
  `CONTRIBUTING.md` (the `generated-project` job and how to run it locally); verify `npm run docs:check` in `docs/`
- [ ] 8.2 `cargo build --workspace --all-targets` with no warnings; `cargo clippy --workspace --all-targets -- -D
  warnings`; `cargo test --workspace` on Linux and Windows
- [ ] 8.3 `node scripts/lint-ratchet.mjs` (lower the `corvis-cli` baseline if counts fell), `cargo deny check`,
  `cargo audit`, `node scripts/check-naming.mjs`, `node scripts/check-crate-files.mjs` pass
- [ ] 8.4 Framework runtime code is not changed (CLI only): state it in Implementation notes; if a task had to touch
  `crates/corvis`, run the `corvis-parity` check and verify the three port apps keep 0 apidiff differences
- [ ] 8.5 Security review with the `security-reviewer` agent (writes in the user's folder: create-new, temporary files
  and renames, rollback that must never delete a file the CLI did not create; argument parsing before validation; the
  process-group sequence of Decision 7); repeat rounds until no open finding; record findings and fixes in design.md
  under "Security review"; verify every finding has a regression test or a recorded reason
- [ ] 8.6 Write in Implementation notes that `generated-project` must be added to the required checks of `main`;
  `openspec validate q-cli-safe-edits-and-generated-project --strict`

## Workflow follow-up

- Archive with `/opsx:archive` (new `cli/arguments`; deltas of `cli/generate`, `security/route-posture`, `infra/ci`).
- Close the issue, tick it in epic #187, move the card to Done (see the `corvis-task` skill).
- The maintainer adds `generated-project` to the required checks of `main`.
- Possible later issue: a `--guard` flag that generates a denying guard with a controller or resource (design.md
  Decision 3 alternative).
