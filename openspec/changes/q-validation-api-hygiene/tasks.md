# Tasks

Start after `q-validation-hot-path` is merged into `dev`. Shared Cargo target directory only. Timing rule for every
test: observe counts or results, never `sleep`, never compare durations. Validation and JSON Schema output must not
change for valid options; public API changes are the ones listed in design.md "Behavior visible to applications".

## 1. Reconcile

- [ ] 1.1 Re-check every file and line of design.md "Findings as checked" against `dev` with the two earlier changes
  merged; write moved lines in a new "Implementation notes" section of design.md before any other task; verify
  `cargo test -p corvis --test zod4_fixtures --test builtin_pipes_fixtures --lib z::` and `cargo test -p corvis-swagger`
  pass on the untouched tree

## 2. JSON Schema options (design.md Decisions 1 and 3)

- [ ] 2.1 `json_schema.rs`: the five enums with `Default`, `#[non_exhaustive]` and `FromStr`; `JsonSchemaOptions` fields
  and setters; `JsonSchemaGen::target()` returning the enum; every comparison in `types.rs`, `compound.rs`,
  `wrappers.rs` and `json_schema.rs` on the enum; re-exports at `corvis::z`; `corvis-swagger/src/zod.rs` adapted (record
  the choice in Implementation notes); the fixture runner parses recorded strings with `FromStr`; unit tests of `FromStr`;
  verify `zod4_json_schema_fixtures` unchanged and `cargo test -p corvis-swagger`
- [ ] 2.2 `ZodSchema::to_json_schema()` returns `Result<Value, String>`; the two unit tests of Decision 3; update the
  callers in the workspace; verify `cargo build --workspace --all-targets`

## 3. Typed arguments and pipe options (Decisions 2 and 7)

- [ ] 3.1 `NormalizationForm` and `ZString::normalize`; `HashAlgorithm`, `HashEncoding` and `z::hash`; the unit tests per
  value; update the fixture runner and any caller; verify the normalize and hash fixture cases and `--lib z::`
- [ ] 3.2 Read the original's option types for the enum and array pipes in `tests/fixtures/builtin_pipes/node_modules`
  and record them in Implementation notes; `ParseEnumPipe::error_http_status_code` and
  `ParseArrayPipe::error_http_status_code`; one fixture case per pipe with a custom status in `generate.mjs`, re-record,
  replay; `ValidationPipeOptions::exception_factory` as `Arc<dyn Fn>` and the generic builder; the capturing-factory
  test; verify `builtin_pipes_fixtures` and the `ValidationPipe` tests

## 4. Internals (Decisions 4, 5 and 6)

- [ ] 4.1 `RawIssue.code` set by every constructor, `code()` returning it; the constructor unit test; verify both fixture
  lists
- [ ] 4.2 `JsRegex::test` warning through a static `WarnLimiter` keyed by pattern source (one line a minute, count left
  out, input length); the test of Decision 5 with a test logger; verify `--lib z::regex`
- [ ] 4.3 The intersection comment, `BigKind::Format` without the name, the `DefaultValuePipe` branch and its comment, the
  `register()` rustdoc; verify `cargo build -p corvis` without warnings and `builtin_pipes_fixtures`

## 5. Docs and review

- [ ] 5.1 Rustdoc of every changed item (the enums, `normalize`, `hash`, `to_json_schema`, `exception_factory`, the two
  pipes); verify `cargo doc -p corvis --no-deps` without warnings and `cargo test -p corvis --doc`
- [ ] 5.2 Docs with the `docs-writer` agent, English then Spanish: `docs/application/validation.md` (JSON Schema options as
  enums, `normalize` and `hash` arguments, `to_json_schema` result, a short migration list), `docs/pipes.md` (status code
  of the enum and array pipes, closures as exception factories); `docs/es/` the same; verify `npm run docs:check` in
  `docs/`
- [ ] 5.3 Security review with the `security-reviewer` agent (Review note 5 and the invariants table); record findings
  and fixes in design.md under "Security review"; verify every finding has a regression test or a recorded reason

## 6. Definition of done

- [ ] 6.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 6.2 `cargo clippy --workspace --all-targets -- -D warnings` passes, and
  `cargo clippy -p corvis --no-default-features -- -D warnings`
- [ ] 6.3 `cargo test --workspace` passes, including `zod4_fixtures`, `builtin_pipes_fixtures`, `zod_pipe` and the
  `corvis-swagger` tests
- [ ] 6.4 Per-feature checks: `cargo check -p corvis --tests --features <f>` for each feature of the CI loop and
  `--no-default-features`, with no warnings
- [ ] 6.5 Regression tests: one per spec scenario of this change and one per row of design.md's invariants table, as
  listed in groups 2 to 4
- [ ] 6.6 Docs pages updated in English and Spanish (`docs/` and `docs/es/`), then `npm run docs:check` in `docs/`
- [ ] 6.7 Framework runtime code changed (`z/`, `validation/`, `common/pipe.rs`): run the `corvis-parity` check and
  verify the three port apps keep 0 apidiff differences
- [ ] 6.8 `cargo deny check` (no new crate) and `node scripts/check-naming.mjs` pass; `openspec validate
  q-validation-api-hygiene --strict` is valid
- [ ] 6.9 The security review of 5.3 is closed; a "Verification" table is added at the end of this file before archiving

## Workflow follow-up

- Archive the change with `/opsx:archive` (its `validation/zod`, `core/validation-pipe` and `core/builtin-pipes` deltas
  are applied).
- Close its issue, tick it in epic #187 and move its card to Done (see the `corvis-task` skill).
- Open the follow-up the maintainer accepts: `exception_factory` on every built-in pipe.
