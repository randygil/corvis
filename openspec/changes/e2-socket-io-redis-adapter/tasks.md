# Tasks

Start after #34 (`e2-socket-io-rooms-and-broadcast`) is merged into `dev` (and with it #32 and #33). It does not
depend on #35. New test files start with `#![cfg(feature = "socket-io-redis")]`. Tests that need Redis read
`CORVIS_REDIS_URL` (for a local run: a Redis 6 or later on `127.0.0.1`), use a key of their own per test, never run
`FLUSHALL` and never kill connections other than the ones that carry their own client name. Run the Node-gated
checks with the harness installed (`npm ci` in `parity/socket-io/`), `CORVIS_NODE_DIFF=1` and `CORVIS_REDIS_URL`.
Timing rule for every test: absence of delivery is observed through a marker event, never by sleeping; timer logic
(deadlines, backoff, tracker lifetimes) runs on Tokio's paused clock; no retries. Test servers bind to `127.0.0.1`
only.

## 1. Reconcile, dependencies and feature

- [ ] 1.1 Compare design.md "The layers this builds on" and Decision 9 with the code merged by #33 and #34 (the
  `Adapter` trait and the factory of `IoOptions::adapter`, `InMemoryAdapter`, `LocalNamespace`, `AckSink`,
  `SocketInfo`, `Packet`, where `Adapter::init` and `close` are awaited, the operator's completeness rule and
  deadline); note renamed or missing items in a new "Implementation notes" section of design.md; if the lockfile of
  `parity/socket-io/` no longer pins `@socket.io/redis-adapter` 8.3.0 and `notepack.io` 3.0.1, re-read design.md "The
  Node adapter" against the installed sources and record the differences; verify
  `cargo test -p corvis --features socket-io` passes before any edit
- [ ] 1.2 `crates/corvis/Cargo.toml`: feature `socket-io-redis = ["socket-io", "dep:redis", "dep:rmp"]`, the two
  optional dependencies with default features off and exact feature names confirmed against the manifest of the
  pinned `redis` version, the `rediss://` integration enabled only together with Corvis' `tls` feature; update
  `Cargo.lock` and `THIRD_PARTY_NOTICES.md`; record in Implementation notes the pinned versions, whether the pub/sub
  connection offers a read limit for incoming messages and whether it applies backpressure to the socket instead of
  buffering (design.md Decision 10, Open question 3); verify `cargo deny check` passes,
  `cargo build -p corvis --features socket-io-redis` and `cargo build -p corvis` (feature off) have no warnings, and
  `cargo tree -p corvis` without the feature lists neither `redis` nor `rmp`

## 2. Public adapter interface and server-side emit

- [ ] 2.1 Apply design.md Decision 9 to `crates/corvis/src/socket_io/adapter.rs`: remove `#[doc(hidden)]` from the
  adapter items, `Packet::event`, `event_name`, `args`, `namespace` with the wire encoding private, `AdapterError`
  `#[non_exhaustive]` with `Unavailable`, `Timeout`, `TooManyRequests`, `MessageTooLarge`, `Unsupported`, `Closed`,
  what `LocalNamespace` needs to answer requests, the defaulted factory method that `IoServer::new` calls with the
  server's options; rustdoc for each item and the stability rule of the trait; verify the tests of #34 pass
  unmodified (`cargo test -p corvis --features socket-io --test socket_io_rooms --test socket_io_rooms_security`, the
  simulated second node included) and the scenarios "Adapter outside the crate", "Errors can grow" and "Refused by
  the adapter" of the `ws/socket-io-rooms` delta have tests
- [ ] 2.2 Server-side emit: `Adapter::server_side_emit` and `server_side_emit_with_ack` with default implementations
  that fail with `AdapterError::Unsupported` and Node's text; `IoNamespace::server_side_emit`,
  `server_side_emit_with_ack`, `on_server_side_emit` and the same on `IoServer` for `/`; `ServerSideAck` consumed by
  `send`; reserved event names and binary arguments rejected before the adapter is called; verify tests for the
  scenarios "In-memory adapter", "Binary argument" and "Server-side emit has a default", and
  `cargo test -p corvis --features socket-io` still passes

## 3. Options, channels and codec

- [ ] 3.1 `socket_io/redis/options.rs`: `RedisAdapterOptions` with every option and default of design.md
  Decision 10, `SharedHandshake`, validation with errors naming the option; verify unit tests for every default and
  every rejected value (spec "Options are validated", the server-side check of `max_message_bytes` included)
