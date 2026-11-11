# Design

## Context

Source: findings S1, S4, S5 and the nice-to-have items of the engineering-quality review recorded in the issue (file
`quality/q-openapi-serving-and-api-hygiene.issue.md`). Code read on `dev` at `d46758a` (2026-11-11). This change is
implemented after `q-openapi-schemas-for-rust-types-and-versions`; task 1.1 re-checks the lines below on the merged
tree. The bench set-up read: `openspec/specs/bench/synthetic`, `bench/run.sh` (scenarios `hello json param post`,
`oha` against `bench/corvis-app` and `bench/node-app`), `bench/corvis-app` (84 lines, no OpenAPI).

Who serves documentation pages among the port apps: facten mounts the Scalar page with its generated document at
`/reference` (`facten-api/src/scalar.setup.rs`), consi mounts two Scalar pages with a static document at `/docs` and
`/docs/public` (`consi-backend/src/main.rs:89-90`); neither uses `SwaggerModule::setup`; houseplan uses no OpenAPI.
Both Scalar middlewares run for every request of those apps, so the corvis-parity check covers the page bodies and the
per-request path matching.

### The findings, checked against the code

| Finding | Where now | Verdict |
|---|---|---|
| S1 allocations per request of any route | `swagger_module.rs:253` (`format!` index path), `:264` (`format!` LICENSE path), `:288-289` (`asset`: two `to_ascii_lowercase`), reached for every `GET`/`HEAD` that is not a docs route when `ui` is on | Confirmed: four allocations for a request the middleware then passes on. Sibling: `scalar.rs:260` (`use_path_matches` lowercases every request path, every method) |
| S1 copies of cached bodies | `:249` (`init_js` clone, which embeds the document), `:258`, `:260` (HTML clones); `scalar.rs:251` (`html.to_string()` of an `Arc<str>` page that embeds the document) | Confirmed |
| S1 JSON and YAML rebuilt per request | `:272`, `:278` → `document.rs:58-72` (`js::stringify` clones and reorders; `to_yaml` clones, reorders and dumps) | Confirmed. The YAML emitter's regexes are compiled once (`LazyLock`, `yaml.rs:111-126`) but run per scalar (`resolve_implicit`, `yaml.rs:232-256`), and each scalar is converted to UTF-16 twice (`yaml.rs:524,533`) |
| S1 blocking read in the middleware | `:296` (`std::fs::read` after `confined_asset`, `custom_swagger_ui_path` only) | Confirmed |
| S1 lazy document under a std mutex | `:170-179` (`get_or_insert_with` under `Mutex` runs the factory, i.e. `create_document`, on a runtime worker; every later request takes the lock) | Confirmed |
| S4 options as free text | `crates/corvis-swagger-macros/src/lib.rs:36-56` (`options_map`), `args.rs:53-79`, and the same in the derive (`derive.rs:112-127`) | Confirmed. The original's option interfaces are TypeScript types: an unknown key in an object literal is a compile error there, so this is a Corvis divergence |
| S4 `x-` checked with `panic!` at run time | `decorators.rs:488-497`; the macro passes any expression (`lib.rs:280-288`) | Confirmed |
| S5 public modules | `lib.rs:20-31` | Confirmed. `decorators` must stay reachable: the attribute macros expand to `::corvis_swagger::decorators::*`, and users call `decorators::zod_response` (`tests/swagger_app.rs:57`) |
| S5 duplicate `probe` | `types.rs:165-211` (public, unused; registers zod DTOs under `full`), the one the macros use is `lib.rs:71-131` (`zod:{full}`) | Confirmed: unused (grep), and wrong if used (a second registry key for the same type) |
| S5 dead code | `zod.rs:26-27` (`_PREFIX_IS_USED`), `args.rs:17-18` (`ident` kept "dead"), `decorators.rs:56-58` (`defined` clones) | Confirmed. `args.rs`'s `ident` becomes useful: it is the span for S4's errors |
| S5 errors as `String`, library panics | `zod.rs:566` (`Result<_, String>`; becomes `SwaggerError` in the previous change), `zod.rs:207` (duplicate `$defs` id), `zod.rs:553,561` (documented panicking wrappers), `schema_factory.rs:663-666` (TypeScript advice, reachable only when a model is not registered) | Confirmed. `cleanup_open_api_doc` stays a documented panicking wrapper (Review note 5 of the previous change); `create_document` gets a `try_` sibling |
| S5 Node-specific log text | `schema_factory.rs:535` | Handled by the previous change (its Decision 8) |
| Startup: `model_info` twice, re-exploration per route, clones in walks | `schema_factory.rs:459` and `:495` (`extract_properties_from_type` then `explore_model_schema`), `:327` and `explorer.rs:456`, `zod.rs:43,48,60,219` | Confirmed. Measured by call counts (Decision 6), not timings |
| YAML double UTF-16 and recursive concatenation | `yaml.rs:524,533` | Confirmed; made irrelevant by caching (Decision 2). No separate fix |
| Duplicated logic | `js_number_string` at `decorators.rs:97` and `yaml.rs:65`; `is_body` at `explorer.rs:486` and `schema_factory.rs:276`; `short_type_name` at `crates/corvis/src/openapi.rs:69` and `crates/corvis/src/di/token.rs:102` | Confirmed. New: the `decorators.rs` copy is wrong for integral values ≥ 2^63 (`f as i64` saturates: `1e20` prints `9223372036854775807`; JavaScript prints `100000000000000000000`). The two `short_type_name` differ on purpose (generic arguments dropped versus kept): documented, not merged |
| `#[matches]` patterns copied as Rust syntax | `constraints.rs` | Already handled: decided and documented by the archived `e13-apischema-ignores-validation-constraints` (Decision 5, Risks). Dropped |
| Doc examples never compiled | `lib.rs:10`, `types.rs:108`, `scalar.rs:6`, `document.rs:94` and one more `ignore` fence | Confirmed; they become `no_run` doctests (Decision 5) and the lint-ratchet baseline drops |

