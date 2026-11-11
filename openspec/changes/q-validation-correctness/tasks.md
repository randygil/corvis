# Tasks

Shared Cargo target directory only (never a target directory per task). Record fixtures from the fixture folders with
`npm ci` and the commands in each recorder's header; never edit a recorded JSON by hand. Timing rule for every test:
observe counts or results, never `sleep`, never compare durations. A fix lands only after its test or fixture case fails
on the unfixed code. zod 4 mode output must not change except where design.md "Behavior visible to applications" says so.

## 1. Reconcile

- [ ] 1.1 Re-check every file and line of design.md "Findings as checked" against `dev`; write moved lines in a new
  "Implementation notes" section of design.md before any other task; verify `cargo test -p corvis --test zod4_fixtures
  --test zod_pipe --test zod_security --test zod --lib z::` passes on the untouched tree

## 2. Fixtures from the originals (design.md Decision 1)

- [ ] 2.1 `crates/corvis/tests/fixtures/zod4/package.json` and `package-lock.json` pinning `zod` 4.4.3; header of
  `cases.mjs`: `npm ci && TZ=UTC node cases.mjs > parse.json`; recorder: `construct: { throws }` records and the
  `{ "$number": .. }` encoding; replay in `tests/zod4_fixtures.rs`: `construct` cases built under `catch_unwind`,
  `$number` compared through `z::parse_val`; re-record with no new case; verify `git diff --stat
  crates/corvis/tests/fixtures/zod4/parse.json` is empty (or lists only cases changed by the new encoding, written in
  Implementation notes) and `cargo test -p corvis --test zod4_fixtures` passes
- [ ] 2.2 The new `c` and `c3` cases of the Decision 1 table in `cases.mjs`, their schemas by name in
  `tests/zod4_fixtures.rs`; record; run `cargo test -p corvis --test zod4_fixtures` and copy the mismatch report into
  Implementation notes ("Fixture before"); for every claim of design.md whose case already passes, write "dropped:
  fixture agrees" next to it; verify the report lists only cases of this change
- [ ] 2.3 `crates/corvis/tests/fixtures/builtin_pipes/` (`package.json` with the pinned versions of Decision 1,
  `package-lock.json`, `generate.mjs`, `builtin_pipes.json`) and `crates/corvis/tests/builtin_pipes_fixtures.rs`
  replaying `pipes` and `rules`; run it and copy the mismatches into Implementation notes; run `generate.mjs` a second time
  with the common package at 12.0.4 (not committed) and record any difference (Review note 4); verify the recorder output
  names the versions and `node scripts/check-naming.mjs` passes (the Rust replay file names no original package)

## 3. Date parser (Decision 2)

- [ ] 3.1 Failing tests in `crates/corvis/src/z/js.rs`: the three inputs of Decision 2 and the wrapped-year search;
  verify they fail on the untouched code with `cargo test -p corvis --lib z::js` (panic) and with
  `cargo test --release -p corvis --lib z::js` (finite result)
- [ ] 3.2 Checked numerals, the year bound before `make_utc`, `i128` in `days_from_civil`/`make_utc`, offsets in `f64`;
  verify the tests of 3.1 pass in debug and release, and the `date.coerce.edges` cases (V4 and V3) pass
- [ ] 3.3 `js_date_parse_is_total`, and totality properties for `string_to_number` and `string_to_bigint`, in
  `crates/corvis/src/fuzz_tests.rs`; verify `cargo test -p corvis --lib fuzz_tests` passes and that the date property
  fails on the code of before 3.2 (checked once with a `git stash` of 3.2, result in Implementation notes)

## 4. zod 3 mode (Decisions 3 and 4)

- [ ] 4.1 `ZString::check` (`types.rs`): length with `js::js_len` (once, and again after a transform), `trim` with
  `js::js_trim`, `coerce` with `js::to_js_string`; messages with `js::num_to_string`; remove `fmt_num`, replace
  `num_value` with `js::num_json`; verify the `*.astral`, `string.trim.edges`, `coerce.string.edges`,
  `number.min.big`, `number.max.small` V3 cases pass and `zod4_parse_fixtures` still passes
