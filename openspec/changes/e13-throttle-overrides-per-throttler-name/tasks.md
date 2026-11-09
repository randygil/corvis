# Tasks

Prerequisite: #152 (`e13-skip-throttle-matches-the-original`) and #154 (`e13-throttler-groups-ipv6-clients-by-prefix`)
are archived on `main` (they are, at `bcc778d`). Line numbers in design.md refer to that commit.

## 1. Reference behavior and fixture

- [ ] 1.1 Re-read `dist/throttler.decorator.js`, `dist/throttler.constants.js`, the resolution part of
  `dist/throttler.guard.js` and `dist/throttler.service.js` in the version pinned by
  `crates/corvis/tests/fixtures/throttler/package.json`, and `getAllAndOverride` in the pinned `@nestjs/core`; if the
  version is not 6.7.1 / 12.4.0 or the behavior differs from design.md (Context, Original), record it in design.md
  before going on; verify by quoting the versions in design.md
- [ ] 1.2 Extend `generate.mjs` as design.md, Decision 10 says: new controllers (`Ovr`, `OvrClass`), the `send` field
  for request headers, module options objects in `record()`, and the apps `overrides-named`, `overrides-default`,
  `module-tracker`, `module-key` and `shared-key`; regenerate `throttler.json` with `npm ci && node generate.mjs`;
  verify the existing cases are byte-identical in the diff, `/ovr/invalid` recorded a 500, and `shared-key` shows
  `x-ratelimit-reset-long` equal to the short window on its first response; commit the JSON (not `node_modules`)
- [ ] 1.3 Add failing integration tests in `crates/corvis/tests/throttler_headers.rs`, one per spec scenario that needs
  the HTTP stack (per-name and partial overrides, null, bare keys, helpers, invalid values, module and route hooks,
  unregistered name, per-throttler counters); verify they fail to compile or fail for the expected reason with
  `cargo test -p corvis --test throttler_headers`

## 2. Override metadata API

- [ ] 2.1 In `crates/corvis/src/modules/throttler.rs` add `THROTTLE_TRACKER`, `THROTTLE_KEY_GENERATOR`,
  `throttle_key(option, name)`, `ThrottleOverride` and `throttle(name)` (design.md, Decisions 2 and 7) with
  `From<ThrottleOverride> for CustomDecorator` writing only the options that were set; update the rustdoc of
  `THROTTLE_LIMIT`, `THROTTLE_TTL` and `THROTTLE_BLOCK_DURATION` (bare form, `default` only) and of the module header;
  verify with unit tests of the key spelling and of the decorator entries
- [ ] 2.2 Export the new items from the `modules::throttler` re-export and the prelude in `crates/corvis/src/lib.rs`;
  verify `use corvis::prelude::*;` reaches them in `tests/throttler_headers.rs`

## 3. Guard: resolution, validation, unknown names

- [ ] 3.1 Replace the reads at `throttler.rs:727-734` with a per-throttler, per-option resolver over `[handler, class]`
  (per-name key first, then the bare key for `default` only, JSON `null` as absent) and use it for limit, ttl and
  block duration (block duration keeps the `with_block_duration` and effective-ttl fallbacks); verify the override
  and bare-key tests of 1.3 pass and the existing tests in `tests/throttler_headers.rs` and `tests/modules.rs` still
  pass
- [ ] 3.2 Validate resolved values (design.md, Decision 4): invalid values return `HttpException::from_anyhow` with
  the message `Invalid "<option>" for the "<name>" throttler: ...`; verify the invalid-value tests of 1.3 (500 body,
  logged message names option, throttler and value)
- [ ] 3.3 Add the unknown-name scan and its warn-once set (design.md, Decision 5) next to #152's `warned` set, and set
  the new field in the hand-built guards of the unit tests; verify in `tests/throttler_skip_log_ws.rs` (process-wide
  logger capture) that three requests to a route with `throttle("shrot")` log one warning naming the route and
  `shrot`, that a bare key without a `default` throttler logs one, and that none is logged when all names exist
- [ ] 3.4 Add the gateway case to `tests/throttler_skip_log_ws.rs`: a handler with
  `#[set_metadata(throttle_key(THROTTLE_LIMIT, "short"), 1)]`; verify the second message gets an exception frame

## 4. Tracker and key generator hooks

- [ ] 4.1 Add `ThrottlerTrackerFn`, `ThrottlerKeyGeneratorFn`, `with_tracker`, `with_key_generator`,
  `with_named_tracker` and `with_named_key_generator` on `ThrottlerStorage` (design.md, Decisions 6 and 7), panicking
  on a duplicate id; verify with unit tests of the duplicate panic and that `for_root_async` factories returning a
  storage keep the hooks
- [ ] 4.2 In `can_activate`, resolve tracker and key generator per throttler (route override, module hook, built-in),
  compute the built-in tracker at most once per request, and store hits under the length-prefixed name plus the
  generated key, with the built-in key `"{class}:{handler}:{tracker}"` (Decisions 8 and 9); an unregistered id or a
  non-string value answers 500; verify the hook tests of 1.3, including the tracker error and the per-throttler
  counters scenario, and that the IPv6 unit tests in `throttler.rs` still pass

## 5. Differential replay

- [ ] 5.1 Declare the fixture routes in `tests/throttler_headers.rs` with `throttle(..)` (bare `THROTTLE_LIMIT` for
  `/ovr/default`, `throttle_key(THROTTLE_LIMIT, "short")` with `"abc"` for `/ovr/invalid`), send the recorded request
  headers, and build each case's storage from a table keyed by case name; list `shared-key` as a known divergence that
  must differ from the recording and match the per-throttler counters scenario; verify
  `replays_the_original_library_fixture` passes for every case

## 6. Docs and example

- [ ] 6.1 `docs/security/rate-limiting.md`: replace "apply to all configured limits" (line 118) with a section on
  per-name overrides (`throttle(name)`, partial overrides, handler over controller per option, gateways with
  `throttle_key`), the bare keys as the `default` entry with a behavior change note, accepted value types and the 500,
  the unknown-name warning; a section on trackers and key generators (module hooks, named hooks, precedence, custom
  tracker skips `normalize_ip`, client-chosen trackers and the shared anonymous counter, the per-throttler counters
  divergence); update the Blocking bullets and the metadata table; same in `docs/es/security/rate-limiting.md`; run
  `npm run docs:check` in `docs/` and verify it passes
- [ ] 6.2 `examples/cats-app/src/auth/auth_controller.rs`: tighten `login` with
  `#[apply_decorators(throttle("default").limit(5).ttl(60_000))]`; verify with a test in `examples/cats-app/tests/`
  that the sixth login attempt in a row answers 429

## 7. Definition of done

- [ ] 7.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 7.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] 7.3 `cargo test --workspace` passes, including the regression tests of 1.3, 3.3, 3.4, 4.1 and 6.2 and the
  differential replay of 5.1
- [ ] 7.4 Docs updated in English and Spanish (`docs/` and `docs/es/`) and `npm run docs:check` passes in `docs/`
- [ ] 7.5 Framework runtime code changed: run the `corvis-parity` check and verify the three port apps keep 0 apidiff
  differences (none uses the throttler)

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #156, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).
