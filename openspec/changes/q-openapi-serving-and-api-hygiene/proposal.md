# Proposal

Implements the issue filed from `quality/q-openapi-serving-and-api-hygiene.issue.md` (epic
[#187](https://github.com/randygil/corvis/issues/187), code quality audit). Source: the engineering-quality review of
`corvis-swagger`, `corvis-swagger-macros`, `corvis-cli` and the workspace, read against `dev` at `d46758a`. This change
covers findings S1, S4, S5 and the nice-to-have items of the OpenAPI generator. It lands after
`q-openapi-schemas-for-rust-types-and-versions` (which introduces `SwaggerError` and edits the same files) and after
`q-workspace-ci-gates-and-lint-ratchet` (whose `semver` job shows the API surface this change removes, and whose lint
ratchet keeps the removed panics out).

## Why

- **Every request of the application pays for the documentation page.** `SwaggerModule::setup` installs a global
  middleware (`crates/corvis-swagger/src/swagger_module.rs:152`) that, for every `GET` / `HEAD` of any route, formats
  two paths and lowercases the request path and the mount path (`:253`, `:264`, `:288-289`); `scalar::mount` lowercases
  every request path (`scalar.rs:260`). The `-json` and `-yaml` routes clone, reorder and serialize the whole document
  on every request (`:272`, `:278`; `document.rs:58-72`), the YAML emitter running six regular expressions per scalar
  (`yaml.rs:111-126`, `232-256`); the cached HTML, the `swagger-ui-init.js` (which embeds the document) and the Scalar
  page are copied per request (`:249`, `:258-260`, `scalar.rs:251`). A custom asset is read with blocking
  `std::fs::read` inside the async middleware (`:296`), and the lazy document is built under a `std::sync::Mutex` on a
  runtime worker (`:174-177`).
- **Attribute options are free text.** `#[api_response(desciption = "x")]` or `#[api_property(min_lenght = 3)]`
  compile and do nothing (`crates/corvis-swagger-macros/src/lib.rs:36-56`, `args.rs:53-79`, `derive.rs:112-127`);
  in the original, TypeScript rejects an unknown key of the options object. A non-`x-` extension key is checked at run
  time with `panic!` (`decorators.rs:492-497`).
- **The public surface is the implementation.** `lib.rs:20-31` publishes `js`, `metadata`, `schema_factory`,
  `explorer`, `yaml` and `types` whole, a second unused `probe` module with a different registry key
  (`types.rs:165-211` versus `lib.rs:113-121`), dead code hidden by `#[allow(dead_code)]` (`zod.rs:26-27`), a no-op
  clone (`decorators.rs:56-58`), errors as `String`, and library panics with TypeScript advice
  (`schema_factory.rs:663-666`, `zod.rs:207`).
- Smaller: `model_info` runs twice per model per exploration (`schema_factory.rs:459,495`), `walk_json_schema` clones
  every subtree (`zod.rs:43,48,60`), two `js_number_string` with different results (`decorators.rs:97` saturates above
  2^63), `is_body` twice, the five doc examples of the crate never compiled.

## What Changes

- **Benchmark first.** `bench/run.sh` gains Corvis-only scenarios `hello-docs` (the `hello` route with the Swagger UI
  mounted), `docs-json` and `docs-yaml` (a 300-operation document); a counting-allocator test measures what the
  middleware allocates for a request outside the documentation. Before and after numbers go to design.md.
- **Nothing per request outside the documentation.** Mount paths precomputed in lowercase; matching by
  `eq_ignore_ascii_case` on slices; no allocation for a request the middleware does not serve.
- **Each body built once.** JSON, YAML, init script and pages cached as `Bytes` when no `patch_document_on_request` is
  set; responses byte for byte as today (body and headers).
- **Off the runtime threads.** Custom assets read in `spawn_blocking` (confinement unchanged); the lazy document built
  once in `spawn_blocking`, concurrent first requests waiting on one cell.
- **Options checked at compile time.** One list of keys per attribute (the original's option interfaces plus Corvis's
  documented additions), unknown keys a compile error with a suggestion, extension keys checked for `x-` at compile time;
  a test compares the lists with keys extracted from the original's type declarations.
- **A public surface that is the API.** Internal modules become `pub(crate)`; the public items are re-exported at the
  root or under `decorators`, `scalar`, `swagger_ui`; the macros' support moves to `__private`. `SwaggerError` gets the
  remaining variants; `SwaggerModule::try_create_document` returns it; library panics become errors; dead code goes;
  doc examples compile.
- **Smaller items.** `model_info` once per model per document; zod JSON Schema walked in place; one `js_number_string`;
  one `is_body`.

## Capabilities

### New Capabilities

- `openapi/serving`: what the mounted middleware costs and serves (no work outside the documentation, cached bodies,
  identical responses, no blocking on runtime threads).
- `openapi/attributes`: compile-time checking of attribute options and extension keys.
- `openapi/api-surface`: the public modules of `corvis-swagger`, typed errors, no library panics.

### Modified Capabilities

- `bench/synthetic`: added requirement for the OpenAPI scenarios.

## Impact

- Edited: `crates/corvis-swagger/src/{lib,swagger_module,scalar,document,decorators,types,zod,schema_factory,explorer,yaml,metadata,js}.rs`,
  `crates/corvis-swagger-macros/src/{lib,args,derive}.rs` (new `keys.rs`), `bench/run.sh`, `bench/corvis-app/{Cargo.toml,src/main.rs}`,
  `bench/README.md`, `bench/results.md`, `crates/corvis-swagger/README.md`, docs pages of the OpenAPI section (English and
  Spanish) where a path or an error changes.
- New: `crates/corvis-swagger/tests/serving.rs` (counting allocator, pointer identity, byte-identical responses),
  `crates/corvis-swagger/tests/attribute_keys.rs`, `crates/corvis-swagger/tests/fixtures/option_keys/{extract.mjs,option_keys.json}`
  (in the fixture package of the previous change), and `compile_fail` doctests for unknown keys.
- Public API (changed now, before 1.0): `corvis_swagger::{js, metadata, schema_factory, explorer, yaml, types}` stop
  being public modules (their public items stay reachable through root re-exports: `SwaggerType`, `ZodIo`,
  `ctor_marker`, `Output`, `OperationIdFactory`, `SwaggerDocumentOptions`, `OpenApiVersion`, `CleanupOptions`);
  `types::probe` is removed; `decorators::api_extension` returns `Result<CustomDecorator, SwaggerError>`;
  `SwaggerModule::try_create_document` is added; unknown attribute keys stop compiling.
- Behavior: responses identical; startup log lines unchanged; a misspelled attribute key is now a compile error.
- Dependencies: none new (`bytes` and `tokio` are already dependencies through `corvis`; `corvis-swagger` declares
  `bytes` with `workspace = true` if needed). `bench/corvis-app` depends on `corvis-swagger`.
- Runtime code of the served pages changes: the corvis-parity check runs (facten and consi serve pages through
  `scalar::mount`).

## Out of scope

- Document content (`q-openapi-schemas-for-rust-types-and-versions`).
- Translating `#[matches]` patterns to ECMA-262 (decided in the archived `e13-apischema-ignores-validation-constraints`).
- A second YAML emitter: once the YAML is cached, its cost is paid once per document.
- Unifying `corvis::openapi::short_type_name` with the dependency-injection `short_type_name`: they answer different
  questions (one drops generic arguments, one keeps them); their doc comments say so.
