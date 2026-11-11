# Design

## Context

Source: findings M1, M2, M3, S2 and S3 of the engineering-quality review recorded in the issue (file
`quality/q-openapi-schemas-for-rust-types-and-versions.issue.md`). Code read on `dev` at `d46758a` (2026-11-11). The
original library was read from the port apps' installed packages: `@nestjs/swagger` 11.4.6 and `nestjs-zod` 5.5.0
(with `zod` 4.4.3) under `C:/Users/Usuario/Devel/corvis-tests/nest/facten-api/node_modules/`, the versions the facten
original and the existing `class_validator_shim` fixture pin. The original is not vendored in the repository: every
parity statement below is pinned by the recorded fixture of Decision 1, not by this reading.

### How parity is checked today

- `crates/corvis-swagger/tests/fixtures/class_validator_shim/`: `generate.mjs` compiles `cases.dto.ts` with the original
  compiler plugin, runs the original `createDocument` and writes `components.schemas` to `class_validator_shim.json`;
  `tests/api_schema_constraints.rs` replays the Rust structs and compares each schema as a JSON value, with intended
  differences listed in `known_differences()`. Regenerable (`npm install && node generate.mjs`).
- `crates/corvis-swagger/tests/swagger_app.rs`: "Expected values were produced by the reference Node.js generators for
  the same app" (line 74), compared as excerpts (one operation, one parameter list, one schema, strings for key order).
  No script was committed: the golden values cannot be regenerated or extended.
- The port apps: facten builds its document with `SwaggerModule::create_document` + `cleanup_open_api_doc`
  (`facten-api/src/app.setup.rs:41-48`, zod DTOs only, `String` path parameters) and serves it through
  `scalar::mount`; consi serves the original's `openapi.json` as a static string through `scalar::mount`
  (`consi-backend/src/main.rs:10,89-90`); houseplan does not use `corvis-swagger`. None derives `ApiSchema` (grep of
  `corvis-tests/corvis/*/src` on 2026-11-11). The corvis-parity check compares their responses byte for byte.

### The findings, checked against the code

| Finding | Where now | Verdict |
|---|---|---|
| M1 generics and homonyms overwrite each other | `derive.rs:153` (`short_type_name(type_name::<Self>())`), `crates/corvis/src/openapi.rs:69-71` (drops `<...>`), `schema_factory.rs:532-539` (warn, then `schemas.insert`) | Generics: confirmed, Corvis's own (no generics at run time in the original). Homonyms (`users::CreateDto` / `posts::CreateDto`): confirmed, but the original does the same for two classes with one name, same warning text (`schema-object-factory.js:244-249`): parity, kept (Decision 8). Sibling found: inside `impl<T> ApiSchema for Page<T>` the probe cannot see a bound on `T`, so a field of type `T` falls back to `opaque_marker(type_name::<T>())`, and `types::register_model` keeps the first entry for a key (`types.rs:46-52`, `or_insert`): if that runs before `T`'s own registration, `T` stays an empty object for the whole process |
| M2 Rust types as empty objects | `types.rs:117-163`, fallback `lib.rs:98-101`; second table for handler parameters `types.rs:216-244` | Confirmed, Corvis's own (TypeScript `Date` is documented as `string`/`date-time`; Rust types have no counterpart). Siblings: `Box`, `Arc`, `Rc`, `HashSet`, `BTreeSet`, `VecDeque`, arrays and `NonZero*` are in the parameter table but have no `SwaggerType` impl; `NaiveDate` is classified as `Date` (`date-time`), which is wrong for a date |
| M3 `exclusiveMinimum` as a number in a 3.0 document | `constraints.rs:181,183`, pinned by `tests/api_schema_constraints.rs:257-258`, documented in `docs/openapi/types-and-parameters.md:143` | Confirmed, Corvis's own: the archived `e13` change chose exclusive bounds for floats (its Review note 2) without the 3.0 form. Fixed (Decision 6) |
| M3 nullable response as `oneOf` + `type: "null"` | `explorer.rs:449-465` | Inherited: `response-object-factory.js:35` and `response-object-mapper.js:18,25` build the same three forms. Kept by default; strict cleanup rewrites it (Decision 7) |
| M3 `search` key on routes of every method | `explorer.rs:312` | Inherited: `swagger-explorer.js:173-183` lists the same eight methods. Kept by default; strict cleanup drops `search` |
| M3 derived DTOs keep `nullable` in 3.1 | `schema_factory.rs:242-262` (`nullable` in `SCHEMA_OPTIONS_KEYS`) | Partly: the derive never writes `nullable` today; a user-written `nullable = true` passes through in every version, as in the original (`setOpenAPIVersion` only sets the string, `document-builder.js:38-45`). The Corvis-written nullability of Decision 5 is version-aware (Decision 6); a user-written one is rewritten only by strict cleanup |
| S2 `Option<T>` without `nullable`, serde attributes ignored | `schema_factory.rs:167-169`, `derive.rs:179` | Confirmed, Corvis's own (the derive is Corvis's translation; the plugin gives `nullable` for `T \| null`, which is what serde writes for `None`). Sibling: `rename(serialize = "..")` / `rename_all(serialize = "..")` are parsed and dropped (`derive.rs:22-27`) |
| S3 cleanup is manual | `zod.rs:548-562`; `lib.rs:14` example | Inherited (the original requires `cleanupOpenApiDoc` too; facten calls it). Kept explicit; a served document with leftover markers is reported (Decision 9) |
| S3 `options` / `search` operations skipped by cleanup | `zod.rs:594` | Inherited: `nestjs-zod` `cleanupOpenApiDoc` destructures `{ get, patch, post, delete, put, head }` (`dist/index.cjs:254-261`). Strict only |
| S3 references under `items` not renamed | `zod.rs:352-374` | Inherited: `fixRefsInBodies` (`dist/index.cjs:322-341`) only renames a direct `schema.$ref`. Strict only |

