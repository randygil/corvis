# Design

## Context

See proposal.md for motivation; the behavior is in the spec delta. Current state verified in the code at `510576f`
(main) and on the #135 branch (`worktree-agent-aaad1f4e637013b99`, `7ddc2d5`, which includes #120). Paths are under
`crates/corvis/` unless stated.

**Corvis: the throttler key**

- On main, `ThrottlerGuard::can_activate` (`src/modules/throttler.rs:189-206`) builds each key as
  `format!("{}:{}:{}:{}", t.name, req.ip(), ctx.get_class().name, ctx.get_handler().name)` (line 199) and passes it to
  `ThrottlerStorage::increment`, which stores `sha256(key)` (lines 102-104). The client part is the full `req.ip()`
  string, so every distinct IPv6 address gets its own counter.
- On the #135 branch the same line is at `throttler.rs:551`; the loop now calls `record_hit` and `set_headers`
  (`impl CanActivate for ThrottlerGuard`, lines 541-569). `ThrottlerStorage` there already carries guard settings next to the counters: `block_durations`,
  `max_blocked`, `headers` (struct at lines 267-274, builders `with_max_blocked`, `with_headers`, `with_max_entries` and
  `with_block_duration` at lines 303, 311, 318 and 330). Keys are still hashed, so the tracker string only matters for which
  requests share a counter.
- `ThrottlerModule::for_root_async` accepts a factory returning a configured `ThrottlerStorage` (main
  `throttler.rs:156-172`), so any new `ThrottlerStorage` builder is reachable from async configuration too.
- Exports: `src/lib.rs:275-277` (`modules::throttler` re-export at the crate root) and the prelude (lines
  284-306), which holds `SKIP_THROTTLE`, `THROTTLE_LIMIT`, `THROTTLE_TTL`, `ThrottlerGuard`, `ThrottlerModule` and
  `ThrottlerOptions` but not `ThrottlerStorage`.

**Corvis: how the client IP is derived (`trust proxy`)**

- `Request::ip()` (`src/http/request.rs:148-153`) returns the last element of `proxy_addrs()` (lines 165-168), which
  is `proxy_addr::all_addrs(self.socket_ip(), &self.headers, &self.trust_proxy)`. `socket_ip()` (lines 162-164) is
  `remote_addr.ip().to_string()`, or `"127.0.0.1"` when there is no socket address.
- `all_addrs` (`src/http/proxy_addr.rs:183-199`) returns only the socket address with the default
  `TrustProxy::None`, so `X-Forwarded-For` is ignored. Otherwise it walks the socket address and the
  `X-Forwarded-For` entries from right to left (`forwarded`, lines 168-181: entries split on `,` and trimmed of
  spaces, not validated) and stops at the first address the setting does not trust (`TrustProxy::trusts`, lines
  84-95). The client entry is returned as the raw string: with `TrustProxy::All` or `Hops(n)` it can be any text the
  client sent.
- Trusted subnets already treat IPv4-mapped IPv6 like IPv4 (`Subnet::contains`, lines 141-149), but `req.ip()` itself
  does not convert the client address.
- The setting is `CorvisApplication::set_trust_proxy` (`src/application.rs:407-408`), stored in the body parser
  options that `Request::with_parser` copies into each request (`src/http/request.rs:82`).
- `#[ip]` parameters read `req.ip()` (`src/router/mod.rs:795`).
- Listening: `listen(port)` binds `0.0.0.0` (`ListenAddr for u16`, `src/application.rs:712-716`), IPv4 only. An app
  that listens on `[::]:port` gets IPv4 clients as `::ffff:a.b.c.d` on Linux, where `IPV6_V6ONLY` is off by default
  (Rust does not change it; `src/application.rs:644`), so `req.ip()` can be an IPv4-mapped string today.
- Gateways: the handshake `Request` is built with `Request::new` (`src/websockets.rs:725-729`), which uses the default
  parser options (`src/http/request.rs:49-50`) and so `TrustProxy::None`: gateway messages are keyed on the socket
  address even when `trust proxy` is set (out of scope, see proposal).
- Tests: `tests/trust_proxy.rs` starts an app with a `ThrottlerGuard` route (`/limited`, limit 2) on `127.0.0.1:0` and
  sends raw requests with `X-Forwarded-For` (lines 21-56); `trust_subnets_stop_at_the_first_untrusted_hop` (lines
  84-96) already checks that the throttler keys on the resolved client. `tests/input_hardening.rs:127-138` checks
  that `X-Forwarded-For` is ignored by default. All use IPv4 clients.
