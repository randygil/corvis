# Design

## Context

Source: the engineering-quality review of the core recorded in the issue (`quality/q-core-request-hot-path.issue.md`).
Code read on `dev` at `d46758a` (2026-11-11); for `websockets.rs`, also on the #35 branch (`wip-35` at `26e8513`), which
rewrites the message pipeline. The review measured nothing: every cost below is a count read from the code, and task
group 2 measures it before anything changes.

**What this change does not own.** `openspec/changes/q-http-request-hot-path/` (epic #187) owns how `Request` holds its
body, query and headers and makes cloning it O(1) (its Decision 2); in the same decision it captures the CORS inputs
before dispatch (`CorsRequest`, `req_for_cors` removed) and shares the route path (`route_path: Option<Arc<str>>`,
`CompiledRoute::path: Arc<str>`). This change assumes that contract and lands after it; where a finding here is resolved
by the HTTP change it is marked so and nothing is done twice. #35 owns the shape
of the gateway message pipeline (`ExecutionContext::for_ws_shared`, the body as one `Arc<Value>`, one copy per
`#[message_body]` parameter, charged to the budget); `e2-socket-io-hot-path-and-api-hygiene` owns the
`tokio-tungstenite` upgrade (text frames that share their bytes). Decision 8 builds on both.

### The findings, confirmed against the code

| Finding | Where now | Verdict |
|---|---|---|
| S1 CORS copies the request | `router/mod.rs:589` (`req.clone()` whenever CORS is on), used at `:595` and `:635` only to call `cors.apply` | Confirmed; **resolved by the HTTP change** (`CorsRequest` captured before dispatch). The `cors` scenario of the benchmark checks it (Decision 2) |
| S1 copy for middleware errors | `router/mod.rs:624` | Resolved by the HTTP change (the clone becomes O(1)). Handing the request back through `Next` would change the public `CorvisMiddleware::use_` signature; not done |
| S1 body and query copies per parameter | `router/mod.rs:902` (`body.clone()`), `:911` (`req.query().clone()`) | Confirmed and kept: `PipeTransform::transform` takes the value by value, so each parameter needs its own value. After the HTTP change the request holds the body once, so the cost is one copy per parameter (proposal "Out of scope") |
| S1 WebSocket copies per message | dev: `websockets.rs:1060` (`t.to_string()`), `:1083` (`parsed.get("data").cloned()`), `:919` (`for_ws` copies `data`), `:846-847` (copy per parameter) | `:919` handled by #35 (`for_ws_shared`, `wip-35` `websockets.rs:1211-1219`); `:846-847` kept by #35's design (one copy per parameter, charged). The text copy and the `data` copy remain on `wip-35` (`:1363`, `:1386`): Decision 8 |
| S2 middleware matching per request | `router/mod.rs:297-300` (`applies`, every pattern), `common/middleware.rs:236-237` (two `Vec` per call), `:249`, `:251-256` (`join` and a growing `String` for globs), `router/mod.rs:540-545` (a controller becomes one pattern per route and optional variant) and `:534-536` (and per version segment); `:601-606` (the chain `Vec` built per request) | Confirmed. The review's fix (a chain per `CompiledRoute`, matching only for 404s) is **rejected**: module middleware runs before routing, also for static files and 404s, and a pattern matches the request path, which can match a pattern without being routed to that route (`for_routes("cats/5")` against route `/cats/:id`). Decision 3 keeps the semantics and removes the allocations |
| S3 throttler per-request work | `modules/throttler.rs:1025-1031` (`throttle_key` formatted per target, option and throttler), `:1042` (`from_value(v.clone())`), `:1124-1134` (`skip_throttle_key` formatted, `Reflector::get` clones), `:1163-1171` (three names and three values formatted), `:1241` (tracker `clone()`), `:1247-1252` (stored key formatted), `:537` and `:864` (one `Mutex<Counters>` for every hit) | Confirmed, with a correction: `check_unknown_overrides` (`:1072-1089`) allocates nothing on the normal path (`BTreeSet::new()` does not allocate; it compares strings); left as is. The mutex is a contention question, measured before deciding (Decision 6) |
| S4 request scope builds before looking | `di/container.rs:565` (`Provider` cloned with its `String` name), `:603-615` (path copied, `Arc::from(name)`, the factory's boxed future created) before `:619` (`get_or_try_init`); `di/module_ref.rs:106-116` (`Arc::from("ModuleRef")`, `current_resolution_path()` allocating an empty `Arc<Vec>` outside a construction, `di/container.rs:33-35`); `common/param.rs:119-120` (`short_type_name` and a `Logger` on success) | Confirmed. Sibling: `RequestCtx::of` clones the request (`di/request_scope.rs:55`): O(1) after the HTTP change |
| S5 `Reflector` copies | `common/reflector.rs:16`; siblings in `get_all_and_merge` (`:30`, `:34`) | Confirmed |
| N3 router allocations | `router/mod.rs:260-262` (normalized and lowercased copies of every path), `:273` (`m.value.clone()` of the candidate `Vec`), `:684` (filtered `Vec`), `:702` (`route.path.clone()` into `route_path`); at startup `:472` (`groups.iter_mut().find`, quadratic) | Confirmed; `:702` **resolved by the HTTP change** (`Arc<str>`), the rest is Decision 4 |
| Sibling: a broadcast value that fails to serialize | `websockets.rs:262`, `:487`, `:515` (`to_value(data).unwrap_or(Value::Null)`) | Found while confirming (and recorded as a follow-up by `q-http-response-consistency`, whose HTTP equivalent becomes a `500`): the clients receive `"data": null`. Decision 8 logs and sends nothing |
| N2 router | `router/mod.rs:567-568` (`#[allow(dead_code)] container`) | Confirmed: nothing reads it; removed |
| M4 broadcast copies and room scans | `websockets.rs:261-264` (`WsClient::emit`: `to_value` deep copy, `json!`, `to_string`), `:486-489` (`WsServer::emit`: one `to_value`, then one deep copy and one serialization per client), `:514-522` (`to(room)`: every client of the server, each one's room mutex), `:476-478` (`clients()` builds a `Vec` per broadcast) | Confirmed on dev and unchanged on `wip-35` (`:371`, `:586-628`). Decision 8 |

### Estimated cost today, to be measured

From reading (task 2.3 replaces these with measured counts): a plain `GET /items/:id` with one global guard costs about
20 to 25 allocations; CORS adds one deep copy of the request; each module middleware pattern adds two vectors; each
throttler adds about 25; each resolution of an existing request-scoped instance adds about 8; a broadcast of a payload
of `P` bytes to `N` clients allocates about `N × 2P` bytes.

## Goals / Non-Goals

**Goals:**

- A benchmark that counts allocations per request for each feature, run before and after on the same machine.
- Remove the per-request work the review found, each removal against a number, with matching rules, limits and key
  formats unchanged.
- Broadcasts whose cost does not multiply the payload by the number of recipients, and room emits that cost the room.

**Non-Goals:**

- The `Request` representation, the JSON parser, response finishing (`q-http-request-hot-path`).
- Changing `PipeTransform`, `CorvisMiddleware` or any public signature.
- Sharding the throttler unless Decision 6's measurement calls for it.

## Decisions

### 1. A request pipeline benchmark that counts allocations

`openspec/specs/bench` has `bench/synthetic` (throughput with `oha`) and `bench/port-vs-original`; the changes in flight
add `bench/socket-io` and, in `q-http-request-hot-path`, `bench/http/` (a server with a counting allocator driven by
`oha` over a socket, for the request parsing and response paths). None of them turns on the features this change is
about (module middleware, throttlers, request-scoped providers), and none can send requests with distinct trackers, which
the throttler's lock measurement needs. New capability `bench/request-pipeline`, following `bench/synthetic`'s rules
(release build in the workspace target directory, results file overwritten per run, raw output ignored by git):

- `bench/pipeline/` is a workspace member (`publish = false`) with one binary, `corvis-pipeline-bench`, and a global
  counting allocator (allocations and bytes, relaxed atomics; the measured mode runs on a current-thread runtime, so the
  counts are the pipeline's own). It drives the application in process through `corvis::testing::request`, the path the
  test client uses (which, after `q-http-server-robustness`, is the server's own `server::respond`), so no socket and no
  load generator take part.
- **`allocs` mode.** For each scenario: build and `init()` the application, send 1,000 warm-up requests, then 10,000
  measured ones, and report allocations and bytes allocated per request (total over the window divided by 10,000), plus
  the difference from `plain`. Scenarios:
  - `plain`: `GET /items/:id` on a controller with a global guard that reads a three-element `roles` array through
    `Reflector` (the review's case);
  - `body-1k`, `body-64k`: `POST /items` with a JSON body of that size into `#[body] Value` and into a typed DTO;
  - `cors`: `plain` with `enable_cors` (one allowed origin, credentials) and a request carrying `Origin` and 20 other
    headers;
  - `middleware`: `plain` plus a module middleware applied with `for_routes(ItemsController)` where the controller has
    20 routes, with URI versioning publishing two versions (40 patterns plus optional variants), and a request to a path
    no module middleware matches (`middleware-miss`);
  - `throttler-1`, `throttler-3`: `plain` under `ThrottlerGuard` with one and with three named throttlers (limits high
    enough that nothing is refused);
  - `request-scope`: a handler with two `Scoped<RequestContext>` parameters of a request-scoped provider that injects the
    `Request` (the second resolution finds the first instance).
- **`contention` mode.** A multi-thread runtime with `W` workers (1, 4, 8); `W` tasks send requests in a loop for 10
  seconds with distinct trackers (a module-level tracker reading `x-client-id`, 10,000 distinct values), for `plain` and
  `throttler-1`. Reported: requests per second per scenario and `W`, and the ratio `throttler-1 / plain`. This is the
  measurement of Decision 6.
- `bench/pipeline/run.sh` builds once (`cargo build --release -p corvis-pipeline-bench`) and runs both modes, three runs
  each, reporting the median with min and max; results go to `bench/pipeline/results.md`, raw output to
  `bench/pipeline/raw/` (ignored by git). `bench/pipeline/README.md` documents it; `bench/README.md` links it.

The before run is made on `dev` with `q-http-request-hot-path` merged (task 2.3), the after run at the end (task 9.1), same
machine. Allocation counts are deterministic for a given build; requests per second are not, so no test compares them.
After the change, `crates/corvis/tests/pipeline_allocations.rs` (its own test binary, counting allocator, as
`redact_allocations.rs`) pins each scenario's count at the after figure plus 10 %, so a regression fails a test; it
asserts counts only, never times.

Alternatives considered: add these scenarios to `bench/run.sh` or to `bench/http/`. Rejected: both drive the server with
`oha` across a socket, which sends one fixed set of headers (no distinct trackers) and whose per-request counts include
hyper's and the connection's work, so a difference of a few allocations is lost; in process, the counts are exact. The two
benchmarks are complementary: `bench/http/` for the HTTP layer, `bench/pipeline/` for the features of the pipeline.

### 2. CORS: checked here, owned by the HTTP change

`q-http-request-hot-path` removes `req_for_cors` (its Decision 2). This change does not touch CORS; the `cors` scenario
checks, in this pipeline, what that change delivered: `cors − plain` at most 4 allocations (the header list and its
values), and the same with 20 extra request headers as with none. If the before run (taken after the HTTP change) already
shows it, nothing is to be done; if not, task 2.3 records the gap for that change's owner instead of fixing it here.

### 3. Middleware patterns compiled once, matched without allocating

- `common/middleware.rs` gains `PathPattern` (crate-private): the pattern trimmed and split once into segments, each
  classified as it is today (whole-path wildcard; segment wildcard `*`, `(.*)`, `*name`, `{*name}`; `*` inside a segment,
  kept as a glob; `:param` or `{param}`; literal). `PathPattern::matches(&self, path: &str) -> bool` walks the request path
  with the same recursion as `path_matches`, on string slices of the path (a glob over several segments is a slice of the
  original path, which is what `segs.join("/")` rebuilds today), so it allocates nothing.
- `path_matches(pattern, path)` stays as the reference implementation (`PathPattern::new(pattern).matches(path)` would be
  the same function); it is kept for the prefix exclusions at startup and as the oracle of the property test.
- `build_middleware` compiles `include` and `exclude` into `Vec<(PathPattern, RequestMethod)>` with duplicates removed
  (the controller expansion produces the same pattern for routes that differ only in method or are repeated per version).
- `dispatch` computes the chain as today (global middleware, then each group whose `applies` is true, in order), with
  `applies` using the compiled patterns. When no module group applies, it reuses an `Arc<Vec<_>>` of the global
  middleware built once at `init()`; otherwise it builds the `Vec` once.

Why the semantics are unchanged: same classification, same recursion, same case-insensitive comparison, same trimming;
only the representation moves to startup. The property test is the proof the change relies on, not the reading.

Numbers it must move: `middleware − plain` at most 3 allocations (the chain, its `Arc`, the endpoint), however many
patterns; `middleware-miss − plain` 0. Tests: a property test in `fuzz_tests.rs` (it runs in the fuzz CI job) generating
patterns from the pattern grammar (literals in mixed case, `:p`, `{p}`, `*`, `(.*)`, `*name`, `{*name}`, `a*b`, `*.json`,
empty segments, leading and trailing slashes) and paths (mixed case, `//`, trailing slash, percent signs, dots) asserts
`PathPattern::new(p).matches(path) == path_matches(p, path)` for 10,000 cases; the existing `matching` and
`segment_wildcards_match_like_the_router` unit tests and the middleware tests of `enhancers_routing.rs` pass unchanged.

### 4. Borrowed router lookups

- `lookup` normalizes into a `Cow<str>`: borrowed when the path has no trailing slash to strip and no ASCII uppercase
  letter (the common case), owned otherwise; parameter values are still cut from the original path.
- `RouteMatch` borrows the candidate slice from the table (`&[usize]`) instead of cloning the `Vec`.
- `select_route` filters by host while it iterates (two passes over the slice when versioning by header or media type),
  without collecting.
- (`CompiledRoute::path` and `Request::route_path` as `Arc<str>` are done by `q-http-request-hot-path`.)
- At startup, the route groups are collected in a `HashMap` keyed by (method, path) instead of a linear search.
- `AppShared::container` (`#[allow(dead_code)]`) is removed.

Number it must move: `plain` falls by at least 3 allocations (normalized path, lowercased path, candidate list; the host
list too when a route of the group has a host). Tests: the routing suites (`enhancers_routing.rs`, `express_parity.rs`,
the router unit tests) pass unchanged; a unit test asserts that a mixed-case path with a trailing slash still binds the
parameter with its original case.

### 5. `Reflector` without copies

`get` becomes `target.metadata.get(key).and_then(|v| T::deserialize(v).ok())` (`&Value` implements `Deserializer` with
the same rules as `from_value`); `get_all_and_merge` does the same per item. Number: in `plain`, the guard's read of the
three-element array allocates only the returned `Vec<String>` (4 allocations instead of 8). Tests: the existing
`Reflector` users (roles guards in `examples/`, throttler skips, cache TTL) pass; a unit test reads every JSON type
through `get` and compares with `from_value`.

### 6. Throttler: fixed work done once, the same stored keys

- **Per-throttler keys at build time.** `ThrottlerStorage` builds, for each throttler, a `ThrottlerKeys` with the five
  override keys (`throttle_key(option, name)`), the skip key, and the three header names (`X-RateLimit-Limit[-name]`,
  `-Remaining`, `-Reset`, `Retry-After[-name]`) as `HeaderName` (a name that is not a valid header token gives `None`, and
  the header is dropped, as today). Built in `new` and wherever throttlers are re-sorted. `override_value`, `is_skipped`
  and `set_headers` look these up; no `format!` per request.
- **Override values without copies.** `number_override` uses `T::deserialize(v)`; `is_skipped` uses the Decision 5
  `Reflector`.
- **Typed headers.** A crate-private `HttpArgumentsHost::set_response_header_value(HeaderName, HeaderValue)` applies the
  same rules as `set_response_header` (HTTP only, reserved names refused, the 128-name cap, `Res` passthrough) without
  parsing; values come from `HeaderValue::from(u64)`.
- **The stored key hashed in parts.** `record_hit` keeps its signature. A crate-private `record_hit_parts(parts: &[&[u8]],
  ..)` feeds the parts into `Sha256` in order and calls the same `record_hit_hashed(hash, ..)` that `record_hit` uses after
  hashing its string. The guard passes the parts of the documented key (`<name length>:<name>:b:<class>:<handler>:<tracker>`
  and the three generator forms), with the length written into a stack buffer. The digest is the same bytes as today's
  `sha256(format!(..))`: same counters across an upgrade, same separation of sources.
- **The tracker by reference.** The built-in tracker is the normalized address computed once per request, borrowed
  per throttler.
- `check_unknown_overrides` is unchanged (no allocation on the normal path).

Numbers it must move: `throttler-1 − plain` at most 6 allocations and `throttler-3 − plain` at most 14 (at most 4 per
counted throttler: three header values and one header-store entry; plus the address string and its normalization once
per request), from the before run's figures (the review estimated about 25 per throttler).

**Sharding is decided by a measurement.** The `contention` mode (Decision 1) gives `throttler-1 / plain` throughput at
1, 4 and 8 workers. If at 8 workers the ratio is below 0.8 while at 1 worker it is above 0.9 (the lock, not the work,
costs), task 6.4 opens a follow-up issue for sharding with the invariants below and this change records the figures; if
not, the figures are recorded and sharding is not pursued. Sharding is not implemented in this change in either case:
it changes the eviction rule of the spec and deserves its own security review.

Invariants that any change to the throttler, including a later sharding, must keep (each with what pins it):

| Invariant | Pinned by |
|---|---|
| At most `max_entries` counters and `max_blocked` blocks; at most 64 buckets per key; idle entries removed | `storage_memory_is_bounded`, `a_key_uses_bounded_memory_whatever_its_hits`, `idle_keys_are_removed`, `a_blocked_key_is_not_removed_while_the_block_lasts` |
| Blocked keys never displace counters; oldest block evicted first | `a_blocked_key_stays_blocked_while_others_flood`, `pinning_the_blocked_table_does_not_disable_other_counters`, `the_oldest_block_is_evicted_first_whatever_its_length` |
| Per-key counting exact, equal to the original's | `replays_the_original_storage_fixture`, `round_robin_keys_get_at_most_the_limit_each`, `no_double_burst_around_a_window_boundary` |
| Stored keys are SHA-256 digests of the documented key string | new property test: `record_hit_parts` digest equals `sha256` of the concatenation for generated names (multi-byte included), class and handler names, trackers and generator outputs |
| Throttlers and key sources never share a counter; a crafted tracker cannot reach another source's key | `counters_stay_separate_per_throttler`, `a_crafted_tracker_cannot_hit_another_routes_builtin_counter`, `a_key_generator_can_share_a_counter_between_routes` |
| Overrides fail closed (unknown name, invalid value, bare key without `default`) | `an_unknown_throttler_name_fails_closed`, `invalid_override_values_fail_closed`, `a_bare_limit_with_named_throttlers_only_fails_closed`, `an_override_for_an_unknown_throttler_answers_500_and_logs_once_per_route` |
| Header names and values as the original; dropped for invalid names; none in WebSocket contexts | `throttler_headers.rs` header tests, `websocket_context_records_no_headers` |
| A sharded storage (if ever): one key always maps to one shard chosen by a keyed hash with a per-storage random key (a client cannot aim keys at one shard); all of a key's state under that shard's lock; per-shard caps that sum to the global caps; the tests above run with 1 and with 16 shards | the follow-up change |

### 7. Request scope: look in the store first

- `Provider::name` becomes `Arc<str>` (crate-private field), so cloning a `Provider` allocates nothing; `requester`
  takes a clone of it.
- `resolve_provider` for `Scope::Request` asks the request's store for an initialized cell (`RequestScope::get(&key)`,
  new, a lock and a lookup) before it copies the path, builds `ResolveCtx` or creates the factory's future; only a miss
  goes on to `get_or_try_init`, unchanged (one constructor for concurrent callers, retry after a failure). The
  `OutsideRequest` check stays before the lookup.
- `ModuleRef::ctx` uses a `static` `Arc<str>` for `"ModuleRef"`, and `current_resolution_path()` returns a clone of a
  `static` empty path outside a construction.
- `Scoped::from_context` computes the type name and the `Logger` only on its error paths.

Numbers it must move: in `request-scope`, the second `Scoped<RequestContext>` resolution allocates nothing beyond the
returned handle (a counting test in `pipeline_allocations.rs` measures one resolution inside the handler); `request-scope
− plain` falls by at least the cost of one construction context (path copy, name, boxed future: at least 3). Tests: the
request-scope suites (`tests/di/request_scope.rs`, the unit tests of `di/request_scope.rs`, `scoped_error_log.rs`) pass
unchanged.

### 8. WebSocket gateways: encode once, index rooms, parse without copies

Starts after #35 and `e2-socket-io-hot-path-and-api-hygiene` are merged (task 8.1 re-reads `websockets.rs` then).

- **One encoding per broadcast.** `WsServer::emit` and `RoomEmitter::emit` serialize `{"event": .., "data": ..}` once,
  straight from the caller's value (a borrowed `Frame { event, data }` struct; no `to_value`), and push a shared text
  (the upgraded library's shared text type) to each recipient. `WsClient::emit` serializes the same way (no `to_value`).
  Per-client limits are unchanged: each push counts the frame's length against `max_buffered_amount`. A value that fails
  to serialize is logged once per call (`Logger::new("WebSocketGateway").error(..)`, with the event name and the error)
  and sent to nobody; `emit` returns `false` / `0`, instead of today's frame with `"data": null`.
- **No snapshot of the clients.** The broadcast iterates the clients under the map's read lock and pushes with a
  non-closing `try_push`; clients whose queue is full are collected and closed with 1008 after the lock is released (the
  close path takes the map's write lock, so it cannot run under the read lock).
- **Room index.** `ServerInner` gains `rooms: RwLock<HashMap<String, HashSet<Arc<str>>>>`. Lock order: the server's room
  index, then the clients map, then a client's own room set. `join` and `leave` update the client's set and the index
  under the index's write lock; removing a client (disconnect, close, full queue) sets the client's `removed` flag and
  deletes its ids from every room of its set under the same lock, then leaves the clients map; `join` checks `removed`
  under the lock and does nothing for a removed client. An empty room is deleted from the index. `to(room).emit` reads the
  room's ids under the index's read lock and looks each up in the clients map.
- **Messages parsed without copies.** The connection loop parses the text frame from the frame's own string (no
  `to_string()`), a binary frame from its bytes when they are UTF-8 (the lossy conversion only when they are not, as
  today), and takes `event`, `data` and `id` out of the parsed object with `remove` instead of cloning them.

Numbers it must move (`crates/corvis/tests/ws_broadcast_memory.rs`, a counting allocator around the synchronous `emit`
call on the test thread, clients connected over `127.0.0.1`): a 16 KiB payload broadcast to 100 clients allocates less
than 48 KiB (today about 100 × 32 KiB); `to(room).emit` for a room of one client among 1,000 allocates less than 2 KiB
(today a vector of 1,000 handles); a received 4 KiB message allocates less than one copy of its text beyond the parsed
value (the count of the loop's per-message allocations, measured with a test hook that reports them); a value whose
`Serialize` fails reaches no client and `emit` returns 0 (fails before: clients get `null`). Tests: the gateway
suites (`websockets.rs`, `ws_security.rs`, `ws_enhancers.rs`, `auth_ws.rs`, `send_timeout_ws.rs`) pass unchanged; a churn
test with 8 tasks joining, leaving and disconnecting 1,000 clients across 50 rooms leaves the index empty and no room
holds an id of a removed client.

## Security invariants this change comes near

From the security reviews of the router and middleware (epic #1), the throttler (e13 changes), the request scope and the
WebSocket gateways. Each refactor that comes near one has a test that pins it:

| Invariant | Source | Decisions | Pinned by |
|---|---|---|---|
| A middleware cannot be skipped by changing the case of the path, adding a trailing slash, or using an optional or versioned form of the route | `common/middleware.rs` rustdoc of `path_matches`, `http/routing` | 3 | the property test of Decision 3; `segment_wildcards_match_like_the_router`; `middleware_text_patterns_match_versioned_routes`, `middleware_exclusions_cover_versioned_routes`, `controller_middleware_covers_the_optional_variants` |
| Module middleware runs before routing, also for static files and unmatched paths; exclusions win over inclusions | router `dispatch` | 3 | existing middleware tests; new: an unmatched path runs a `for_routes("*")` middleware |
| Throttler bounds, exact accounting, key format and separation, fail-closed overrides | `security/rate-limiting` | 6 | the table of Decision 6 |
| Request-scoped instances live exactly as long as the dispatch (removed on return, panic, cancellation); one constructor per key; retry after failure; singletons never capture a request | `core/injection-scopes`, `di/request_scope.rs` | 7 | `dropping_the_dispatch_future_removes_the_slot`, `a_panic_removes_the_slot`, `a_failed_attempt_is_retried`, `two_concurrent_callers_of_one_key_run_the_constructor_once`, `instances_are_released_when_the_handler_panics` |
| `Scoped<T>` errors expose no internals | `core/injection-scopes` | 7 | `scoped_error_log.rs` |
| Each WebSocket client bounded by `max_buffered_amount`; a full queue closes it with 1008; frames after a disconnect are not dispatched | `ws_security.rs`, gateway rules | 8 | `ws_security.rs` buffered-amount and close tests, unchanged |
| No room index entry for a removed client; empty rooms removed (new) | here | 8 | the churn test of Decision 8 |

## Behavior visible to applications

| What | Before | After | Consequence |
|---|---|---|---|
| A broadcast or `WsClient::emit` whose data fails to serialize | Sent with `"data": null` | Logged, sent to nobody, `emit` returns `false` / `0` | Clients no longer receive a null they cannot tell from a real one |
| Order in which `to(room).emit` reaches recipients | Order of the clients map | Order of the room's id set | Both are hash orders; no order was promised |
| Everything else | | | Same responses, headers, limits and keys; fewer allocations |

## Risks / Trade-offs

- A compiled matcher that differs from `path_matches` in a corner case would skip or add a middleware → the property test
  against the old function as oracle, in the fuzz CI job, and the security review reads the matcher.
- Incremental hashing that feeds different bytes would reset every counter on upgrade and could merge sources → the
  digest-equality property test, and the key-separation tests.
- Checking the request store before building opens a window where two callers both miss and both enter
  `get_or_try_init` → that is today's path; `OnceCell` still runs one constructor (`two_concurrent_callers_...`).
- The room index adds a lock to `join` / `leave` and doubles the memory of room memberships → bounded by the memberships
  that exist today; churn test; lock order written in the module docs.
- The benchmark runs in process, so it measures the pipeline, not the socket → the socket path is `bench/synthetic`'s job;
  stated in the README.

## Open questions

1. Pin allocation counts in a test (`pipeline_allocations.rs`), or only record them in the benchmark? Recommendation: pin
   with a 10 % margin; counts are deterministic for a build and the repository already pins allocation counts
   (`redact_allocations.rs`).
2. Implement throttler sharding here if the measurement calls for it? Recommendation: no; a follow-up change with its own
   security review (it changes the eviction rule).
3. Should the room index be added before or after the `tokio-tungstenite` upgrade? Recommendation: after (task order),
   so the shared text type is used from the start.
4. A broadcast value that fails to serialize: send `null` as today, send nothing, or panic? Recommendation: log and send
   nothing; `emit` already reports how many clients were reached.

## Review notes

These settle the open questions and override the design where they differ.

1. `pipeline_allocations.rs` pins every scenario at the after figure plus 10 %, rounded up; it asserts counts only.
2. No sharding in this change; task 6.4 records the contention figures and opens a follow-up only if Decision 6's
   threshold is crossed.
3. Task group 8 starts after `e2-socket-io-hot-path-and-api-hygiene` task 5.4 is merged.
4. A value that fails to serialize is logged and sent to nobody; the gateway docs say so.
5. Implemented after `q-http-request-hot-path`; if that change slips, groups 4 to 8 may land first (they do not depend on
   the clone), and the before run is then taken on `dev` as it is, with the `cors` row marked as measured before the
   HTTP change.
6. The security review is mandatory before the merge: the compiled middleware matcher, the throttler's key hashing and
   header setter, the request-store lookup, the room index and its lock order.
