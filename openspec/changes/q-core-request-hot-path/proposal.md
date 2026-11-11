# Proposal

Implements the issue filed from `quality/q-core-request-hot-path.issue.md` (epic
[#187](https://github.com/randygil/corvis/issues/187), code quality audit). Source: the engineering-quality review of the
core of the framework, read against `dev` at `d46758a`. Second of three changes cut from that review: it lands after
`q-core-lifecycle-and-cache-correctness` and before (or beside) `q-core-macros-diagnostics-and-api-hygiene`.

**Dependencies, stated plainly:**

- **The representation of `Request` is not designed here.** `openspec/changes/q-http-request-hot-path/` (epic #187)
  makes cloning a `Request` O(1) (body, query and headers behind `Arc`), captures the three CORS inputs before dispatch
  (`CorsRequest`, removing `req_for_cors`) and shares the route path as `Arc<str>` with `CompiledRoute::path`. This change
  is written against the contract **"cloning a `Request` is O(1)"**, lands after it, and does not touch those items. It
  owns what the router's lookup, the middleware chain, the throttler, the request scope and the reflector do per
  request.
- **The plain WebSocket gateway work lands after** the gateway adapter (#35,
  `openspec/changes/e2-socket-io-gateway-adapter/`: `ExecutionContext::for_ws_shared`, the message body shared as one
  `Arc<Value>`) and after `openspec/changes/e2-socket-io-hot-path-and-api-hygiene/` (its task 5.4 moves
  `tokio-tungstenite` to a release whose text frames share their bytes). Task group 8 starts only when both are merged.

## Why

The review measured nothing; it estimated about 20 to 25 allocations for a plain `GET /cats/:id`, which is reasonable,
and found that the costs jump with each feature an application turns on:

- **CORS** copies the whole `Request` on every request only to compute headers from three of its headers
  (`router/mod.rs:589`): fixed by `q-http-request-hot-path`; this change's benchmark checks the result.
- **Module middleware** is matched on every request against every pattern, and the matcher builds two vectors per pattern
  and joins strings for globs (`router/mod.rs:297-300`, `common/middleware.rs:236-251`); `for_routes(SomeController)`
  becomes one pattern per route of the controller and per version (`router/mod.rs:540-545`).
- **The throttler** rebuilds per request what is fixed per route and per throttler: metadata keys, header names, the
  stored key (about 25 allocations per throttler by the review's count), and every hit goes through one global mutex.
- **The request scope** builds the resolution context and the factory's future before it looks in the request's store,
  so every resolution of an instance that already exists pays for a construction (`di/container.rs:556-620`).
- **`Reflector::get`** deep-copies the metadata value before deserializing it (`common/reflector.rs:16`), on every guard
  or interceptor that reads metadata.
- **The router** allocates where it could borrow: path normalization and lowercasing, a copy of the candidate list, a
  filtered list per request (`router/mod.rs:259-287`, `679-697`).
- **WebSocket broadcasts** deep-copy and serialize the payload once per recipient, and `to(room)` walks every client of
  the server and locks each one's room set (`websockets.rs:261-264`, `486-522`); each received message copies its text
  and deep-copies its `data` (`websockets.rs:1060`, `1083`).

There is no benchmark of the request pipeline, so the first task group builds one that counts allocations per request,
and every decision names the count it must move.

## What Changes

- **Benchmark.** `bench/pipeline/`: an in-process benchmark (workspace member, `publish = false`) with a counting
  allocator that reports allocations and bytes allocated per request for a plain route, a route with a JSON body (1 KiB
  and 64 KiB), with CORS, with module middleware, with the throttler (one and three throttlers) and with a request-scoped
  provider, plus a multi-worker throughput mode for the throttler's lock. It complements the socket-level `bench/http/`
  of `q-http-request-hot-path` (design.md Decision 1 says why both). Run before and after; numbers go to design.md.
- **Middleware patterns are compiled at startup** into an allocation-free matcher with exactly the current matching rules,
  deduplicated per middleware group; the request path is split once; a request that no module middleware applies to
  reuses a chain built at startup.
- **Throttler**: per-throttler metadata keys, skip key and header names built once when the storage is built; the stored
  key hashed incrementally to the same SHA-256 digest; override values deserialized without copies. Sharding the counters
  is decided by the throughput measurement, with its invariants written down; it is not part of this change unless the
  measurement crosses the threshold of design.md Decision 6, and then it becomes its own change.
- **Request scope**: the request's store is checked before anything is built; `Provider::name` becomes `Arc<str>`;
  `ModuleRef` and `Scoped<T>` stop allocating on success.
- **Reflector** deserializes from `&Value`.
- **Router**: borrowed lookups (no copy of the candidate list, no normalization string when the path needs none), host
  filtering without a vector.
- **WebSocket gateways**: a broadcast serializes its frame once and every recipient gets a shared copy (a value that
  fails to serialize is logged and sent to nobody, instead of being sent as `null`); the server keeps a room index so
  `to(room)` costs the room's size; received messages are parsed without copying the text and their `data` is moved out
  of the parsed object.

## Capabilities

### New Capabilities

- `bench/request-pipeline`: the request pipeline benchmark (scenarios, what it reports, where it builds, how results are
  recorded).

### Modified Capabilities

- `http/routing`: added requirement that compiled middleware patterns match exactly as the pattern rules say.
- `security/rate-limiting`: added requirement that the stored key is the SHA-256 digest of the documented key string,
  however it is computed.
- `core/injection-scopes`: added requirement that resolving an existing request-scoped instance builds nothing.
- `ws/gateways`: added requirements for broadcasts encoded once and room emits that reach exactly the room's members.

## Impact

- New: `bench/pipeline/{Cargo.toml,src/main.rs,run.sh,README.md}` (workspace member), `crates/corvis/tests/
  pipeline_allocations.rs`, `crates/corvis/tests/ws_broadcast_memory.rs`.
- Edited: `crates/corvis/src/router/mod.rs`, `crates/corvis/src/common/{middleware,reflector,param,context}.rs`,
  `crates/corvis/src/di/{container,provider,module_ref,request_scope}.rs`, `crates/corvis/src/modules/throttler.rs`,
  `crates/corvis/src/websockets.rs`, `crates/corvis/src/fuzz_tests.rs`, `Cargo.toml` (workspace member), `.gitignore`,
  `bench/README.md`.
- Public API: none changes signature. `HttpArgumentsHost` gains no public method (the throttler uses a crate-private
  setter with a typed name and value).
- Behavior visible to applications: a broadcast or `WsClient::emit` whose data fails to serialize is logged and sent to
  nobody instead of being sent with `"data": null` (design.md "Behavior visible to applications"); nothing else is
  observable except allocations and speed.
- Dependencies: none. No new crate (`sha2` is already a dependency; incremental hashing is its normal API).
- Framework runtime code changes, so the corvis-parity check is run. The router, the middleware matcher and the
  throttler are security-relevant: the security review is mandatory.

## Out of scope

- How `Request` stores its body, query and headers, the CORS inputs and the route path (`q-http-request-hot-path`).
- The copy of the body handed to pipes: `PipeTransform::transform` takes the value by value, so one copy per body
  parameter stays (the same rule #35 records for gateways). Changing the pipe API is a separate question.
- Sharding the throttler's counters unless Decision 6's measurement calls for it (then a follow-up change).
- A cap on rooms per plain gateway client (none exists today; the index adds no new growth path): possible later issue.
- The Socket.IO server (`e2-socket-io-hot-path-and-api-hygiene`).
