# Tasks

## 1. Reference behavior and failing tests

- [x] 1.1 Add `crates/corvis/tests/fixtures/ws_gateways/` with a `package.json` pinning `@nestjs/core`,
  `@nestjs/common`, `@nestjs/websockets`, `@nestjs/platform-ws`, `ws`, `reflect-metadata` and `rxjs`, and a
  `generate.mjs` that starts a gateway on the `ws` adapter (bound to `127.0.0.1`) and records the frames of the
  scenarios in design.md, Decision 7, into `ws_gateways.json`; if the recorded behavior differs from the spec deltas
  (global filters applied, guards or pipes not applied, a different filter order), stop and record it under Open
  questions in design.md; verify by running `node generate.mjs` once and committing the JSON (not `node_modules`)
- [x] 1.2 Add failing tests in `crates/corvis/tests/websockets.rs` (or a new `ws_enhancers.rs`), one per spec scenario
  group: global guard refusal, global guard order, `ctx.get_type()` guard, public metadata, global pipe, global filter
  not applied, switch on/off, filter order, frame translation (JSON, text, empty, stream, panic), filter host, `id`
  echo, internal-error logging; verify with
  `cargo test -p corvis --test websockets` that they fail for the expected reason on the current code
- [x] 1.3 The workspace has no compile-fail harness (no `trybuild` dependency): add a unit test in
  `crates/corvis-macros` that calls `handlers::expand` on an `impl` with `#[use_filters(F)]` on an `#[on_event]`
  method and asserts the error message; verify with `cargo test -p corvis-macros`

## 2. Macros

- [x] 2.1 Add `filters: Vec<FilterRef>` and `.filter()` to `WsEnhancers` (`crates/corvis/src/discovery.rs`); verify
  `cargo test -p corvis --lib` still passes
- [x] 2.2 In `crates/corvis-macros/src/gateway.rs`, map `#[use_filters]` to `.filter(..)` instead of the compile error;
  in `crates/corvis-macros/src/handlers.rs`, accept `use_filters` on `#[subscribe_message]` methods; update the
  `use_filters` message in the `misplaced!` list (`crates/corvis-macros/src/lib.rs`); verify the compile-fail case of
  1.3 and that a gateway with filters at both levels compiles

## 3. Runtime

- [x] 3.1 Add `gateway_global_enhancers: bool` (default `true`) to `RouterConfig` and
  `CorvisApplication::apply_global_enhancers_to_gateways(bool)`, which logs a warning when called after `init()`;
  verify the "switch called too late" test
- [x] 3.2 In `build_gateways` (`crates/corvis/src/websockets.rs`), use `cfg`: prepend `cfg.global_guards`,
  `cfg.global_interceptors` and `cfg.global_pipes` when the switch is on; resolve gateway and handler filters into
  `CompiledWsHandler::filters` (handler first, then gateway, inner order as #17 settles for HTTP); verify the global
  guard, order, pipe and switch tests pass
- [x] 3.3 Move the `ExecutionContext::for_ws` construction from `run_handler` to `dispatch`; on error, try the filters
  (first `catches`), translate the returned `Response` as in design.md, Decision 3, catch filter panics, and log
  internal errors only when the built-in mapping handles them; verify the filter order, frame translation and logging
  tests pass, and `crates/corvis/tests/ws_security.rs` still passes
- [x] 3.4 Add the Rust test that replays the fixture of 1.1 against a Corvis gateway and compares frames (ignoring
  `cause` and `id`); verify `cargo test -p corvis --test websockets`

## 4. Example

- [x] 4.1 In `examples/cats-app/src/chat.rs`, add `#[set_metadata(IS_PUBLIC_KEY, true)]` below
  `#[websocket_gateway]` and update the doc comment (public gateway; the global throttler counts messages); verify
  by running the example on `127.0.0.1` and sending `join` and `message` with `websocat` and no token

## 5. Docs

- [x] 5.1 `docs/websockets/exception-filters.md`: replace the "not supported" text with `#[use_filters]` on gateways
  and handlers, the order, the frame translation table and the empty-body rule; note that global filters do not apply;
  same in `docs/es/websockets/exception-filters.md`
- [x] 5.2 `docs/websockets/gateways.md` (section "Global enhancers"), `guards.md`, `pipes.md`, `interceptors.md`:
  global guards, interceptors and pipes now apply, the order, `ctx.get_type()`, public metadata and the switch;
  same in `docs/es/websockets/`
- [x] 5.3 `docs/migration-guide.md` (gateways bullet, behavior change), `docs/deployment.md` (checklist line on
  WebSockets), `docs/security/rate-limiting.md` (limitation about gateways becomes: the global throttler counts
  messages per handshake IP); same in `docs/es/`
- [x] 5.4 Run `npm run docs:check` in `docs/`; verify it passes

## 6. Definition of done

- [x] 6.1 `cargo build --workspace --all-targets` with no warnings
- [x] 6.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 6.3 `cargo test --workspace` passes, including the regression tests of 1.2, the macro test of 1.3 and the
  differential replay of 3.4
- [x] 6.4 Docs updated in English and Spanish (`docs/` and `docs/es/`) and `npm run docs:check` passes in `docs/`
- [x] 6.5 Framework runtime code changed: run the `corvis-parity` check and verify the three port apps keep 0 apidiff
  differences (none of them has a gateway) (not run by the implementer: the maintainer runs it) (maintainer batch run on batch/phase04-f 00df2bb: consi 832/832, facten 727/727, houseplan 636/636; routes identical, databases equivalent)

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #116, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).
