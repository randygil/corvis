# Design

## Context

Design source: the Socket.IO spike #31 (tasks.md group 9: rewrite `docs/websockets/adapter.md`, extend
`gateways.md`, move Tertulia's chat "or add a new example, per the issue", a migration note whose samples compile or
come from the example). State of the repository verified at `c837dcb` (`origin/dev`); the documentation that #32
already wrote was read in its worktree; #33 to #37 are read as designed in `openspec/changes/`, including their
Review notes.

**Documentation today (on `dev`)**

- `docs/websockets/adapter.md:7` (status `partial`) says gateways use a single built-in adapter and that there is no
  "socket.io-compatible adapter". The rest of the page documents `corvis::ws`, quoting Tertulia's firehose (`:15`).
- `docs/websockets/gateways.md` (status `documented`) describes the JSON `{ event, data }` protocol (`:13-27`) and
  the `#[websocket_gateway]` options (`:87`), and quotes Tertulia's gateway at `:31`, `:85`, `:96`, `:102`, `:173`,
  `:221` and `:352`. `guards.md` (`:43`, `:73`, `:157`), `pipes.md` (`:25`, `:91`), `interceptors.md` (`:13`) and
  `exception-filters.md` (`:87`) quote Tertulia too.
- `docs/migration-guide.md:66` is the only other mention of socket.io (one handler per event).
- Sidebar: the WebSockets group lists six pages (`docs/.vitepress/sidebar.mts:142-149`). `docs/faq/examples.md:69`
  describes `tertulia`; the root `README.md` lists the examples (`:113-115`) and maps the original's gateway
  decorators to Corvis (`:230`).
- `npm run docs:check` (`docs/scripts/check.mjs:1-8`) requires every page in English and Spanish with the same
  `status`, the same headings per level and the same number of code fences, the same sidebar in both locales,
  resolving links, and the public naming rule.

**What the earlier changes of the epic write**

| Change | Docs it writes |
|---|---|
| #32 (task 8.4) | section "Socket.IO (preview)" in `websockets/adapter.md`: the feature flag, the engine path, shutdown, the bounds table (`### Bounds`); rustdoc of `EngineOptions` |
| #33 (8.3, 8.4) | the low-level API with a short example, namespaces, the limits table, divergences and what is not supported, in the same section; rustdoc examples on `IoServer`, `IoSocket::on`, `emit_with_ack`, `use_middleware` |
| #34 (6.1, 6.2) | rooms and broadcast, the operators, broadcast acknowledgements, the limits table, in the same section; rustdoc examples of the operators |
| #35 (7.1, 7.2) | gateways over Socket.IO in the same section (selecting the adapter, handles, body and result mapping, exceptions, hooks, limits); the per-adapter option table in `gateways.md`; "Authenticating on connect" and "Route posture for gateways" in `guards.md`; rustdoc of `IoAdapter`, `IoGatewayServer` |
| #36 (9.1, 9.2) | the Redis adapter in the same section (when to use it, `connect` / `connect_lazy`, mixed clusters, server-side emit, outages, the trust boundary, redaction, the limits table); rustdoc of `RedisAdapter` |
| #37 | none on the site (`CONTRIBUTING.md` only) |

So the reference material exists when this change starts, as one long section ordered bottom-up. In the #32
worktree that section already starts at `adapter.md:151`, after the `corvis::ws` material and before "Security".
Each of those changes leaves "user guide", "migration notes" and "example application" to #38.

**Examples today**

- `examples/tertulia` is the chat example of the `ws`-style adapter: `#[websocket_gateway(path = "/chat")]`
  (`src/chat/chat_gateway.rs:38`), `WsClient` parameters (`:55`), the token read from the query string in
  `handle_connection` and a close with `4401` (`:212-218`), a browser client built on `new WebSocket(...)`
  (`public/app.js:41`), and tests that speak the JSON protocol with `tokio-tungstenite`
  (`tests/chat.rs:14`, `:58`). `openspec/specs/examples/tertulia/spec.md` fixes its posture report, including the
  `/chat` handshake as "not checked". #35 task 2.4 verifies that Tertulia builds and passes unmodified.
