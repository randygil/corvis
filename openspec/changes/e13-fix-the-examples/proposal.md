# Fix the examples

Implements [randygil/corvis#21](https://github.com/randygil/corvis/issues/21) (epic
[#13](https://github.com/randygil/corvis/issues/13)).

## Why

Examples are copied. The docs review found two examples that teach a bug and a documented crate that no example uses:

- `cats-app` falls back to the JWT secret `"change-me"` when `JWT_SECRET` is not set
  (`examples/cats-app/src/auth/auth_module.rs:13`), and `.env.example` ships the same value. Anyone who runs it as is,
  or copies the module, signs tokens with a public secret. `tertulia` already does it right: no default, the app
  refuses to start.
- `blog-orm` validates `?published=` with `z::coerce::boolean()` (`examples/blog-orm/src/posts/dto.rs:68`), which
  follows JavaScript's `Boolean(x)`: `?published=false` becomes `true` and the endpoint returns published posts. The
  validation docs already warn about this and recommend `z::stringbool()`.
- No example uses `corvis-swagger` or `#[derive(ApiSchema)]`, although the OpenAPI section of the docs describes them,
  and `#[derive(ApiSchema)]` has no test in the repository (`docs/openapi/types-and-parameters.md:118` says so).

## What Changes

- `cats-app` requires `JWT_SECRET`: without it (or with an empty value) the application fails at startup with an
  error naming the variable, like `tertulia`. `.env.example` no longer contains a usable secret and explains how to
  generate one.
- `blog-orm` parses `published` with `z::stringbool()`: `true/false/1/0/yes/no/...` map to the expected boolean and any
  other value is a `400`.
- `cats-app` gains an OpenAPI document with `corvis-swagger`: Swagger UI at `/docs`, JSON at `/docs-json`, its DTOs and
  `Cat` entity described with `#[derive(ApiSchema)]`, controllers tagged, bearer auth declared.
- `cats-app` and `blog-orm` get a `lib.rs` with `create_app()` (the `tertulia` pattern) so integration tests in their
  `tests/` folders can cover the three fixes.
- Docs (`faq/examples`, `openapi/introduction`, `openapi/types-and-parameters`, and any page quoting the changed code)
  in English and Spanish.

## Capabilities

### New Capabilities
- `examples/cats-app`: security defaults and OpenAPI coverage of the `cats-app` example.
- `examples/blog-orm`: query-string parsing in the `blog-orm` example.

## Impact

- Code: `examples/cats-app/` (`Cargo.toml` adds `corvis-swagger`; `src/lib.rs` new; `src/main.rs`,
  `src/auth/auth_module.rs`, DTOs, entity, controllers, `.env.example`; `tests/` new) and `examples/blog-orm/`
  (`src/lib.rs` new, `src/main.rs`, `src/posts/dto.rs`; `tests/` new).
- No framework crate changes.
- Docs: `docs/faq/examples.md`, `docs/openapi/introduction.md`, `docs/openapi/types-and-parameters.md` and their
  `docs/es/` translations.

## Out of scope

- The rustdoc of `z::coerce::boolean()` (`crates/corvis/src/z/types.rs:3190-3191`) claims zod 3 mode maps `"false"`
  to `false`, but the zod 3 path (`ZBoolean::check`, `types.rs:2024`) uses `Boolean(x)` too: fix the comment in a
  separate issue (framework doc, not an example).
- `cats-app` listens on all interfaces (`app.listen(port)` in `src/main.rs:27`) while `blog-orm` and `tertulia` bind
  to `127.0.0.1`: separate small issue.
- `#[derive(ApiSchema)]` reading `#[derive(Validate)]` constraints (`#[max_length]`, `#[is_in]`...) into the schema, as
  the original framework's CLI plugin does with class-validator: feature request.
- OpenAPI for `blog-orm`'s zod DTOs and for `tertulia`.
- A minimum length check for `JWT_SECRET`.
