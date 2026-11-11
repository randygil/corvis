# Tasks

Start after `q-validation-correctness` is merged into `dev`. Shared Cargo target directory only (never a target directory
per task). Timing rule for every test: observe counts, never `sleep`, never compare durations. The benchmark informs; no
test compares its timings. Output must not change: issues, messages, key order and values stay byte for byte.

## 1. Reconcile

- [ ] 1.1 Re-check every file and line of design.md "Findings as checked" against `dev` with `q-validation-correctness`
  merged (the zod 3 checks, the intersection's throw and the pipe's error mapping moved there); write moved lines in a new
  "Implementation notes" section of design.md before any other task; verify `cargo test -p corvis --test zod4_fixtures
  --test zod_pipe --test zod_security --test zod_thrown --lib z::` passes on the untouched tree

## 2. Benchmark (design.md Decision 1)

- [ ] 2.1 `bench/validation/` workspace member (`Cargo.toml` with `publish = false`, added to the workspace `members`):
  binary `corvis-validation-bench` with the counting allocator, the `dto-ok` DTO and the nine scenarios of Decision 1,
  result checks before measuring, median of five runs, allocations, bytes and clone ratio per parse; verify
  `cargo build --release -p corvis-validation-bench` without warnings and a manual run of each scenario
- [ ] 2.2 `bench/validation/run.sh` (`--label`, `SKIP_BUILD`, the missing-binary message, results JSON with machine,
  rustc, commit and parameters, the Markdown table) and `bench/validation/README.md`; `bench/validation/results/` in
  `.gitignore`; `bench/validation/**` among the published surfaces of `scripts/check-naming.mjs`; link from
  `bench/README.md`; verify the scenarios of the spec requirement "The validation benchmark covers the hot paths" by hand
  and `node scripts/check-naming.mjs`