- Examples are workspace members through `examples/*` (`Cargo.toml:11`); `corvis`' default features are `jwt` and
  `websockets` (`crates/corvis/Cargo.toml:57`). `scripts/smoke-examples.mjs` starts `blog-orm`, `tertulia` and
  `cats-app` (`:106`, `:132`, `:151`) and is described in `CONTRIBUTING.md:102`.
- The harness pins `socket.io-client` 4.8.4 (`parity/socket-io/package.json`). #33 does not serve the client bundle
  (its proposal, Out of scope: `serveClient`).

## Goals / Non-Goals

**Goals:**

- A reader with a Node `socket.io` backend can go from "does Corvis support this" to a running gateway, and knows
  every difference before switching.
- One place for limits and differences, checked against what the harness lists.
- A runnable example that the official client uses unchanged, safe by default, tested without Node and, in CI, with
  the official client.
- Samples that cannot rot silently.

**Non-Goals:**

- Changing `tertulia` or the `ws`-style pages beyond pointers.
- New framework behavior, rustdoc rewrites, a recipe section, client-side documentation (proposal, Out of scope).

## Decisions

### 1. A new example; Tertulia stays on the `ws`-style adapter

The issue allows either. A new example, `examples/socket-io-chat`, because:

- Five documentation pages of the `ws`-style adapter quote Tertulia's files (Context). Moving it would leave those
  pages without a source, or force this change to rewrite them.
- Tertulia is the unchanged-behavior proof of #35 and has its own posture spec with the handshake "not checked";
  under Socket.IO with connection guards the report is different.
- The two protocols differ in what a good example shows: Tertulia authenticates with a query-string token and a
  close code and correlates replies with an `id` field; under Socket.IO the idiomatic forms are the handshake `auth`
  with a connect error, and acknowledgements. A converted Tertulia would keep `ws` idioms.
- An application cannot serve some gateways with each adapter (#35, Out of scope), so one example cannot show both.

The new example is deliberately smaller than Tertulia: no SSE, no firehose, no persistence (rooms and the last 50
messages per room live in memory). It is about Socket.IO, and what Tertulia shows about guards, pipes and
interceptors per message is the same pipeline.

Alternatives considered: converting Tertulia (rejected above); a second gateway inside Tertulia selected by an
environment variable (rejected: #35 fails `init()` when a gateway's handles do not match the adapter, so both
gateways would have to be written against both handle types); converting `cats-app`'s small chat gateway
(`examples/cats-app/src/chat.rs`; rejected: cats-app is the tour of every HTTP feature and its tests would then
need a Socket.IO client for an unrelated topic).

### 2. What the example contains

```
examples/socket-io-chat/
  Cargo.toml            corvis with features ["socket-io", "socket-io-redis"]; publish = false
  README.md             English; how to run, variables, the protocol table, two instances with Redis, the warning
  src/main.rs           binds 127.0.0.1:$PORT (default 5160)
  src/lib.rs            create_app(): adapter, options, posture, static files, global prefix /api
  src/auth/             login (any nickname, as Tertulia), ChatUser, AuthGuard for HTTP and for sockets
  src/chat/chat_gateway.rs   the gateway on namespace /chat
  src/chat/dto.rs       payloads validated with zod
  src/rooms/            in-memory rooms and history; REST: announcements, members
  public/               index.html, app.js, styles.css, vendor/socket.io.min.js (+ its LICENSE)
  tests/support/mod.rs  raw Socket.IO client over tokio-tungstenite
  tests/chat.rs, tests/startup.rs, tests/official_client.rs, tests/cluster.rs
```

- **Adapter**: `app.use_websocket_adapter(IoAdapter::new().guard_connections(true).options(io_options))`, where
  `io_options` sets `allowed_origins` to the example's own origin (from `ALLOWED_ORIGINS`, default
  `http://127.0.0.1:$PORT,http://localhost:$PORT`), declares no CORS and, when `REDIS_URL` is set, the Redis adapter.
