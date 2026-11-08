# Tasks

## 1. Failing tests first

- [x] 1.1 In `crates/corvis/tests/websockets.rs`, add a gateway with `#[subscribe_message("message")] fn on_message` and `#[subscribe_message("message")] fn on_message_v2` in its own module, and a test that builds it with `Test::create_testing_module()` (as `duplicate_names_fail_the_bootstrap` in `crates/corvis/tests/schedule.rs:299-305`) and asserts the exact error of `app.init()` from the spec; verify with `cargo test -p corvis --test websockets` that it fails on the current code because `init()` succeeds
- [x] 1.2 Add tests for the other scenarios: a guarded handler followed by an unguarded duplicate (error names both); a gateway implementing `OnGatewayInit` whose `after_init` sets a flag that stays unset after the failed `init()`; `"message"` and `"Message"` in one gateway (both answer their own frames); two gateways on different paths with `ping` (both answer); verify the error tests fail and the two success tests pass on the current code

## 2. Startup check

- [x] 2.1 In `build_gateways` (`crates/corvis/src/websockets.rs`, or `compile_gateways` if #27 has landed), before logging and inserting a `WsMessage` handler, look its event up in `handlers`; if present, return `DiError::Other` with the message from the spec, taking the first method name from the stored handler's `target.name` and the second from `h.method_name`; verify the group 1 tests pass
- [x] 2.2 Run the existing gateway tests and the examples' gateways (`cats-app`, `tertulia`) to confirm none trips the check; verify `cargo test -p corvis --test websockets` and `cargo build -p cats-app -p tertulia` pass

## 3. Docs

- [x] 3.1 `docs/websockets/gateways.md`, section `#[handlers]` and `#[subscribe_message]`: each event can have one handler per gateway; a second one makes startup fail with the message from the spec; event names are case-sensitive; same in `docs/es/websockets/gateways.md`; verify both pages say the same
- [x] 3.2 `docs/migration-guide.md`: a bullet next to "Global enhancers and gateways" explaining that the original's `ws` adapter keeps the last handler and its socket.io adapter runs all of them, and that Corvis refuses to start instead (remove the dead handler, or merge the two); same in `docs/es/migration-guide.md`; verify both pages say the same
- [x] 3.3 Run `npm run docs:check` in `docs/`; verify it passes

## 4. Definition of done

- [x] 4.1 `cargo build --workspace --all-targets` with no warnings
- [x] 4.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 4.3 `cargo test --workspace` passes, including the regression tests of group 1
- [x] 4.4 Docs updated in English and Spanish (`docs/` and `docs/es/`) and `npm run docs:check` passes in `docs/`
- [x] 4.5 Framework runtime code changed (`websockets.rs`): run the `corvis-parity` check and verify the three port apps keep 0 apidiff differences (none has a gateway, so no difference is expected) (not run by the implementer: the maintainer runs it) (maintainer batch run on batch/phase03-c cd79e44: consi 832/832, facten 727/727, houseplan 598/598 quiet-hours count; routes identical, databases equivalent)

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #125, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).
