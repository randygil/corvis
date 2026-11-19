---
status: documented
---

# Pipes

A pipe processes the value of a handler parameter **before** the handler runs. It is used for two things:

- **transforming** the value (the URL text `"42"` into the number `42`, a missing value into a default one), and
- **validating**: if the value is not acceptable, the pipe returns an error and the handler never runs.

## How the value travels

In Corvis, pipes work on `serde_json::Value`. For each parameter with an attribute (`#[body]`, `#[param]`, `#[query]`, `#[headers]`, `#[ip]`, `#[uploaded_file]`, ...):

1. Corvis extracts the raw value from the request as JSON (`null` if it is missing).
2. It passes it through the pipes that apply to it, in order. Each pipe receives the output of the previous one.
3. It deserializes the final result into the parameter's declared type.

Parameters resolved with `FromContext` (without an attribute, or with `#[req]`) do not go through pipes.

## Built-in pipes

| Pipe | What it does | Error |
|---|---|---|
| `ParseIntPipe` | Converts a numeric string into an integer | `Validation failed (numeric string is expected)` |
| `ParseFloatPipe` | Converts a numeric string into a number | `Validation failed (numeric string is expected)` |
| `ParseBoolPipe` | `"true"`/`"false"` to boolean | `Validation failed (boolean string is expected)` |
| `ParseUUIDPipe` (alias `ParseUuidPipe`) | Validates a UUID | `Validation failed (uuid is expected)`; with `.version(4)`, `Validation failed (uuid v 4 is expected)`; a value that is not a string, `The value passed as UUID is not a string` |
| `ParseArrayPipe` | Splits `"1,2,3"` into an array | `Validation failed (parsable array expected)` |
| `ParseEnumPipe` | Accepts only the given values | `Validation failed (enum string is expected)` |
| `DefaultValuePipe` | Sets a value when it is missing | — |
| `ValidationPipe` | Validates DTOs with `#[derive(Validate)]` | See [Validation](/application/validation) |
| `ZodValidationPipe` | Validates with `corvis::z` schemas | See [Validation](/application/validation) |
| `ParseFilePipe` | Validates uploaded files | See [File upload](/http/file-upload) |

All of them respond `400 Bad Request` in the standard format:

```json
{ "message": "Validation failed (numeric string is expected)", "error": "Bad Request", "statusCode": 400 }
```

### Options

`ParseIntPipe`, `ParseFloatPipe`, `ParseBoolPipe`, and `ParseUUIDPipe` share:

- `.optional()`: lets `null` (missing parameter) through without an error.
- `.error_http_status_code(406)`: changes the error status.

In addition:

```rust
ParseUUIDPipe::new().version(4)           // UUID v4 only (accepts 3, 4, 5, or 7)
ParseArrayPipe::new().separator(";").numbers().optional()
ParseEnumPipe::new(["persian", "siamese"]) // explicit values
ParseEnumPipe::of::<Breed>()               // variants of a serde enum
DefaultValuePipe::new(1)                   // replaces null (missing); "" is kept
```

### What the built-in pipes accept

These pipes were checked against the output of the original pipes (common package 11.2.1), so the edge cases below are the original's:

| Pipe | Accepts | Refuses |
|---|---|---|
| `ParseIntPipe` | `"5"`, `"-0"`, `"007"`; a numeral beyond 2^53 is rounded like `parseInt` (`"9007199254740993"` gives `9007199254740992`) | `"+5"`, `" 5"`, `"5 "`, `""`, `"0x10"`, `"1e3"` |
| `ParseFloatPipe` | `"1.5"`, `" 1.5 "`, `".5"`, `"5."`, `"1e3"`; the text is read as `parseFloat` does, so `"0x10"` gives `0` | `"Infinity"`, `"NaN"`, `""`, `"1,5"`, `"1.5abc"` |
| `ParseBoolPipe` | `"true"`, `"false"` and booleans | `"TRUE"`, `"1"`, `""` |
| `ParseArrayPipe` | The string is trimmed and split on the separator; with `.numbers()` each item is read as `Number()` (`" 2"` is `2`, `"0x10"` is `16`) | An empty or missing value (unless `.optional()`); with `.numbers()`, an empty item, `NaN` or an infinity, with `[index] item must be a number` |
| `DefaultValuePipe` | Replaces only `null` (a missing value) | Nothing: `""` and `0` are kept |

The items of `ParseArrayPipe` are not trimmed one by one: `"a, b"` gives `["a", " b"]`.

Two divergences from the original, because Corvis works on JSON values and never accepts a different value silently:

- `ParseIntPipe` refuses an integer of 2^63 or more (`"99999999999999999999"`) with its 400, where the original returns a `Number`.
- `ParseArrayPipe` with `.numbers()` refuses `"Infinity"` as an item (JSON has no infinity), with the same `[0] item must be a number` error.

Two more behaviors of the original's array pipe are not ported: it runs `JSON.parse` on each item before converting it, so `"true"` becomes `1` there and is refused here, and it falls back to `,` when the separator is empty (here an empty separator splits into characters).

`ParseEnumPipe::of::<T>()` panics when it is built if the variant names of `T` cannot be read (an untagged, internally or adjacently tagged enum, or a type that is not an enum), instead of building a pipe that refuses every value. For those types use `ParseEnumPipe::new([..])` with explicit values.