No finding is dropped. Three are inherited forms (kept, opt-in strict fix), one is partly inherited (homonyms), and
the rest are Corvis's own and fixed by default.

## Goals / Non-Goals

**Goals:**

- Every Rust type a DTO commonly holds is documented as what serde writes, and the user can describe any other.
- Keywords Corvis adds are valid for the version the document declares.
- Documents of DTOs the port apps already describe correctly do not change by a byte.
- The forms inherited from the original are recorded by a fixture the original generates, and a user who wants a
  valid document can get one with one option.

**Non-Goals:**

- Changing an inherited form by default.
- `type: integer` for Rust integers, ECMA-262 translation of patterns (proposal, Out of scope).
- Serving, caching, macro option validation, public surface (`q-openapi-serving-and-api-hygiene`).

## Decisions

### 1. A recorded fixture of document forms, generated by the original

`crates/corvis-swagger/tests/fixtures/document_forms/`, built like `class_validator_shim`:

- `package.json` pinning `@nestjs/common`, `@nestjs/core`, `@nestjs/platform-express` 11.2.1, `@nestjs/swagger` 11.4.6,
  `nestjs-zod` 5.5.0, `zod` 4.4.3, `reflect-metadata` 0.2.2, `rxjs` 7.8.2, `typescript` 5.9.3; `package-lock.json`
  committed; `node_modules` and `out/` ignored by git.
- `app.ts`, compiled by `generate.mjs` like `class_validator_shim` does, declares: (a) the cats app of
  `tests/swagger_app.rs` (same controllers, decorators, zod DTOs, global prefix, `DocumentBuilder` calls); (b) a
  controller with `@ApiOkResponse({ type: CatDto, nullable: true })`, `{ type: [CatDto], nullable: true }` and
  `{ type: String, nullable: true }`; (c) an `@All('any')` route; (d) a zod DTO with `.meta({ id: 'Kitten' })` used as
  `@ApiOkResponse({ type: [KittenDto] })` and as a body; (e) a second document with `setOpenAPIVersion('3.1.0')` and an
  `@ApiProperty({ nullable: true })` property.
- `generate.mjs` runs `SwaggerModule.createDocument` and `cleanupOpenApiDoc` and writes `document_forms.json`:
  `{ versions, documents: { cats, forms, forms31 } }`, each document as the exact string `JSON.stringify(doc)` returns
  (the text the original serves at `-json`).
- `tests/document_forms.rs` builds the equivalent Corvis apps and asserts `doc.to_json()` equals each recorded string
  byte for byte. `tests/swagger_app.rs` keeps its serving assertions and drops the hand-pasted excerpts the fixture now
  covers in full.

What the fixture is expected to record (from the reading above; if it records something else, the fixture wins and
Implementation notes say so): the three `oneOf` + `{ type: "null" }` forms, eight operations for `@All` including
`search`, a dangling `items.$ref` to `KittenDto` next to a component `Kitten`, and `nullable: true` inside a `3.1.0`
document. These are the inherited forms Decision 7 rewrites only on request.