- [ ] 4.2 `ZNumber::check`: `z.coerce.number()` through `js::string_to_number` / `js::to_number`; `multipleOf` through
  `float_safe_remainder`, or `float_safe_remainder_v3` if the cases differ (comment naming them); verify
  `coerce.number.edges` and `number.multipleOf.*` V3 cases
- [ ] 4.3 zod 3 email, cuid and ip patterns as `JsRegex` (reusing `regexes::` statics with identical sources), `url` as
  recorded; remove the unused `regex`-crate statics; unit test that the zod 3 email pattern is the `JsRegex` of zod 3's
  source and a test naming which zod 3 patterns are linear; verify the `string.*.edges` V3 cases and the `regex.rs` tests

## 5. Check-like wrappers (Decision 5)

- [ ] 5.1 `check_wrapper_structure!` in `wrappers.rs` for `ZRefine`, `ZSuperRefine`, `ZCheckFn`, `ZOverwrite`; zod 3
  methods per wrapper as `object.required.refine` and `discriminatedUnion.refinedKey` show; the table-driven unit test;
  verify the `du.*`, `templateLiteral.*` and S3b cases pass in both lists

## 6. Coercion (Decisions 6 and 7, built-in pipes and rules)

- [ ] 6.1 `ZDate`: bounds only in `checks`, parsed once with `js::parse_date`; zod 3 mode input through `js::parse_date`
  / `js::to_date` with the same accepted shapes; the zone-less bound test of Decision 6; verify
  `cargo test -p corvis --lib z::` and both fixture lists
- [ ] 6.2 `js::to_bigint` returns `None` outside `i128`; unit test with `1e39` and `-1e39`; rustdoc of
  `z::coerce::bigint()`; verify `cargo test -p corvis --lib z::js`
- [ ] 6.3 `common/pipe.rs`: `ParseArrayPipe`, `ParseIntPipe`, `ParseFloatPipe`, `ParseBoolPipe`, `DefaultValuePipe` as
  the `pipes` records show, number text read with one JavaScript-semantics helper (`z::js::string_to_number`, plus a
  `parseFloat` helper in `z::js` if the records need it); unrepresentable values refused with the pipe's 400 and marked
  as divergences in `generate.mjs` (a `divergence` field the replay accepts); verify `cargo test -p corvis --test
  builtin_pipes_fixtures` and the existing pipe tests
- [ ] 6.4 `validation/rules.rs`: `is_number_string`, `is_ip`, `min_length`, `max_length` as the `rules` records show;
  verify `builtin_pipes_fixtures` and `cargo test -p corvis --lib validation::`

## 7. Thrown errors (Decision 8)

- [ ] 7.1 `Thrown` payload and `resume_unwind` at `throw_encode_error` and the unmergeable intersection; `catch_unwind`
  at `parse_val` and `run_v3`; `ZodError::thrown()`, the private field, `Display`; the `throws` branch of
  `tests/zod4_fixtures.rs` expects `Err` with the recorded message; verify both fixture lists and
  `cargo test -p corvis --lib z::`
- [ ] 7.2 `z/pipe.rs`: `validate_opt` and the serializer map a thrown `ZodError` to today's bodies; `catch_thrown` logs
  user panics through a `WarnLimiter` (one line a minute, pipe name and parameter kind, no panic text);
  `crates/corvis/tests/zod_thrown.rs` with the four tests of Decision 8; verify `cargo test -p corvis --test zod_thrown
  --test zod_pipe`

## 8. Construction errors (Decision 9)

- [ ] 8.1 `ZLiteral::new` asserts non-empty with the recorded message; `z::native_enum` and `ParseEnumPipe::of` panic
  for an empty variant list naming the type; `# Panics` rustdoc; `#[should_panic]` tests (empty literal, untagged enum,
  internally tagged enum) and a plain enum test; verify the `literal.empty` case and `cargo test -p corvis --lib`

