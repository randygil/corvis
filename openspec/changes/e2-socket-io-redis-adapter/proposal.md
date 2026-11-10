# Proposal

Implements [randygil/corvis#36](https://github.com/randygil/corvis/issues/36) (epic
[#30](https://github.com/randygil/corvis/issues/30)). Design source: the Socket.IO spike #31
(`openspec/changes/archive/2026-11-08-e2-spike-engine-io-v4-socket-io-v5-in-rust/`: design.md Decision 1 (the Redis
compatibility is the deciding requirement), Decision 2 (`redis.rs` behind its own feature), Review note 4 (the `redis`
crate), delta `specs/websockets/socket-io-cluster`, tasks.md group 7). Builds on three approved changes that are not
merged yet, whose APIs are referred to **as designed there, including their Review notes**: #32
(`openspec/changes/e2-engine-io-handshake-polling-upgrade-and-heartbeat/`), #33
(`openspec/changes/e2-socket-io-namespaces-events-and-acks/`) and above all #34
(`openspec/changes/e2-socket-io-rooms-and-broadcast/`), whose `Adapter` trait and `InMemoryAdapter` are
`#[doc(hidden)]` until this change implements the trait a second time and makes them public (its Review note 6). The
issue and `ROADMAP.md` name the change `e2-redis-adapter-for-multiple-instances`; this change uses
`e2-socket-io-redis-adapter`, the name it was assigned when drafted (design.md Open question 1).

## Why

With the in-memory adapter of #34 a broadcast reaches only the sockets connected to the process that emits it. A
Socket.IO backend that runs more than one instance uses `@socket.io/redis-adapter`: every instance publishes its
broadcasts and room operations on Redis and applies the ones the others publish. A team migrating such a backend to
Corvis one instance at a time needs Corvis instances and Node instances on the same Redis to behave as one cluster
during the migration; that requirement is why the spike chose an own implementation over `socketioxide`, whose Redis
adapter states it is not compatible with the Node one. So the adapter has to be wire-compatible: the same channels,
the same messages (MessagePack as `notepack.io` writes it, JSON where the Node adapter uses JSON), the same
request / response protocol.

Messages arriving from Redis are a new input to the server. A Node peer, an older adapter version, another
application sharing the Redis, or anyone who can publish there decides their bytes; Corvis has to parse them as
untrusted input, within bounds, without ever panicking or holding unbounded state.

## What Changes

- New cargo feature `socket-io-redis` (implies `socket-io`, off by default) and module
  `crates/corvis/src/socket_io/redis/`: `RedisAdapter` (one publisher connection and one subscriber connection shared
  by every namespace of a server; passed to `IoOptions::adapter`), `RedisAdapterOptions`, connection state and
  counters.
- Wire compatibility with `@socket.io/redis-adapter` 8.3.0 (classic, non-sharded), read from the package pinned by the
  harness: channels `<key>#<nsp>#` (pattern subscription) and `<key>#<nsp>#<room>#` for single-room broadcasts,
  `<key>-request#<nsp>#`, `<key>-response#<nsp>#` and `<key>-response#<nsp>#<uid>#`; broadcasts as MessagePack
  `[uid, packet, {rooms, except, flags}]`; the ten request types, JSON except broadcasts with acknowledgements and
  their answers (MessagePack), both accepted on input; server count from `PUBSUB NUMSUB`; no heartbeat, as in the
  Node adapter.
- Cluster-wide operations through the adapter trait: broadcast, broadcast with acknowledgements, `fetch_sockets`,
  `sockets_join`, `sockets_leave`, `disconnect_sockets`, and (new on `IoNamespace` / `IoServer`)
  `server_side_emit`, `server_side_emit_with_ack` and `on_server_side_emit`.
- The `Adapter` trait, `InMemoryAdapter` and the types an adapter needs (`Packet` with accessors, `LocalNamespace`,
  `BroadcastOptions`, `AckSink`, `SocketInfo`, `AdapterError`) become public and documented, adjusted where the Redis
  implementation showed a need (design.md Decision 9).
- Dependencies: the `redis` crate (redis-rs, BSD-3-Clause, Tokio, RESP2 pub/sub with an explicit reconnect and
  resubscribe loop in Corvis) and `rmp` (MIT) for a bounded MessagePack codec written against `IoValue`. Both
  optional, only with the feature; licenses already allowed by `deny.toml`.
- Reconnect behavior: local delivery never depends on Redis; broadcasts to the cluster are best effort and are dropped,
  counted and logged while Redis is unreachable (never queued without bound); operations that wait for other
  instances fail with an error no later than `requests_timeout`; the subscriber reconnects with backoff and
  resubscribes every namespace.
- Bounded defaults, documented, each with a regression test: message size in both directions, MessagePack and JSON
  nesting depth, no allocation from declared lengths, pending requests, collected response bytes, request timeout,
  incoming request queue, remote acknowledgement trackers, publish queue, reconnect backoff. Handshake data sent to
  other instances for `fetch_sockets` is redacted by default (credentials headers and `auth`), a divergence from Node
  on the side of security. The full table is design.md Decision 10.
- Tests against a real Redis: `crates/corvis/tests/socket_io_redis.rs` and `socket_io_redis_security.rs` run when
  `CORVIS_REDIS_URL` is set; CI gets a Redis service container in the Linux `test` job, the same way it provides
  Postgres, with `CORVIS_REDIS_REQUIRED=1` so the tests cannot pass by skipping. Harness: mixed-cluster scenarios
  (one Node instance and one Corvis instance on the same Redis compared with two Node instances) and a recorder of the
  Node adapter's messages for codec fixtures.
- Docs (EN + ES): the Redis adapter in the "Socket.IO (preview)" section of `docs/websockets/adapter.md`.

## Capabilities

### New Capabilities

- `ws/socket-io-cluster`: the Redis adapter: channels, message formats, cluster operations, server-side emit,
  failure and reconnect behavior, input validation and limits. (The spike named it `websockets/socket-io-cluster`;
  the main spec tree uses `ws/`.)

### Modified Capabilities

- `ws/socket-io-rooms`: one added requirement, the adapter interface is public.
- `infra/ci`: one added requirement, Redis tests cannot pass by skipping in CI.

## Impact

- New: `crates/corvis/src/socket_io/redis/{mod,options,connection,channels,codec,requests,adapter,fuzz_codec}.rs`,
  `crates/corvis/tests/socket_io_redis.rs`, `crates/corvis/tests/socket_io_redis_security.rs`,
  `crates/corvis/tests/fixtures/redis_adapter_8.3.0.json`, `parity/socket-io/scripts/record-redis-adapter.mjs`,
  `parity/socket-io/scenarios/cluster/**`.
- Edited: `crates/corvis/Cargo.toml` (feature, optional dependencies), `Cargo.lock`, `THIRD_PARTY_NOTICES.md`,
  `crates/corvis/src/socket_io/{mod,adapter,broadcast,namespace,server}.rs` (public adapter items, server-side emit),
  `.github/workflows/ci.yml` (Redis service, feature in the clippy loop and the test job),
  `crates/corvis/tests/socket_io_diff.rs`, `parity/socket-io/reference/socket-server.mjs`,
  `parity/socket-io/lib/targets.mjs`, `parity/socket-io/run.mjs`, `parity/socket-io/expected-differences.json`,
  `docs/websockets/adapter.md`, `docs/es/websockets/adapter.md`.
- Public API: additions under `corvis::socket_io` (the adapter items, `redis::*`, server-side emit).
- Operational: an application using the adapter needs a Redis 6 or later reachable by every instance, and that Redis
  becomes part of the application's trust boundary (whoever can publish to it can emit to every client; design.md
  Decision 8). Documented.
- Port apps do not use Socket.IO and the code is behind a feature they do not enable; the corvis-parity check is still
  run because the definition of done asks for it when framework runtime code changes.

Note for the maintainer: the Redis session store planned as a follow-up of epic #39 (to be opened when #44 is
archived) and the queues of epic #48 (#50) should reuse the `redis` crate at the version pinned here, so the workspace
keeps one Redis client. The connection helper of this change (URL handling with credential redaction, timeouts,
reconnect with backoff) is written in its own file (`socket_io/redis/connection.rs`) with no Socket.IO types in its
signatures so it can be moved to a shared module when the second user arrives.