Alternatives considered: assert the forms by hand from the reading. Rejected: the brief of the parity work and the
existing practice (`class_validator_shim`) require output recorded from the original.

### 2. One component per generic instantiation

- New `corvis_swagger::__private::schema_name(type_name: &str) -> String`: the short name of the base type, then, for
  each generic type argument, `_` and the argument's `schema_name` (recursively; lifetimes dropped; `&T` read as `T`;
  any character outside `[A-Za-z0-9._-]`, the set OpenAPI allows in component keys, becomes `_`). `Page<User>` is
  `Page_User`, `Page<Vec<User>>` is `Page_Vec_User`, `Envelope<Page<User>, u32>` is `Envelope_Page_User_u32`. For a
  type without generic arguments it returns exactly what `short_type_name` returns today.
- Used everywhere a component name comes from a Rust type: `derive.rs:153`, `types::api_schema_marker` (`types.rs:90`),
  `types::opaque_marker` (`types.rs:96`), the zod probe (`lib.rs:119`), `explorer.rs:554`, `types::Output<T>`
  (`types.rs:154`). `#[api_schema(name = ...)]` still wins. `corvis::openapi::short_type_name` is not changed (it has
  other callers).
- **Bounds.** For each type parameter used by a field, the derive adds `T: ::corvis::openapi::ApiSchema` to the impl's
  where-clause, so the probe sees `T` as a model. `#[api_schema(bound = "T: ::corvis_swagger::SwaggerType")]` replaces
  the generated bounds (same rule as serde's `bound`), for wrappers over scalars.
- **Registry.** `register_model` lets an `ApiSchema` or zod entry replace an `Opaque` entry with the same key (never the
  reverse), so the order of registration cannot leave a described type as an empty object.

Alternatives considered: `PageUser` (can collide with a real `PageUser`), a hash suffix (unreadable in tools), refusing
generics (common pattern). Rejected.

Verified by (fail before the fix): `tests/rust_types.rs` documents `Page<User>` and `Page<Post>` in one app: two
components `Page_User` and `Page_Post`, each `items.$ref` to its own type, no duplicate warning; `Page<String>` with
`bound`; a registration-order test where `T` is first seen through the generic field.

### 3. Rust scalar types: one table, two users

- `types::RUST_SCALARS`: rows `(last path segment, constructor, format)`. Constructors are the ones the generator already
  maps (`String`, `Number`, `Boolean`, `Date`, `Array`, `Object`). Rows: `String`, `str`, `char`, `Cow` → `String`;
  integer and float primitives and `NonZero*` → `Number`; `bool` → `Boolean`; `Vec`, `VecDeque`, `HashSet`, `BTreeSet`,
  `IndexSet` → `Array`; `DateTime`, `NaiveDateTime`, `OffsetDateTime`, `PrimitiveDateTime`, `SystemTime` → `Date`;
  `NaiveDate`, `Date` (time) → `String` + `date`; `NaiveTime`, `Time` (time) → `String` + `time`; `Uuid` → `String` +
  `uuid`; `Value`, `Map`, `HashMap`, `BTreeMap`, `IndexMap` → `Object`.
- **Handler parameters** (`ctor_from_type_name`, `explorer.rs` `param_type`) read the table; a format is added to the
  parameter's schema. Only `NaiveDate` changes constructor (`Date` → `String` + `date`); `Uuid`, `NaiveTime` gain a
  format.
- **`SwaggerType` impls** are generated from the same rows by a `macro_rules!` that takes the table: std rows always,
  plus `Box<T>`, `Arc<T>`, `Rc<T>` (as `T`), `[T; N]`; `chrono` rows behind the `chrono` feature, `uuid` behind `uuid`,
  `time` behind `time`. `SwaggerType` gains `fn swagger_format() -> Option<&'static str> { None }`; the derive inserts
  `format` before the constraint and explicit options (so `#[is_uuid]` or `api_property(format = ..)` still win).
- **Features.** `chrono` and `uuid` on by default (`chrono` is already in every application through `corvis`; `uuid` is
  small and in every application with the ORM), `time` off. The features are optional dependencies on the workspace
  versions; no new crate in `Cargo.lock`.
- **Foreign types per document.** `SwaggerDocumentOptions::with_type::<T>(schema: Value)` stores `type_name::<T>()` →
  schema; when an `Opaque` model resolves (`schema_factory.rs` `model_info`, `ModelKind::Opaque`) and the document's
  map has its key, the property gets that schema instead of a `$ref` to an empty component. Recipe in the docs for the
  ORM's `Decimal` and `rust_decimal::Decimal` (`{ "type": "string", "format": "decimal" }`).
- **Warning.** When a type still falls back to an empty object, `create_document` logs once per type per document:
  the Rust type, and the three ways out (feature, derive, `with_type`). Logs only; the document is the same.

Verified by: a test that, for every row, compares the parameter classification with the `SwaggerType` impl of the
same type (constructor and format), with all three features on; field tests for `DateTime<Utc>`, `NaiveDate`, `Uuid`,
`Box<User>`, `HashSet<String>`, `[u8; 4]`, `NonZeroU32` (fail before the fix: empty components); a `with_type` test;
the `features` job of `q-workspace-ci-gates-and-lint-ratchet` builds each feature alone.

Alternative considered: a single literal table read at run time by both paths. Rejected: the derive needs a type-level
answer (`SwaggerType`) for generics and probes; the macro generating impls from the rows keeps one source.

### 4. Unit enums

`#[derive(ApiSchema)]` on an enum whose variants are all unit variants implements `SwaggerType` with an enum marker
(`{"$enum": [...]}`) holding the names serde writes, computed at macro time from `rename_all` (enum rules: the variant
name in PascalCase converted) and per-variant `rename` / `rename(serialize = ..)`. A data-carrying variant,
`#[serde(untagged)]`, `tag` or `content` give a compile error naming the variant or attribute. `property_metadata` and
the parameter path turn the marker into `type: "string"` plus `enum` (unless `enum` was given explicitly or by
`#[is_in]`; `#[is_enum]` yields the same list). Inline, not a component (the original inlines unless `enumName` is
given); `#[api_property(enum_name = "Role")]` keeps working as today.

Verified by: a test deriving both `ApiSchema` and `Deserialize` on enums with each `rename_all` rule and asserting the
list equals `corvis::validation::rules::enum_variants::<T>()`; a field, an `Option` field and a query parameter typed
with the enum; a `compile_fail` doctest for a data variant.

### 5. Fields as serde writes them

- `Option<T>` with no `skip_serializing_if`: `required: false` and nullable. With `skip_serializing_if`: `required: false`
  only (today's output).
- `#[serde(default)]` (field or container) and `skip_serializing_if` on a non-`Option` field: `required: false`.
- `#[serde(flatten)] inner: T`: the derive splices `<T as ApiSchema>::api_schema_def().properties` at the field's
  position (required flags kept); `ApiSchema` gets `#[diagnostic::on_unimplemented]` saying that a flattened field's type
  must derive `ApiSchema`. A flattened map gives that compile error.
- `rename(serialize = "x")` and `rename_all(serialize = "...")` are read (the serialized name is the documented one).
- Nullability is written through the private marker of Decision 6, never as a raw keyword.
- The fixture DTO `OptionalDto` of `api_schema_constraints.rs` (TypeScript `nick?: string`) gains
  `#[serde(skip_serializing_if = "Option::is_none")]`, which is its exact Rust equivalent, so the comparison with the
  recorded schema is unchanged; a new test covers the plain `Option`.

Verified by (fail before the fix): `tests/rust_types.rs` for each bullet, and a serde round trip in the same test that
serializes a value with `None` and checks the document allows what was written (`null` present → nullable).

### 6. Corvis keywords written in the declared version

Corvis's own keywords are recorded with private keys and rewritten at the end of `create_document` by a finishing pass
(`crates/corvis-swagger/src/version.rs`), which reads `doc.openapi`:

| Private key (from) | 3.0 | 3.1 |
|---|---|---|
| `x-corvis-exclusive-minimum: n` (`is_positive` on floats) | `exclusiveMinimum: true` with `minimum: n`; if the schema already has a `minimum` ≥ `n`, the private key is dropped (the explicit bound is tighter) | `exclusiveMinimum: n`, next to any `minimum` |
| `x-corvis-exclusive-maximum: n` (`is_negative`) | the same with `maximum` | `exclusiveMaximum: n` |
| `x-corvis-nullable: true` (Decision 5) | `nullable: true`; for a `$ref`, `allOf: [{ $ref }]` with `nullable: true` | `type: [T, "null"]`; for a `$ref` or a schema without `type`, `anyOf: [schema, { type: "null" }]` |

The pass touches only these keys: a document without them is returned byte for byte (`OpenApiDocument` is not
re-serialized for it). The keys are placed where `constraints.rs` and `property_metadata` put the keywords today, so key
order of the rest of the property is unchanged. `docs/openapi/types-and-parameters.md:143` is corrected.

Verified by: `api_schema_constraints.rs` asserts the 3.0 form (`minimum: 0, exclusiveMinimum: true`) and a 3.1
document the numeric form; `min(5)` + `is_positive` on `f64` gives `minimum: 5` alone in 3.0; a test that runs the
pass on the recorded documents of Decision 1 and asserts the output string is identical.

### 7. Strict cleanup, opt-in

- `CleanupOptions { version: OpenApiVersion, strict: bool }` (`#[non_exhaustive]`, `Default` = auto version, not
  strict). `try_cleanup_open_api_doc(doc, CleanupOptions) -> Result<OpenApiDocument, SwaggerError>`;
  `cleanup_open_api_doc(doc)` keeps its behavior. `SwaggerError` is introduced here with the variants this change needs
  (`DuplicateSchema`, `RecursiveParameter`, `NonObjectQuery`, `DanglingRef`, `LeftoverMarker`);
  `q-openapi-serving-and-api-hygiene` adds the rest.
- With `strict`: renames are applied to every `$ref` under `paths` (nested `items`, `oneOf` / `anyOf` / `allOf`,
  `additionalProperties`) and in every operation key including `options` and `search`; then the version normalization:
  in 3.0, `oneOf: [X, { type: "null" }]` becomes `X` with `nullable: true` (`allOf: [X]` for a `$ref`); in 3.1,
  `nullable: true` becomes the type-array / `anyOf` form; in both, `search` operations are removed and listed in a
  warning; finally every `$ref` must resolve to a component (`DanglingRef { pointer, target }` otherwise) and no
  `x-corvis_zod-` key may remain (`LeftoverMarker`).
- It also works on documents without zod DTOs (it is then only the normalization).

Verified by: the recorded `forms` and `forms31` documents through strict cleanup: no `oneOf` with null in 3.0, no
`nullable` in 3.1, no `search`, `items.$ref` to `Kitten`; Decision 10's validation; the default path still equals the
fixture byte for byte.

Alternative considered: change the inherited forms by default. Rejected: facten's document would move away from its
original (the parity check compares bytes), and so would any migrated application.

### 8. Homonyms: the original's behavior, a useful warning

Two different Rust types with one component name keep the original's behavior (warning, last definition wins). The
warning (`schema_factory.rs:535`) names both Rust type paths and the fix (`#[api_schema(name = "...")]` on one of them),
and comes from a logger named `SwaggerModule` instead of `"Logger"`. The text that refers to TypeScript decorators goes.
Log text only.

Verified by: a test capturing the log with two `CreateDto` types in different modules.

### 9. Leftover zod markers are reported where the document is served

`SwaggerModule::setup` and `scalar::mount` with `.document(&doc)` check once, at mount time, whether the document holds
any `x-corvis_zod-` key; if so they log a warning naming `cleanup_open_api_doc`. The document served is unchanged.

Verified by: a test mounting an uncleaned document and capturing the warning; facten's cleaned document mounts silently
(the parity run shows no new warning line).

### 10. Valid against the official schemas

`crates/corvis-swagger/tests/fixtures/oas/3.0.json` and `3.1.json`: the OpenAPI Initiative's published JSON Schemas
(Apache-2.0; source URL and retrieval date in a `README.md` next to them; the folder excluded from the package with
`exclude` in `crates/corvis-swagger/Cargo.toml`). `tests/openapi_schema.rs` validates with the dev-dependency
`jsonschema` (default features off, exact version, checked by `cargo deny`): the strict-cleaned `cats`, `forms` and
`forms31` documents and a Rust-types document (generics, chrono, uuid, enums, `Option`, flatten, exclusive bounds) in
3.0 and in 3.1 are valid; the recorded default `forms` document is reported invalid, with the inherited forms as the
reasons (the test documents the trade-off).

Fallback, recorded so nobody decides it again: if `jsonschema` brings a dependency `cargo deny` refuses, the validation
runs in the fixture's Node package with a pinned validator and the Rust test reads its recorded verdicts.

## Security invariants this change comes near

| Invariant | Source | Pinned by |
|---|---|---|
| The document embedded in the Swagger UI and Scalar pages is escaped (`escape_inline_json`), `$js` values are only the developer's | archived swagger work, `scalar.rs:142-150` | Unchanged code; existing `tests/swagger_security.rs` passes unchanged |
| `confined_asset` keeps `custom_swagger_ui_path` reads inside the folder | `swagger_module.rs:305-330` | Not touched here; `swagger_security.rs` |
| A public function does not panic on input a caller passes at run time | house rule | New code returns `SwaggerError`; `schema_name` and the finishing pass are total functions (tests with empty, odd and very long type names) |

No input parsing, limits, authentication, SQL or file system code changes: no security review is required.

## Behavior visible to applications

| What | Before | After | Consequence |
|---|---|---|---|
| Component of `Page<User>` | `Page` (shared, last wins) | `Page_User` | `$ref`s change for generic DTOs; clients regenerated get one type per instantiation |
| Generic `#[derive(ApiSchema)]` | No bound | `T: ApiSchema` per used parameter | A generic DTO over a non-`ApiSchema` type needs `#[api_schema(bound = ..)]` (compile error with the hint) |
| `DateTime`, `NaiveDate`, `Uuid`, sets, `Box`… fields | Empty object component | `string` with format / `array` / the inner type | Documents change for such DTOs; empty components disappear |
| `NaiveDate`, `Uuid`, `NaiveTime` handler parameters | `string` `date-time` / `string` | `string` `date` / `uuid` / `time` | Parameter schemas gain or correct a format |
| Unit enum fields | Empty object | `type: string` + `enum` | Documents change |
| `Option<T>` fields | `required: false` | `required: false` + nullable (version form) | Clients accept `null` |
| `default`, `skip_serializing_if`, `flatten`, `rename(serialize)` | Ignored | Read | Required lists and property names follow serde |
| Float `is_positive` / `is_negative` | `exclusiveMinimum: 0` in 3.0 | `minimum: 0` + `exclusiveMinimum: true` in 3.0; numeric in 3.1 | Valid 3.0 |
| `try_cleanup_open_api_doc` | `(doc, OpenApiVersion) -> Result<_, String>` | `(doc, CleanupOptions) -> Result<_, SwaggerError>` | Callers of the try variant change one argument and the error type |
| Inherited forms (nullable responses, `search`, cleanup renames) | As the original | Unchanged by default; rewritten with `strict` | Opt-in |
| Duplicate component warning, opaque-type warning, leftover-marker warning | Original text / none | Rust paths and fixes | Log text only |
| New features `chrono`, `uuid` (default), `time` | — | Optional dependencies | `default-features = false` removes them |

## Risks / Trade-offs

- **The parity check decides.** facten's document is generated by this code. The design keeps every path facten uses
  (zod DTOs, decorators, `String` parameters, `cleanup_open_api_doc`) unchanged and the finishing pass byte-neutral for
  documents without private keys; task 9.4 runs the corvis-parity check, and any difference in facten or consi is a
  defect of this change, not an accepted divergence.
- New default features add `uuid` to applications that did not have it → small crate, opt-out documented; the
  alternative (silent empty objects) is what the review found.
- The `T: ApiSchema` bound breaks generic DTOs that compiled before over scalars → compile error pointing to `bound`;
  before 1.0, said plainly in the docs.
- The fixture needs Node and network once, to install → like `class_validator_shim`, the JSON is committed and the Rust
  test only reads it.
- `jsonschema` is a new dev-dependency → Decision 10's fallback.

## Open questions

1. `uuid` on by default, or opt-in like `time`? Recommendation: on (the ORM already brings it; silent empty objects are
   worse than a small crate).
2. Strict cleanup drops `search` operations; should it move them to an `x-search` extension instead? Recommendation:
   drop with a warning; an extension no tool reads only hides the route.
3. Should two distinct Rust types with one component name be an error in strict mode? Recommendation: not in this change
   (the cleanup does not see Rust types); the warning names both, and a later issue can add a `create_document` option.
4. Enum as a component (`$ref` to `Role`) by default? Recommendation: inline, like the original without `enumName`.

## Review notes

These settle the open questions and override the design where they differ.

1. `chrono` and `uuid` are default features; `time` is opt-in.
2. Strict cleanup drops `search` operations and warns with their paths.
3. No error for homonyms in this change; the warning names both types.
4. Enums are inline; `enum_name` keeps making a component as today.
5. `cleanup_open_api_doc` keeps panicking on the conditions the original throws on, as a documented convenience over
   `try_cleanup_open_api_doc` (facten calls it); new public functions of this change return `Result`.
6. The fixture of Decision 1 wins over this design's reading of the original: if it records a form different from the
   table above, Implementation notes record it and the default path follows the fixture.