- Docs: `docs/security/rate-limiting.md:43-46` (key = name, `req.ip()`, controller, handler) and lines 120-132
  ("Behind a proxy"); same lines in `docs/es/security/rate-limiting.md`.

**Original (`@nestjs/throttler` 6.7.1, the latest release and the version pinned by the #120 / #135 fixtures; read
from the tarball fetched with `npm pack @nestjs/throttler@6.7.1`)**

- Default tracker: `ThrottlerGuard.getTracker(req)` returns `normalizeIp(req.ip, this.ipv6SubnetPrefix)`
  (`dist/throttler.guard.js:150-152`). The grouping happens **in the throttler**, not in Express.
- `this.ipv6SubnetPrefix` starts as `DEFAULT_IPV6_SUBNET_PREFIX` (`dist/throttler.guard.js:33`) and is replaced by the
  module option `ipv6SubnetPrefix` in `onModuleInit` only when the options are the object form
  `{ throttlers, ... }` (lines 52-64); with the array form of `forRoot([...])` it stays 64.
- `ipv6SubnetPrefix?: number` is a module-level option only (`dist/throttler-module-options.interface.d.ts:23`); it
  is not in the per-throttler `ThrottlerOptions` (lines 5-15) nor in `@Throttle` (`dist/throttler.decorator.d.ts:2-8`).
- A custom `getTracker` replaces the default: route/class `@Throttle({ name: { getTracker } })`, then the throttler's,
  then the module's, then the default (`dist/throttler.guard.js:65`, `:89`, `:94`); the result is passed to
  `generateKey` (line 124). A custom tracker does not go through `normalizeIp`, which is the original's way to opt out.
- `normalizeIp(ip, ipv6SubnetPrefix = 64)` (`dist/ip.js:76-110`), exported from the package index
  (`dist/index.js:18`):
  - non-string or empty input is returned as is (lines 77-79);
  - the zone is cut at the first `%` (lines 80-81) and `net.isIP` decides the family on the rest (line 82);
  - IPv4: the address without zone (lines 83-85);
  - not an IP: the original input, zone included (lines 86-88), and the same when `toHextets` fails (lines 89-92);
  - `toHextets` (lines 7-36) expands `::` and a dotted IPv4 tail into eight 16-bit groups;
  - loopback `::1` returns `'::1'` before any masking (lines 93-96);
  - IPv4-mapped `::ffff:0:0/96` returns the dotted IPv4 (lines 97-105); IPv4-compatible `::a.b.c.d` and NAT64
    `64:ff9b::/96` are not special;
  - the prefix is `Math.min(Math.max(Math.trunc(p), 0), 128)` when finite, else 64 (lines 106-108);
  - output `${formatIpv6(maskHextets(hextets, prefix))}/${prefix}` (line 109): `maskHextets` (lines 37-49) clears
    the bits after the prefix; `formatIpv6` (lines 50-75) writes lowercase hex groups without leading zeros and
    replaces the first longest run of at least two zero groups with `::`.
- History: 6.6.0's `getTracker` returned `req.ip` (`dist/throttler.guard.js:141-143` of that tarball) and had no
  `dist/ip.js`; 6.7.0 (2026-09-17) introduced `ip.js` identical to 6.7.1's. The README of 6.7.1 does not mention
  `ipv6SubnetPrefix`; its "Proxies" example overrides `getTracker` to return `req.ips[0]` or `req.ip`, which skips
  normalization.

**Express (5.2.1, from `corvis-tests/nest/consi-backend/node_modules`)**

- `req.ip` is `proxyaddr(this, trust)` (`express/lib/request.js:327-330`); `proxyaddr` returns the last address of
  `alladdrs` (`proxy-addr@2.0.7/index.js:222-235`), whose first entry is `req.socket.remoteAddress`
  (`forwarded@0.2.0/index.js:46-50`). No normalization: a Node server listening without a host binds `::` and reports
  IPv4 clients as `::ffff:a.b.c.d`, which is why the original maps them back to IPv4.

## Goals / Non-Goals

**Goals**

