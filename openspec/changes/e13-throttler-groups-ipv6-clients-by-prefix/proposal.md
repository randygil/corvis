# Proposal

Implements [randygil/corvis#154](https://github.com/randygil/corvis/issues/154) (epic
[#13](https://github.com/randygil/corvis/issues/13)). Found by the security review of #135
(`e13-throttler-block-duration-like-the-original`, findings 1 and 4). It edits the same guard loop as #120
(`e13-rate-limit-headers-in-the-throttler`) and #135 and is implemented **after both** (see Impact, Order).

## Why

Since 6.7.0, `@nestjs/throttler` identifies a client by `normalizeIp(req.ip, ipv6SubnetPrefix)`: IPv4 addresses as
they are, IPv4-mapped IPv6 addresses as their IPv4 form, `::1` as `::1`, and every other IPv6 address reduced to its
/64 prefix by default. Corvis keys the counters on the full `req.ip()`
(`crates/corvis/src/modules/throttler.rs:199`). A host or home connection usually gets a whole IPv6 /64, so a client
can rotate through 2^64 source addresses and get a fresh limit with each, and each new address creates a new
throttler key, which is what makes filling the counter table cheap. The original is stricter here, so matching it
closes a bypass and costs no security.

## What Changes

- **Behavior change:** `ThrottlerGuard` builds the client part of each counter key with the new
  `normalize_ip(&req.ip(), prefix)`, a port of the original's `normalizeIp`:
  - a valid IPv4 address is kept as is;
  - `::1` stays `::1`;
  - an IPv4-mapped IPv6 address (`::ffff:a.b.c.d`) becomes `a.b.c.d`, so it shares the counter of the IPv4 client;
  - any other valid IPv6 address becomes its masked prefix in compressed form plus the length, e.g.
    `2001:db8:1:2::/64`;
  - a zone identifier (`%eth0`) is dropped from valid addresses; anything that is not an IP address (a forged
    `X-Forwarded-For` entry under `trust proxy`) is kept unchanged.
- **Configurable prefix:** `ThrottlerStorage::with_ipv6_subnet_prefix(prefix: u8)` (default
  `DEFAULT_IPV6_SUBNET_PREFIX` = 64, values above 128 clamp to 128), like the original's module option
  `ipv6SubnetPrefix`. It applies to every throttler. `128` keeps each IPv6 address apart (the opt-out); smaller
  values such as `56` or `48` group larger allocations.
- `normalize_ip` and `DEFAULT_IPV6_SUBNET_PREFIX` are public (the original exports `normalizeIp`), so code that calls
  `ThrottlerStorage::record_hit` directly can build the same tracker.
- `req.ip()`, `req.ips()` and `#[ip]` keep returning the full address, as Express's `req.ip` does; only the throttler
  key changes.
- The tracker follows the `trust proxy` setting through `req.ip()`, as today: with a trusted proxy it is the
  normalized client entry of `X-Forwarded-For`.
- Differential test: a fixture runs the original `normalizeIp` (6.7.1) over a corpus of addresses and prefixes and
  the Rust test compares every output string; guard-level tests for IPv4, IPv6, IPv4-mapped IPv6, loopback, the
  prefix setting and `X-Forwarded-For` with `trust proxy` on.
- Docs (EN + ES): the rate limiting page.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `security/rate-limiting` (created by #120): requirements added for how the guard derives the client part of the
  key, the prefix setting, the public helper and its parity with the original.

## Impact

- `crates/corvis/src/modules/throttler.rs`: `normalize_ip`, `DEFAULT_IPV6_SUBNET_PREFIX`, the
  `ipv6_subnet_prefix` field and `with_ipv6_subnet_prefix` on `ThrottlerStorage`, the key built in
  `ThrottlerGuard::can_activate`; unit tests.
- `crates/corvis/src/lib.rs`: export `normalize_ip` and `DEFAULT_IPV6_SUBNET_PREFIX` in the `modules::throttler`
  re-export (not the prelude).
- `crates/corvis/tests/fixtures/throttler/` (created by #120): `normalize-ip.mjs` and its JSON output.
- `crates/corvis/tests/trust_proxy.rs`: integration tests with IPv6 and IPv4-mapped entries in `X-Forwarded-For`.
- Docs: `docs/security/rate-limiting.md` and `docs/es/security/rate-limiting.md`.
- Existing tests keep their expectations: every throttler test uses IPv4 clients (`127.0.0.1`, or IPv4
  `X-Forwarded-For` entries in `crates/corvis/tests/trust_proxy.rs:84-96` and
  `crates/corvis/tests/input_hardening.rs:127-138`), and IPv4 is unchanged.
- Apps whose clients reach them over IPv6: clients in one /64 now share their limits. Apps that listen on `[::]` on a
  dual-stack Linux socket, where IPv4 clients appear as `::ffff:a.b.c.d`, are keyed by the IPv4 form (same
  equivalence as before, different string).
- Port apps: none uses the throttler (#135's proposal, Impact), so apidiff should not move; the parity check confirms
  it.
- **Order:** after #120 and #135. They rewrite the guard loop (headers, `record_hit`, `ttl` order) and #120 creates
  the fixture folder and the `security/rate-limiting` spec. This change touches one line of that loop plus a field
  and a builder, so it does not depend on #152 (skips) or #156 (per-name overrides) and can go before or after them.

## Out of scope

- A custom tracker hook (`getTracker` at module, throttler or route level): Corvis has none; the original's way to
  opt out of normalization is a custom `getTracker`. #156 checks the route-level `getTracker` override; a module-level
  hook may become a new issue.
- A custom key generator (`generateKey`): Corvis keeps its own key layout (throttler name, tracker, controller,
  handler); possible new issue together with the tracker hook.
- Gateway handshakes ignore `trust proxy`: the WebSocket handshake `Request` is built with
  `Request::new` (`crates/corvis/src/websockets.rs:725-729`), whose default parser options have `TrustProxy::None`, so
  a global `ThrottlerGuard` keys gateway messages on the proxy's address behind a proxy; possible new issue.
- Validating or normalizing `req.ip()` itself (Express returns the raw `X-Forwarded-For` entry; Corvis keeps that).
- Per-throttler prefix lengths (the original has one module-level `ipv6SubnetPrefix`).