- [ ] 3.2 `socket_io/redis/channels.rs`: channel names for a key and a namespace, the pattern with glob
  metacharacters escaped, room extraction from a channel, the prefix and namespace checks, uid and request id
  generation; verify unit tests including a namespace and a key with `*`, `?`, `[`, `]` and `\`, a room containing
  `#`, and the id alphabet and length
- [ ] 3.3 `socket_io/redis/codec.rs`, MessagePack against `IoValue`: decoding rules and bounds of design.md
  Decision 2 (depth 128, no allocation from declared lengths, declared length against remaining bytes, rejected
  forms), encoding rules (smallest integers, integral floats, `str 8`, `bin`, insertion order); verify unit tests for
  every scenario of the spec requirements "MessagePack values convert like notepack.io" and "Decoding is bounded"
- [ ] 3.4 Message structs and their validation (design.md Decision 5): broadcast `[uid, packet, opts]`, the request
  types 0 to 7 in JSON and MessagePack, the answers (fetch, server-side emit, client count, acknowledgement), rooms
  as strings or integers, the whole-message rejection rules, id lengths, reserved events, the attachment count;
  verify unit tests for the scenarios of "A remote message never widens delivery" and "Messages from Redis are
  untrusted input" that need no Redis
- [ ] 3.5 `socket_io/redis/fuzz_codec.rs`: proptest properties (arbitrary bytes into the three decoders never panic
  and allocate at most a multiple of the input; depth; declared lengths; `IoValue` round trip; every message struct
  round trips); verify `cargo test -p corvis --features socket-io-redis --lib fuzz` and with `PROPTEST_CASES=20000`
- [ ] 3.6 `parity/socket-io/scripts/record-redis-adapter.mjs` (the Node adapter driven with stub clients, no Redis)
  and `crates/corvis/tests/fixtures/redis_adapter_8.3.0.json` with the package versions inside; Rust tests decode
  every recorded message, compare Corvis' encoding of the same operation as decoded structures, and fail naming the
  script when the fixture's versions differ from `parity/socket-io/package-lock.json`; verify the scenarios "Recorded
  Node broadcast", "Recorded Node requests", "Requests written by Corvis" and "Fixture versions"

## 4. Connections

- [ ] 4.1 `socket_io/redis/connection.rs` (no Socket.IO type in its signatures): URL parsing with the scheme check,
  the `rediss://` error without the `tls` feature, user information removed from `Debug`, errors and logs, the
  client name, `connect_timeout`; `RedisAdapter::connect`, `connect_lazy`, `close`, `state`, `stats`; verify tests
  against a closed loopback port for "Wrong address", "Lazy start without Redis", "TLS without the feature",
  "Connection failure" and "Debug output"
- [ ] 4.2 Publisher task: queue bounded by `max_publish_queue_bytes`, order kept, drop with counters and
  rate-limited log lines when full or not connected, nothing replayed; verify unit tests with a stalled publisher
  for "Queue full" and "Log flood", and the rate limit on the paused clock
- [ ] 4.3 Subscriber task and registry: one pub/sub connection for every namespace, subscriptions taken from the
  registry, messages routed by channel to their namespace, requests handed to the bounded queue
  (`max_queued_requests`) and its worker, reconnect with backoff and jitter, resubscription before the state is
  `Connected`; verify unit tests of the backoff on the paused clock ("Backoff") and of the queue bound ("Request
  flood") with an injected source of messages

## 5. The adapter

- [ ] 5.1 `socket_io/redis/adapter.rs`: the factory (one server per handle, the `max_message_bytes` check),
  `RedisNamespaceAdapter` holding an `InMemoryAdapter`, `init` (register, wait for the subscription when connected),
  `close`, membership delegated; outgoing broadcast in the order validate, encode, size check, enqueue, deliver
  locally; incoming broadcast with the room filter before decoding and the checks of Decision 5; verify
  `crates/corvis/tests/socket_io_redis.rs` tests with two servers for the scenarios of "Same channels as the Node
  adapter", "An instance is counted once" and the Corvis-only parts of "Broadcasts reach the whole cluster" (all,
  room, several rooms, except, from a socket, binary, local, own message, room without local members)
- [ ] 5.2 Bulk operations (`add_sockets`, `del_sockets`, `disconnect_sockets`): apply locally and publish, apply
  remote requests with #34's limits, legacy `sid` forms ignored, types 0 and 1 answered; verify tests for "Remote
  join over the room cap", "Requests Node no longer sends", "Legacy single-socket requests" and join, leave and
  disconnect across two Corvis servers