## Out of scope

- The sharded adapter (`createShardedAdapter`, Redis 7 sharded pub/sub), Redis Cluster and Sentinel topologies, and
  `@socket.io/redis-streams-adapter`: possible later issues. The adapter targets one Redis endpoint.
- `@socket.io/redis-emitter` (emitting from a process that is not a Socket.IO server): its messages use the broadcast
  format this change accepts, but no scenario verifies it; possible later issue.
- The legacy single-socket requests (`REMOTE_JOIN`, `REMOTE_LEAVE`, `REMOTE_DISCONNECT` addressed by `sid`) that the
  Node adapter 8.3.0 still answers but never sends (they come from peers that predate the bulk operations of
  Socket.IO 4): ignored. Mixed clusters must run a Node adapter that sends the `opts` form, as 8.x does.
- A custom parser (the Node adapter's `parser` option): MessagePack as `notepack.io` only.
- An API to list the rooms of the whole cluster (the Node adapter's `allRooms()`): Corvis answers the request when a
  peer sends it, but #34's adapter interface has no such operation to call; possible later issue.
- Several `IoServer`s sharing one `RedisAdapter` handle: each server takes its own handle and connection pair
  (design.md Decision 3).
- `volatile` and `compress` flags, adapter events, connection state recovery: out of scope of #34 too.
- Persisting or replaying broadcasts missed while Redis was unreachable: pub/sub is at most once, as with Node.
- The Redis session store and BullMQ-compatible queues: their own issues (see the note above).
- The CI job of the Node differential harness with its Redis service: #37. This change adds Redis only to the Rust
  test job.
- User guide and migration notes: #38.
