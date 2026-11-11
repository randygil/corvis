# Tasks

Start after `q-workspace-ci-gates-and-lint-ratchet` is merged into `dev`. Shared Cargo target directory only. Node is
needed once, to record the fixture (`npm install` in the fixture folder, network allowed for that step only); the Rust
tests read the committed JSON. No test compares timings or sleeps. Documents must not change except where design.md
"Behavior visible to applications" says so.

## 1. Reconcile and record

- [ ] 1.1 Re-check every file and line of design.md "The findings, checked against the code" against `dev`; write moved
  lines in a new "Implementation notes" section of design.md; verify `cargo test -p corvis-swagger` passes on the
  untouched tree
- [ ] 1.2 `crates/corvis-swagger/tests/fixtures/document_forms/`: `package.json` (versions of Decision 1),
  `app.ts`, `generate.mjs` (compile like `class_validator_shim/generate.mjs`, create the three documents, run
  `cleanupOpenApiDoc`, write `document_forms.json` with `JSON.stringify(doc)` strings), `.gitignore` for `node_modules/`
  and `out/`; run it; commit `package-lock.json` and `document_forms.json`; verify the recorded documents contain the
  forms design.md Decision 1 expects, or record what differs in Implementation notes (Review note 6)
- [ ] 1.3 `crates/corvis-swagger/tests/document_forms.rs`: the equivalent Corvis apps; `doc.to_json()` equals each
  recorded string; slim `tests/swagger_app.rs` to its serving assertions; verify the test passes on the untouched
  generator (it records today's parity)

## 2. Failing tests first

- [ ] 2.1 `crates/corvis-swagger/tests/rust_types.rs`: the generics tests of Decision 2, the scalar field tests of
  Decision 3, the enum tests of Decision 4, the serde tests of Decision 5, the version tests of Decision 6 (3.0 and 3.1);
  verify each fails on the untouched tree for the reason the finding gives, and record the failures in Implementation
  notes

## 3. Component names of generic types (Decision 2)

- [ ] 3.1 `__private::schema_name` with unit tests (generic, nested, lifetimes, references, arrays, odd characters,
  empty input, a 10,000-character name: total, never panics); use it at the six places Decision 2 lists
- [ ] 3.2 `derive.rs`: the `T: ::corvis::openapi::ApiSchema` bound per type parameter used by a field,
  `#[api_schema(bound = "...")]` replacing it; `compile_fail` doctest for a generic over `String` without `bound`
- [ ] 3.3 `types::register_model`: a described entry replaces an `Opaque` one with the same key, never the reverse; the
  registration-order test; verify the generics tests of 2.1 pass and `document_forms` is unchanged

## 4. Rust scalar types (Decision 3)

- [ ] 4.1 `types::RUST_SCALARS`, `ctor_from_type_name` and `explorer.rs` `param_type` reading it (format added to the
  parameter schema); `SwaggerType::swagger_format` with its default; impls generated from the rows (std always; `chrono`,
  `uuid`, `time` behind their features); `Box`, `Arc`, `Rc`, `[T; N]`
- [ ] 4.2 `crates/corvis-swagger/Cargo.toml`: features `chrono` and `uuid` (default) and `time`, optional dependencies
  with `workspace = true` where the workspace declares them; `derive.rs` inserts `format` before constraints and
  explicit options; verify `cargo check -p corvis-swagger --no-default-features` and each feature alone, and the
  table-consistency test of Decision 3 with all features
- [ ] 4.3 `SwaggerDocumentOptions::with_type::<T>(Value)` and its use where an `Opaque` model resolves; the once-per-type
  warning for types that still fall back; tests for `with_type` and for the warning (captured log)

## 5. Enums and serde attributes (Decisions 4 and 5)

- [ ] 5.1 `derive.rs`: unit enums implement `SwaggerType` with the enum marker (names computed from serde attributes),
  compile errors for data variants and `untagged` / `tag` / `content`; `property_metadata` and the parameter path read
  the marker; tests against `enum_variants::<T>()` for every `rename_all` rule
- [ ] 5.2 `derive.rs` / `schema_factory.rs`: `Option` nullability through `x-corvis-nullable`, `default` and
  `skip_serializing_if` making fields optional, `flatten` splicing the inner properties, `rename(serialize = ..)` and
  `rename_all(serialize = ..)` read; `#[diagnostic::on_unimplemented]` on `corvis::openapi::ApiSchema`;
  `OptionalDto` of `api_schema_constraints.rs` gains `skip_serializing_if`; verify `fixture_replay_matches_the_original`
  passes unchanged and the serde tests of 2.1 pass

## 6. Version-correct Corvis keywords (Decision 6)

- [ ] 6.1 `crates/corvis-swagger/src/version.rs`: the finishing pass for `x-corvis-exclusive-minimum`,
  `x-corvis-exclusive-maximum` and `x-corvis-nullable`, run at the end of `create_document`; `constraints.rs` emits the
  private keys for floats; update `float_sign_uses_exclusive_bounds_and_integers_keep_the_original_values` to the 3.0
  form and add the 3.1 and `min(5)` cases
- [ ] 6.2 The byte-neutrality test: the pass on the documents of `document_forms.json` returns the same strings;
  verify `document_forms`, `swagger_app` and `api_schema_constraints` pass

## 7. Strict cleanup and validation (Decisions 7 and 10)

- [ ] 7.1 `SwaggerError` (the five variants of Decision 7) and `CleanupOptions`; `try_cleanup_open_api_doc(doc,
  CleanupOptions)`; strict renames everywhere and in every operation key; the version normalization; dangling `$ref` and
  leftover marker errors; tests on the recorded `forms` and `forms31` documents (strict output) and the default path
  byte for byte
- [ ] 7.2 `tests/fixtures/oas/` (schemas, `README.md` with source and date, `exclude` in `Cargo.toml`) and
  `tests/openapi_schema.rs` with the dev-dependency `jsonschema` (default features off, exact version); verify
  `cargo deny check` accepts it, or apply the fallback of Decision 10 and record it

## 8. Warnings (Decisions 8 and 9)

- [ ] 8.1 The duplicate-component warning with both Rust paths and the fix, logger `SwaggerModule`; the leftover-marker
  warning in `SwaggerModule::setup` and `scalar::mount` with a document; tests capturing both

## 9. Docs and definition of done

- [ ] 9.1 `docs/openapi/types-and-parameters.md`: generic names and `bound`, the scalar table and features,
  `with_type` (recipe for the ORM `Decimal`), enums, `Option` and serde attributes, the corrected float row;
  `docs/openapi/other-features.md`: strict cleanup, what the default keeps from the original and why; Spanish pages under
  `docs/es/openapi/`; rustdoc of every new or changed public item; verify `npm run docs:check` in `docs/` and
  `cargo doc -p corvis-swagger --no-deps --all-features` without warnings
- [ ] 9.2 `cargo build --workspace --all-targets` with no warnings; `cargo clippy --workspace --all-targets -- -D
  warnings`; `cargo test --workspace`; `cargo test -p corvis-swagger --no-default-features` and with each feature alone
- [ ] 9.3 `node scripts/lint-ratchet.mjs` passes (lower the `corvis-swagger` and `corvis-swagger-macros` baselines if
  counts fell); `cargo deny check`, `cargo audit`, `node scripts/check-naming.mjs` and `node
  scripts/check-crate-files.mjs` pass
- [ ] 9.4 Framework code changed (`corvis-swagger`, `corvis-swagger-macros`, `corvis::openapi`): run the `corvis-parity`
  check and verify the three port apps keep 0 apidiff differences; any difference in facten's or consi's document is a
  defect to fix here, not a divergence to record
- [ ] 9.5 No security review required (design.md "Security invariants"); verify `tests/swagger_security.rs` passes
  unchanged; `openspec validate q-openapi-schemas-for-rust-types-and-versions --strict`

## Workflow follow-up

- Archive with `/opsx:archive` (deltas of `openapi/api-schema` and the new `openapi/document` applied).
- Close the issue, tick it in epic #187, move the card to Done (see the `corvis-task` skill).
- Start `q-openapi-serving-and-api-hygiene`, which builds on `SwaggerError` and the module layout left here.
- Possible later issues if the maintainer wants them: homonyms as an error behind an option (Open question 3), `integer`
  for Rust integers behind an option.
