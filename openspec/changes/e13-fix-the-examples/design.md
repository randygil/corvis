# Design

## Context

The workspace has three examples (`Cargo.toml` members include `examples/*`): `cats-app`, `blog-orm` and `tertulia`.

- **cats-app secret.** `examples/cats-app/src/auth/auth_module.rs:11-15` registers
  `JwtModule::register_async(true, ...)` with `config.get_or("JWT_SECRET", "change-me".to_string())`.
  `examples/cats-app/.env.example:2` is `JWT_SECRET=change-me`. `docs/faq/examples.md:41-44` (and its Spanish
  translation) warns about the default instead of removing it.
- **tertulia, the pattern to copy.** `examples/tertulia/src/app_module.rs:13-19` reads
  `config.get::<String>("JWT_SECRET").filter(|s| !s.trim().is_empty())` and returns
  `anyhow!("JWT_SECRET is required (set it in the environment or in .env)")`; `examples/tertulia/src/lib.rs` exposes
  `create_app()` and `examples/tertulia/tests/startup.rs` asserts the startup failure (removing the variable and
  switching to an empty temp working directory so no `.env` is read). The same snippet appears in
  `docs/security/authentication.md`, `docs/modules.md` and `docs/deployment.md`.
- **cats-app layout.** Binary only: `src/main.rs` declares the modules and configures the app inline (global prefix
  `api`, `ValidationPipe`, `HttpExceptionFilter`, CORS, URI versioning with default `1`, shutdown hooks) and listens on
  `PORT` (lines 16-28). No `tests/` folder. `Cargo.toml` depends only on `corvis` and `serde`. DTOs use
  `#[derive(Deserialize, Validate)]` (`src/cats/dto/mod.rs`, `src/auth/sign_in_dto.rs`); `Cat`
  (`src/cats/entities/mod.rs`) is `Serialize` with `rename_all = "camelCase"`.
- **blog-orm flag.** `examples/blog-orm/src/posts/dto.rs:61-71`: `ListPostsQuery` uses
  `published: z::coerce::boolean().optional()`. `ZBoolean::check` (`crates/corvis/src/z/types.rs:2020-2031`) and
  `run_v4` (`:2035-2043`) both apply JavaScript truthiness (`js::truthy`, `crates/corvis/src/z/js.rs:215-226`: any
  non-empty string is `true`). `z::stringbool()` (`types.rs:3101`, defaults at `:2662-2671`) maps
  `true/1/yes/on/y/enabled` and `false/0/no/off/n/disabled` (case-insensitive) and rejects other strings.
  `PostsService::find_all` (`src/posts/posts_service.rs:37-39`) filters with `post::published::equals(p)`. The app is
  binary only (`src/main.rs`), with `ZodValidationPipe::auto()` and `OrmExceptionFilter`, and the database URL comes
  from `DATABASE_URL` (`src/app_module.rs:14`). `docs/application/validation.md:148` already recommends `stringbool`.
- **OpenAPI.** `corvis-swagger` exposes `DocumentBuilder`, `SwaggerModule::create_document` / `setup`,
  `cleanup_open_api_doc` and the `#[api_*]` attributes (`crates/corvis-swagger/src/lib.rs:31-38`);
  `#[derive(ApiSchema)]` accepts only `api_property`, `api_property_optional`, `api_response_property`,
  `api_hide_property` and `api_schema` helper attributes (`crates/corvis-swagger-macros/src/lib.rs:378`), so it ignores
  the validation attributes on the same struct. The explorer reads the application's versioning options
  (`crates/corvis-swagger/src/explorer.rs:287`). The only tests are `crates/corvis-swagger/tests/swagger_app.rs` and
  `swagger_security.rs`, which use zod DTOs; no example depends on the crate (grep over `examples/*/Cargo.toml`).

## Goals / Non-Goals

**Goals:**
- No example ships a usable default secret or a boolean filter that inverts `false`.
- One example shows the OpenAPI path end to end with `#[derive(Validate)]` + `#[derive(ApiSchema)]` DTOs, and its test
  is the first repository test of `#[derive(ApiSchema)]`.
- Every fix has an integration test in the example's own `tests/` folder.

**Non-Goals:**
- Framework changes (the `coerce::boolean` rustdoc, ApiSchema reading validation constraints) and loopback binding of
  `cats-app` (see proposal, Out of scope).