- [ ] 5.3 `fetch_sockets` and `server_count`: pending request table with `max_pending_requests`, `requests_timeout`,
  `max_response_bytes`, answers matched by id and shape, remote `SocketInfo` parsed tolerantly, the answer side with
  the handshake in Node's field names and `SharedHandshake` redaction, oversized answers not sent; verify tests for
  every scenario of "Requests that wait for other instances are bounded" and "Handshake data shared with other
  instances is redacted by default", "Single instance", "Remote socket operations" between two Corvis servers and
  "Oversized fetch answer"
- [ ] 5.4 Broadcast with acknowledgements: the `BROADCAST` request, client counts into `AckSink::expect`, answers
  into `respond`, the answering side with trackers bounded by `max_remote_ack_requests` and removed at their
  deadline together with the pending acknowledgements on local sockets; verify tests between two Corvis servers for
  "Every client answers" and "One remote client silent", and for "Tracker cap" and "Tracker deadline"
- [ ] 5.5 Server-side emit over Redis: publish, own uid ignored, handlers on the request worker under panic
  containment, acknowledgements counted against `server_count - 1` with the timeout text; verify tests among three
  Corvis servers for "Emit to the other instances", "An instance does not answer", "Alone" and "Handler panics"
- [ ] 5.6 Failure behavior: operations that need other instances fail with `AdapterError::Unavailable` when not
  connected, plain broadcasts and membership keep working, `AdapterError::MessageTooLarge` before any delivery;
  verify tests for the scenarios of "Local delivery never depends on Redis", "Oversized broadcast" and "No replay
  after an outage"

## 6. Tests against a real Redis

- [ ] 6.1 Test helper in `crates/corvis/tests/` for Redis: reads `CORVIS_REDIS_URL`, prints a skip notice without
  it, fails naming the variable when `CORVIS_REDIS_REQUIRED=1` and the server is missing or unreachable, gives each
  test a unique key and client name, and a control connection that finds the adapter's connections in `CLIENT LIST`
  and cuts them with `CLIENT KILL ID`; verify "Local run without Redis" and "Redis required" by running the tests
  with and without the variables
- [ ] 6.2 Reconnect tests in `socket_io_redis.rs`: "Connections cut", "Namespace added while disconnected",
  "Pending requests during an outage", "Close", "Counters after an outage", "Client name"; verify
  `CORVIS_REDIS_URL=... cargo test -p corvis --features socket-io-redis --test socket_io_redis` passes five runs in
  a row, and two runs at the same time against one server
- [ ] 6.3 `crates/corvis/tests/socket_io_redis_security.rs`: a raw Redis client publishes the forged messages of
  design.md Decision 11 on the adapter's channels (malformed and truncated MessagePack, depth 129, declared lengths,
  unknown extensions, non-string keys, invalid UTF-8, wrong field types, reserved events, 11 binary values, invalid
  `rooms` and `except` entries, an oversized message, another namespace's packet, answers for unknown ids and for
  other request types, the request flood, the tracker flood, a huge `flags.timeout`) and exercises the caller-side
  caps; each test asserts nothing was delivered that should not be, the counters moved, the instance keeps serving
  and bounds are observed as table and queue sizes; verify
  `cargo test -p corvis --features socket-io-redis --test socket_io_redis_security`

## 7. Harness: mixed cluster

- [ ] 7.1 `parity/socket-io/reference/socket-server.mjs` accepts `--redis-url` and `--redis-key` and installs the
  Node adapter; the Corvis scenario app of `crates/corvis/tests/socket_io_diff.rs` takes the same and gains the
  capability `redis`; scenario support for `cluster: 2` in `lib/targets.mjs` and `run.mjs` (the scenario runs
  against two Node instances as the reference and against one Node and one Corvis instance, each run with its own
  key, clients and commands on both instances); commands `server-side-emit` and `server-count` added to both apps;
  verify `node --test "test/*.test.mjs"` in `parity/socket-io/` and a Node-only run of one cluster scenario
- [ ] 7.2 Scenarios `scenarios/cluster/broadcast.mjs` (all, room, except, from a socket, binary, numeric room) and
  `bulk.mjs`; verify `CORVIS_NODE_DIFF=1 CORVIS_REDIS_URL=... cargo test -p corvis --features socket-io-redis --test
  socket_io_diff` reports no unlisted and no stale difference
- [ ] 7.3 Scenarios `cluster/fetch-sockets.mjs` (Corvis with `SharedHandshake::Full`), `cluster/broadcast-ack.mjs`
  (all answer; one silent with a 200 ms timeout, answers sorted before recording) and `cluster/server-side-emit.mjs`
  (plain, with acknowledgement); verify no unlisted and no stale difference