## Goals / Non-Goals

**Goals:**

- A request outside the documentation costs nothing in the middleware; a documentation request costs one lookup and a
  reference-count increment.
- Responses identical to today's, byte for byte, headers included.
- A misspelled option does not compile.
- The public surface is what users need; no library panic on input a caller can pass at run time.

**Non-Goals:**

- Document content, a new YAML emitter, ECMA-262 patterns.

## Decisions

### 1. Benchmark first

- `bench/corvis-app` reads `CORVIS_BENCH_OPENAPI=1`: it then mounts `SwaggerModule::setup("docs", ..)` with a generated
  document of 300 operations over 150 `ApiSchema` DTOs (built by a function in the bench app, same document every run).
  Without the variable the app is exactly what it is today, so the existing scenarios and results do not move.
- `bench/run.sh` gains three scenarios that run only for the Corvis app (the Node app has no OpenAPI module; the report
  prints `n/a` for it): `hello-docs` (`GET /` with the variable set), `docs-json` (`GET /docs-json`), `docs-yaml`
  (`GET /docs-yaml`). Same `oha` parameters, same release build in the shared target directory, same results file.
- Allocation counts are not timings and are tested, not benchmarked: `tests/serving.rs` installs a counting global
  allocator (the pattern of `crates/corvis/tests/socket_io_memory.rs`) and drives the middleware with
  `corvis::testing::request`.

Numbers it must move (same machine, before in task 2.3 and after in task 8.1): allocations in the middleware for a
request it does not serve, from at least 4 to 0 (test); `docs-json` requests per second at least 5 times the before
figure and `docs-yaml` at least 20 times; `hello-docs` requests per second within 3% of `hello` (before and after both
recorded). A figure that is missed is explained in Implementation notes.

### 2. Nothing per request outside the documentation; each body built once

- `Served` holds the mount paths already normalized and lowercased (`final_path`, `index`, `license`, `asset_prefix`,
  init paths, JSON and YAML URLs). `route_matches` and the asset prefix test compare with `eq_ignore_ascii_case` on
  slices; the asset path keeps the original case for the file lookup, as today. `scalar::mount` keeps its mount path in
  lowercase and compares the request path's prefix the same way.
- A request whose method is not `GET` / `HEAD`, or whose path does not start with the mount path (case-insensitive),
  returns before any other work.
- Bodies: `OnceLock<Bytes>` for the JSON, the YAML, the init script and the two HTML variants, filled on first use
  when `patch_document_on_request` is `None` (with a patch function the bodies are built per request, as today, since
  they depend on the request). The Scalar page is `Bytes` built at mount time. Responses are built with the same
  status, headers (`Content-Type` strings exactly as today, including the `charset` the current constructors add) and
  body bytes.

Verified by: the allocation test of Decision 1; pointer identity (two requests to `-json` return bodies whose
`Bytes::as_ptr()` are equal); a byte-equality test that records, before the change (task 2.2), the status, headers and
body of every route the middleware serves (UI, init script both paths, `-json`, `-yaml`, LICENSE, a redirect asset, a
custom asset, the Scalar page on two mounts, mixed-case paths, trailing slashes, `HEAD`) into a test-local snapshot and
compares after; `tests/swagger_app.rs` and `tests/swagger_security.rs` unchanged.

Alternative considered: build the bodies eagerly in `setup`. Rejected: `setup_lazy` exists to defer the document, and
the bodies of an unused format would be built for nothing.