## Decisions

1. **cats-app copies tertulia's secret handling verbatim.** Same filter (`trim().is_empty()`), same error text, so the
   docs snippet already used in three pages describes both examples. `.env.example` becomes `JWT_SECRET=` with a
   comment `# required; generate one with: openssl rand -hex 32`. Alternative: generate a random secret at startup when
   missing; rejected, tokens would stop validating after every restart and it hides the misconfiguration.
2. **`lib.rs` + `create_app()` in cats-app and blog-orm.** Move module declarations and the app configuration from
   `main.rs` to `src/lib.rs` (`pub async fn create_app() -> corvis::Result<CorvisApplication>`), leaving `main.rs` with
   shutdown hooks, port and `listen`, exactly like `examples/tertulia/src/{lib,main}.rs`. Tests then use
   `request(&app)` without a socket. Alternative: unit tests inside the binary crates; rejected because the secret
   check runs in the module factory and needs the whole application.
3. **`z::stringbool().optional()` for `published`.** It is the schema the validation docs recommend and needs no custom
   code. Invalid values become a `400` through `ZodValidationPipe::auto()`. Alternative: a typed `Option<bool>` with
   serde's `FromStr` (only accepts `true`/`false`, inconsistent with the zod-based example).
4. **Swagger in cats-app, not blog-orm.** cats-app uses `#[derive(Validate)]` DTOs, which is the case
   `#[derive(ApiSchema)]` exists for, and it has auth, versioning and several controllers to tag. blog-orm's zod DTOs
   are already covered by `corvis-swagger`'s own `swagger_app.rs` test. The document is built inside `create_app()`
   after versioning is enabled, with `DocumentBuilder::new().set_title("Cats").set_version("1.0").add_bearer_auth()`,
   `cleanup_open_api_doc` and `SwaggerModule::setup("docs", ...)`. Controllers get `#[api_tags(...)]`; protected
   controllers get `#[api_bearer_auth]`; DTOs and `Cat` derive `ApiSchema` next to their existing derives. The flat
   `/docs` routes bypass the global `AuthGuard` (documented in `docs/openapi/introduction.md`), which is acceptable for
   an example and noted in its docs.
5. **Tests.**
   - `examples/cats-app/tests/startup.rs`: one test per binary (environment is process-global), as in tertulia: no
     `JWT_SECRET` + empty temp cwd → error mentions `JWT_SECRET`. A second binary for the empty-string case.
   - `examples/cats-app/tests/openapi.rs`: sets `JWT_SECRET`, builds the app, requests `/docs-json` and asserts the
     paths, tags, schemas (`CreateCatDto`, `Cat.ownerId`, `SignInDto`) and the bearer scheme from the spec.
   - `examples/blog-orm/tests/posts_filter.rs`: `DATABASE_URL=sqlite::memory:`, creates a user and two posts through
     the API, then checks `published=false|true|maybe|<absent>`.

## Risks / Trade-offs

- [Moving code to `lib.rs` changes paths quoted in docs] → task 5.1 greps `docs/` for `examples/cats-app/src/main.rs`
  and `examples/blog-orm/src/main.rs` references and updates them.
- [`cargo run -p cats-app` now fails out of the box] → intended; the error and `docs/faq/examples.md` give the
  one-line fix (`JWT_SECRET="$(openssl rand -hex 32)" cargo run -p cats-app`), as for tertulia.
- [`sqlite::memory:` with a connection pool may give each connection its own database] → tertulia's startup test uses
  the same URL; if the blog-orm test sees empty tables, use a temp-file SQLite URL instead and note it in tasks.md.

## Open questions

- Should `/docs` in cats-app be mounted only when an environment flag is set (to show the "do not expose in
  production" practice), or always? Recommendation: always, with a comment, to keep the example simple.
- Is a `JWT_SECRET` minimum length (for example 32 bytes) worth enforcing in the examples? Recommendation: not in this
  change (listed as out of scope); it would also need to change tertulia and the docs snippet.

## Review notes (2026-11-08)

- `/docs` is always mounted in `cats-app`; no minimum length check for `JWT_SECRET` in this change.
- Also bind `cats-app` to `127.0.0.1` like the other examples (one-line fix, same issue: examples should be safe to run).