- The guard's client key matches the original's default tracker for every input, including the exact string.
- The prefix is configurable like `ipv6SubnetPrefix`, and there is a documented way to keep full IPv6 addresses.
- Tests for IPv4, IPv6, IPv4-mapped IPv6 and `X-Forwarded-For` with `trust proxy` on, plus a differential replay.

**Non-Goals**

- A tracker or key generator hook, per-throttler prefixes, changing `req.ip()`, gateway `trust proxy` support.

## Decisions

1. **Normalize in the guard, not in `req.ip()`.** Same place as the original (`getTracker`), so `#[ip]`, `req.ip()`
   and `req.ips()` keep Express's values and only rate limiting changes.
   *Alternative:* normalize in `Request::ip()`. Rejected: it changes a public value Express does not change and would
   affect logging, auth checks and the port apps' apidiff.

2. **Port `normalizeIp` exactly, output string included.** `pub fn normalize_ip(ip: &str, ipv6_subnet_prefix: u8) ->
   String`, following `dist/ip.js` step by step: cut at the first `%`; parse the rest with `Ipv4Addr` / `Ipv6Addr`
   from `std`; IPv4 → the cut string; IPv6 `::1` → `"::1"`; `Ipv6Addr::to_ipv4_mapped` → dotted IPv4; otherwise mask
   the `u128` and format. The formatter is a private function that mirrors `formatIpv6` (lowercase groups, first
   longest zero run of length ≥ 2 replaced by `::`), not `Ipv6Addr`'s `Display`, so the output does not depend on how
   `std` prints special forms. Since keys are hashed, only the equivalence classes matter for counting; keeping the
   exact string makes the differential test a plain string comparison and lets users reuse the helper.
   *Alternatives:* a key from the masked `u128` (same classes, cheaper, but not comparable with the original's
   output); returning `Cow<str>` (saves an allocation on the IPv4 path, but the key is formatted right after, and
   `String` is simpler to expose).
   `std`'s parsers and `net.isIP` must agree on what is an address (leading zeros in IPv4 octets, `::` covering a
   single group, embedded IPv4 in non-mapped addresses). The fixture corpus includes those cases; a disagreement is
   fixed in `normalize_ip` to follow `net.isIP`, and recorded here.

3. **Configuration on `ThrottlerStorage`.** `pub fn with_ipv6_subnet_prefix(self, prefix: u8) -> Self`, stored as
   `ipv6_subnet_prefix: u8` (default `DEFAULT_IPV6_SUBNET_PREFIX`), values above 128 clamped to 128 like the
   original's clamp. One value for every throttler, like the module-level `ipv6SubnetPrefix`. It works with
   `for_root_storage` and with `for_root_async` factories that return a storage.
   *Alternatives:* a field on `ThrottlerOptions` (per throttler; not in the original, and a new public field breaks the
   struct literals in users' code and the docs); an application setting (`app.set_...`; the throttler is the only
   user); rejecting values above 128 with a panic (the original clamps; `u8` already bounds the input and 128 is the
   only sensible reading).

4. **Opting out.** `with_ipv6_subnet_prefix(128)` keeps each IPv6 address apart (still mapping IPv4-mapped addresses
   to IPv4 and keeping `::1`). Corvis has no tracker hook, so this is the only opt-out inside `ThrottlerGuard`; code
   that needs another tracker can call `ThrottlerStorage::record_hit` from its own guard with `normalize_ip` or any
   string. The docs say so, and say that a custom tracker in the original skips normalization.
   *Alternative:* add a tracker hook now. Rejected as scope: the issue asks to match the default tracker; the hook is
   listed as a possible new issue.

5. **One tracker per request.** The prefix is global, so the guard computes `normalize_ip(&req.ip(), prefix)` once
   before the throttler loop and uses it in every key. Equivalent to the original, which calls the same default
   tracker for each throttler.

6. **Non-IP trackers stay as they are.** A forged `X-Forwarded-For` entry under `TrustProxy::All` or `Hops(n)` is
   kept unchanged, as in the original. Normalization cannot help when the client chooses its address: the docs already
   warn about `trust proxy` with `true`, and #135's caps (`with_max_entries`, `with_max_blocked`) bound the memory.

