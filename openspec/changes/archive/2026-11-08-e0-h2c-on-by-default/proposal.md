# Turn h2c off by default

Implements [randygil/corvis#144](https://github.com/randygil/corvis/issues/144) (epic
[#1](https://github.com/randygil/corvis/issues/1)).

## Why

`ServerOptions::default()` enables HTTP/2 with prior knowledge (h2c) on the plain-text listener
(`crates/corvis/src/server.rs:63`). Node's `http` module does not speak it, so a migrated application accepts a
protocol its original never did, and a front proxy or a security scanner may not expect it. Measuring what h2c
changes (design.md, Context) shows no cost for an HTTP/1 client, but also no user: nothing in the workspace, the
examples, the benchmark or the three port apps uses it, and common proxies need explicit configuration to send h2c.
With h2c on, the connection limits stop meaning what they say: one connection carries up to 200 concurrent streams,
so `max_connections` limits sockets, not requests. The maintainer decided on 2026-11-08 to turn it off by default.

## What Changes

- **BREAKING (behavior)**: `ServerOptions::default()` has `h2c: false`. A client that connects with HTTP/2 prior
  knowledge gets the connection closed without a response, as it already did with `h2c(false)`. HTTP/1.x is not
  affected.
- `ServerOptions::h2c(true)` is the opt-in; its rustdoc states the default and the limits that change with it.
- Tests: the stalled-preface test sets `h2c(true)` explicitly; new tests for the default (a prior-knowledge
  connection is closed without a response) and for `h2c(true)` (an HTTP/2 request succeeds).
- Docs in English and Spanish: the default is HTTP/1.1 only, with `h2c(true)` as the opt-in and its limits.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `http/connection-close`: a connection that starts with the HTTP/2 preface is refused by default; adds the
  requirement that HTTP/2 without TLS is opt-in.

## Impact

- Code: `crates/corvis/src/server.rs` (default at line 63, rustdoc of `h2c` at lines 107-112, struct doc at line 33).
- Tests: `crates/corvis/tests/transport_security.rs`.
- Docs: `docs/http/performance.md`, `docs/security/http-parsing.md`, `docs/deployment.md`,
  `docs/faq/multiple-servers.md` and the copies under `docs/es/`.
- Behavior change for anyone relying on h2c without setting it; there is no CHANGELOG in the repository to note it in.

## Out of scope

- Mapping the HTTP/2 limits (concurrent streams, header list size) onto `ServerOptions`: a separate issue.
- Changing the `TimedIo` preface detection or the framing check beyond what the default needs (design.md, Decision 3).
- Any change to `e0-reject-ambiguous-http1-framing` (#146), on which this change builds.