- **Gateway**: `#[websocket_gateway(namespace = "/chat")]`, `#[use_guards(AuthGuard)]`, a zod pipe; handlers `join`
  and `leave` (return values answer the acknowledgement), `message` (returns the stored message to the sender's
  acknowledgement and emits to the room with `socket.to(room)`, which excludes the sender), `typing` (no return, no
  acknowledgement), `rooms`; a `WsException::object` with a code for `NOT_IN_ROOM`. Handles are `IoSocket` and
  `IoGatewayServer`; `OnGatewayConnection<IoSocket>` emits `welcome` and registers the low-level `avatar` handler
  with `IoSocket::on` (a binary argument of at most 64 KiB answered with a binary acknowledgement);
  `OnGatewayDisconnect<IoSocket>` tells the rooms the socket was in.
- **Authentication**: `AuthGuard` reads `handshake().auth.token` through
  `ctx.switch_to_ws().get_client::<IoSocket>()` when the context is the `connect` pseudo-handler, verifies the JWT,
  stores the user on the socket and fails with an unauthorized exception otherwise, so the client's `connect_error`
  carries `Unauthorized` (#35: the message of the exception; a plain `false` would read `Forbidden resource`). On
  messages it only checks that the socket carries a user. `JWT_SECRET` is required, as in Tertulia.
- **REST**: `POST /api/auth/login` (`#[public]`), `GET /api/rooms/:room/members` (reads
  `server.within(room).fetch_sockets()`), `POST /api/rooms/:room/announcements` (moderators; emits with the
  gateway's `IoGatewayServer`).
- **Posture**: `require_route_posture()`; with connection guards on, the `connect` entry of `/chat` is protected.
- **Web client**: plain HTML and JavaScript, no build step: login, room list, messages, typing indicator, the
  `connect_error` and `exception` handlers, `emit(..., callback)` for acknowledgements, an avatar picker for the
  binary handler.

### 3. The official client is vendored at the harness's version

`public/vendor/socket.io.min.js` is a copy of `dist/socket.io.min.js` of `socket.io-client` at the version
`parity/socket-io/package.json` pins, with the package's `LICENSE` next to it and an entry in
`THIRD_PARTY_NOTICES.md`. The README gives the copy command
(`cp parity/socket-io/node_modules/socket.io-client/dist/socket.io.min.js examples/socket-io-chat/public/vendor/`).
The Node-gated test (Decision 4) compares the vendored file byte for byte with the installed package's file, so the
copy cannot drift from the version the harness verifies, and cannot be altered unnoticed.

Alternatives considered: a `<script>` from a CDN with an integrity hash (rejected: the example would need network
access, and local servers are run offline on loopback); bundling with npm in the example (rejected: a build step and
a second lockfile for one file); serving the bundle from the framework (out of scope of #33).

### 4. Tests

- `tests/support/mod.rs`: a minimal Socket.IO client over `tokio-tungstenite` (WebSocket transport only): open
  `ws://.../socket.io/?EIO=4&transport=websocket`, read the open packet, send `40/chat,{"token":...}`, read the
  CONNECT or CONNECT_ERROR answer, send `42/chat,<id>[...]`, read events and acknowledgements, answer pings. About a
  hundred lines; it lives in the example because the helpers of #32 and #33 are under `crates/corvis/tests/` and
  cannot be used from another package. It handles text packets only; the binary handler is covered by the
  official-client test.
- `tests/chat.rs`: one test per scenario of `examples/socket-io-chat` that needs one instance and no Node: connect
  without and with a token, join acknowledged, broadcast without the sender, `NOT_IN_ROOM` as `exception` with the
  acknowledgement unanswered, validation error, typing, announcement through REST, members, the handshake from
  another origin, no CORS headers, the posture report. `tests/startup.rs`: no `JWT_SECRET`; `REDIS_URL` pointing at a
  closed loopback port (the error does not hold the credentials).
- `tests/official_client.rs`, gated like `socket_io_diff.rs` (`CORVIS_NODE_DIFF=1`, otherwise a notice): starts the
  application in-process on `127.0.0.1:0` and runs `node parity/socket-io/example-checks/socket-io-chat.mjs <url>`.
  The script lives under `parity/socket-io/` so that it resolves `socket.io-client` from the harness's
  `node_modules` (one lockfile). It drives two official clients through connect error, connect, join, message,
  exception and the binary acknowledgement, and compares the vendored bundle with the installed one. It asserts
  expected values; it is a check of the example, not a differential scenario (there is no Node twin of the example).
- `tests/cluster.rs`, run only with `CORVIS_REDIS_URL` (skips with a notice otherwise; fails when
  `CORVIS_REDIS_REQUIRED=1` and Redis is unreachable, the convention of #36): two applications in-process with one
  key of their own, a raw client on each, a message crossing instances.
- `scripts/smoke-examples.mjs`: a `socket-io-chat` entry (login, then `GET /socket.io/?EIO=4&transport=polling`
  answers an open packet); the script stays dependency-free.
- CI: the `socket-io-diff` job of #37 gains one step, `cargo test -p socket-io-chat --test official_client --test
  cluster` (it already has Node, the harness install, Redis and the variables).

### 5. Three pages, and what each takes from the preview section

| Page | Content | Source |
|---|---|---|
| `websockets/socket-io` (guide) | 1 Enabling it (feature, `use_websocket_adapter`, opt-in). 2 A first gateway and the client. 3 Options per adapter: `namespace`, `path`, `cors`, shared Engine.IO servers. 4 Handles and the message body. 5 Results, acknowledgements and events. 6 Exceptions. 7 Lifecycle hooks. 8 Authenticating connections. 9 Rooms and broadcast. 10 Acknowledgements from the server and timeouts. 11 Binary payloads and the low-level API. 12 CORS and origins. 13 Several instances with Redis. 14 Shutdown. 15 Testing. 16 See also | written here, top-down; the reference paragraphs of #33 to #36 are moved and edited, not duplicated |
| `websockets/socket-io-limits` | one table per layer (Engine.IO, Socket.IO, rooms, gateways, Redis): option, default, Node's value, what the client observes; "Differences from Node" (every intentional divergence with its reason); "Not supported"; "How this is verified" (pinned versions, suites, harness, CI) | the five tables and divergence lists of the preview section, moved; the verification paragraph is new |
| `websockets/socket-io-migration` | what stays the same for clients; the equivalence table (server, namespace, socket, operators, middleware, adapter); gateways written for the original framework; differences to review; rolling migration with a mixed cluster; checklist | new |

`websockets/adapter.md` keeps its `corvis::ws` material and gets a short section "Gateway adapters": the `ws`-style
adapter (default) and the Socket.IO adapter (opt-in), two sentences each, links to `gateways` and `socket-io`; its
opening paragraph no longer says there is no Socket.IO adapter. Its status stays `partial` only if it still says
that custom adapters cannot be registered (true after #35, whose adapter trait is sealed).

Sidebar order of the WebSockets group: Gateways, Socket.IO, Socket.IO limits, Socket.IO migration, Exception
filters, Pipes, Guards, Interceptors, Adapters. Spanish titles: "Socket.IO", "Límites de Socket.IO", "Migración a
Socket.IO".

The section named "preview" disappears: after #37 the behavior is verified by a required CI job. "Opt-in" is the
accurate word (spike Review note 1: the default adapter and the default features are decided for 1.0).

Alternatives considered: one page (rejected: the guide plus five tables plus the migration is several thousand
words, and the limits are reference material people search, not read); keeping the limits in `adapter.md`
(rejected: a reader of "Adapters" looks for `corvis::ws`); a page per layer (rejected: the layers are an
implementation order, and a gateway user never needs "Engine.IO" as a topic).

### 6. Samples come from code that is compiled

- Rust samples on the three pages are excerpts of `examples/socket-io-chat` (the text names the file, as the
  `ws`-style pages do for Tertulia) or the body of a rustdoc example of `corvis::socket_io` written by #33 to #36,
  which `cargo test --doc` compiles. The example is written first (tasks group 2) so the guide can quote it.
- Client samples use only calls made by `public/app.js` or by the official-client check.
- The equivalence table of the migration page has a Node column that nothing in the repository compiles. Its rows
  are restricted to calls that the reference servers of the harness make (`parity/socket-io/reference/*.mjs`), so
  each row is a pair the harness has compared.
- The limits tables are checked by hand against the option defaults in the rustdoc and against
  `parity/socket-io/expected-differences.json` (task 4.2 lists every entry and the row that documents it in
  Implementation notes).

Alternative considered: a script that extracts fenced samples from the pages and compiles them. Rejected for this
change: the site has no such tool for any page (the README has its own rule, `openspec/specs/docs/readme`), and
building one is the docs epic's decision.

### 7. Features of the example and the workspace build

The example enables `socket-io` and `socket-io-redis` unconditionally and chooses the adapter at run time from
`REDIS_URL`. Consequence: Cargo unifies features across workspace members, so `cargo build --workspace`,
`cargo clippy --workspace` and `cargo test --workspace` build `corvis` with both features once the example exists,
and the `#![cfg(feature = "socket-io")]` test files of `crates/corvis` run in the workspace test step on Linux and
on Windows (Redis tests skip on Windows, which has no service). The feature-off configuration stays covered by
`cargo build -p corvis`, by the per-feature clippy loop of `ci.yml` and by the packaging job. The explicit steps
`cargo test -p corvis --features socket-io` and `--features socket-io-redis` that #32 and #36 added to the Linux job
become redundant with the workspace step; task 5.3 measures the job and removes them if the workspace step runs the
same tests, or keeps them and says why.

Alternative considered: Redis behind a cargo feature of the example, off by default. Rejected: the Redis code of
the example would not be compiled by `cargo clippy --workspace` or tested by `cargo test --workspace`, and would rot;
and `socket-io` alone would unify anyway.

### 8. The migration page

Sections and what each must say (the content is taken from the approved designs):

- **Clients**: `socket.io-client` 4.x unchanged; Engine.IO v3 clients are refused (#32); the client bundle route is
  not served (#33).
- **Server API table**: `new Server(httpServer, opts)` to `IoAdapter` options or `IoServer::new`; `io.of`,
  `io.use` / `nsp.use` to `use_middleware`; `socket.on` / `emit` / `emitWithAck` / `timeout`; `socket.join` /
  `leave` / `rooms`; `io.to` / `in` (named `within`) / `except` / `local`; `socket.broadcast`; `fetchSockets`,
  `socketsJoin`, `socketsLeave`, `disconnectSockets`; `serverSideEmit`; `socket.handshake`, `socket.data`;
  `createAdapter(pub, sub)` to `RedisAdapter::connect`.
- **Gateways written for the original framework**: what is the same (decorator equivalents, body mapping, result
  mapping, `exception` event) and the listed differences (#35 Review notes 4 and 8, binary bodies, connection
  guards as an opt-in addition).
- **Differences to review**: a link to the limits page, with the ones most likely to matter called out (limits that
  Node does not have, handshake redaction in `fetch_sockets`, no state recovery, no dynamic namespaces).
- **Rolling migration**: prerequisites (Node instances on `@socket.io/redis-adapter` 8.x, the classic adapter, the
  same key; #36 Review note 11), session affinity for polling clients, replace one instance, watch the adapter's
  counters, repeat; what a mixed cluster does not share (in-process state, connection state recovery); rollback.
- **Checklist**.

### 9. Spanish translation and naming

Each page is written in English first and translated to `docs/es/` with the same headings, fences and status
(`docs/scripts/check.mjs`). Spanish uses neutral Latin American Spanish, without voseo (the check enforces it). The
pages and the example call the original Node library "socket.io" and the protocol "Socket.IO"; the framework the
gateway decorators come from is "the original framework", never by name (`scripts/check-naming.mjs` scans `docs/**`
and `examples/**`). The vendored bundle is minified third-party code: task 2.6 runs the naming check on it and adds
an allow entry with the reason only if a banned word appears in it.

## Risks / Trade-offs

- Two chat examples to maintain → the new one is small and shares no code with Tertulia; its tests run in the
  workspace step.
- The guide describes APIs of five changes that are approved but not merged when this is drafted → tasks group 1
  reconciles with the merged code before anything is written; every sample comes from compiled code.
- Workspace feature unification changes what `cargo test --workspace` builds (Decision 7) → the feature-off build is
  still checked by three other steps; build time of the workspace step grows by the `redis` and `rmp` crates.
- A vendored minified file in the repository → about 50 KiB, MIT, listed in `THIRD_PARTY_NOTICES.md`, compared with
  the pinned package in CI.
- The limits page can drift from the option defaults → the same risk as every reference table of the site; the
  expected-differences cross-check is a task of this change and a line of the checklist in `CONTRIBUTING.md` for
  later changes of those defaults.
- The raw client in the example's tests is a third copy of a minimal Socket.IO client (after the helpers of #32 and
  #33) → it is WebSocket-only and text-only; a shared test-support crate is a possible later issue.
- The example's login accepts any nickname → stated in the README and on the examples page, as for Tertulia; the
  server binds to loopback.

## Open questions

1. New example (`examples/socket-io-chat`) or move Tertulia? The draft adds a new example and leaves Tertulia
   unchanged (Decision 1). Recommendation: keep the new example; revisit if the maintainer wants a single chat
   example for 1.0.
2. The example's name. Alternatives: `tertulia-io`, `sala`. Recommendation: `socket-io-chat`, which says what it
   shows in the examples table.
3. Three pages or one (Decision 5)? Recommendation: three.
4. Drop the word "preview" once #37's job is required, keeping "opt-in"? Recommendation: yes. If the maintainer
   wants a stability note until 1.0, one sentence on the guide ("the low-level API may change before 1.0") instead
   of a section title.
5. Vendoring the client bundle (Decision 3) or loading it from a CDN with an integrity hash. Recommendation: vendor.
6. `socket-io-redis` always enabled in the example (Decision 7), which makes workspace builds compile `corvis` with
   both Socket.IO features. Recommendation: accept; say so in `CONTRIBUTING.md`.
7. Should the README of the example be in English? Tertulia's is in Spanish, the other published texts are in
   English. Recommendation: English, and translating Tertulia's README is left to the docs epic.
8. `ROADMAP.md` already names this change `e2-socket-io-docs-and-example`; nothing to rename here. It does need the
   name of #37 updated (`e2-socket-io-differential-tests`), noted in that change.

## Review notes

These settle the open questions and override the design where they differ.

1. A new example is added; Tertulia is not touched.
2. The example is `examples/socket-io-chat`.
3. Three pages, in English and Spanish.
4. "preview" is removed; the feature is described as opt-in, with one sentence on stability until 1.0.
5. The official client bundle is copied into the repository at the version the harness pins, with its license
   file next to it, and compared byte for byte in CI.
6. The example always enables `socket-io-redis`; the feature unification it causes in workspace builds is
   documented in `CONTRIBUTING.md`, and CI keeps a build of `corvis` without features.
7. The example's README is in English.
8. Implemented after #37 lands.
