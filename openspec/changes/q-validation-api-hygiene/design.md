# Design

## Context

Source: the engineering-quality review of the validation library recorded in the issue filed from
`quality/q-validation-api-hygiene.issue.md`. The reviewer read the code only. Every line below was read on `dev` at
`d46758a` (2026-11-11); the two earlier changes of the same review edit `types.rs`, `compound.rs`, `core.rs` and
`common/pipe.rs`, so **task 1.1 re-checks every line after they are merged** and records moved lines in
"Implementation notes".

Who calls the affected API: `corvis-swagger` builds `JsonSchemaOptions` with `.io("input" | "output")`
(`crates/corvis-swagger/src/zod.rs:183-194`) and keeps its own `ZodIo` enum; `tests/zod4_fixtures.rs` sets the option
fields from recorded strings (`613-627`). The port apps build no `JsonSchemaOptions`, call neither `normalize` nor
`hash`, and set no `exception_factory` (read in `corvis-tests`, read-only).

### Findings as checked

| Id | Finding | Code now (`d46758a`) | Verdict |
|---|---|---|---|
| S6a | JSON Schema options are strings | `JsonSchemaOptions` (`json_schema.rs:133-147`), setters (`163-194`), comparisons at `263-274`, `393`, `417`, `542`, and in the nodes (`types.rs:1669,2407,2615`, `compound.rs:455,706`, `wrappers.rs:173`) | Confirmed: `io("Input")` is output, an unknown `target` takes the default branches. Decision 1 |
| S6b | `z::hash` panics on an unknown algorithm | `types.rs:3029-3051` (`panic!("Unrecognized hash format")`) | Confirmed. The original throws when the schema is built; a typed argument removes the case. Decision 2 |
| S6c | `normalize("NFX")` silently normalizes to NFC | `types.rs:882-884`, `js_normalize` (`678-686`, `_ => nfc`) | Confirmed. The original's `String.prototype.normalize` throws a `RangeError` at parse time; a typed argument removes the case. Decision 2 |
| S6d | `def_type() == "never"` / `"transform"` compare strings | `compound.rs:1029`, `wrappers.rs:1122` | **Dropped.** `def_type` is the public trait method that mirrors zod's `def.type` string; custom types return their own (`"custom"` by default). An enum would need an open variant and changes nothing observable |
| S6e | `RawIssue::code()` parses the string on every call | `core.rs:420-422`, called by `finalize` (`533`) and the intersection loop (`compound.rs:1561`) | Confirmed (failure path). Decision 4 |
| S6f | `ZRecord { loose, partial }` admits both | `compound.rs:509-515` | **Dropped.** Both fields are `pub(crate)` and each constructor sets one (`1648-1659`): the combination cannot be built outside the crate |
| N5 | A warning per regex test that hits the backtracking limit | `regex.rs:57-72` | Confirmed: a client that sends inputs which exhaust the limit writes one line per request. Decision 5 |
| N6a | Comment contradicts the code | `compound.rs:1531-1532` ("use passthrough semantics") above code that runs each side with its own unknown-key policy | Confirmed. Decision 6 |
| N6b | `#[allow(dead_code)]` hides an unread field | `BigKind::Format(&'static str, i128, i128)` (`types.rs:1850-1852`); the name is never read (`1988`, `3135`) | Confirmed. Decision 6 |
| N6c | Unreachable branch | `DefaultValuePipe` `NaN` branch (`common/pipe.rs:380`): a `serde_json::Number` is never NaN | Confirmed. Decision 6 |
| N8a | Built-in pipes with fewer options | `ParseEnumPipe` (`common/pipe.rs:328-347`) and `ParseArrayPipe` (`255-286`) have no `error_http_status_code`; every other failing pipe has it (`pipe_options!`, `84-107`; `ParseUUIDPipe` `199-203`) | Confirmed. The original's options for both pipes are checked in the fixture package of `q-validation-correctness` (task 3.2). Decision 7 |
| N8b | `exception_factory` is a `fn` pointer | `validation/mod.rs:114`, `182-185` | Confirmed: it cannot capture configuration; `ZodValidationPipeOptions` uses `Arc<dyn Fn>` (`z/pipe.rs:86-103`). Decision 7 |
| N8c | `ZodSchema::to_json_schema` hides the error | `mod.rs:734-736` (`unwrap_or(Value::Null)`) while `z::to_json_schema` returns `Result<Value, String>` (`json_schema.rs:578`) | Confirmed. Decision 3 |
| S2c (identity part) | A concrete schema used twice is two schemas for the registry | `ZodSchema::register` (`mod.rs:896-901`) returns the registered handle; using the original value again makes a new `Arc` | Narrowed to documentation. Decision 6 |

