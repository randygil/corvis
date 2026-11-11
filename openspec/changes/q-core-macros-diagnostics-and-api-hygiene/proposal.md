# Proposal

Implements the issue filed from `quality/q-core-macros-diagnostics-and-api-hygiene.issue.md` (epic
[#187](https://github.com/randygil/corvis/issues/187), code quality audit). Source: the engineering-quality review of the
core of the framework, read against `dev` at `d46758a`. Third of three changes cut from that review; it lands after
`q-core-lifecycle-and-cache-correctness` (both edit `modules/schedule/cron_job.rs`) and after the gateway adapter (#35,
`openspec/changes/e2-socket-io-gateway-adapter/`), which rewrites `corvis-macros/src/gateway.rs`, much of `websockets.rs`
and the gateway part of `posture.rs`. It does not depend on `q-core-request-hot-path`; if both are in flight, the one that
lands second rebases `router/mod.rs` (one helper moves, Decision 3).

## Why

- **The macros decide what a `Result` is by its name.** A handler that returns a DTO named `SearchResult` or
  `LoginResult` gets code that matches `Ok` and `Err` on it and fails to compile with a type error inside generated code
  (`corvis-macros/src/controller.rs:426-429`, `handlers.rs:71-76`). The reverse also happens: an alias such as
  `type Out = Result<Cat, NotFound>` is not recognized; when both sides serialize, the handler answers 200 with
  `{"Err": ...}` instead of the error's status.
- **Some attribute mistakes panic the macro or are ignored.** `#[http_code]` and `#[redirect]` without arguments index an
  empty list and panic (`controller.rs:237`, `:250`); `#[set_metadata]` with the wrong arity points at the whole item
  (`:25`); `#[module(global = SOME_CONST)]` is silently `false` (`module.rs:39-41`); `#[init(a, b)]` ignores `b`.
- **Log lines with runs of spaces** in the CORS startup warning and two throttler errors (string literals continued
  without `\`), which the CORS test does not catch.
- **Duplication and dead code**: the enhancer-attribute `match` written three times in the macros, `resolve_all` twice,
  the two posture loops; statements that only silence warnings; a rustdoc that describes another function.
- **API hygiene left by the review:** the gateway reply travels as a JSON object with a magic key plus a side channel;
  the health check rebuilds its TLS configuration on every probe; event wildcards allocate per listener per emit and a
  listener whose payload type does not match is skipped without a word; `CronJob::from` shadows `From` and returns a
  `Result`.

## What Changes

- **Results recognized by type.** The generated code asks the type, with the autoref probe already used for replies:
  `std::result::Result<T, E>` (under any name or alias) is split into its value and error, anything else is a value. The
  name of the type plays no part.
- **Attribute errors with spans.** Every malformed attribute named above is a compile error that points at the attribute
  and says what it expects.
- **Log literals** continued with `\`; the CORS test asserts no run of two spaces.
- **One implementation each** of the enhancer-attribute parsing (macros), `resolve_all` (router and gateways) and the
  per-entry posture logic (routes and gateway handlers).
- **Dead code and docs**: the silencing statements go, `controller.rs:64` interpolates the left side only, the rustdoc of
  `__private::type_name_of` says what it returns, `__ctx` in `#[init(...)]` expressions is no longer nameable by user code.
- **Typed gateway replies**: the internal handler result is an enum; the magic `__corvis_same_event` key and the
  `AtomicU8` side channel go (in the `ws` adapter and in the Socket.IO adapter added by #35). A reply or a
  `WsException::object` value that fails to serialize becomes an internal error instead of `null`.
- **Health**: the TLS client configuration is built once per crypto provider.
- **Events**: wildcard matching without allocations; a listener skipped because the payload is of another type logs one
  warning per listener and type.
- **`CronJob`**: `CronJob::from(params)` is replaced by `CronJob::try_new(params)` and `impl TryFrom<CronJobParams>`
  (breaking, before 1.0).

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `http/responses`: added requirement that handler results are recognized by type, not by name.
- `core/enhancers`: added requirement that malformed route, module and injectable attributes are compile errors that
  point at the attribute.

## Impact

- New: `crates/corvis/tests/return_types.rs`; `compile_fail` doctests in `crates/corvis/src/doctests.rs`.
- Edited: `crates/corvis-macros/src/{controller,handlers,gateway,module,injectable,utils}.rs`,
  `crates/corvis/src/__private.rs`, `crates/corvis/src/websockets.rs`, `crates/corvis/src/socket_io/gateway.rs` (after
  #35), `crates/corvis/src/router/{mod,cors}.rs`, `crates/corvis/src/common/enhancer.rs`, `crates/corvis/src/posture.rs`,
  `crates/corvis/src/modules/{throttler,health,event_emitter}.rs`, `crates/corvis/src/modules/schedule/{mod,cron_job}.rs`,
  the scheduling docs page and its Spanish version.
- Public API: `CronJob::from` is removed in favor of `CronJob::try_new` and `TryFrom<CronJobParams>` (code calling
  `CronJob::from(params)` changes to `CronJob::try_new(params)`; the compiler points at every call); hidden `__private`
  items used by generated code change shape (not part of the API). Handlers that did not compile now compile; handlers
  returning a `Result` alias without the suffix answer their error status instead of `{"Err": ...}` (design.md "Behavior
  visible to applications").
- Dependencies: none.
- Framework runtime code and the macros change, so the corvis-parity check is run (the ports compile their controllers
  with the new macros).

## Out of scope

- Configurable crate path in generated code (`::corvis::` is fixed): a feature request, not a defect (design.md "Dropped").
- Splitting the generated `routes()` function per route: measured here (Decision 10), restructured only in a follow-up if
  the measurement shows a large function.
- The per-request cost of the router and the gateways (`q-core-request-hot-path`).
