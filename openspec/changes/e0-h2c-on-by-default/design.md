# Design

## Context

- **Default.** `crates/corvis/src/server.rs:63` set `h2c: true` in `ServerOptions::default()` (now `false`). The
  setter is `ServerOptions::h2c` at `:112`; its rustdoc said "Enabled by default; `false` accepts only HTTP/1.x".
- **How it is enforced.** `serve` builds a hyper-util `auto::Builder` (`server.rs:812`) that serves both protocols and
  detects the HTTP/2 preface itself. h2c off is enforced one layer below, in `TimedIo`: `TimedIo::new` sets
  `preface: Some(0)` when h2c is off (`:575`) and `TimedIo::on_read` (`:598-613`) matches incoming bytes against
  `H2_PREFACE` (`:470`); a complete preface sets `no_linger` and returns `InvalidData`, so the connection is closed
  without a response and without a drain. Any other first bytes clear the matcher (`:605`).
- **The framing check of #146.** `h1_framing::Framing` starts in `St::Preface` (`h1_framing.rs:121`, matching at
  `:257-270`): a full preface switches it to `St::Passthrough`, so an h2c connection is not parsed as HTTP/1. That
  state is needed whenever h2c is on; with h2c off `on_read` has already closed the connection by the time the
  preface is complete.
- **Measured cost for HTTP/1 clients.** None either way: the preface matcher compares the first bytes of a connection
  and drops out at the first mismatch, and the HTTP/2 limits are never reached by an HTTP/1 request.
- **Who uses h2c.** Nothing in the workspace: not the crates, the examples, `bench/`, or the three port apps. The only
  uses are the tests in `crates/corvis/tests/transport_security.rs` (the stalled preface at `:578-580` and
  `h2c_can_be_disabled` at `:608-616`, line numbers before this change). Common proxies need explicit configuration
  to send it (Caddy `h2c://`, Traefik `scheme=h2c`, nginx `grpc_pass`, Cloud Run with an `h2c` port).
- **What h2c changes when on.**
  - One connection carries up to 200 concurrent streams (hyper's default), so `max_connections` limits sockets, not
    requests: with `max_connections(1)`, about 100 requests were in flight and served.
  - The HTTP/2 header list limit is 16 KiB, separate from `max_header_size`, which is configured on the HTTP/1
    builder only (`server.rs:815-821`).
  - The headers and keep-alive timeouts in `TimedIo::poll_idle` (`:617`) apply per connection and only while no
    request is in flight, so they do not bound an individual HTTP/2 stream.
- **Docs that said "on by default".** `docs/http/performance.md:17-19`, `docs/security/http-parsing.md:99-101`,
  `docs/deployment.md:63,96`, `docs/faq/multiple-servers.md:9` and the copies under `docs/es/`. They pointed at
  issue #144 as undecided.

## Goals / Non-Goals

**Goals:**
- A migrated application accepts the same protocols as its Node original by default.
- The opt-in exists, is documented with the limits that change, and is tested.

**Non-Goals:**
- Mapping the HTTP/2 limits (max concurrent streams, header list size) onto `ServerOptions`: a separate issue.
- Restructuring `TimedIo` or the framing check.

## Decisions

1. **Flip the default, keep the option.** `h2c: false` in `Default`; `h2c(true)` is the opt-in and `h2c(false)` stays
   valid (a no-op by default). No new option, no deprecation: the setter keeps its name and meaning.
2. **Keep the refusal where it is.** With h2c off the connection is still closed by `TimedIo::on_read` when a full
   preface arrives, with no response, as `h2c_can_be_disabled` already tested. The alternative,
   `auto::Builder::http1_only()`, would let hyper parse the preface as an HTTP/1 request line and answer with an error
   response, which changes what a prior-knowledge client observes and what the spec scenario states.
3. **Do not simplify the preface handling of #146.** The `St::Preface` state of `Framing` could be skipped when h2c is
   off, because `on_read` closes the connection first. It stays: it is a few lines, it is needed for `h2c(true)`, and
   making `Framing::new` depend on `h2c` would add a parameter to a change that is still pending review and touch the
   files #146 reworked. It can be revisited after #146 is archived.
4. **Tests.**
   - `silent_and_slow_header_connections_are_closed` relied on the default (a stalled preface is closed by the headers
     timeout); it now sets `.h2c(true)`.
   - `h2c_is_off_by_default`: default options, a prior-knowledge connection is closed with no bytes; an HTTP/1.1
     request still works.
   - `h2c_can_be_enabled`: `h2c(true)`, a hyper HTTP/2 client with prior knowledge gets `200`, version HTTP/2, and the
     body.
   - `h2c_can_be_disabled` stays as it is.
5. **Docs text** states: HTTP/1.1 only like Node's `http`; `h2c(true)` for a proxy or platform configured to send it
   (the four examples above); with h2c on, up to 200 concurrent requests per connection, `max_connections` limits
   connections, the HTTP/2 header limit is 16 KiB and `max_header_size` is HTTP/1 only. Deployment and FAQ pages say
   "HTTP/1.1 (HTTP/2 with `h2c(true)`)".

## Risks / Trade-offs

- [A deployment already behind an h2c-speaking proxy breaks on upgrade] -> it is a documented behavior change in the
  proposal and docs; the fix is one call, `h2c(true)`. Nothing in the repository depends on it.
- [Merge conflicts with #146] -> this change is built on top of that branch and touches only the default, two doc
  comments in `server.rs` and the tests; it does not modify the change folder of #146.
- [gRPC clients need HTTP/2] -> Corvis has no gRPC support; those users opt in.

## Open questions

- None.

## Review notes (2026-11-08)

Maintainer decision, 2026-11-08: turn h2c off by default; `ServerOptions::h2c(true)` becomes the opt-in, like Node's
`http`. Mapping the HTTP/2 limits onto `ServerOptions` is a separate issue and not part of this change.