## Using a pipe on a parameter

Pipes are listed in the parameter's attribute, after the name:

```rust
#[get(":id")]
fn find_one(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<Cat> {
    self.cats_service.find_one(id)
}
```

You can pass the pipe's **type** (resolved with dependency injection) or an **expression** that builds the instance, for example to configure it:

```rust
#[get(":id")]
fn find_one(&self, #[param("id", ParseIntPipe::new().error_http_status_code(406))] id: u32) -> HttpResult<Cat> { ... }

#[get]
fn list(&self, #[query("page", DefaultValuePipe::new(1), ParseIntPipe)] page: u32) -> Vec<Cat> { ... }
```

In the second example the order matters: `DefaultValuePipe` sets `1` if `page` is missing and then `ParseIntPipe` converts the value.

::: tip Pipes by type
Pipes referenced by type must implement `Injectable` (your own, with `#[injectable]`). `ParseIntPipe`, `ParseFloatPipe`, `ParseBoolPipe`, `ParseUUIDPipe`, `ParseArrayPipe`, `ValidationPipe`, `ZodValidationPipe`, and `ParseFilePipe` already do. `ParseEnumPipe` and `DefaultValuePipe` need arguments, so they are always used as an expression.
:::

Without a pipe, route, query, and header parameters are already converted to the declared type when possible (`#[param("id")] id: u32` works with `/cats/42`). The difference is the error message: a pipe produces the standard validation message and deserialization produces the `serde` one.

## Pipes at other levels

| Scope | Form |
|---|---|
| One parameter | `#[param("id", ParseIntPipe)]` |
| All the parameters of a route | `#[use_pipes(...)]` on the method |
| All the routes of a controller | `#[use_pipes(...)]` below `#[controller]` |
| The whole application | `app.use_global_pipes(ValidationPipe::new())` |
| The whole application, with DI | `Provider::app_pipe::<MyPipe>()` in `providers` |

For each parameter the order is: globals (first those registered as a provider, then those from `use_global_pipes`), controller, route, and finally the parameter's own.

```rust
let mut app = CorvisFactory::create::<AppModule>().await?;
app.use_global_pipes(ValidationPipe::new().whitelist(true).forbid_non_whitelisted(true));
```

Broad-scope pipes apply to **all** parameters with an attribute. That is why validation pipes check the parameter's type and let through those that don't concern them: `ValidationPipe` only acts on types with `#[derive(Validate)]` and `ZodValidationPipe::auto()` only on types declared with `zod_dto!`. They can coexist.

If several parameters fail, the error from the pipe that fails earliest in its chain wins (globals first) and, if they tie, that of the last declared parameter.

## Custom pipes

A pipe implements `PipeTransform`:

```rust
#[async_trait]
pub trait PipeTransform: Send + Sync + 'static {
    async fn transform(&self, value: Value, metadata: &ArgumentMetadata) -> Result<Value, HttpException>;
}
```

`ArgumentMetadata` describes the parameter:

| Field | Contents |
|---|---|
| `kind` | `ParamKind::Body`, `Query`, `Param`, `Headers`, `Ip`, `Custom`, `File`, or `Files` |
| `data` | The name passed to the attribute (`Some("id")` in `#[param("id")]`) |
| `metatype.name` | The declared type, as text (`"u32"`, `"CreateCatDto"`) |
| `metatype.validator` | Present if the type derives `Validate` |
| `metatype.zod` | Present if the type has a `zod_dto!` schema |

A pipe that normalizes a text and rejects empty ones:

```rust
use corvis::prelude::*;

#[injectable]
pub struct TrimPipe;

#[async_trait]
impl PipeTransform for TrimPipe {
    async fn transform(&self, value: Value, meta: &ArgumentMetadata) -> Result<Value, HttpException> {
        match value {
            Value::String(s) if !s.trim().is_empty() => Ok(Value::String(s.trim().to_string())),
            Value::String(_) => Err(BadRequestException::new(format!("{} must not be empty", meta.data.unwrap_or("value")))),
            other => Ok(other),
        }
    }
}
```

```rust
#[get("search")]
fn search(&self, #[query("q", TrimPipe)] q: String) -> Vec<Cat> { ... }
```

Since the pipe is `#[injectable]`, it can have dependencies (`Arc<ConfigService>`, a repository, ...). They are resolved in the module of the controller that uses it.

## Validating the body

The most common DTO validation cases are already covered by two pipes:

```rust
#[derive(Debug, Deserialize, Validate)]
pub struct CreateCatDto {
    #[is_string]
    #[is_not_empty]
    #[max_length(50)]
    pub name: String,
    #[is_int]
    #[min(0)]
    pub age: u32,
}
```

```rust
use corvis::zod_dto;

#[derive(Debug, Deserialize)]
pub struct LoginDto {
    pub username: String,
}

zod_dto!(
    LoginDto,
    z::strict_object! {
        username: z::string().trim().min(2).max(20).regex("^[A-Za-z0-9_.-]+$"),
    }
);
```

With `ValidationPipe` or `ZodValidationPipe::auto()` as a global pipe, declaring the parameter `#[body] dto: CreateCatDto` is enough. All the rules and options are in [Validation](/application/validation).

::: info
Guards and interceptors see the **unvalidated** body: pipes validate the handler's argument, and their output does not replace `req.body()`.
:::
