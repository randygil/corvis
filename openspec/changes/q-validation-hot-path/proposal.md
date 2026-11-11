# Proposal

Implements the issue filed from `quality/q-validation-hot-path.issue.md` (epic
[#187](https://github.com/randygil/corvis/issues/187), code quality audit). Source: the same engineering-quality review of
the validation library as `q-validation-correctness`, read against `dev` at `d46758a`. Second of three changes, after
`q-validation-correctness` (whose thrown-error and zod 3 edits touch some of the same functions) and before
`q-validation-api-hygiene`.

## Why

A valid request pays for copies the validation does not need:

- The pipe owns the request value but parses it by reference, so the whole body is copied once before validation
  starts; the serializer copies the response body twice.
- Every check of a string copies the string twice before it knows whether it fails (`z.string().min(1).max(255).email()`
  copies a valid string about six times); the length checks of arrays copy the whole array.
- `.refine()`, `.superRefine()`, `z.custom()` and `z.preprocess()` copy the value they look at; `.transform()` copies it
  twice; a `.refine()` on the root of a 1 MiB body copies 1 MiB per request.
- A union copies its input once per option it tries, including options that fail on the type alone; with a recursive
  union every level copies its subtree, so the cost grows with depth times size. The merge of an intersection copies
  each subtree again at every level.
- An enum recomputes and copies its values on every parse; each parse takes the global configuration lock twice and
  allocates a snapshot; a strict object scans its shape once per input key and allocates its catch-all schema per parse.

There is no benchmark of the validation library, so the first task group builds one, and every decision names the number
it must move.

## What Changes

- **Benchmark.** `bench/validation/`: an in-process, release-mode benchmark of `ZodValidationPipe` and `parse` over a
  realistic DTO (nested object, arrays, strings with several checks, an enum, a union and a discriminated union) validated
  successfully and with errors, a recursive union, an intersection, a refinement over a 1 MiB body, an enum-heavy object,
  a wide strict object, the serializer, and the pipe on 1 and 8 threads. It reports time per parse and allocations and
  bytes per parse, also as a multiple of one copy of the input. Run before and after; numbers go to design.md.
- **Issue inputs built at the push.** Checks copy the value only when they push an issue.
- **Callbacks borrow or take the value.** Refinements, `z.custom()` and `z.preprocess()` borrow it; `.transform()` and
  `.overwrite()` take it; the refinement context borrows its input.
- **Unions without speculative copies.** An option that fails on the JSON type of the input alone is answered without
  running it on a copy; when a single option is left it runs on the value itself if nothing can observe the difference;
  the merge of an intersection moves values instead of copying them. A test-only switch keeps the old paths as an oracle
  for a differential property test.
- **Owned parse.** `ZodSchema::parse_owned` and `parse_owned_opt`, `z::parse_val_owned`; the pipe and the serializer use
  them; concrete schema types parse without cloning themselves.
- **Smaller per-parse costs.** Enum values computed once; the configuration read through a per-thread snapshot refreshed
  by a generation counter; strict objects look keys up in a set and share their catch-all schemas.

## Capabilities

### New Capabilities

- `bench/validation`: the validation benchmark (scenarios, what it reports, where it builds, how results are recorded).

### Modified Capabilities

- `validation/zod`: added requirements for owned parse entry points, the copies a successful parse may make, and
  unions whose cost does not grow with the depth of a recursive input.

## Impact

- New: `bench/validation/{Cargo.toml,src/main.rs,run.sh,README.md}` (workspace member, `publish = false`),
  `crates/corvis/tests/zod_hot_path_heap.rs`, `crates/corvis/tests/zod_union_paths.rs`.
- Edited: `crates/corvis/src/z/{core,mod,types,compound,wrappers,pipe}.rs`, `Cargo.toml` (workspace member),
  `.gitignore` (`bench/validation/results/`), `scripts/check-naming.mjs` (the new bench as a published surface),
  `bench/README.md` (link), `docs/application/validation.md`, `docs/es/application/validation.md`.
- Public API: added `ZodSchema::parse_owned`, `ZodSchema::parse_owned_opt`, `z::parse_val_owned`. Nothing else changes
  signature. `RefinementCtx` keeps its signature (it already has a lifetime).
- Behavior visible to applications: none intended; issues, messages, key order and outputs stay byte for byte, pinned by
  the fixtures and by the differential property test.
- No new crate dependency. Framework runtime code changes, so the corvis-parity check runs.

## Out of scope

- Storing a copy of the input in every issue on the failure path (finding N2): measured by the `dto-fail` scenario and
  decided from that number (design.md Decision 10).
- `ZodGuard`, which validates a borrowed request body (deprecated in favor of the pipe).
- zod 3 mode performance: no port app uses it (`q-validation-correctness` Decision 4).
- Typed options and API consistency (`q-validation-api-hygiene`).