7. **Differential fixture and tests.**
   - `crates/corvis/tests/fixtures/throttler/normalize-ip.mjs` imports `normalizeIp` from the pinned
     `@nestjs/throttler` and writes `normalize-ip.json`: `[{ ip, prefix, out }]` for every corpus entry at prefixes 0, 1,
     15, 16, 17, 48, 56, 63, 64, 65, 96, 127 and 128. Corpus: `1.2.3.4`, `0.0.0.0`, `255.255.255.255`, `01.2.3.4`, `1.2.3.4%x`, `::1`, `0:0:0:0:0:0:0:1`,
     `::`, `::2`, `2001:db8::1`, `2001:DB8:0:0:1:0:0:1`, `2001:0db8:0001:0002:0003:0004:0005:0006`,
     `2001:db8:1:2:ffff:ffff:ffff:ffff`, `fe80::1%eth0`, `fe80::1%`, `::ffff:1.2.3.4`, `::FFFF:102:304`,
     `::ffff:0:0`, `::1.2.3.4`, `64:ff9b::1.2.3.4`, `1:2:3:4:5:6:1.2.3.4`, `1::2:3:4:5:6:7`, `1:2:3:4:5:6:7::`,
     `0:0:1::1`, `1:0:0:1::1`, `[::1]`, `1.2.3.4:80`, `::ffff:1.2.3.256`, `1:2:3:4:5:6:7:8:9`, `unknown` and the empty string.
     Prefixes above 128 are not representable in the original's call with a different result, so the Rust test
     checks `normalize_ip(ip, 129)` and `normalize_ip(ip, 255)` against the recorded output for 128.
   - Unit tests in `throttler.rs` build an `ExecutionContext` with a `Request` whose `remote_addr` is set (as the #135
     branch's `websocket_context_records_no_headers` builds one, `throttler.rs:586-606`), for IPv4, two IPv6 addresses
     in one /64, two /64s, `::ffff:1.2.3.4` against `1.2.3.4`, prefix 128 and prefix 48.
   - Integration tests in `tests/trust_proxy.rs` with `TrustProxy::parse("loopback")`, sending IPv6 and IPv4-mapped
     `X-Forwarded-For` entries from `127.0.0.1`, and a check that `#[ip]` still returns the full address.

## Risks / Trade-offs

- **Shared /64s.** Clients that share a /64 (some hosting providers give each VM a /128 out of a shared /64; some
  networks put many hosts on one /64) now share a limit, as IPv4 clients behind one NAT do. Mitigation: documented,
  and `with_ipv6_subnet_prefix(128)` restores per-address limits.
- **Larger allocations still rotate.** Many ISPs delegate a /56 or /48 to one customer, so a client can still rotate
  through 256 or 65,536 /64s. The default stays 64 for parity; the docs suggest 56 or 48 for endpoints such as login.
- **Behavior change for IPv6 deployments.** Limits are reached sooner by clients in one /64. Called out in the docs.
- **Parser differences between `std` and `net.isIP`.** Covered by the corpus (Decision 2); a gap outside the corpus
  would change only which odd strings are treated as addresses, and forged strings are a `trust proxy` configuration
  issue anyway.
- **Prefix 0.** Every IPv6 client shares one counter (`::/0`), as in the original. Accepted for parity (Open question
  1).

## Open questions

1. Prefix 0 (and very small prefixes) put every IPv6 client in one counter. The original accepts it silently. Keep it
   silent, or log a warning when the prefix is below, say, 32?
2. Keep the default at 64 for parity, or diverge to a smaller default (56) under "security over parity"? This design
   keeps 64 and documents smaller values.
3. Export `normalize_ip` and `DEFAULT_IPV6_SUBNET_PREFIX` at the crate root but not in the prelude (Decision 2). Is
   the root re-export wanted, or only `corvis::modules::throttler::normalize_ip`?
4. Gateway handshakes ignore `trust proxy` (`src/websockets.rs:725-729`). Open a separate issue?
5. A tracker hook (`getTracker`) and key generator (`generateKey`) at module level: open a separate issue, or fold
   them into #156?

## Review notes (2026-11-09)

- Q1: a prefix below /32 is accepted, as in the original, and a warning is logged once when it is configured.
- Q2: the default stays 64, like the original.
- Q3: `normalize_ip` is exported only from `corvis::modules::throttler`, not from the crate root.
- Q4: the gateways ignoring `trust proxy` is filed as #158 (phase 0.7).
- Q5: module-level `getTracker` and `generateKey` hooks belong to #156.