- [ ] 2.3 Before run: `bash bench/validation/run.sh --label before`; copy the table into Implementation notes ("Benchmark
  before"); verify `git status` shows no file under `bench/validation/results/`

## 3. Copies in checks and callbacks (Decisions 2 and 3)

- [ ] 3.1 `compound.rs` `length_check`, `types.rs` `ZString::apply_v4`, `Fmt::check_v4`, `url_check`, the number,
  bigint and date checks: issue input cloned only when an issue is pushed; `v4_map` only when pushing; verify
  `cargo test -p corvis --test zod4_fixtures` and `--lib z::`
- [ ] 3.2 `wrappers.rs`: `.refine`, `z.custom`, `.superRefine` borrow; `RefinementCtx` borrows its input (`RefTarget::V4`
  with `&'a Val`); `z.preprocess` borrows; `.transform`, `.overwrite` and `codec` take the value (stub for the
  transform context when inputs are not observable, Decision 4); unit test of the `superRefine` input with `reportInput`;
  verify both fixture lists and `--test zod_pipe`

## 4. Unions and intersections (Decisions 4 and 5)

- [ ] 4.1 `core.rs`: `RunCtx::inputs_observable` (computed in `RunCtx::new`), the value-observer counter with a helper
  that holds it while an inner schema runs, used by `ZCatchFn` and by every node with a user `when` (the list written in
  a comment on the counter), `Val::type_stub()`, the hidden test-only switch; unit tests of `type_stub` (same
  `parsed_type` for every kind); verify `cargo test -p corvis --lib z::`
- [ ] 4.2 `type_rejects` and `type_issue_for` for the nodes of Decision 4 (and the forwarding wrappers); `union_v4` and
  `ZXor` quick rejection; the single-option move in `union_v4`; unit tests: each `type_issue_for` equals the issue of
  running the node on that input, `catch_fn` over a recursive union sees its input, a `when` refinement over an array of
  unions sees the original elements; verify both fixture lists and `zod_security`
- [ ] 4.3 `crates/corvis/tests/zod_union_paths.rs`: the proptest property of Decision 4 (switch on and off, `reportInput`
  off and on, a global error map); verify it passes with the default number of cases and with 10,000 cases once (result
  in Implementation notes)
- [ ] 4.4 `merge_vals(a: Val, b: Val)` by value; the depth-10 merge unit test; verify the intersection fixtures,
  `zod_thrown` (the conflict case) and `--lib z::`

## 5. Enums (Decision 6)

- [ ] 5.1 `ZEnum` stores its values at construction (`new`, `from_entries`, `extract`, `exclude`); `run_v4`, `values`,
  `pattern`, `literal_values`, `json_schema` read them; `enum_values()` keeps its signature; verify the enum fixtures
  (parse and JSON Schema) and `--lib z::`

## 6. Owned parse (Decision 7)

- [ ] 6.1 `ZodSchema::parse_owned`, `parse_owned_opt`, `z::parse_val_owned`; the hidden node accessor used by every parse
  method (no `to_schema()` clone for concrete types); rustdoc with examples; unit tests: the owned and borrowed paths give
  the same output and errors for the `dto-ok` DTO; verify `cargo test -p corvis --lib z:: --doc`
- [ ] 6.2 `ZodValidationPipe::transform` parses its owned value; the serializer takes the body through `body_mut()` and
  serializes by value; the array schema of an array DTO built at registration; verify `cargo test -p corvis --test
  zod_pipe` (fixture and HTTP tests)

## 7. Configuration and objects (Decisions 8 and 9)

- [ ] 7.1 Generation counter in `config`/`configure`; per-thread snapshot read by `RunCtx::new` and `version()`; tests:
  a change made on another thread is seen by the next parse of a thread that already parsed (threads joined, no sleep),
  `config`/`get_config` round-trip; verify `--test zod_security` (the configuration test) and `--lib z::`
- [ ] 7.2 `ZObject` key set (`OnceLock<HashSet<String>>` above 16 keys, reset by `field`, `extend`, `safe_extend`,
  `merge`, `pick`, `omit`), shared `static` catch-all schemas, zod 3 extra keys only when not strip; the builder unit test
  of Decision 9; verify the object fixtures (strict, passthrough, catchall, strip in both lists)

## 8. Heap test (Decision 11)

- [ ] 8.1 `crates/corvis/tests/zod_hot_path_heap.rs` with the four cases of Decision 11, bounds taken from the measured
  numbers; check once that each bound fails on the code of `dev` before this change (a `git stash` run, result in
  Implementation notes); verify it passes

## 9. Docs and review

- [ ] 9.1 Rustdoc of `parse_owned`, `parse_owned_opt`, `parse_val_owned`, and of `ZodSchema::parse` (one copy of the input;
  prefer the owned method when the value is yours); `docs/application/validation.md` (owned parsing, a short note on what a
  parse copies) with the `docs-writer` agent, then `docs/es/application/validation.md`; verify `cargo doc -p corvis
  --no-deps` without warnings and `npm run docs:check` in `docs/`
- [ ] 9.2 Security review with the `security-reviewer` agent (Review note 6 and the invariants table); repeat rounds until
  no open finding; record findings and fixes in design.md under "Security review"; verify every finding has a regression
  test or a recorded reason

## 10. Benchmark after

- [ ] 10.1 `bash bench/validation/run.sh --label after` on the same machine and parameters as 2.3; "Benchmark after" table
  in Implementation notes with, per decision, the figure it had to move (Decision 2: allocations of `dto-ok` down by at
  least twice the passing string checks; Decision 3: `refine-root` clone ratio at most 0.5; Decision 4: `recursive-union`
  clone ratio at most 2; Decision 5: `intersection` at most 2.5; Decision 6: `enum-heavy` 240 allocations fewer at least;
  Decision 7: `dto-ok` pipe ratio down by 1.0 and `serialize` by 2.0; Decision 8: one allocation fewer per parse;
  Decision 9: one allocation fewer in `strict-wide`); explain any figure missed and any scenario more than 5% slower;
  record the `dto-fail` bytes per parse and the decision of Decision 10

## 11. Definition of done

- [ ] 11.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 11.2 `cargo clippy --workspace --all-targets -- -D warnings` passes, and
  `cargo clippy -p corvis --no-default-features -- -D warnings`
- [ ] 11.3 `cargo test --workspace` passes, including `zod4_fixtures`, `zod_pipe`, `zod_security`, `zod_thrown`,
  `zod_linear_traversal`, `zod_union_paths`, `zod_hot_path_heap` and `builtin_pipes_fixtures`
- [ ] 11.4 Per-feature checks: `cargo check -p corvis --tests --features <f>` for each feature of the CI loop and
  `--no-default-features`, with no warnings
- [ ] 11.5 Regression tests: one per spec scenario of this change and one per row of design.md's invariants table, as
  listed in groups 3 to 8
- [ ] 11.6 Docs pages updated in English and Spanish (`docs/` and `docs/es/`), then `npm run docs:check` in `docs/`
- [ ] 11.7 Framework runtime code changed (`z/`): run the `corvis-parity` check and verify the three port apps keep 0
  apidiff differences
- [ ] 11.8 `cargo deny check` (no new crate in `corvis`; the bench adds none either) and `node scripts/check-naming.mjs`
  pass; `openspec validate q-validation-hot-path --strict` is valid
- [ ] 11.9 The security review of 9.2 is closed; a "Verification" table is added at the end of this file before archiving

## Workflow follow-up

- Archive the change with `/opsx:archive` (its `validation/zod` delta is applied and `bench/validation` is created).
- Close its issue, tick it in epic #187 and move its card to Done (see the `corvis-task` skill).
- Open the N2 follow-up if Decision 10's number calls for it.
- Then start `q-validation-api-hygiene`.