## Goals / Non-Goals

**Goals:**

- Closed sets of values are enums; a typo does not compile, and configuration text is parsed with an error.
- No public builder panics on an argument a typed API can exclude.
- Options with the same meaning have the same shape across the pipes.
- Log lines a client can provoke are rate limited.

**Non-Goals:**

- Behavior changes of validation or JSON Schema output for valid options (pinned by the fixtures).
- New options (`exception_factory` on every built-in pipe).

## Decisions

### 1. Typed JSON Schema options

- `pub enum JsonSchemaTarget { Draft2020_12, Draft07, Draft04, OpenApi3_0 }` (default `Draft2020_12`),
  `pub enum JsonSchemaIo { Output, Input }`, `pub enum Unrepresentable { Throw, Any }`, `pub enum Cycles { Ref, Throw }`,
  `pub enum Reused { Inline, Ref }`, each `#[non_exhaustive]`, `Copy`, `Default` and with `FromStr` accepting the
  original's strings (`"draft-2020-12"`, `"draft-07"` and `"draft-7"`, `"draft-04"` and `"draft-4"`, `"openapi-3.0"`;
  `"output"`, `"input"`; ...). An unknown string is an error that names the accepted values.
- `JsonSchemaOptions` fields take the enums; the setters keep their names and take the enum (`.target(JsonSchemaTarget::
  Draft07)`); `input()` stays. `JsonSchemaGen::target()` returns `JsonSchemaTarget`; `io_input()` and `legacy_target()`
  keep their names. Every comparison in the nodes uses the enum.
- `corvis-swagger` maps its `ZodIo` to `JsonSchemaIo` (or re-exports the new enum as its own; task 2.1 picks the smaller
  change and records it). The fixture runner parses the recorded strings with `FromStr`.

Verification: `zod4_json_schema_fixtures` passes unchanged (same output for every recorded option); unit tests of
`FromStr` (accepted aliases, an unknown value's error text); `cargo test -p corvis-swagger`.

Alternative considered: keep `String` and validate in the setters. Rejected: the fields are public, so a struct literal
or a field assignment would bypass the check.

### 2. Typed `normalize` and `hash` arguments

- `pub enum NormalizationForm { Nfc, Nfd, Nfkc, Nfkd }` (default `Nfc`); `ZString::normalize(NormalizationForm)`; the
  empty-string default goes. `js_normalize` matches the enum.
- `pub enum HashAlgorithm { Md5, Sha1, Sha256, Sha384, Sha512 }`, `pub enum HashEncoding { Hex, Base64, Base64Url }`
  (default `Hex`); `z::hash(HashAlgorithm, HashEncoding)` cannot panic; the format name (`"md5_hex"`, ...) is unchanged.

Verification: the `string.normalize` and hash fixture cases, unchanged; a unit test per enum value of each (the format
name and the regex chosen).

### 3. `ZodSchema::to_json_schema()` returns `Result<Value, String>`

It returns what `z::to_json_schema(&self.to_schema(), &JsonSchemaOptions::default())` returns. Test: a `z.bigint()`
schema gives `Err` with the unrepresentable message; `z.string()` gives the recorded schema.

### 4. `RawIssue` stores its code

`RawIssue` gains a `code: ZodIssueCode` field set by every constructor (`RawIssue::new` parses the `"code"` field once;
the struct literals in `mod.rs` and `core.rs` set it); `code()` returns it. The `"code"` entry stays in `fields`, which
fixes the key order. A unit test: a raw issue built through each constructor reports the same code as its `"code"` field.

### 5. The backtracking warning is rate limited

`JsRegex::test` writes its warning through a `static` `auth::warn_limit::WarnLimiter` keyed by the pattern's source (a
schema string, never client data): at most one line per pattern per minute, carrying the count of lines left out and
the input's length; the limiter's cap of 64 keys bounds its table (patterns beyond it are not logged). If
`e3-authentication-hot-path-and-consolidation` has merged, its `line` helper builds the message only when it is written.
Test: 100 tests of a pattern that exhausts the limit write one line (a test logger), and a second pattern writes its
own.

