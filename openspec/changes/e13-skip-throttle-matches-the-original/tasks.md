# Tasks

Prerequisite: #120 (`e13-rate-limit-headers-in-the-throttler`) and #135
(`e13-throttler-block-duration-like-the-original`) are implemented and archived, so
`openspec/specs/security/rate-limiting/spec.md` holds the requirement this change modifies, the guard loop sorts by
`ttl` and calls `record_hit`, and `crates/corvis/tests/fixtures/throttler/` has `generate.mjs`, `throttler.json` and
the replay test. If they are not, stop and implement them first.

## 1. Reference behavior and failing tests

- [ ] 1.1 Re-read `dist/throttler.decorator.js` and the skip part of `dist/throttler.guard.js` in the version pinned
  by `crates/corvis/tests/fixtures/throttler/package.json`; if it is not 6.7.1 or the skip logic differs from
  design.md (Context, Original), record the difference in design.md before going on; verify by quoting the version
  in design.md
- [ ] 1.2 Extend `generate.mjs` with the apps and routes of design.md, Decision 7 (bare skip with named throttlers
  only, bare skip next to `default`, skip by name, controller skip undone by its handler, controller and handler
  skipping different names) and regenerate `throttler.json`; verify the new entries show `X-RateLimit-*-short` headers
  on the bare-skip route of the named-only app and a `429` on its fourth request, and commit the JSON (not
  `node_modules`)
- [ ] 1.3 Add failing integration tests next to #120's throttler tests, one per scenario of the spec delta that needs
  the HTTP stack (bare skip with `default` only, bare skip next to `short`, bare skip with named throttlers only, skip
  by name, handler undoes the controller's skip, controller and handler skip different names, `String` key on a
  route); verify they fail to compile or fail for the expected reason with `cargo test -p corvis --test <file>`

## 2. Metadata API

- [ ] 2.1 Change `RouteDef::set_metadata` in `crates/corvis/src/router/controller.rs` to take `key: impl
  Into<String>`; verify `cargo test -p corvis-macros` and the existing route metadata tests still pass and the
  `String` key test of 1.3 compiles
- [ ] 2.2 Add `skip_throttle_key(name) -> String` and `skip_throttle(names) -> CustomDecorator` in
  `crates/corvis/src/modules/throttler.rs` with rustdoc mapping them to `@SkipThrottle({ ... })`; update the rustdoc of
  `SKIP_THROTTLE` (bare form, `default` only); export both from the `modules::throttler` re-export and the prelude in
  `crates/corvis/src/lib.rs`; verify with a unit test of the key spelling (`THROTTLER:SKIPshort`) and the decorator's
  entries, and that `use corvis::prelude::*;` reaches both in an integration test

## 3. Guard

- [ ] 3.1 In `ThrottlerGuard::can_activate`, remove the early return on `SKIP_THROTTLE` and resolve the skip per
  throttler at the top of the loop (design.md, Decision 2: per-name key, `SKIP_THROTTLE` as the `default` entry of a
  target without its own, handler before controller, `continue` when skipped); verify the tests of 1.3 pass and
  `crates/corvis/tests/modules.rs`, `crates/corvis/tests/input_hardening.rs` and #120 / #135's throttler tests still
  pass
- [ ] 3.2 Add the warn-once set and the warning of design.md, Decision 6; verify with a test that captures the logger
  with `app.use_logger(..)`, as `crates/corvis/tests/cors_startup_warning.rs:38-41` does, that three requests to the
  same route log one `ThrottlerGuard` warning naming the route and that none is logged when a throttler is named
  `default`
- [ ] 3.3 Replay the new cases of `throttler.json` in the Rust replay test, declaring the routes with `SKIP_THROTTLE`,
  `skip_throttle([..])` and `skip_throttle_key(..)`, and switch the existing `/skip` route to
  `skip_throttle(["default", "short", "long"])`; verify the replay passes
- [ ] 3.4 Add a gateway test: a global `ThrottlerGuard` with throttlers `short` (`limit: 1`) and `long` and a handler
  with `#[set_metadata(skip_throttle_key("short"), true)]`; verify two messages get no `exception` frame and a third
  past `long`'s limit does

## 4. Docs

- [ ] 4.1 `docs/security/rate-limiting.md`: metadata table (`SKIP_THROTTLE` skips the `default` throttler, like
  `@SkipThrottle()`; `skip_throttle([...])` and `skip_throttle_key(name)`); a "Skipping throttlers" section with the
  per-name rule, handler over controller per name, `false` to undo a controller skip, gateways with
  `#[set_metadata(skip_throttle_key(..), ..)]`, the warning, and the behavior change for apps with named throttlers
  only; remove the bare-skip limitation #120 added; same in `docs/es/security/rate-limiting.md`
- [ ] 4.2 Run `npm run docs:check` in `docs/`; verify it passes

## 5. Definition of done

- [ ] 5.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 5.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] 5.3 `cargo test --workspace` passes, including the regression tests of 1.3, 3.2 and 3.4 and the differential
  replay of 3.3
- [ ] 5.4 Docs updated in English and Spanish (`docs/` and `docs/es/`) and `npm run docs:check` passes in `docs/`
- [ ] 5.5 Framework runtime code changed: run the `corvis-parity` check and verify the three port apps keep 0 apidiff
  differences (none uses the throttler)

## Workflow follow-up

- Archive the change with `/opsx:archive` after review (after the changes of #120 and #135 are archived).
- Close issue #152, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).
