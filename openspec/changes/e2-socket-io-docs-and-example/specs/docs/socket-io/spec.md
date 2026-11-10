# Spec Delta

## Purpose

Users can learn from the documentation site how to serve gateways over Socket.IO, what limits and differences from
Node apply, and how to migrate a Node `socket.io` backend, in English and in Spanish.

## ADDED Requirements

### Requirement: Socket.IO guide page
The documentation site SHALL have a Socket.IO guide at `/websockets/socket-io` and `/es/websockets/socket-io`,
listed in the WebSockets section of the sidebar in both languages, with status `documented`. It MUST show how to
enable the `socket-io` feature, select the adapter with `use_websocket_adapter`, declare a gateway and connect to it
with the official client.

#### Scenario: Reader starting from nothing
- **WHEN** a reader follows the first section of `/websockets/socket-io` in a new application
- **THEN** they have the Cargo feature, the adapter call, a gateway with one handler and a client snippet that
  receives its acknowledgement

#### Scenario: Spanish translation
- **WHEN** a reader opens `/es/websockets/socket-io`
- **THEN** the same content is shown in Spanish with the same headings and code samples, and `npm run docs:check`
  passes

#### Scenario: Opt-in is stated
- **WHEN** a reader looks for whether Socket.IO is on by default
- **THEN** the guide states that the feature and the adapter are opt-in and that gateways use the `ws`-style adapter
  otherwise

### Requirement: The guide covers gateways over Socket.IO end to end
The guide MUST document the gateway options per adapter (`namespace`, `path`, `cors`), the handles `IoSocket` and
`IoGatewayServer`, how event arguments form the message body, how handler results map to acknowledgements and
events, the `exception` event, lifecycle hooks, rooms and broadcast operators, acknowledgements with timeouts, and
binary payloads.

#### Scenario: The two meanings of namespace
- **WHEN** a reader who used `#[websocket_gateway(namespace = "/chat")]` with the default adapter reads the options
  section
- **THEN** a table shows that it is the URL path under the `ws`-style adapter and the Socket.IO namespace under the
  Socket.IO adapter, with the Engine.IO path set by `path`

#### Scenario: Return value and acknowledgement
- **WHEN** a reader looks for how to answer a client's `emit` with a callback
- **THEN** the guide shows a handler returning a value, the client receiving it in its callback, and states what
  happens to the value when the client sent no callback

#### Scenario: Binary payloads
- **WHEN** a reader needs to receive or send binary data
- **THEN** the guide states that the gateway pipeline carries JSON and shows a low-level handler registered with
  `IoSocket::on` in the connection hook

### Requirement: The guide states how connections are authenticated
The guide MUST state that guards run per message and not on connection unless `guard_connections` is enabled, show
how to enable it and how to read the handshake `auth` in a guard, and state what a refused client observes. It MUST
link to the route posture page for what the posture check reports about Socket.IO connections.

#### Scenario: Reader assuming a global guard protects the connection
- **WHEN** a reader with a global authentication guard reads the authentication section
- **THEN** a warning states that an unauthenticated client can connect and receive broadcasts unless connection
  guards are on, and the next sample enables them

#### Scenario: Refused client
- **WHEN** a reader looks for what the client sees when a connection guard denies
- **THEN** the guide shows the `connect_error` handler of the official client and the message it receives

### Requirement: The guide documents scaling out with the Redis adapter
The guide MUST document the `socket-io-redis` feature, `RedisAdapter::connect` and `connect_lazy`, passing the
adapter to the gateway adapter's options, what happens while Redis is unreachable, and that the Redis server is part
of the application's trust boundary. It MUST state that long-polling clients need session affinity at the load
balancer.

#### Scenario: Two instances
- **WHEN** a reader runs two instances of an application behind a load balancer
- **THEN** the guide shows the configuration with which a broadcast from one instance reaches clients of the other

#### Scenario: Who can publish to Redis
- **WHEN** a reader looks for the security implications of the adapter
- **THEN** the guide states that whoever can publish to that Redis can emit to every client, and recommends
  credentials, an ACL limited to the adapter's channels and TLS

### Requirement: Limits and differences page
The site SHALL have a page at `/websockets/socket-io-limits` and `/es/websockets/socket-io-limits`, listed in the
sidebar, with every bounded default of the Engine.IO, Socket.IO, rooms, gateway and Redis layers: the option, its
default, Node's value and what a client observes at the limit. It MUST also list every intentional difference from
Node with its reason.

