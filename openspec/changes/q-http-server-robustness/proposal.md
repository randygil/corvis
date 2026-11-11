# Proposal

Part of epic [#187](https://github.com/randygil/corvis/issues/187) (code quality audit); implements the issue filed from
`quality/q-http-server-robustness.issue.md`. Source: an engineering-quality review (not a security review) of the HTTP
layer, the server and the multipart module, read against `dev` at `d46758a`. The review's findings were split by theme
into four changes, in this landing order:

1. **`q-http-server-robustness` (this change)**: the accept loop, the graceful shutdown, `@Res()` responses that nobody
   can end, the test client's own copy of the request path, and the split of `server.rs`.
2. `q-http-response-consistency`: one implementation of each helper that reproduces an original package (cookies,
   `encodeurl`, `fresh`, `vary`, ...) settled by recorded fixtures, and public response APIs that return errors instead
   of panicking or answering `null`.
3. `q-http-request-hot-path`: a benchmark, then a request that clones in constant time, a JSON parser that works on
   UTF-8, and fewer copies when a response is finished.
4. `q-http-multipart-streaming`: the `StorageEngine` trait receives a stream, the parser stops copying files, and file
   buffers live with their request.

This change touches neither `Request` nor the response types, so it composes with
`openspec/changes/e3-authentication-hot-path-and-consolidation/` and `openspec/changes/e2-socket-io-hot-path-and-api-hygiene/`
in any order (design.md "Order with the other changes").

## Why

The review found no vulnerability in this area. It found operational gaps that hit a production server exactly when
it is under pressure or going down, and structure that makes the security-critical code harder to review:

- **The accept loop spins.** `serve` answers every error of `listener.accept()` with `continue`
  (`crates/corvis/src/server.rs:2302-2306`). When the process runs out of file descriptors (`EMFILE`, `ENFILE`) the call
  fails at once, again and again: one worker burns a core, nothing is logged, and the connections that would free
  descriptors get less CPU.
- **The graceful shutdown has a fixed deadline and leaves work running.** `serve` waits at most 10 s for the
  connections (`server.rs:2347-2353`) and then returns while their tasks, spawned without being tracked
  (`server.rs:2323`, `2328`, and the body drains of `server.rs:607`), keep running. The application then runs its
  shutdown hooks (`application.rs:653-655`, `687-688`): a handler still running after 10 s uses database pools and
  other providers that `on_module_destroy` already closed. The deadline cannot be configured and no test covers a
  request in flight during the shutdown.
- **An `@Res()` handler that returns without ending the response waits for ever.** The router waits on the response's
  notification with no bound (`res.rs:857-866`, `router/mod.rs:730-737`), and when no handle that could end it is left
  nothing ever will. The client gives up eventually, as with the original framework, but until then the request holds
  an HTTP/2 handler permit (`server.rs:2174`) and its share of the connection's response budget.
- **The test client has its own copy of the request path** (`testing.rs:383-399` against `server.rs:551-615`): it skips
  the body limits, the pending multipart body and the drain, so a test can pass against behavior the server does not
  have. Tests written with `TestClient` never run `from_hyper`.
- **`from_hyper` can never fail** (`server.rs:471-529`), so `Err(resp) => resp` (`server.rs:612`) is dead code hidden by
  `#[allow(clippy::result_large_err)]`.
- **`server.rs` has 2,897 lines** mixing the options, the request conversion, the drain, the response budget, the
  rapid-reset rule, `GuardedBody`, `TimedIo` (more than 700 lines), the watchdog and the accept loop; `ConnState`
  (`server.rs:621-667`) holds 19 fields of five concerns. Every security change of epic #1 edited this file.

## What Changes

- **Accept errors back off.** A per-connection error (`ConnectionAborted`, `ConnectionReset`, `ConnectionRefused`,
  `Interrupted`) is retried at once as today. Any other error waits 5 ms, doubling up to 1 s, until an accept succeeds,
  and is logged as a warning at most once a minute with the number of errors since the last line.
- **A configurable shutdown that ends what it started.** `ServerOptions::shutdown_timeout(Option<Duration>)`, default
  10 s (today's value), `None` to wait without limit. Connection tasks, TLS handshakes and body drains are tracked; when
  the deadline passes, whatever is left is cancelled and awaited before `serve` returns, so no handler of the server runs
  when the application's shutdown hooks start. One warning line says how many connections were cut.
- **An `@Res()` response nobody can end is answered.** When the handler (and the interceptors) have finished and no
  handle of the response is left outside the framework, the request is answered with the generic `500` and one error
  line, instead of waiting until the client leaves. A handler that hands its `Res` to a task that ends it later works as
  today; if that task drops the last handle without ending the response, the `500` is sent then.
- **One request path for the server and the test client.** `server::respond` (the conversion with its limits, the
  pending multipart body, the dispatch, the drain and the response finishing) is used by `handle` and by
  `TestRequest::send`. The pending body holds a boxed body instead of hyper's `Incoming`, so the test client feeds the
  same path with its in-memory body.
- **`from_hyper` returns `Request`**; the dead arm and the `allow` go.
- **`server.rs` split by concern** into `server/{options,request,response,budget,rapid_reset,guarded_body,timed_io,
  watchdog}.rs`, with `ConnState` grouped into one struct per concern. Pure moves: the same tests, the same count.
- **Small `Res` fixes met on the way**: `finish` notifies once; `take_response` moves the headers instead of cloning them.

## Capabilities

### New Capabilities

- `http/server-lifecycle`: how the built-in server accepts connections and shuts down.

### Modified Capabilities

- `http/responses`: added requirement for an `@Res()` response that nobody can end.
- `core/testing`: added requirement that the test client reads a request like the server.

## Impact

- New: `crates/corvis/src/server/{options,request,response,budget,rapid_reset,guarded_body,timed_io,watchdog,accept}.rs`,
  `crates/corvis/tests/server_lifecycle.rs`.
- Edited: `crates/corvis/src/server.rs`, `crates/corvis/src/http/res.rs`,
  `crates/corvis/src/router/mod.rs`, `crates/corvis/src/testing.rs`, `crates/corvis/src/multer/mod.rs` (`BodySource`
  holds the boxed body), `crates/corvis/Cargo.toml` (`tokio-util` with `rt`), `Cargo.lock`, tests that relied on the test
  client ignoring the body limits (task 6.4), and in `docs/` and `docs/es/`: `deployment.md`,
  `fundamentals/lifecycle-events.md`, `controllers.md`.
- Public API: one addition, `ServerOptions::shutdown_timeout`. Nothing changes signature.
- Behavior visible to applications: listed in design.md "Behavior visible to applications". The ones that matter: at
  the shutdown deadline in-flight handlers are cancelled instead of left running; an `@Res()` response nobody can end is
  a `500`; the test client answers `413` for a body over the parser limit, as the server does.
- Dependencies: `tokio-util` becomes a direct dependency (`rt` feature for `TaskTracker`). It is already in `Cargo.lock`
  at 0.7.20 through `h2`, under MIT, allowed by `deny.toml`: no new crate is compiled.
- Framework runtime code changes (`server.rs`, `router/mod.rs`, `res.rs`): the corvis-parity check runs.

## Out of scope

- Closing upgraded WebSocket sessions at shutdown (`websockets.rs:758` spawns them untracked): owned by
  `q-core-lifecycle-and-cache-correctness` (Decision 6 there: close frame `1001` from `close()`), which also restructures
  `listen_with` and stops the background servers, and lands after this change.
- Rewriting the 52 `sleep` calls of the server's integration suites (`response_memory.rs`, `http2_limits.rs`,
  `send_timeout.rs`, `transport_security.rs`, `http1_framing.rs`): a follow-up issue; new tests of this change follow the
  timing rule.
- The watchdog's timer per response, the framing check's copies of the head, and the two layers of `headers_timeout`
  (design.md "Findings as checked").
