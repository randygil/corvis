# Tasks

Start after `q-openapi-schemas-for-rust-types-and-versions` is merged into `dev`. Shared Cargo target directory only.
The benchmark binds to `127.0.0.1` only. No test compares timings or sleeps; the lazy-document test synchronizes with a
barrier the test releases. Responses must stay byte for byte as today (body and headers).

## 1. Reconcile

- [ ] 1.1 Re-check every file and line of design.md "The findings, checked against the code" against `dev` with the
  previous change merged; write moved lines in a new "Implementation notes" section of design.md; verify
  `cargo test -p corvis-swagger` passes on the untouched tree

## 2. Measure before

- [ ] 2.1 Test-build counter of `ApiSchemaDef` builds per model in one `create_document` (behind `cfg(test)` or a
  hidden `__private` counter); record the before figure for a DTO used by three routes in Implementation notes
- [ ] 2.2 `crates/corvis-swagger/tests/serving.rs`: the counting allocator and the allocation test of a request the
  middleware does not serve (record the before count; the assertion `== 0` is added in group 3), and the response
  snapshot of design.md Decision 2 recorded from the untouched code into the test
- [ ] 2.3 `bench/corvis-app`: `CORVIS_BENCH_OPENAPI=1` mounts the Swagger UI with the generated 300-operation document
  (dependency on `corvis-swagger`); `bench/run.sh`: scenarios `hello-docs`, `docs-json`, `docs-yaml` for Corvis only,
  `n/a` for Node in the report; `bench/README.md` documents them; run the before matrix (`hello`, `hello-docs`,
  `docs-json`, `docs-yaml`) and copy the table into Implementation notes ("Benchmark before")

## 3. Serving without per-request work (Decisions 2 and 3)

- [ ] 3.1 `swagger_module.rs`: lowercased mount paths in `Served`, slice comparisons, early return for requests outside
  the mount; `scalar.rs`: the same for `use_path_matches`; assert 0 allocations in `serving.rs`
- [ ] 3.2 `OnceLock<Bytes>` bodies (JSON, YAML, init script, both HTML variants) when no patch function; the Scalar page
  as `Bytes`; the pointer-identity test; verify the response snapshot and `tests/swagger_app.rs` pass unchanged
- [ ] 3.3 Custom assets: `confined_asset` and the read in one `spawn_blocking`; lazy document in
  `tokio::sync::OnceCell` built with `spawn_blocking`, failure path answering 500 and logging; the concurrent
  first-request test of Decision 3; verify `tests/swagger_security.rs` passes unchanged

## 4. Attribute options (Decision 4)

- [ ] 4.1 `tests/fixtures/option_keys/extract.mjs` (TypeScript compiler API over the pinned type declarations) and the
  committed `option_keys.json`
- [ ] 4.2 `crates/corvis-swagger-macros/src/keys.rs` with the lists and the commented Corvis additions; checks in
  `options_map` and `options_tokens` with the error format of Decision 4 (span of the key, suggestion, valid keys);
  hidden `__option_keys!`; `tests/attribute_keys.rs`
- [ ] 4.3 `#[api_extension]`: literal `x-` key checked in the macro, `__private` constructor; `decorators::api_extension`
  returning `Result<CustomDecorator, SwaggerError>`; `compile_fail` doctests for a misspelled route option, a
  misspelled `#[api_property]` option and a non-`x-` extension key; verify every example under `examples/` still builds

## 5. Public surface and errors (Decision 5)

- [ ] 5.1 `lib.rs`: internal modules `pub(crate)`, root re-exports, macro support under `__private`; delete
  `types::probe`; update the macros' expansion paths; a `compile_fail` doctest that `corvis_swagger::schema_factory` is
  private; verify `cargo build --workspace --all-targets`
- [ ] 5.2 `SwaggerError` variants `DuplicateDefsId`, `UnregisteredModel`, `ExtensionKey` with Rust-only messages;
  `SwaggerModule::try_create_document`; `zod.rs:207` and `schema_factory.rs:663` through the `try_` path; rustdoc
  `# Panics` / `# Errors` sections; one test per error
- [ ] 5.3 Dead code (`_PREFIX_IS_USED` and the unused constant, `defined`, the `#[allow(dead_code)]` on `ident`); the
  five `ignore` doc examples as `no_run` doctests; lower `lint-baseline.json`; verify
  `cargo test -p corvis-swagger --doc` and `node scripts/lint-ratchet.mjs`

## 6. Startup and small rules (Decisions 6 and 7)

- [ ] 6.1 `model_info` memoized per `create_document`, explored models reused by `get_custom_type` and
  `create_response`; `walk_json_schema` and `schema_metadata` without subtree clones; verify the counter of 2.1 reads 1
  and the `document_forms`, `class_validator_shim` replays and `swagger_app` pass unchanged
- [ ] 6.2 One `js_number_string` (the `yaml.rs` one) with the test values of Decision 7; one `is_body`; doc comments
  of both `short_type_name`

## 7. Docs

- [ ] 7.1 Rustdoc of every changed public item; `crates/corvis-swagger/README.md`; the OpenAPI docs pages that name a
  removed path, `create_document` errors or extension keys (English and Spanish); verify `npm run docs:check` in
  `docs/` and `cargo doc -p corvis-swagger --no-deps --all-features` without warnings

## 8. Benchmark after

- [ ] 8.1 After run on the same machine and matrix as 2.3; "Benchmark after" table in Implementation notes with each
  figure of design.md Decision 1 (allocations 0, `docs-json` ×5, `docs-yaml` ×20, `hello-docs` within 3% of `hello`);
  explain any missed figure; update `bench/results.md`

## 9. Definition of done

- [ ] 9.1 `cargo build --workspace --all-targets` with no warnings; `cargo clippy --workspace --all-targets -- -D
  warnings`; `cargo test --workspace`; `cargo test -p corvis-swagger --no-default-features` and with each feature alone
- [ ] 9.2 `node scripts/lint-ratchet.mjs` (baselines lowered), `cargo deny check`, `cargo audit`,
  `node scripts/check-naming.mjs`, `node scripts/check-crate-files.mjs` pass
- [ ] 9.3 The `semver` job's summary lists the removals of design.md "Behavior visible to applications" and nothing
  else; record it in Implementation notes
- [ ] 9.4 Framework runtime code changed (`swagger_module.rs`, `scalar.rs`): run the `corvis-parity` check and verify
  the three port apps build and keep 0 apidiff differences (facten's `/reference`, consi's `/docs` and `/docs/public`
  included)
- [ ] 9.5 Security review with the `security-reviewer` agent (asset serving and confinement in the blocking task, path
  matching without allocation, mixed case and trailing slashes, the lazy document's failure path, the cached bodies
  and the embedded document's escaping); repeat rounds until no open finding; record findings and fixes in design.md
  under "Security review"; verify every finding has a regression test or a recorded reason
- [ ] 9.6 `openspec validate q-openapi-serving-and-api-hygiene --strict`

## Workflow follow-up

- Archive with `/opsx:archive` (new `openapi/serving`, `openapi/attributes`, `openapi/api-surface`; delta of
  `bench/synthetic`).
- Close the issue, tick it in epic #187, move the card to Done (see the `corvis-task` skill).
