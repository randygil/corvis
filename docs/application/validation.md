---
status: documented
---

# Validation

All data entering through HTTP is suspect until it is checked. Corvis offers two ways to validate a handler's arguments, and they can coexist in the same application:

- **DTOs with attributes** (`#[derive(Validate)]` + `ValidationPipe`): declarative rules on each field, with class-validator's messages.
- **Schemas** (`corvis::z` + `ZodValidationPipe`): a port of zod 4 that validates, transforms and applies default values.

Both are [pipes](/pipes): they receive the argument's JSON value before it is deserialized into the parameter's type and can reject it with a `400`.

## DTOs with `#[derive(Validate)]`

```rust
use corvis::Validate;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Validate)]
pub struct CreateCatDto {
    #[is_string]
    #[is_not_empty]
    #[max_length(50)]
    pub name: String,
    #[is_int]
    #[min(0)]
    #[max(40)]
    pub age: u32,
    #[is_string]
    #[is_in("persian", "siamese", "maine coon", "mixed")]
    pub breed: String,
}
```

(Taken from `examples/cats-app/src/cats/dto/mod.rs`.) Enable validation with a global pipe in `main`:

```rust
app.use_global_pipes(ValidationPipe::new().whitelist(true).forbid_non_whitelisted(true));
```

With that, any `#[body]`, `#[query]` or `#[param]` whose type derives `Validate` is checked before reaching the handler. If it fails, the response is:

```json
{ "message": ["name should not be empty", "age must not be less than 0"], "error": "Bad Request", "statusCode": 400 }
```

Types that do not derive `Validate` pass through unchanged.

### Available rules

| Group | Attributes |
|---|---|
| Type | `is_string`, `is_int`, `is_number`, `is_boolean`, `is_array`, `is_object`, `is_enum` |
| Presence | `is_defined`, `is_optional`, `is_not_empty`, `is_empty` |
| Text format | `is_email`, `is_url`, `is_uuid`, `is_alpha`, `is_alphanumeric`, `is_number_string` (alias `is_numeric_string`), `is_date_string` (alias `is_iso8601`), `is_hex_color`, `is_jwt`, `is_mongo_id`, `is_ip`, `is_lowercase`, `is_uppercase`, `matches("regex")` |
| Length | `min_length(n)`, `max_length(n)`, `length(min, max)` |
| Numbers | `min(n)`, `max(n)`, `is_positive`, `is_negative` |
| Content | `contains("s")`, `not_contains("s")`, `is_in(a, b, ...)`, `is_not_in(a, b, ...)`, `equals(v)`, `not_equals(v)` |
| Arrays | `array_min_size(n)`, `array_max_size(n)`, `array_not_empty`, `array_unique` |
| Composition | `validate_nested`, `validate_if(f)`, `validate_with(f)` |

Useful details:

- **Custom message**: any rule accepts `message = "..."`, for example `#[min_length(3, message = "too short")]`.
- **Each element**: `each` applies the rule to each element of an array (`#[max_length(20, each)]`); the message begins with `each value in ...`.
- **Optionals**: `#[is_optional]` skips the other rules if the field is missing or `null`. Use it with `Option<T>` fields.
- **Implicit rules**: a field without attributes is checked according to its Rust type (text, integer, number, boolean).
- **Nested**: `#[validate_nested]` validates a field whose type also derives `Validate` (or each element of a `Vec` of that type).
- **Condition**: `#[validate_if(|obj| ...)]` receives the full JSON object (`&serde_json::Map<String, Value>`) and only validates the field if it returns `true`.
- **Free-form rule**: `#[validate_with(f)]` receives the field's value (`Option<&Value>`) and returns `bool`; the default message is `<field> is invalid`.
- **Enums**: `#[is_enum]` checks that the value is one of the variants of an enum that implements `Deserialize` (the field's own or the one given: `#[is_enum(Role)]`).
- Field names in messages respect `#[serde(rename = "...")]` and `#[serde(rename_all = "...")]`.

### Pipe options

`ValidationPipe::new()` is configured with chained methods (or with `ValidationPipe::with_options(ValidationPipeOptions { .. })`):

| Method | Effect |
|---|---|
| `whitelist(true)` | Removes the properties that the DTO does not declare |
| `forbid_non_whitelisted(true)` | Instead of removing them, responds with an error |
| `skip_missing_properties(true)` | Does not validate missing or `null` properties |
| `stop_at_first_error(true)` | Stops validating a property after its first error |
| `disable_error_messages(true)` | Responds without the list of messages |
| `error_http_status_code(422)` | Changes the status code (`400` by default) |
| `exception_factory(f)` | `fn(Vec<ValidationError>) -> HttpException` to build your own response |
| `transform(bool)` | Accepted for compatibility; in Corvis the conversion to the declared type always happens |

In `#[query]` and `#[param]` the pipe first converts numeric and boolean text according to the field's type (`"2"` becomes `2` in a `u32`). In `#[body]` there is no conversion: a JSON with `"age": "2"` fails on `#[is_int]`.

To validate a DTO already built outside a handler, use `corvis::validation::validate(&dto)`, which returns the list of `ValidationError`.

## Schemas with `corvis::z`

`corvis::z` is a port of [zod](https://zod.dev) **4.4.3** and of its usual integration with DTOs, pipes and serialization in Node.js frameworks (version **5.5.0**). It produces the same error codes, messages (`en` locale), key order, check order and JSON shape as zod 4, and `ZodValidationPipe` responds with the same error bodies as that integration. The zod 3 behavior is still available with `z::set_version(z::ZodVersion::V3)` (global), `z::with_version(..)` (current thread) or `ParseOptions::v3()` (a single call).

It reproduces the zod 4 API with Rust functions. A DTO is associated with its schema with the `zod_dto!` macro:

```rust
use corvis::prelude::*;
use corvis::zod_dto;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRoomDto {
    pub name: String,
    pub topic: String,
    pub max_members: i64,
}

zod_dto!(
    CreateRoomDto,
    z::strict_object! {
        name: z::string().min(1).max(24).regex("^[a-z0-9][a-z0-9-]*$"),
        topic: z::string().trim().max(120).default(""),
        maxMembers: z::int().min(1.0).max(500.0).default(50),
    }
);
```

(Adapted from `examples/tertulia/src/rooms/dto.rs`.) With `ZodValidationPipe::auto()` as a global pipe, every parameter whose type has a schema is validated, and what is deserialized into the struct is the schema's **output**: with `default`s applied, text trimmed and unknown keys removed or rejected depending on the object type.

```rust
app.use_global_pipes(ZodValidationPipe::auto());
```

An error responds like this:

```json
{
  "statusCode": 400,
  "message": "Validation failed",
  "errors": [
    { "origin": "string", "code": "too_small", "minimum": 1, "inclusive": true,
      "path": ["name"], "message": "Too small: expected string to have >=1 characters" }
  ]
}
```

A missing body, query, parameter or header is validated as `undefined`, as in Express. `ZodValidationPipe` and `ValidationPipe` can coexist: each ignores the types that do not belong to it.

Other ways to use the pipe:

- `ZodValidationPipe::new(schema)` validates a specific argument with an explicit schema, for example for query strings: `#[query(ZodValidationPipe::new(z::object! { page: z::coerce::number().int().min(1.0).default(1) }))] q: ListQuery`.
- `ZodValidationPipe::dto::<Dto>()` uses another DTO's schema.
- `z::create_zod_validation_pipe(ZodValidationPipeOptions::new().create_validation_exception(|e| ...).strict_schema_declaration(true))` customizes the exception and, with `strict_schema_declaration`, rejects (500) parameters whose type has no schema.

### Query strings and `coerce`

Query values arrive as text. Use `z::coerce::number()` to convert them before validating. Be careful with `z::coerce::boolean()`: it follows the JavaScript rule (`Boolean(x)`), so `"false"` gives `true`. For text flags use `z::stringbool()`.

### Validating without a pipe

A schema can be used directly:

```rust
let schema = z::object! { name: z::string().min(2), email: z::email() };
let result = schema.safe_parse(&json!({ "name": "A" }));
if let Some(error) = result.error {
    let flat = z::flatten_error(&error); // { formErrors, fieldErrors }
}
```

`parse`, `safe_parse`, `parse_as::<T>` (validates and deserializes), `flatten_error`, `treeify_error` and `prettify_error` are available. `format_error`, `error.to_json()` and `error.message()` are also available. The full catalog of types and methods follows the zod 4 API; the equivalence tables are below.

### Messages in Spanish

zod messages use the `en` locale by default. `corvis::z` also includes `es`:

```rust
z::configure(|c| c.locale_error = Some(z::locales::es()));
```

### zod 4 equivalence

| zod 4 | `corvis::z` |
|---|---|
| `z.string().min(3, "msg")`, `.min(3, { error: (iss) => ... })` | `z::string().min(3).message("msg")`, `.min(3).error(z::error_fn(\|iss\| Some(..)))` |
| `z.string({ error: "msg" })` | `z::string().error("msg")` (without checks, applies to the schema) |
| `.max() .length() .nonempty() .regex(/re/i) .startsWith() .endsWith() .includes(s, { position })` | `.max() .length() .nonempty() .regex(("re", "i")) .starts_with() .ends_with() .includes_at(s, pos)` |
| `.trim() .toLowerCase() .toUpperCase() .normalize() .slugify() .lowercase() .uppercase()` | `.trim() .to_lower_case() .to_upper_case() .normalize("NFC") .slugify() .lowercase() .uppercase()` |
| `z.email()`, `z.email({ pattern: z.regexes.html5Email })` | `z::email()`, `z::email_with(z::regexes::HTML5_EMAIL.clone())` |
| `z.uuid() z.uuidv4() z.guid() z.url({ hostname, protocol, normalize }) z.httpUrl()` | `z::uuid() z::uuidv4() z::guid() z::url_with(UrlOptions {..}) z::http_url()` |
| `z.cuid() z.cuid2() z.ulid() z.xid() z.ksuid() z.nanoid() z.emoji()` | the same |
| `z.ipv4() z.ipv6() z.cidrv4() z.cidrv6() z.mac({ delimiter }) z.hostname()` | `z::ipv4() ... z::mac(Some("-")) z::hostname()` |
| `z.base64() z.base64url() z.hex() z.hash("sha256") z.jwt({ alg }) z.e164()` | `z::base64() ... z::hash("sha256", None) z::jwt(Some("HS256")) z::e164()` |
| `z.iso.datetime({ offset, local, precision }) z.iso.date() z.iso.time() z.iso.duration()` | `z::iso::datetime_with(IsoDatetime {..})`, `z::iso::date()`, `z::iso::time_with(p)`, ... (`z::time_precision::MILLISECOND`) |
| `z.stringFormat("code", /re/ \| fn)` | `z::string_format("code", "re")`, `z::string_format_fn("code", \|s\| ..)` |
| `z.number().int().gt().gte().lt().lte().positive().multipleOf().step()` | the same (`gt(0.0)`, ...); `z::int()`, `z::int32()`, `z::uint32()`, `z::float32()`, `z::float64()` |
| `z.bigint() z.int64() z.uint64()`, `z.boolean()`, `z.null()`, `z.undefined()`, `z.void()`, `z.nan()` | the same |
| `z.any() z.unknown() z.never() z.date()` | the same (`z::date()` receives the dates from `z::coerce::date()` or from a transform; with `ParseOptions::date_strings(true)` it also accepts ISO strings) |
| `z.literal("a")`, `z.literal(["a", 1, null])` | `z::literal("a")`, `z::literal(json!(["a", 1, null]))` |
| `z.enum(["a", "b"])`, `z.enum({ A: "x" })`, `z.nativeEnum(E)`, `.extract() .exclude()` | `z::enum_(["a", "b"])`, `z::enum_entries([("A", json!("x"))])`, `z::native_enum::<E>()`, `.extract(&[..]) .exclude(&[..])` |
| `z.stringbool({ truthy, falsy, case })` | `z::stringbool()`, `z::stringbool_with(StringBoolOptions {..})` |
| `z.templateLiteral(["id-", z.number()])` | `z::template_literal([TemplatePart::from("id-"), z::number().into()])` |
| `z.object({...})`, `z.strictObject`, `z.looseObject` | `z::object! { field: s, "with-hyphen": s }`, `z::strict_object! {..}`, `z::loose_object! {..}` |
| `.strict() .strip() .passthrough() .loose() .catchall(s)` | the same |
| `.extend() .safeExtend() .merge() .pick({ a: true }) .omit() .partial() .required() .keyof() .shape` | `.extend(o) .safe_extend(o) .merge(o) .pick(&["a"]) .omit(&[..]) .partial() .partial_fields(&[..]) .required() .required_fields(&[..]) .keyof() .shape()` |
| `z.array(s).min().max().length().nonempty()`, `s.array()` | the same |
| `z.tuple([a, b], rest)` | `z::tuple([a, b]).rest(rest)` |
| `z.record(k, v)`, `z.partialRecord(k, v)`, `z.looseRecord(k, v)` | `z::record_with(k, v)`, `z::partial_record(k, v)`, `z::loose_record(k, v)` (`z::record(v)` uses string keys) |
| `z.union([..])`, `z.xor([..])`, `z.discriminatedUnion("k", [..])`, `z.intersection(a, b)` | `z::union([..])`, `z::xor([..])`, `z::discriminated_union("k", [..])`, `z::intersection(a, b)`; `a.or(b)`, `a.and(b)` |
| `.optional() .exactOptional() .nullable() .nullish() .nonoptional() .readonly() .brand()` | the same (`.exact_optional()`) |
| `.default(v) .default(() => v) .prefault(v) .catch(v) .catch((ctx) => v)` | `.default(v) .default_fn(\|\| v) .prefault(v) .catch(v) .catch_fn(\|ctx\| v)` |
| `.refine(fn, { message, path, abort, when, params })` | `.refine(fn, "msg")`, `.refine(fn, z::params().message(..).path([..]).abort().when(..).params(json!(..)))`, `.refine_at(fn, msg, [..])` |
| `.superRefine((v, ctx) => ctx.addIssue({...}))`, `.check((ctx) => ...)`, `.overwrite(fn)` | `.super_refine(\|v, ctx\| ctx.add(z::issue().code(..).message(..).path([..])))`, `.check_fn(\|payload\| ..)`, `.overwrite(\|v\| ..)` |
| `.transform(fn)`, `.transform((v, ctx) => { ctx.addIssue(..); return z.NEVER })` | `.transform(\|v\| ..)`, `.try_transform(\|v, ctx\| { ctx.add(..); None })` |
| `.pipe(s)`, `z.preprocess(fn, s)`, `z.codec(a, b, { decode, encode })`, `z.invertCodec(c)` | the same (`z::invert_codec(&c)`); `schema.decode(&v)`, `schema.encode(&v)` |
| `z.lazy(() => s)`, recursive getters, `z.custom(fn)`, `z.json()`, `z.success(s)` | `z::lazy(f)`, `z::custom(fn, msg)`, `z::json()`, `z::success(s)` |
| `z.coerce.number()/string()/boolean()/bigint()/date()` | `z::coerce::number()/...` (`boolean` is `Boolean(x)`: `"false"` gives `true`) |
| `schema.parse / safeParse / parse(v, { reportInput, error })` | `parse`, `safe_parse`, `parse_with(&v, &ParseOptions::new().report_input(true).error(..))`, `parse_as::<T>`, `parse_str`, `is_valid`; with `undefined`: `parse_opt(None, ..)`, `safe_parse_opt(None, ..)` |
| `z.flattenError / treeifyError / prettifyError / formatError`, `error.issues`, `error.message` | `z::flatten_error(&e)` (or `e.flatten()`), `z::treeify_error`, `z::prettify_error`, `z::format_error`, `e.issues`, `e.message()` |
| `z.config({ customError })`, `z.config(z.locales.es())` | `z::configure(\|c\| c.custom_error = Some(..))`, `z::configure(\|c\| c.locale_error = Some(z::locales::es()))` |
| `.describe()`, `.meta({ id, title })`, `z.registry()`, `z.globalRegistry`, `.register(r, meta)` | the same (`z::Registry::new()`, `z::global_registry()`) |
| `z.toJSONSchema(s, { target, io, unrepresentable, cycles, reused, override })` | `z::to_json_schema(&s, &JsonSchemaOptions::new().target("draft-07").input()...)` or `s.to_json_schema()` |

### Differences specific to Rust

- Values are `serde_json::Value`. `undefined` is the absence of a value (`parse_opt(None, ..)` or a missing key), and an `undefined` output is omitted in objects, like `JSON.stringify`. Numbers follow JavaScript semantics (`f64`, `Number(x)`, `String(x)`) and object keys are ordered as in V8 (numeric indexes first).
- `.refine()`, `.super_refine()`, `.overwrite()` and `.check_fn()` return a `Schema`, so the type's own checks (`.min()`, ...) go before them. zod also accepts `z.string().refine(..).min(3)`; that order cannot be written in Rust.
- What zod *throws* as an `Error` (an intersection that cannot be merged, a `transform` when doing `encode`) is a `panic!` in Rust. `ZodValidationPipe` and the serializer turn it into the same response as the original integration.
- Recursive schemas: a function that returns the schema and `z::lazy(f)` where it repeats (`fn category() -> Schema { z::object! { children: z::array(z::lazy(category)) }.into() }`). For `to_json_schema` to produce `"$ref": "#"` as zod does, use `z::lazy(category)` as the root.
- `z.map`, `z.set`, `z.symbol`, `z.file`, `z.promise`, `z.function` and `z.instanceof` do not exist because they describe values that do not travel in JSON. Of the locales only `en` (default) and `es` exist; add other languages with `locale_error` and `z::error_fn`.
- Depth of recursive schemas (`z.lazy`, `z.json()`): at most 128 levels per parse. See [Security limits](#security-limits).

### Behaviors inherited from zod

These come from zod and from its Node.js integration, and are worth keeping in mind:

- Guards and interceptors see the **unvalidated** body (see [What guards and interceptors see](#what-guards-and-interceptors-see)).
- A parameter whose type has no schema (`zod_dto!`) passes through unvalidated with `ZodValidationPipe::auto()`.
- `.strict()` does not report the `__proto__` key: it silently drops it, like zod. With duplicate keys in the JSON the last one wins.
- `z::coerce::number()` converts `""` and `null` into `0`; `z::coerce::boolean()` is `Boolean(x)`, so `"false"` gives `true` (use `z::stringbool()` for text).
- `z::jwt()` only checks the shape of the token; it does not verify the signature.
- The only known divergence: a number that does not fit in an `f64` (`1e309`) arrives as `null` (JSON has no `Infinity`), so `z::number().nullable()` accepts it; in Node it would be `Infinity` and zod would reject it.

### Equivalence with the Node.js integration

| Node.js (zod integration 5) | Corvis |
|---|---|
| `class Dto extends createZodDto(schema)`, `{ codec: true }`, `Dto.create(input)` | `zod_dto!(Dto, schema)`, `zod_dto!(Dto, schema, codec = true)`, `Dto::create(&input)` |
| `new ZodValidationPipe(schemaOrDto)` | `ZodValidationPipe::new(schema)`, `ZodValidationPipe::dto::<Dto>()` |
| `{ provide: APP_PIPE, useClass: ZodValidationPipe }` | `app.use_global_pipes(ZodValidationPipe::auto())` |
| `createZodValidationPipe({ createValidationException, strictSchemaDeclaration })` | `z::create_zod_validation_pipe(ZodValidationPipeOptions::new().create_validation_exception(\|e\| ..).strict_schema_declaration(true))` |
| `ZodValidationException`, `getZodError()` | `z::ZodValidationException::new(e)`, `ZodValidationException::get_zod_error(&ex)` |
| `ZodSerializationException`, `ZodSchemaDeclarationException` | the same (`z::...::new(..)`) |
| `@ZodSerializerDto(Dto)` / `([Dto])` + `ZodSerializerInterceptor` | `#[set_metadata(z::ZOD_SERIALIZER_DTO_OPTIONS, z::ZodSerializerDto::dto::<Dto>())]` (`dto_array`, `schema`, `schema_array`) + `app.use_global_interceptors(z::ZodSerializerInterceptor::new())` (see [Serialization](/application/serialization)) |
| `createZodSerializerInterceptor({ reportInput })` | `z::ZodSerializerInterceptor::create(Some(true))` |
| `@ZodResponse({ status, type })` | `#[apply_decorators(corvis_swagger::decorators::zod_response::<Dto>(Some(status), None))]`: status code + `ZodSerializerDto` + the OpenAPI response (`Dto_Output`) |
| `ZodGuard`, `UseZodGuard('body', Dto)`, `createZodGuard({ createValidationException })` | `#[use_guards(z::ZodGuard::dto::<Dto>(z::ZodGuardSource::Body))]`, `.create_validation_exception(..)` |
| `validate(value, schemaOrDto, createValidationException?)` | `z::validate(&value, &schema, None)` |

### How fidelity is tested

Fidelity is checked against the real packages: `tests/fixtures/zod4/cases.mjs` runs zod 4.4.3 (and its `zod/v3` build) over almost 300 schemas with about 600 inputs and almost 100 `toJSONSchema` cases, and `tests/fixtures/zod4/pipe.mjs` runs the original 5.5.0 integration. `tests/zod4_fixtures.rs` and `tests/zod_pipe.rs` require the same result, including key order.

## What guards and interceptors see

Pipes validate the handler's argument; they do not replace `req.body()`. A guard or an interceptor that reads the body sees the **unvalidated** value. If a guard needs already validated data, validate there or move the logic to the handler.

## Security limits

- **Depth of recursive schemas.** `z::lazy` and `z::json()` allow at most 128 levels of nesting per parse. Past that limit the parse stops recursing and returns a `custom` issue with the message `Too deeply nested: the input exceeds the maximum depth of 128` at the path of the first value that exceeds it (without `input`, to avoid copying the subtree). zod has no such limit: in Node.js the parse throws `RangeError: Maximum call stack size exceeded`; in Rust a stack overflow aborts the whole process, so the limit is mandatory. Unions do not wrap it in `invalid_union`, so it reaches the `400` of `ZodValidationPipe` as is. Change it with `z::configure(|c| c.max_depth = n)` (global, `z::DEFAULT_MAX_DEPTH` = 128) or per parse with `ParseOptions::new().max_depth(n)`. 128 levels leave plenty of margin: a typical recursive tree needs about 670 levels to exhaust the 2 MiB stack of a tokio worker in release (about 220 in debug).
- **Linear cost.** Objects, records, arrays and tuples move each value to its child instead of copying the subtree at every level (3000 levels used to take minutes; now it takes milliseconds), and `z::json()` validates a JSON input in a single pass with the same result as zod's `z.lazy` union. A value is copied only when an issue needs it.
- **Regular expressions.** Patterns without lookarounds or backreferences (including those that use `\b`) run with the `regex` crate, in linear time. Those that need them stay on the backtracking `fancy-regex` engine, as in V8, with a cap of 1,000,000 steps per `test`: when it is reached the text is considered invalid and a `[JsRegex]` `WARN` is logged. Avoid backreferences and lookarounds in `.regex()` rules applied to client text.
- **`#[array_unique]`** compares with a hash table (linear) instead of pairwise.
- `z::url()` accepts any scheme (`javascript:` included); for links that will be displayed use `z::http_url()`.