## 9. Tests without timing bounds (Decision 10)

- [ ] 9.1 `crates/corvis/tests/zod_linear_traversal.rs` (counting allocator, depth n and 2n, lazy object and `z.json()`)
  replacing `recursive_traversal_is_linear` in `zod_security.rs`; remove the elapsed-time assertion of
  `word_boundary_patterns_are_linear_and_exact`; comparison counter for `array_unique_is_linear`; verify the three pass
  5 runs in a row, the allocation test fails against a deliberately quadratic variant (a local `p.value.clone()` per
  level, checked once and reverted, noted in Implementation notes), and `grep -rn "elapsed()" crates/corvis/src/z
  crates/corvis/src/validation crates/corvis/tests/zod*.rs` finds nothing

## 10. Docs and review

- [ ] 10.1 Rustdoc: `ZodError::thrown`, `z::coerce::date()` (the range), `ZDate::min/max` (one parser, local time),
  `z::coerce::bigint()`, `z::literal`, `z::native_enum`, `ParseEnumPipe::of` (`# Panics`), the built-in pipes whose
  behavior changed, the zod 3 notes on `ZodVersion::V3`; verify `cargo doc -p corvis --no-deps` without warnings
- [ ] 10.2 Docs with the `docs-writer` agent, English then Spanish: `docs/application/validation.md` (thrown errors and
  `thrown()`, the date range, the zod 3 mode changes the fixture confirmed, unwinding needed for thrown errors),
  `docs/pipes.md` (the built-in pipe behaviors the fixture changed and the recorded divergences); `docs/es/` the same;
  verify `npm run docs:check` in `docs/`
- [ ] 10.3 Security review with the `security-reviewer` agent (Review note 6 and the invariants table); repeat rounds
  until no open finding; record findings and fixes in design.md under "Security review"; verify every finding has a
  regression test or a recorded reason

## 11. Definition of done

- [ ] 11.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 11.2 `cargo clippy --workspace --all-targets -- -D warnings` passes, and
  `cargo clippy -p corvis --no-default-features -- -D warnings`
- [ ] 11.3 `cargo test --workspace` passes, including `zod4_fixtures`, `zod_pipe`, `zod_security`, `zod_thrown`,
  `zod_linear_traversal`, `builtin_pipes_fixtures` and the `fuzz_tests` module; `cargo test --release -p corvis --lib
  z::js` passes
- [ ] 11.4 Per-feature checks: `cargo check -p corvis --tests --features <f>` for each feature of the CI loop
  (`.github/workflows/ci.yml`) and `--no-default-features`, with no warnings
- [ ] 11.5 Regression tests: one per spec scenario of this change and one per row of design.md's invariants table, as
  listed in groups 2 to 9
- [ ] 11.6 Docs pages updated in English and Spanish (`docs/` and `docs/es/`), then `npm run docs:check` in `docs/`
- [ ] 11.7 Framework runtime code changed (`z/`, `validation/`, `common/pipe.rs`): run the `corvis-parity` check and
  verify the three port apps keep 0 apidiff differences
- [ ] 11.8 `cargo deny check` (no new crate) and `node scripts/check-naming.mjs` pass; `openspec validate
  q-validation-correctness --strict` is valid
- [ ] 11.9 The security review of 10.3 is closed; a "Verification" table (check and result, as archived changes have)
  is added at the end of this file before archiving

## Workflow follow-up

- Archive the change with `/opsx:archive` (its `validation/zod` and `core/validation-pipe` deltas are applied and
  `core/builtin-pipes` is created).
- Close its issue, tick it in epic #187 and move its card to Done (see the `corvis-task` skill).
- Then start `q-validation-hot-path`, whose task 1.1 re-checks its lines against the result of this change.
- Open the follow-ups the maintainer accepts: Open questions 1 to 3, a fixture for the remaining class-validator rules
  and for the implicit conversion of query values, the OpenAPI `enum: []` for types without readable variants.
