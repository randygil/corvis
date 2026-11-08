# Tasks

## 1. Cookie max age in milliseconds

- [ ] 1.1 Add regression tests first: a unit test in `crates/corvis/src/http/response.rs` (or `crates/corvis/tests/res.rs`) for the three `http/cookies` scenarios (`90_500` → `Max-Age=90` with both APIs, `3_600_000` → `Max-Age=3600`, `-1` → `Max-Age=-1`); verify with `cargo test -p corvis cookie` that they fail on the current code
- [ ] 1.2 Change `Cookie::max_age` to milliseconds and write `Max-Age={ms.div_euclid(1000)}` in `Cookie::try_to_string`; update the doc comments of the method and the field; verify the new tests pass
- [ ] 1.3 Update the existing assertion in `crates/corvis/tests/static_response_security.rs` (`max_age(60)` → `max_age(60_000)`, still `Max-Age=60`); verify `cargo test -p corvis --test static_response_security` passes

## 2. Deprecate `ValidationPipe` `transform`

- [ ] 2.1 Mark `ValidationPipe::transform(bool)` and `ValidationPipeOptions::transform` `#[deprecated]` with the note from design.md decision 3; add `#[allow(deprecated)]` only where the framework itself must touch the field (`Default` impl); verify `cargo build -p corvis --all-targets` shows no deprecation warning from the crate
- [ ] 2.2 Add a regression test (in `crates/corvis/tests/`, with `#[allow(deprecated)]`) showing `transform(false)` and the default answer the same for `?page=2` on a `#[query]` DTO with `page: u32`; verify it passes
- [ ] 2.3 Search `crates/`, `examples/` and the docs code samples for calls to the deprecated option and remove them; verify with a grep that none remain outside the test of 2.2

## 3. Docs

- [ ] 3.1 `docs/http/cookies.md`: `Cookie` example with `max_age(3_600_000)`, replace the seconds-vs-milliseconds note with "both APIs take milliseconds, like Express" and a short note that `Cookie::max_age` took seconds before; same in `docs/es/http/cookies.md`
- [ ] 3.2 `docs/application/validation.md`: mark `transform(bool)` as deprecated (no effect, conversion always happens); same in `docs/es/application/validation.md`
- [ ] 3.3 Run `npm run docs:check` in `docs/`; verify it passes

## 4. Definition of done

- [ ] 4.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 4.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] 4.3 `cargo test --workspace` passes, including the regression tests of 1.1 and 2.2
- [ ] 4.4 Docs updated in English and Spanish (`docs/` and `docs/es/`) and `npm run docs:check` passes in `docs/`
- [ ] 4.5 Framework runtime code changed (`Cookie` serialization): run the `corvis-parity` check and verify the three port apps keep 0 apidiff differences

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #18, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).
