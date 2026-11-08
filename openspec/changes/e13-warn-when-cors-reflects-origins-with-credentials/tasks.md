# Tasks

## 1. The check

- [x] 1.1 Add `pub(crate) fn startup_warning(&self) -> Option<String>` to `CorsOptions` in
  `crates/corvis/src/router/cors.rs` (design decisions 1 and 3) with unit tests in the existing `tests` module: `Some`
  only for `Reflect` + `credentials(true)` (built both with `reflect_origin()` and with
  `origin_value(CorsOrigin::Reflect)`), `None` for `Any`, `origin("*")`, `Fixed`, `List` (strings and a regex),
  `Disabled`, `Dynamic` with credentials, and for `Reflect` without credentials; the message contains
  `reflect_origin()`, `credentials(true)` and the docs URL; verify with `cargo test -p corvis --lib cors`

## 2. Log it at startup

- [x] 2.1 In `CorvisApplication::init()` (`crates/corvis/src/application.rs:361-424`), right after `build_router`
  (`:399`; after `posture::check_routes` if #26 has landed), log `Logger::new("CorsOptions").warn(..)` when
  `self.config.cors` yields a warning; verify `cargo build -p corvis` with no warnings
- [x] 2.2 New `crates/corvis/tests/cors_startup_warning.rs` with a capturing `LoggerService` (installed with
  `use_logger`, tests serialized with a `static Mutex`) covering the spec scenarios: reflection + credentials via
  `enable_cors` and `listen_in_background("127.0.0.1:0")` (warning logged before it returns), via
  `CorvisApplicationOptions::cors`, a second `init()` plus two in-memory requests (still one warning), explicit list
  with credentials, reflection without credentials, `origin_fn` with credentials (no warning); verify with
  `cargo test -p corvis --test cors_startup_warning`
- [x] 2.3 In the same file, "Reflected response unchanged": `GET /health` with `Origin: http://z.io` returns
  `Access-Control-Allow-Origin: http://z.io`, `Vary: Origin` and `Access-Control-Allow-Credentials: true`; verify it
  passes and `cargo test -p corvis --test express_parity` still passes

## 3. Docs

- [x] 3.1 `docs/security/cors.md`: replace the warning box (lines 99-103) with a section
  `## Credentials and open origins {#credentials-and-open-origins}` that keeps the advice (explicit list, `origin_fn`
  that accepts any origin), shows the startup warning text, says it is a warning for compatibility with `cors` and how
  to filter it with a custom logger; link it from the `reflect_origin()` row of the options table; same in
  `docs/es/security/cors.md` with a Spanish heading and the same explicit anchor
- [x] 3.2 Run `npm run docs:check` in `docs/`; verify it passes and that
  `/security/cors#credentials-and-open-origins` resolves in the built site

## 4. Definition of done

- [x] 4.1 `cargo build --workspace --all-targets` with no warnings
- [x] 4.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 4.3 `cargo test --workspace` passes, including the regression tests of 1.1 and 2.2-2.3
- [x] 4.4 Docs updated in English and Spanish (`docs/` and `docs/es/`) and `npm run docs:check` passes in `docs/`
- [x] 4.5 (NOT RUN: the maintainer runs corvis-parity) Framework runtime code changed (`init()`): run the `corvis-parity` check and verify the three port apps keep
  0 apidiff differences and log no `CorsOptions` warning at startup (maintainer batch run on batch/phase03-e 3cab956: consi 832/832, facten 727/727, houseplan 636/636; routes identical, databases equivalent)

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #134, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).
