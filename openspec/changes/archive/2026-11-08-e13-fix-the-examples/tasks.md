# Tasks

## 1. Testable entry points

- [x] 1.1 cats-app: move module declarations and the app configuration from `src/main.rs` to a new `src/lib.rs` with
  `pub async fn create_app() -> corvis::Result<CorvisApplication>` (tertulia pattern); `main.rs` keeps shutdown hooks,
  `PORT` and `listen`; verify `cargo build -p cats-app` with no warnings and `cargo run -p cats-app` behaves as before
  (with `JWT_SECRET` set)
- [x] 1.2 blog-orm: same split (`src/lib.rs` with `create_app()`, `main.rs` with shutdown hooks and listen); verify
  `cargo build -p blog-orm` with no warnings

## 2. cats-app JWT secret

- [x] 2.1 Replace `get_or("JWT_SECRET", "change-me")` in `src/auth/auth_module.rs` with the tertulia check
  (`get::<String>`, reject empty/whitespace, `JWT_SECRET is required (set it in the environment or in .env)`); change
  `.env.example` to `JWT_SECRET=` plus a comment with `openssl rand -hex 32`; verify by grepping the repository for
  `change-me` (no hit left in `examples/`)
- [x] 2.2 Add `examples/cats-app/tests/startup.rs` (missing variable, empty temp cwd) and
  `examples/cats-app/tests/startup_empty_secret.rs` (`JWT_SECRET=""`); verify `cargo test -p cats-app`

## 3. blog-orm boolean filter

- [x] 3.1 Change `published` in `ListPostsQuery` (`src/posts/dto.rs`) to `z::stringbool().optional()` and fix the
  comment above the schema; verify `cargo build -p blog-orm`
- [x] 3.2 Add `examples/blog-orm/tests/posts_filter.rs` with `DATABASE_URL=sqlite::memory:` (or a temp file, see
  design.md risks): create a user and one published and one unpublished post through the API, then assert
  `?published=false`, `?published=true`, `?published=maybe` (400) and no parameter; verify `cargo test -p blog-orm`

## 4. OpenAPI in cats-app

- [x] 4.1 Add `corvis-swagger = { path = "../../crates/corvis-swagger" }` to `examples/cats-app/Cargo.toml`; derive
  `ApiSchema` on `CreateCatDto`, `UpdateCatDto`, `ListCatsQuery`, `SignInDto` and `Cat` (with a few
  `#[api_property(description = ..., example = ...)]`); verify `cargo build -p cats-app`
- [x] 4.2 Tag controllers with `#[api_tags(...)]`, add `#[api_bearer_auth]` to the protected ones and a couple of
  response attributes on `CatsController`; build the document in `create_app()` after versioning
  (`DocumentBuilder`, `add_bearer_auth`, `cleanup_open_api_doc`, `SwaggerModule::setup("docs", ...)`); verify by
  running the app and opening `http://127.0.0.1:3000/docs`
- [x] 4.3 Add `examples/cats-app/tests/openapi.rs` asserting the scenarios of `specs/examples/cats-app/spec.md`
  (versioned cats paths with the `cats` tag, `CreateCatDto` properties, `Cat.ownerId`, `SignInDto` reference, bearer
  scheme); verify `cargo test -p cats-app`

## 5. Docs

- [x] 5.1 `docs/faq/examples.md`: replace the `change-me` warning with "`JWT_SECRET` is required" and the run command
  with a generated secret; mention `/docs` in the cats-app bullet list, the `published` flag in blog-orm, and the new
  tests in "Example tests"; update `docs/openapi/introduction.md` (point to `examples/cats-app` as the full example) and
  the tip at `docs/openapi/types-and-parameters.md:118` (ApiSchema is now covered by the cats-app test); grep `docs/`
  for references to the moved `main.rs` code and update them; verify the snippets match the code
- [x] 5.2 Same changes in `docs/es/faq/examples.md`, `docs/es/openapi/introduction.md` and
  `docs/es/openapi/types-and-parameters.md`; verify `npm run docs:check` in `docs/`

## 6. Loopback binding and smoke test

- [x] 6.1 `cats-app` listens on `127.0.0.1:{port}` in `src/main.rs`; verify with the smoke script
- [x] 6.2 Add `scripts/smoke-examples.mjs` (blog-orm, tertulia, cats-app; free port, temporary database, random
  `JWT_SECRET`, all processes and temp files removed) and a section in `CONTRIBUTING.md`; verify
  `node scripts/smoke-examples.mjs` after `cargo build -p blog-orm -p tertulia -p cats-app`

## 7. Definition of done

- [x] 7.1 `cargo build --workspace --all-targets` with no warnings
- [x] 7.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 7.3 `cargo test --workspace` passes, including the new regression tests in `examples/cats-app/tests/` and
  `examples/blog-orm/tests/`
- [x] 7.4 Docs updated in English and Spanish (`docs/` and `docs/es/`); `npm run docs:check` in `docs/` passes
- [x] 7.5 corvis-parity: not required, only examples change (no framework runtime code); state this in the commit
  message

## Workflow follow-up

- [x] Archive the change with `/opsx:archive`
- [x] Close issue [#21](https://github.com/randygil/corvis/issues/21)
- [x] Tick #21 in its epic [#13](https://github.com/randygil/corvis/issues/13)
- [x] Move the card to Done in the Corvis Roadmap project
