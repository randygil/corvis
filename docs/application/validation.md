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

`corvis::z` reproduces the zod 4 API with Rust functions. A DTO is associated with its schema with the `zod_dto!` macro:

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

`parse`, `safe_parse`, `parse_as::<T>` (validates and deserializes), `flatten_error`, `treeify_error` and `prettify_error` are available. The full catalog of types and methods follows the zod 4 API; the equivalence table is in the repository's `README.md`.

### Messages in Spanish

zod messages use the `en` locale by default. `corvis::z` also includes `es`:

```rust
z::configure(|c| c.locale_error = Some(z::locales::es()));
```

## What guards and interceptors see

Pipes validate the handler's argument; they do not replace `req.body()`. A guard or an interceptor that reads the body sees the **unvalidated** value. If a guard needs already validated data, validate there or move the logic to the handler.

## Security limits

- Recursive schemas (`z::lazy`, `z::json()`) allow at most 128 levels per parse; change it with `z::configure(|c| c.max_depth = n)`.
- `regex` patterns without lookarounds or backreferences run in linear time; those that use them have a step cap. Avoid them in rules that receive text from the client.
- `z::url()` accepts any scheme (`javascript:` included); for links that will be displayed use `z::http_url()`.