### 3. Off the runtime threads

- Custom assets: `confined_asset` and the read run together inside `tokio::task::spawn_blocking`, so the confinement
  check and the read are as close as today; the result is served the same way.
- The lazy document: a `tokio::sync::OnceCell<Arc<OpenApiDocument>>`; the first request runs the factory with
  `spawn_blocking`, concurrent first requests await the same cell, later requests read it without a lock. A factory
  that panics leaves the cell empty and answers 500 to that request (the panic is logged), as an uncaught handler error
  does today.

Verified by: a test whose factory blocks until the test releases it, with 20 concurrent first requests on a
current-thread runtime that also serves another route meanwhile (the other route answers while the factory is
blocked; the factory runs once; all 20 get the document); the confinement tests of `swagger_security.rs` unchanged.

### 4. Attribute options checked at compile time

- `crates/corvis-swagger-macros/src/keys.rs`: one list of camelCase keys per attribute (`api_operation`,
  `api_response` and the status shortcuts, `api_param`, `api_query`, `api_header`, `api_body`, `api_property`,
  `api_property_optional`, `api_response_property`, `api_schema`, `api_tags`… every attribute that takes options),
  taken from the original's option interfaces in `@nestjs/swagger` 11.4.6 plus Corvis's documented additions (each
  addition with a comment naming its doc page). Source comments say "the original library", never its package name
  (`scripts/check-naming.mjs` scans `crates/*/src`; only `tests/fixtures/**` may name it).
- `options_map` (`lib.rs`) and `options_tokens` (`derive.rs`) check each key; an unknown key is a `syn::Error` at the
  key's span (the `ident` that `args.rs` keeps): ``unknown option `desciption` for #[api_response]; did you mean
  `description`?`` (suggestion when the edit distance is at most 2), followed by the valid keys.
- `#[api_extension(key, value)]`: the key must be a string literal starting with `x-`, checked in the macro; the macro
  then calls a `__private` constructor that cannot fail. `decorators::api_extension` (the function) returns
  `Result<CustomDecorator, SwaggerError>` (`SwaggerError::ExtensionKey`).
- The lists are tested against the original: `tests/fixtures/option_keys/extract.mjs` (in the existing fixture package
  of the previous change, which pins `@nestjs/swagger` 11.4.6 and `typescript`) reads the option interfaces from the
  installed type declarations with the TypeScript compiler API and writes `option_keys.json`; a hidden proc macro
  `__option_keys!(api_property)` expands to the macro's list, and `tests/attribute_keys.rs` asserts each list equals the
  recorded keys plus the commented additions.

Verified by: `compile_fail` doctests for a misspelled key in a route attribute, in `#[api_property]` on a field, and a
non-`x-` extension key; `tests/attribute_keys.rs`; the port apps build (task 9.4) with every key they use.

Alternative considered: a warning instead of an error. Rejected: proc macros cannot emit warnings on stable without a
hack, and the original rejects the key at compile time.

### 5. The public surface

- `lib.rs`: `js`, `metadata`, `schema_factory`, `explorer`, `yaml`, `types` and `zod` become `pub(crate)`. Root
  re-exports keep every public item users need: `SwaggerType`, `ZodIo`, `ctor_marker`, `Output`, `OperationIdFactory`,
  `SwaggerDocumentOptions`, `OpenApiVersion`, `CleanupOptions`, `SwaggerError`, `cleanup_open_api_doc`,
  `cleanup_open_api_doc_with`, `try_cleanup_open_api_doc` (and the existing ones). `decorators`, `scalar`, `swagger_ui`
  and `document` stay public modules. What the macros need (`property_metadata`, `register_model`, markers, the probe)
  moves under `__private` (`#[doc(hidden)]`).
- `types::probe` (`types.rs:165-211`) is deleted.
- Errors: `SwaggerError` (`#[non_exhaustive]`, `thiserror`-style `Display` without TypeScript terms) gains
  `DuplicateDefsId`, `UnregisteredModel { property, model }` and `ExtensionKey`; `SwaggerModule::try_create_document`
  returns `Result<OpenApiDocument, SwaggerError>`; `create_document` keeps its signature and is documented as panicking
  on those conditions, like `cleanup_open_api_doc`. `zod.rs:207` and `schema_factory.rs:663` return the errors through
  the `try_` path.
- Dead code: `_PREFIX_IS_USED` and the unused constant, `defined`, `#[allow(dead_code)]` on `ident` (now used).
- Doc examples: the five `ignore` fences become `no_run` doctests that compile (a small `CorvisApplication` setup in a
  hidden line); `lint-baseline.json` is lowered.