#### Scenario: Looking up a limit
- **WHEN** a reader's client is disconnected after sending many events at once
- **THEN** the page lets them find the bound that applies, its default and the option that changes it

#### Scenario: Every expected difference is documented
- **WHEN** the entries of `parity/socket-io/expected-differences.json` are compared with the page
- **THEN** each intentional difference a client can observe has a row with its reason

#### Scenario: What is not supported
- **WHEN** a reader looks for connection state recovery, dynamic namespaces, the `volatile` flag, Engine.IO v3
  clients or the client bundle route
- **THEN** the page lists each as not supported, with the alternative where there is one

### Requirement: The limits page states how parity is verified
The limits page MUST state that behavior is checked against pinned versions of Node `socket.io`, `engine.io` and
`@socket.io/redis-adapter` by the official protocol suites and by a differential harness that runs in CI, name those
versions, and state that any other difference from Node is a bug to report.

#### Scenario: Reader evaluating compatibility
- **WHEN** a reader wants to know which Node versions Corvis was compared with
- **THEN** the page names the pinned versions, the same ones `parity/socket-io/package.json` holds

### Requirement: Migration page
The site SHALL have a page at `/websockets/socket-io-migration` and `/es/websockets/socket-io-migration`, listed in
the sidebar, for moving a Node `socket.io` backend to Corvis. It MUST hold an equivalence table from the Node server
API to Corvis, the differences to review before switching, and the steps of a rolling migration with Node and Corvis
instances on the same Redis.

#### Scenario: Looking up an API
- **WHEN** a reader looks for the equivalent of `io.to(room).except(other).emit(...)`, `socket.join(room)`,
  `io.of("/admin").use(fn)`, `socket.timeout(ms).emitWithAck(...)` or `io.fetchSockets()`
- **THEN** the table shows the Corvis call for each

#### Scenario: Clients are not changed
- **WHEN** a reader asks what to change in the browser code
- **THEN** the page states that the official client is used unchanged, and lists the client-visible differences

#### Scenario: Rolling migration
- **WHEN** a reader runs several Node instances with the Redis adapter
- **THEN** the page gives the order of steps to replace them one at a time, the adapter version the Node instances
  need, and what is not shared across a mixed cluster

#### Scenario: Gateway written for the original framework
- **WHEN** a reader has gateways written for the original framework's Socket.IO platform adapter
- **THEN** the page lists what behaves the same and the listed differences (HTTP exception payloads, binary message
  bodies, one handler per event)

### Requirement: Socket.IO code samples are taken from tested code
Every Rust sample on the three pages MUST be an excerpt of `examples/socket-io-chat`, named next to the sample, or
the body of a rustdoc example of `corvis::socket_io` that `cargo test --doc` runs. Every client sample MUST use only
calls that the example's web client or the official-client check of the example makes.

#### Scenario: API renamed later
- **WHEN** a later change renames a method that a page's sample calls
- **THEN** the example or the rustdoc example it was taken from fails to compile, which points at the sample

#### Scenario: Reviewing a sample
- **WHEN** a reviewer reads a Rust sample on the guide
- **THEN** the text names the example file it comes from, or links the documented item whose example it is

### Requirement: Existing pages lead to the Socket.IO pages
`/websockets/adapter` MUST no longer hold the per-layer Socket.IO material: it SHALL name both gateway adapters and
link to the guide. `/websockets/gateways`, `/websockets/guards`, `/migration-guide` and `/faq/examples`, in both
languages, and the root `README.md` MUST link to the guide or to the example where they describe behavior that
differs under Socket.IO.

#### Scenario: Reader on the gateways page
- **WHEN** a reader of `/websockets/gateways` reaches the protocol section
- **THEN** a note says the page describes the `ws`-style adapter and links to `/websockets/socket-io`

#### Scenario: No leftover preview
- **WHEN** the documentation sources are searched for "preview" next to "Socket.IO"
- **THEN** nothing is found, and `/websockets/adapter` has no limits table for Socket.IO

#### Scenario: Examples page
- **WHEN** a reader opens `/faq/examples`
- **THEN** `socket-io-chat` is listed with the command to run it and what it shows
