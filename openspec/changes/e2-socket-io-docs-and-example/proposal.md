# Proposal

Implements [randygil/corvis#38](https://github.com/randygil/corvis/issues/38) (epic
[#30](https://github.com/randygil/corvis/issues/30)). Design source: the Socket.IO spike #31
(`openspec/changes/archive/2026-11-08-e2-spike-engine-io-v4-socket-io-v5-in-rust/`: tasks.md group 9, and the risk
"mixed `ws` / socket.io semantics of `namespace` ... covered in #38"). Last change of the epic: it documents what
#32 (`e2-engine-io-handshake-polling-upgrade-and-heartbeat`), #33 (`e2-socket-io-namespaces-events-and-acks`), #34
(`e2-socket-io-rooms-and-broadcast`), #35 (`e2-socket-io-gateway-adapter`) and #36 (`e2-socket-io-redis-adapter`)
build, referred to **as designed there, including their Review notes**, and states what #37
(`e2-socket-io-differential-tests`) verifies. The change name is the one in the issue and in `ROADMAP.md`.

## Why

Each change of the epic documents its own layer by appending to one section, "Socket.IO (preview)", of
`docs/websockets/adapter.md` (#32 tasks 8.4, #33 8.4, #34 6.2, #35 7.2, #36 9.2). After #36 that section holds, in
the order the layers were built, the Engine.IO bounds, the low-level server API, rooms, gateways and the Redis
adapter, with five limit tables, on a page whose title is "Adapters" and whose first half is about plain
WebSockets. Every one of those changes leaves the "user guide", the "migration notes" and the "example application"
to #38 (their proposals, Out of scope).

A reader who arrives with a Node backend that uses `socket.io` needs the opposite order: how to turn it on for
gateways, what the official client will observe, what to change in the code, what differs from Node and why, and how
to run Node and Corvis instances side by side during the move. And the project's definition of done asks for a
runnable example of every user-facing feature: no example uses Socket.IO. `tertulia`, the chat example, is built on
the `ws`-style adapter and is quoted by five documentation pages for it.

## What Changes

- **Three new pages** in the WebSockets section, in English and Spanish, status `documented`:
  - `websockets/socket-io` — the guide: enabling the feature and the adapter, a first gateway, namespaces and the
    Engine.IO path, the handles (`IoSocket`, `IoGatewayServer`), how event arguments become the message body and how
    results become acknowledgements or events, exceptions, authenticating on connect (`guard_connections`), rooms and
    broadcast, acknowledgements with timeouts, binary payloads, the low-level API (`IoServer`, `IoSocket::on`),
    CORS and origins, scaling out with the Redis adapter, testing, and what is not supported.
  - `websockets/socket-io-limits` — every bounded default of the five layers in one place, each with Node's value,
    and every intentional difference from Node with its reason. Replaces the five tables of the preview section.
  - `websockets/socket-io-migration` — moving a Node `socket.io` backend: what stays the same for clients, an
    equivalence table from the Node server API to Corvis, the differences to review before switching, a rolling
    migration with a mixed cluster on Redis, and a checklist.
- **`docs/websockets/adapter.md`**: the "Socket.IO (preview)" section becomes a short section that names the two
  gateway adapters and links to the guide; the page keeps `corvis::ws`. The word "preview" is dropped (the feature
  stays opt-in).
- **Cross-links** (EN + ES): `websockets/gateways.md` (which adapter a reader is on, the two meanings of
  `namespace`), `websockets/guards.md`, `migration-guide.md`, `faq/examples.md`, the root `README.md` (examples
  table and the WebSockets row of its equivalence table), and the sidebar.
- **New example `examples/socket-io-chat`**: a chat served by a gateway over Socket.IO, with a browser client that
  uses the official `socket.io-client`: namespace `/chat`, token in the handshake `auth` checked by a connection
  guard, rooms, broadcast without the sender, acknowledgements from return values, `exception` events, a REST
  endpoint that broadcasts through the gateway's server handle, one low-level handler with a binary acknowledgement,
  and the Redis adapter when `REDIS_URL` is set. Rust tests speak the protocol over a raw WebSocket; a Node-gated
  test drives it with the official client; `scripts/smoke-examples.mjs` starts it.
- `tertulia` is **not** moved: it stays the example of the `ws`-style adapter (design.md Decision 1).

## Capabilities

### New Capabilities

- `docs/socket-io`: what the three Socket.IO pages must document, where they are linked from, and how their code
  samples are kept true.
- `examples/socket-io-chat`: the example application, its security posture and its tests.

### Modified Capabilities

None. `docs/localization` already requires every page in both languages with the same structure; `examples/tertulia`
keeps its requirements because the example is not changed.

## Impact

- Docs: new `docs/websockets/{socket-io,socket-io-limits,socket-io-migration}.md` and their `docs/es/` translations;
  edited `docs/.vitepress/sidebar.mts`, `docs/websockets/{adapter,gateways,guards}.md`, `docs/migration-guide.md`,
  `docs/faq/examples.md` (and `docs/es/`), `README.md`.
- Example: new `examples/socket-io-chat/**` (a workspace member through `examples/*`, `publish = false`), including
  the vendored client bundle `public/vendor/socket.io.min.js` with its license; `THIRD_PARTY_NOTICES.md`;
  `scripts/smoke-examples.mjs`; `CONTRIBUTING.md` ("Smoke test of the examples"); `parity/socket-io/example-checks/`
  (the official-client script) and one step in the `socket-io-diff` job of #37.
- Build: the example depends on `corvis` with `socket-io` and `socket-io-redis`. Cargo unifies features across the
  workspace, so `cargo build --workspace` and `cargo test --workspace` compile `corvis` with both features from then
  on, and the tests of those features run in the Windows job too (design.md Decision 7 and Risks).
- No framework runtime code changes; the corvis-parity check does not apply (recorded in the definition of done). A
  defect of the framework found while writing the example is reported as an issue, not fixed here.

## Out of scope

- Moving `tertulia` to Socket.IO, or making it selectable between adapters: the `ws`-style pages quote its code, and
  #35 uses it as the proof that the default adapter did not change.
- Rewriting the `ws`-style pages (`gateways.md`, `pipes.md`, `interceptors.md`, `exception-filters.md`): they gain a
  pointer where behavior differs under Socket.IO, nothing else.
- Rustdoc of `corvis::socket_io`: written by #33 to #36 with their code. This change links to it and fixes only a
  sample that the guide shows and that turns out wrong.
- A guide for writing a third-party adapter: the trait is public since #36, but it has one implementation besides
  the default; possible later issue.
- Client-side documentation (reconnection, offline buffering, client options): the official client's own docs.
- A deployment recipe for sticky sessions behind a load balancer: one paragraph and a link in the guide; a full
  recipe under `docs/recipes/` is a possible later issue.
- Serving the client bundle from the framework (`GET /socket.io/socket.io.js`): not planned by #33; the example
  vendors the file.
- A Docker Compose file with Redis and a Node instance for the mixed-cluster walkthrough: the migration page gives
  the commands; possible later issue.