### 6. Comments, dead code, documentation

- `compound.rs:1531-1532`: the comment says that each side runs with its own unknown-key policy on the full input and
  that the merge keeps the keys each side kept.
- `BigKind::Format` drops the unused name (`Format(i128, i128)`); the `#[allow(dead_code)]` goes.
- `DefaultValuePipe`: the `NaN` branch is removed and a comment says why the original's NaN case cannot occur (a JSON
  number is never NaN).
- `ZodSchema::register` rustdoc: the returned `Schema` is the registered one; reuse it (a concrete type used again is
  another schema for the registry and for `reused: "ref"`).

### 7. Pipe options with the same shape

- `ValidationPipeOptions::exception_factory: Option<Arc<dyn Fn(Vec<ValidationError>) -> HttpException + Send + Sync>>`;
  `ValidationPipe::exception_factory(f: impl Fn(..) + Send + Sync + 'static)`. A function item still compiles with the
  builder. Test: a factory that captures a status code chosen at run time.
- `ParseEnumPipe::error_http_status_code(u16)` and `ParseArrayPipe::error_http_status_code(u16)`, with the behavior of the
  other pipes (an invalid code falls back to 400). Task 3.2 first reads the original's option types in the fixture package
  of `q-validation-correctness` (`node_modules` of `tests/fixtures/builtin_pipes/`) and records that both pipes accept
  `errorHttpStatusCode`; a fixture case per pipe with a custom status is added to `generate.mjs` and replayed.

## Security invariants this change comes near

| Invariant | Decisions near it | Pinned by |
|---|---|---|
| Log lines a client can provoke are rate limited, keyed by static strings, never carrying client data; the table is capped | 5 | the new `regex.rs` test; `auth::warn_limit` tests |
| The backtracking engine is bounded and answers no match at the limit | 5 | `regex.rs` tests unchanged |
| JSON Schema output for every recorded option is unchanged | 1 | `zod4_json_schema_fixtures` |
| The pipes answer the recorded bodies | 7 | `builtin_pipes_fixtures` (with the new status cases) |

## Behavior visible to applications

| What | Before | After | Consequence |
|---|---|---|---|
| `JsonSchemaOptions` fields and setters | `String` | Enums (`FromStr` for text) | Breaking at compile time; an unknown value cannot be set |
| `ZString::normalize`, `z::hash` | `&str` arguments; unknown form silently NFC; unknown hash panics | Enum arguments | Breaking at compile time; no panic, no silent fallback |
| `ZodSchema::to_json_schema()` | `Value::Null` on error | `Result<Value, String>` | Breaking at compile time; the reason is visible |
| `ValidationPipeOptions::exception_factory` | `fn` pointer | `Arc<dyn Fn>` | Builder callers unaffected; direct field assignment wraps in `Arc::new` |
| `ParseEnumPipe`, `ParseArrayPipe` | Always 400 | Configurable status | Additive |
| Backtracking warning | One line per input | One line per pattern per minute with a count | Less log volume |

## Risks / Trade-offs

- Breaking changes for users of these APIs → all are compile-time, listed in the docs pages, made before 1.0.
- `corvis-swagger` changes with the enum → its tests run in the definition of done.
- The limiter drops lines of patterns beyond its 64 keys → acceptable: patterns are schema strings, and 64 distinct
  patterns hitting the limit at once is an application bug the first lines already show.

## Open questions

1. Keep the setters' names (`.target(..)`) or rename them after the enums?
2. Re-export the JSON Schema enums at `corvis::z` or keep them in a `json_schema` path?
3. Add `exception_factory` to every built-in pipe now?

## Review notes

These settle the open questions and override the design where they differ.

1. The setters keep their names, mirroring the original's option names.
2. Re-exported at `corvis::z`, next to `JsonSchemaOptions`.
3. No: it adds API beyond the finding; a follow-up if the maintainer wants it.
4. Starts after `q-validation-hot-path` is merged; task 1.1 is mandatory.
5. The security review covers the limiter (keys, cap, message content) and the pipe option changes.
