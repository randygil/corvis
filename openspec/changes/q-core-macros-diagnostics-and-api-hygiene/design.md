# Design

## Context

Source: the engineering-quality review of the core recorded in the issue
(`quality/q-core-macros-diagnostics-and-api-hygiene.issue.md`). Code read on `dev` at `d46758a` (2026-11-11) and, for the
files #35 rewrites, on its branch (`wip-35` at `26e8513`): `corvis-macros/src/gateway.rs` (341 changed lines: options
parsed with errors), `websockets.rs`, `socket_io/gateway.rs` (new) and `posture.rs`. **Task 1.1 re-checks every line
below against `dev` with #35 and `q-core-lifecycle-and-cache-correctness` merged.**

### The findings, confirmed against the code

| Finding | Where now | Verdict |
|---|---|---|
| M5 `Result` decided by name | `corvis-macros/src/controller.rs:426-429` and `handlers.rs:71-76` (`last_ident(ty).ends_with("Result")`), used for routes (`controller.rs:430-441`), event and scheduled handlers (`handlers.rs:214`) and gateway handlers (`handlers.rs:338-346`) | Confirmed both ways: `-> SearchResult` (a struct) generates `match __ret { Ok(..), Err(..) }` and fails to compile; `type Out = Result<Cat, NotFound>` is treated as a value, so with `Cat` and `NotFound` both `Serialize` the route answers 200 `{"Err":...}`; for an event handler the alias's `Err` is dropped instead of logged. Fix as the review suggests (Decision 1); the probe machinery exists (`__private.rs:27`, `:296-359`) |
| S6 `#[http_code]` / `#[redirect]` without arguments | `controller.rs:235-238` (`&a[0]`), `:247-252` | Confirmed: the macro panics ("proc macro panicked", no useful span). Sibling: extra arguments are ignored (`#[http_code(201, 202)]`) |
| S6 `#[set_metadata]` arity span | `controller.rs:23-30` (`Span::call_site()`) | Confirmed. The same check in `handlers.rs:119-121` and `gateway.rs:52-55` already spans the attribute |
| S6 gateway arguments ignored | `gateway.rs:14-34` on dev | **Already handled by #35**: `wip-35` parses `path`, `namespace` and `cors` with errors for unknown keys and non-literals (`ACCEPTED`, `string_literal`). Nothing to do here; task 1.1 confirms after the merge |
| S6 `#[module(global = <not a literal>)]` | `module.rs:39-41` | Confirmed (silently `false`). Sibling: `#[init(a, b)]` ignores `b` (`injectable.rs:52-56`) |
| S11 log literals with runs of spaces | `router/cors.rs:194` (CORS startup warning), `modules/throttler.rs:1105` and `:1110` (unknown-override error) | Confirmed: three literals continued without `\`, about 18 to 22 spaces each. The CORS unit test checks only for `'\n'` (`router/cors.rs:375`). Other continued literals in the same files use `\` (`throttler.rs:1147-1148`) |
| N1 enhancer-attribute `match` three times | `controller.rs` (route and class attributes), `handlers.rs:100-130`, `gateway.rs:44-70` | Confirmed |
| N1 `resolve_all` twice | `router/mod.rs:350-359`, `websockets.rs:551` (`wip-35`: `:748`) | Confirmed, identical |
| N1 posture loops | `posture.rs:659-707` (routes) and `:708-757` (gateway handlers): exemptions, public source, guard summaries and status repeated | Confirmed; #35 edits `posture.rs` (78 lines), so the refactor is done on the merged file |
| N2 dead code and docs | `injectable.rs:116` (`let _ = format_ident!`), `:136` (`let _: Option<&Type> = None;`), `module.rs:108` (`let _ = Punctuated::new();`), `controller.rs:64` (`quote!(#a.left)` interpolates the whole assignment), `__private.rs:415-418` (doc says "`true` when" for a function returning a name) | Confirmed. The other N2 items are in the other two changes: `container.rs:753-756` and `application.rs:730` (lifecycle), `router/mod.rs:567` (hot path) |
| N4 string protocol of gateway replies | `websockets.rs:225-232` (`{"__corvis_same_event": true, "data": ..}`), `:802-810`, `:831`, `:940`, `AtomicU8` shape channel (`:802`, `:820`); on `wip-35` also `socket_io/gateway.rs:717` | Confirmed. No collision is reachable (user data is always nested under `data`), so this is hygiene, not a bug |
| Sibling: gateway values that fail to serialize become `null` | `websockets.rs:98` (`WsException::object`), `:160` (`WsResponse::new`), `:207` (`WsSerializeTag::convert`, a handler's reply) | Found while confirming (listed as a follow-up by `q-http-response-consistency`, whose HTTP equivalent becomes a `500`): the client gets `null` as if it were the value. Decision 5 |
| N5 TLS configuration per probe | `modules/health.rs:370-383` (root store of every WebPKI root and a `ClientConfig` built for each HTTPS probe) | Confirmed. A plain `LazyLock` would freeze the provider that happened to be installed first; Decision 7 caches per provider |
| N7 events | `modules/event_emitter.rs:77-79` (two `Vec` per listener per wildcard emit), `corvis-macros/src/handlers.rs:220-225` (payload of another type: `return Ok(())`, no log) | Confirmed |
| N8 `CronJob::from` | `modules/schedule/cron_job.rs:207-209` (`#[allow(clippy::should_implement_trait)]`, inherent `from` returning `Result`) | Confirmed. It mirrors `CronJob.from(params)` of the original cron library; `TryFrom` keeps the mirror readable (`CronJob::try_from(params)`). Decision 9 |
| N9 `__ctx` visible to `#[init(...)]` | `injectable.rs` (generated `construct(__ctx: ..)` with call-site hygiene) | Confirmed; nobody uses it (grep of the workspace, docs and examples) |
| N9 one large `routes()` per controller | `controller.rs` (all routes' closures and probes in one function) | Needs a measurement before any restructuring (Decision 10) |
| N9 fixed `::corvis::` paths | all generated code | **Dropped**: a renamed dependency is a feature request (a `crate = "..."` option, as serde has), not a defect; nobody asked for it |

## Goals / Non-Goals

**Goals:**

- Handlers compile and behave according to their return type, never its name.
- Every malformed attribute is a spanned compile error, never a panic or a silent default.
- One implementation of each duplicated rule; no statements whose only job is to silence a warning.
- Internal protocols typed instead of encoded in JSON keys.

**Non-Goals:**

- New attributes or options; a configurable crate path.
- Changing what a handler's value answers (the `http/responses` rules stay).

## Decisions

### 1. Results recognized by type

Generated code stops branching on the name. For a route, after the call:

```rust
let __ret = (&__ret).__corvis_result_kind().split(__ret);
match __ret {
    Ok(__v) => (&__v).__corvis_reply_kind().convert(__v),
    Err(__e) => Err((&&::corvis::__private::err_probe(&__e)).__corvis_err_kind().convert(__e)),
}
```

`__private` gains two hidden traits with the autoref probe already used for replies: `ResultKind` implemented for
`Result<T, E>` (`fn __corvis_result_kind(&self) -> ResultTag`, `ResultTag::split(self, r: Result<T, E>) -> Result<T, E>`)
and `PlainKind` implemented for `&X` (`PlainTag::split(self, x: X) -> Result<X, Infallible>`). Method resolution on
`&__ret` picks `ResultKind` for any `std::result::Result`, under any name or alias, and `PlainKind` for everything else,
because the concrete type is known where the generated code is type-checked. The error probes gain an arm for
`Infallible` (an uninhabited error converts to nothing; the arm is unreachable). Event and scheduled handlers use the same
split before `map_err`; gateway handlers before their `Ok`/`Err` conversion. `is_result` and `last_ident(..).ends_with(..)`
go from both files.

Verified by `crates/corvis/tests/return_types.rs` (fails to compile before the fix): a route returning `SearchResult`
(struct) and `LoginResult` (enum) answers their JSON; `type Out = Result<Cat, NotFound>` with both sides `Serialize`
answers the `NotFound` status and body when `Err`; `corvis::Result<T>`, `HttpResult<T>` and `HandlerResult` behave as
today; an `#[on_event]` handler returning `TaskResult` (struct) runs; an `#[on_event]` handler returning the alias logs
its `Err`; a gateway handler returning `SearchResult` replies with it. The existing macro unit tests and every suite that
compiles controllers pass unchanged.

Alternative considered: require generic arguments (`Foo<..>` ending in `Result`). Rejected: still a name rule; an alias
without generics (`type Out = Result<A, B>`) stays wrong.

### 2. Attribute errors with spans

- `#[http_code(..)]`: exactly one argument; zero or more than one is `syn::Error::new_spanned(&attr, "#[http_code(status)]
  expects one argument")`.
- `#[redirect(..)]`: one or two arguments, same style of message.
- `metadata_tokens` takes the attribute and spans the error on it.
- `#[module(global = ..)]`: a boolean literal (`true` / `false`); anything else is an error on the value ("`global`
  expects `true` or `false`").
- `#[init(..)]`: exactly one expression; more is an error on the attribute.

Verified by unit tests in `corvis-macros` (the pattern of the existing `m[0].starts_with(..)` tests: expand, collect the
errors, check the message), one per case, and a `compile_fail` doctest per attribute in `crates/corvis/src/doctests.rs`
so the user-facing error is pinned. Before the fix, the `http_code` and `redirect` cases panic and the `global` and
`init` cases expand without error.

### 3. One implementation of each duplicated rule

- `corvis-macros/src/utils.rs`: `enhancer_attr(attr, ctx) -> syn::Result<Option<Vec<TokenStream>>>` parses
  `use_guards`, `use_interceptors`, `use_pipes`, `use_filters`, `public` and `set_metadata` into builder calls; the three
  call sites use it, keeping their own placement rules (which attributes are accepted where) and their error messages.
- `common/enhancer.rs`: `pub(crate) async fn resolve_all(..)`, used by `router/mod.rs` and `websockets.rs`.
- `posture.rs`: one `fn entry(.., guards, global_count, owner_count, targets, matches_exemption, scope_of) -> PostureEntry`
  used by both loops; the report is unchanged.

Verified by: the macro unit tests, `route_posture.rs` and the posture unit tests pass unchanged; the posture report of the
three example apps (`CORVIS_ROUTE_POSTURE_REPORT`) is byte-identical before and after (task 4.3 diffs them).

### 4. Dead code and docs

The three silencing statements go (with the imports they kept alive, if unused); `controller.rs:64` interpolates
`a.left`; the rustdoc of `__private::type_name_of` says it returns the type's name for error messages. `__ctx` in the
generated `construct` gets `Span::mixed_site()`, so an `#[init(...)]` expression cannot name it (nothing in the workspace
does). Verified by `cargo build` without warnings and the macro unit tests.

### 5. Typed gateway replies

The handler result type used between generated code and the dispatchers becomes a hidden enum
`WsHandlerReply { None, Data(Value), Response { event: String, data: Value } }` (what `WsReply` already distinguishes
before `encode_reply` flattens it). `encode_reply`, the `__corvis_same_event` key, the `SHAPE_*` constants and the
`AtomicU8` channel go: the interceptor chain carries the enum alongside the `Response` it shows interceptors (the
interceptors still see the JSON body they see today: the data for a plain value, `{ event, data }` for a response, `null`
for none). The Socket.IO adapter's `classify` (#35) matches the enum.

Values that fail to serialize stop turning into `null`, as `q-http-response-consistency` does for HTTP (a `500`): a
handler reply whose serialization fails becomes an internal error that goes through the exception path (filters, then the
`exception` frame with `Internal server error`, logged); `WsException::object` with such a value builds an internal-error
exception instead of a `400` with `null`; `WsResponse::new` keeps its signature and, on failure, logs an error and carries
`null` (it cannot return an error; its rustdoc says to serialize first when the value can fail).

Verified by: the gateway suites (`websockets.rs`, `ws_enhancers.rs`, `ws_security.rs`, `auth_ws.rs`), the Socket.IO
gateway suites added by #35 and the `ws_gateways` fixture replay pass unchanged; a handler returning a value whose
`Serialize` fails answers the `exception` frame with `Internal server error` (fails before: a reply with `null`).

### 6. Log literals

The three literals use `\` continuation. The CORS unit test asserts the warning contains no run of two spaces; a
throttler log test (`throttler_skip_log_ws.rs`, which captures the line) asserts the same. Before the fix both new
assertions fail.

### 7. Health: TLS configuration per provider

`tls::connect` keeps a `static CONFIG: Mutex<Option<(usize, Arc<ClientConfig>)>>` keyed by the address of the crypto
provider in use (`CryptoProvider::get_default()` or the ring fallback); the root store and the configuration are built
when the key changes, otherwise the `Arc` is reused. The "never panic" rule of the existing comment stays. Verified by:
the health suites (`health.rs`, `health_panic_log.rs`) pass; a unit test calls the configuration getter twice and gets the
same `Arc` (`Arc::ptr_eq`).

### 8. Events

- Wildcard matching walks both split iterators without collecting when the pattern has no `**`; with `**`, it collects
  only the event's segments once per emit (not per listener). Verified by the existing wildcard tests and a property test
  comparing the new matcher with the old one (kept as the oracle in the test module).
- A listener whose payload is of another type logs one warning per listener and payload type (`"<Class>.<method>
  listens to <event> with <Type> but received <OtherType>; skipped"`), remembered in a set bounded by the number of
  listeners times the payload types seen (the set stops growing at 1,024 entries). Verified by a capturing-logger test:
  two emits of a mismatched type give one warning.

### 9. `CronJob::try_new` and `TryFrom`

`CronJob::from(p)` is removed; `CronJob::try_new(p) -> Result<CronJob, CronError>` does what it did, and
`impl TryFrom<CronJobParams> for CronJob` calls it (`#[doc(alias = "from")]` on `try_new` so a search for the original's
name finds it). The `clippy::should_implement_trait` allow goes. Every use in the workspace, the docs and the examples is
updated (a grep of `CronJob::from` over `crates/`, `docs/` and `examples/` lists them). This is a breaking change made before 1.0, stated in the scheduling docs.

### 10. Size of the generated `routes()`: measure, then decide

A unit test in `corvis-macros` expands a 20-route controller with three parameters per route and reports the token count
and the number of closures of the generated `routes()`; task 6.2 records the figure (no restructuring in this change).
If the function exceeds 10,000 tokens, task 6.2 drafts a follow-up issue (one generated function per route, with a
compile-time measurement of a crate that has such controllers) in Implementation notes for the maintainer.

## Dropped

- **Configurable crate path** (N9): `::corvis::` in generated code assumes the dependency is named `corvis`. A
  `crate = "..."` option would be a feature; no user or port renames the crate.
- **Gateway argument parsing** (S6, `gateway.rs`): done by #35.

## Security invariants this change comes near

| Invariant | Source | Decisions | Pinned by |
|---|---|---|---|
| Enhancer attributes on a method without a route or handler attribute are compile errors (an unguarded route cannot be written by mistake) | `core/enhancers` | 2, 3 | the `compile_fail` doctests of `doctests.rs` and `lib.rs`, the macro unit tests, unchanged |
| `#[public]` and guard placement read by the route posture | `security/route-posture` | 3 | `route_posture.rs`, posture report diff of task 4.3 |
| An error returned by a handler answers its status, never a 200 with the error serialized | `http/responses` | 1 | `return_types.rs` |
| Gateway handler errors go through the exception path (filters, `exception` frame), never as data | `ws/exception-filters` | 5 | `ws_enhancers.rs`, `ws_gateways` fixture replay |
| Health probes never panic on TLS setup; redirects keep the SSRF policy | `reliability/health-checks` | 7 | `health.rs`, `health_panic_log.rs` |
| Log lines a client can provoke stay bounded | event listeners | 8 | the warning set's cap test |

## Behavior visible to applications

| What | Before | After | Consequence |
|---|---|---|---|
| Handler returning a type named `*Result` that is not `Result` | Compile error in generated code | Compiles; answered as its value | Code that could not be written now works |
| Handler returning a `Result` alias without the suffix | Answered as a value (`{"Ok":..}` / `{"Err":..}` with 200 when both serialize) | `Err` answers its status | A route that returned 200 for errors now returns the error; listed in the docs' changelog line |
| `#[http_code]` / `#[redirect]` without arguments | Macro panic | Spanned compile error | Better message |
| `#[module(global = CONST)]`, `#[init(a, b)]` | Silently `false` / `b` ignored | Compile error | Code that relied on the silent default must be fixed (none in the workspace) |
| `CronJob::from(params)` | Inherent method | Removed; `CronJob::try_new` / `TryFrom` | Breaking before 1.0; the compiler names each call |
| Event listener receiving another payload type | Skipped silently | Skipped with one warning per listener and type | Log line |
| Gateway handler reply or `WsException::object` that fails to serialize | Sent as `null` (reply) or a `400` with `null` | `exception` frame `Internal server error`, logged | Matches the HTTP rule of `q-http-response-consistency` |
| CORS and throttler log lines | Runs of spaces | Clean text | Log text only |

## Risks / Trade-offs

- The autoref split must pick `ResultKind` for every `Result` and only for it → the tests of Decision 1 cover struct, enum,
  alias, `corvis::Result`, `HttpResult`, `HandlerResult`; the trick is the same one the reply conversion has used since the
  first release.
- Removing `CronJob::from` breaks callers → the compiler points at each; the docs say what to write instead.
- Refactoring the posture loops could change the report → the byte-for-byte diff of task 4.3 and the posture suites.
- Typed gateway replies touch both adapters → every gateway suite of both adapters and the fixture replay run.

## Open questions

1. Keep `CronJob::from` as a deprecated alias for one release? Recommendation: no; the project is before 1.0 and the
   compiler finds every call.
2. Should a mismatched event payload be an error instead of a warning? Recommendation: a warning; the emitter has no way to
   reject an emit, and the original has no types to mismatch.
3. Should `#[set_metadata]` accept a single argument as a flag (`true`)? Recommendation: no; out of scope, the arity error
   is enough.

## Review notes

These settle the open questions and override the design where they differ.

1. `CronJob::from` is removed without a deprecated alias.
2. A mismatched payload logs a warning, once per listener and type, capped.
3. `#[set_metadata]` keeps exactly two arguments.
4. Implemented after #35 and `q-core-lifecycle-and-cache-correctness`; task 1.1 is mandatory.