Verified by: `cargo doc -p corvis-swagger` lists no internal module; the `semver` job's summary lists exactly the
removals above; the port apps build with `use corvis_swagger::*` (the glob loses the module names; task 9.4);
`compile_fail` doctest that `corvis_swagger::schema_factory` is not reachable; tests for each new error.

### 6. Startup without repeated work

- `create_document` memoizes `model_info` by model key for the duration of the call (a `HashMap` in the explorer's
  state), and `get_custom_type` / `create_response` reuse an explored model's name instead of exploring again.
- `walk_json_schema` and `schema_metadata` take `&mut Value` / `&Value` instead of cloning subtrees.

Verified by: a test-build counter of `ApiSchemaDef` builds per model per document (1 after, the before figure
recorded in task 2.1); the recorded documents of the previous change's `document_forms` fixture and
`class_validator_shim` replay unchanged; `tests/swagger_app.rs` unchanged.

### 7. One copy of each small rule

`js_number_string` is the `yaml.rs` port of `Number.prototype.toString` for both callers (a test with `1e20`, `-0`,
`1e21`, `0.1`, `2^53 + 1`); one `is_body` in `schema_factory.rs` used by `explorer.rs`; doc comments of both
`short_type_name` say what they keep and why they differ.

## Security invariants this change comes near

| Invariant | Source | Pinned by |
|---|---|---|
| `custom_swagger_ui_path` serves only regular files inside the folder (percent-decoding, no dot segments, no device names, canonical path inside canonical folder) | `confined_asset` (`swagger_module.rs:305-330`) | `tests/swagger_security.rs` unchanged; the check and the read stay together in one blocking task (Decision 3) |
| The embedded document is escaped in the init script and the Scalar page; `$js` values only from the developer | `swagger_ui.rs`, `scalar.rs:142-150` | Byte-equality snapshot of Decision 2; `swagger_security.rs` |
| Matching is case-insensitive like Express, one optional trailing slash | `route_matches` (`swagger_module.rs:201-206`) | Snapshot cases with mixed case and trailing slashes |
| A public function does not panic on input a caller passes at run time | house rule | Decision 5 tests; the lint ratchet counts `panic` in `corvis-swagger` |

The change touches file-system serving and request path matching: the security review is mandatory (task 9.5).

## Behavior visible to applications

| What | Before | After | Consequence |
|---|---|---|---|
| Responses of the Swagger UI, `-json`, `-yaml`, assets, Scalar | Built or copied per request | Cached `Bytes` | Same bytes and headers; less CPU and memory per request |
| A request outside the documentation | Four allocations in the middleware | None | — |
| Lazy document | Built on a runtime worker under a mutex | Built once in a blocking task | Other requests are served while it builds |
| Misspelled attribute key | Compiles, ignored | Compile error with a suggestion | Code that compiled may stop compiling: the key never worked |
| Non-`x-` extension key | Panic at startup | Compile error (attribute) / `Err` (function) | — |
| `corvis_swagger::{js, metadata, schema_factory, explorer, yaml, types, zod}` | Public | Private; items re-exported | Paths to internal items stop compiling; documented items keep a path |
| `decorators::api_extension` | Returns `CustomDecorator`, may panic | Returns `Result` | Direct callers handle the error |
| `SwaggerModule::try_create_document` | — | Added | — |

## Risks / Trade-offs

- **The parity check decides.** facten and consi serve Scalar pages through the middleware this change rewrites; their
  apidiff suites compare those responses byte for byte. Task 9.4 runs the check; any difference is a defect.
- The key lists may miss a key the original accepts → they are compared with the original's type declarations by a
  test, and the port apps build with every key they use.
- Removing public modules breaks code that used internals → before 1.0, said plainly; the `semver` job lists the
  removals.
- Caching keeps the serialized document in memory for the process's life → the same order as the document itself, and
  the init script already did so.

## Open questions

1. Keep `create_document` panicking (documented) with a `try_` sibling, or make it return `Result`? Recommendation:
   keep it (the original's call shape; facten uses it), add `try_create_document`.
2. Should unknown-key errors list all valid keys or only the suggestion? Recommendation: suggestion first, then the list
   (lists are short).
3. Expose `metadata` for users writing their own decorators? Recommendation: no; `decorators` has the public helpers.

## Review notes

These settle the open questions and override the design where they differ.

1. `create_document` keeps its signature and documented panics; `try_create_document` is added.
2. Unknown-key errors give the suggestion, then the valid keys.
3. `metadata` stays private.
4. Implemented after `q-openapi-schemas-for-rust-types-and-versions` is merged; task 1.1 is mandatory.
5. The security review is mandatory before the merge (asset serving, path matching, the lazy document's failure path).
