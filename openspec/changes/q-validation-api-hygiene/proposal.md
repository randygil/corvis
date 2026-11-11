# Proposal

Implements the issue filed from `quality/q-validation-api-hygiene.issue.md` (epic
[#187](https://github.com/randygil/corvis/issues/187), code quality audit). Source: the same engineering-quality review of
the validation library as `q-validation-correctness` and `q-validation-hot-path`, read against `dev` at `d46758a`. Third
of the three changes; it changes public API, so it lands before 1.0.

## Why

Public API of the validation library accepts strings and booleans where a closed set of values is meant, and fails
silently or by panicking on a typo:

- `JsonSchemaOptions` takes `target`, `io`, `unrepresentable`, `cycles` and `reused` as `String`: `io("Input")` is
  silently treated as output, and an unknown target as the default.
- `z::hash("sha265", None)` panics; `z::string().normalize("NFX")` silently normalizes to NFC, where the original throws.
- `ZodSchema::to_json_schema()` returns `Value::Null` when the schema cannot be represented, hiding the reason.
- `ValidationPipeOptions::exception_factory` is a plain `fn` pointer, so it cannot capture anything, unlike
  `ZodValidationPipeOptions::create_validation_exception`; `ParseEnumPipe` and `ParseArrayPipe` have no
  `error_http_status_code`, unlike the other built-in pipes.
- `JsRegex::test` writes a warning on every input that hits the backtracking limit: a client can flood the log.
- A comment contradicts the code it describes, a field is never read, a branch is unreachable, and `RawIssue::code()`
  parses the code string on every call.

## What Changes

- **Typed JSON Schema options**: `JsonSchemaTarget`, `JsonSchemaIo`, `Unrepresentable`, `Cycles`, `Reused` enums, each
  with `FromStr` for configuration text (an unknown value is an error naming the accepted ones); `JsonSchemaGen::target()`
  returns the enum; the internal string comparisons go.
- **Typed string arguments**: `normalize(NormalizationForm)`; `hash(HashAlgorithm, HashEncoding)`, which cannot panic.
- **`ZodSchema::to_json_schema()` returns `Result<Value, String>`**, like `z::to_json_schema`.
- **Pipe options**: `ValidationPipeOptions::exception_factory` and `ValidationPipe::exception_factory` take any
  `Fn(Vec<ValidationError>) -> HttpException + Send + Sync` (stored as an `Arc`); `ParseEnumPipe` and `ParseArrayPipe`
  gain `error_http_status_code`.
- **Rate-limited regex warning**: one line per pattern per minute, with the count of lines left out.
- **Small fixes**: the intersection comment; the unused name of the bigint formats removed; the unreachable `NaN` branch
  of `DefaultValuePipe` removed with a comment; `RawIssue` stores its code; the rustdoc of `register()` says that the
  returned handle is the registered one.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `validation/zod`: added requirements for typed JSON Schema options, typed `normalize` and `hash` arguments, a
  `to_json_schema` that reports its error, and a rate-limited backtracking warning.
- `core/validation-pipe`: added requirement that the exception factory accepts closures.
- `core/builtin-pipes` (created by `q-validation-correctness`): added requirement that every built-in pipe that can fail
  accepts `error_http_status_code`.

## Impact

- Edited: `crates/corvis/src/z/{json_schema,types,compound,wrappers,core,mod,regex}.rs`,
  `crates/corvis/src/common/pipe.rs`, `crates/corvis/src/validation/mod.rs`, `crates/corvis-swagger/src/zod.rs`,
  `crates/corvis/tests/zod4_fixtures.rs` (the JSON Schema runner maps the recorded strings with `FromStr`),
  `docs/application/validation.md`, `docs/pipes.md` and their `docs/es/` translations.
- Public API, changed now, before 1.0: the five `JsonSchemaOptions` fields and setters take enums (setters keep their
  names); `JsonSchemaGen::target()` returns `JsonSchemaTarget`; `ZString::normalize` and `z::hash` take enums;
  `ZodSchema::to_json_schema` returns a `Result`; `ValidationPipeOptions::exception_factory` changes type (code that
  passed a function still compiles with `ValidationPipe::exception_factory`; code that set the field directly wraps the
  function in `Arc::new`). Added: `ParseEnumPipe::error_http_status_code`, `ParseArrayPipe::error_http_status_code`.
- Behavior visible to applications: a JSON Schema with an unknown option no longer silently uses the default (it cannot
  be written); a backtracking regex logs at most one line a minute per pattern. Listed in design.md.
- No new crate dependency. Framework runtime code changes, so the corvis-parity check runs.

## Out of scope

- `def_type()` returning `&'static str` and the string comparisons on it (design.md, dropped).
- `exception_factory` on the other built-in pipes (an addition, not a consistency fix): a possible follow-up.
- Everything of the first two changes.