- [ ] 7.4 Scenario `cluster/wire/messages.mjs` (a raw subscriber records what each instance publishes for the same
  commands; decoded structures compared with ids normalized) and `cluster/fetch-sockets-redacted.mjs` with its entry
  in `parity/socket-io/expected-differences.json` (reason and `ws/socket-io-cluster` requirement); verify the listed
  difference occurs and the full harness run is clean three times in a row

## 8. CI

- [ ] 8.1 `.github/workflows/ci.yml`: the `redis:7` service container of design.md Decision 12 in the Linux `test`
  job, `CORVIS_REDIS_URL` and `CORVIS_REDIS_REQUIRED: "1"` in its `env`, the step
  `cargo test -p corvis --features socket-io-redis --no-fail-fast`, `socket-io-redis` in the per-feature clippy
  loop, `cargo test -p corvis --features socket-io-redis --lib fuzz` in the fuzz job; the Windows job runs the same
  tests without a server; verify with a pull request run that the Linux job ran the Redis tests (their names appear
  in the log, none skipped) and that stopping the service in a throwaway run makes them fail

## 9. Docs and review

- [ ] 9.1 Rustdoc for every public item added (`RedisAdapter` with a compiling example for `IoServer` and one for
  the gateway adapter marked as depending on #35, each option of `RedisAdapterOptions` with its default, the Node
  adapter's value and the divergence, `SharedHandshake`, `RedisState`, `RedisAdapterStats`, the server-side emit
  methods); verify `cargo doc -p corvis --features socket-io-redis --no-deps` without warnings and
  `cargo test -p corvis --features socket-io-redis --doc`
- [ ] 9.2 Docs: the Redis adapter in the "Socket.IO (preview)" section of `docs/websockets/adapter.md` (when to use
  it, the feature, `connect` and `connect_lazy`, one handle per server, mixed clusters with the Node adapter 8.x and
  what is not supported, server-side emit, what happens while Redis is down with a health-check example reading
  `state()`, the trust boundary of the Redis server with the ACL and TLS advice, the handshake redaction, the limits
  table of design.md Decision 10 and the two bounds set on the Redis server) and its translation in
  `docs/es/websockets/adapter.md`; verify `npm run docs:check` in `docs/`
- [ ] 9.3 Security review of the change with the `security-reviewer` agent (the MessagePack and JSON decoders,
  validation of remote broadcasts and requests and the rule that nothing widens a delivery, pending tables and
  queues, remote acknowledgement trackers, the publish queue, reconnect, what is published about clients, credential
  handling in URLs and logs, the measured behavior of the client crate for oversized messages and for a subscriber
  that falls behind, the public adapter interface); repeat rounds until no open finding; record findings and fixes
  in design.md under "Security review"; verify every finding has a regression test or a recorded reason

## 10. Definition of done

- [ ] 10.1 `cargo build --workspace --all-targets` with no warnings, and `cargo build -p corvis --features
  socket-io-redis` with no warnings
- [ ] 10.2 `cargo clippy --workspace --all-targets -- -D warnings` passes, and
  `cargo clippy -p corvis --no-default-features --features socket-io-redis -- -D warnings`
- [ ] 10.3 `cargo test --workspace` passes, `cargo test -p corvis --features socket-io-redis` passes with
  `CORVIS_REDIS_URL` set and `CORVIS_REDIS_REQUIRED=1`, and with `CORVIS_NODE_DIFF=1` the `socket_io_diff` and
  `socket_io_conformance` tests pass
- [ ] 10.4 Regression tests: one per spec scenario of the three deltas (unit, fixture, property, integration,
  security or harness scenario), as listed in groups 2-8
- [ ] 10.5 Docs pages updated in English and Spanish (`docs/` and `docs/es/`), then `npm run docs:check` in `docs/`
- [ ] 10.6 Framework runtime code changed (`crates/corvis/src/socket_io/`): run the `corvis-parity` check and verify
  the three port apps keep 0 apidiff differences
- [ ] 10.7 `cargo deny check` passes with the new dependencies, and the security review of 9.3 is closed before
  archiving

## Workflow follow-up

- Archive the change with `/opsx:archive` (its `ws/socket-io-cluster`, `ws/socket-io-rooms` and `infra/ci` deltas
  are applied).
- Close issue #36, tick it in epic #30 and move its card to Done (see the `corvis-task` skill).
- Open the follow-up issues noted in proposal.md "Out of scope" that the maintainer accepts (sharded adapter and
  Redis Cluster, the emitter, a cluster-wide room list), and tell the owners of the Redis session store and of the
  queues (#50) which `redis` version this change pinned and where the connection helper lives.
