# Tasks

## 1. Reference behavior

- [x] 1.1 Add `crates/corvis-swagger/tests/fixtures/class_validator_shim/` with `package.json` (versions in design.md,
  Decision 6), `dtos.ts` with the DTOs listed there, and `generate.mjs` that compiles `dtos.ts` with the
  `@nestjs/swagger/plugin` `before` transformer (`classValidatorShim: true`, `introspectComments: false`), builds the
  document with `SwaggerModule.createDocument` without listening on a port, and writes `components.schemas` to
  `class_validator_shim.json`; if the recorded keywords or precedence differ from the spec delta, stop and record it
  under Open questions in design.md; verify by running `node generate.mjs` once and committing the JSON (not
  `node_modules`)

## 2. Failing tests

- [x] 2.1 Add `crates/corvis-swagger/tests/api_schema_constraints.rs` with one test per spec scenario (length and
  range, email format, allowed values, unmapped rules, array elements, precedence pairs, explicit option wins, enum)
  and the fixture replay that derives the equivalent Rust structs and compares each `components.schemas` entry as a
  `serde_json::Value`; verify with `cargo test -p corvis-swagger --test api_schema_constraints` that they fail because
  the keywords are missing
- [x] 2.2 In the same file, a DTO that uses every mapped rule once with both `#[derive(Validate, ApiSchema)]` and
  `#[derive(ApiSchema, Validate)]` orders, asserting each keyword; verify it compiles (rule names in sync with
  `crates/corvis-macros/src/validate.rs` `RULES`) and fails on the current code

## 3. Derive

- [x] 3.1 Add `crates/corvis-swagger-macros/src/constraints.rs`: rule-name recognition, argument parsing (positional,
  `message = ...` ignored, bare `each` or `each = ...`, array arguments flattened for `is_in`), and a function that
  returns the `__o.insert(...)` statements in the precedence order of design.md, Decision 2 (upper attribute wins for
  a repeated rule); add unit tests on the token output for `length(2)`, `length(2, 20)`, `is_in(["a", "b"], each)` and
  `min(5)` + `is_positive`; verify with `cargo test -p corvis-swagger-macros`
- [x] 3.2 In `crates/corvis-swagger-macros/src/derive.rs`, call it per field and place its statements before the
  explicit `api_*` inserts (`derive.rs:191-206`), so explicit options win; emit `is_enum` first with
  `::corvis::validation::rules::enum_variants::<T>()` (element type with `each`, like `validate.rs:441-456`); add a
  comment on `RULES` in `crates/corvis-macros/src/validate.rs` pointing to `constraints.rs`; verify that the tests of
  2.1 and 2.2 pass, and that `cargo test -p corvis-swagger` and `cargo test -p cats-app` still pass
- [x] 3.3 Update the rustdoc of `derive_api_schema` (`crates/corvis-swagger-macros/src/lib.rs:372-377`) to mention the
  validation constraints; verify with `cargo doc -p corvis-swagger --no-deps` without warnings

## 4. Example

- [x] 4.1 In `examples/cats-app/tests/openapi.rs`, assert `CreateCatDto`'s derived keywords (`name.maxLength == 50`,
  `age.minimum == 0`, `age.maximum == 40`, `breed.enum`) and that `UpdateCatDto.name` is still not required; verify
  with `cargo test -p cats-app --test openapi`

## 5. Docs

- [x] 5.1 `docs/openapi/types-and-parameters.md`: replace the warning at lines 117-119 with a section "Validation
  constraints" holding the mapping table, the precedence note, the `is_positive` and `contains` quirks, the `is_enum`
  addition, the "explicit options win" rule, and the list of rules that add nothing; same in
  `docs/es/openapi/types-and-parameters.md`
- [x] 5.2 `docs/openapi/cli-plugin.md`: a table row "Reflect class-validator constraints" pointing to that section;
  same in `docs/es/openapi/cli-plugin.md`
- [x] 5.3 Run `npm run docs:check` in `docs/`; verify it passes

## 6. Port apps

- [x] 6.1 Repeat the check of design.md, Context (no `ApiSchema` or `Validate` derive in the three ports under
  `C:/Users/Usuario/Devel/corvis-tests/corvis/`) and record it in design.md, Implementation notes; verify the grep
  finds none

## 7. Definition of done

- [ ] 7.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 7.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] 7.3 `cargo test --workspace` passes, including the regression tests of 2.1 and 2.2, the unit tests of 3.1 and the
  fixture replay
- [ ] 7.4 Docs updated in English and Spanish (`docs/` and `docs/es/`) and `npm run docs:check` passes in `docs/`
- [ ] 7.5 Generated framework code changed (`corvis-swagger-macros`): run the `corvis-parity` check and verify the three
  port apps keep 0 apidiff differences

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #128, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).
