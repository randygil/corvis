# Spec Delta

## Purpose

How `ThrottlerGuard` identifies a client, compatible with `@nestjs/throttler` 6.7.1: the client part of each counter
key is the client IP normalized like the original's `normalizeIp`, which groups IPv6 clients by a configurable subnet
prefix (/64 by default). Requires the `security/rate-limiting` spec created by `e13-rate-limit-headers-in-the-throttler`
(#120).

## ADDED Requirements

### Requirement: The guard tracks clients by their normalized IP
`ThrottlerGuard` SHALL build the client part of every counter key, for HTTP requests and gateway messages alike, as
`normalize_ip(&req.ip(), prefix)`, where `prefix` is the storage's IPv6 subnet prefix. The rest of the key (throttler
name, controller and handler) SHALL stay as it is.

#### Scenario: Two addresses of one /64
- **WHEN** a route is limited to 1 request per minute and requests come from `2001:db8:1:2::a` and then from
  `2001:db8:1:2:ffff:ffff:ffff:ffff`
- **THEN** the second request is rejected with status 429

#### Scenario: Addresses of different /64 prefixes
- **WHEN** a route is limited to 1 request per minute and requests come from `2001:db8:1:2::a` and then from
  `2001:db8:1:3::a`
- **THEN** both requests are accepted

#### Scenario: IPv4-mapped address shares the IPv4 counter
- **WHEN** a route is limited to 1 request per minute and requests come from `1.2.3.4` and then from
  `::ffff:1.2.3.4`
- **THEN** the second request is rejected with status 429

#### Scenario: IPv4 clients are unchanged
- **WHEN** a route is limited to 1 request per minute and requests come from `1.2.3.4` and then from `1.2.3.5`
- **THEN** both requests are accepted

### Requirement: normalize_ip keeps IPv4 addresses and values that are not addresses
`normalize_ip` SHALL return the input unchanged when it is empty. It SHALL cut the input at its first `%` (zone
identifier) and, when the rest is a valid IPv4 address, return that rest. When the rest is not a valid IP address, it
SHALL return the input unchanged, zone included.

#### Scenario: Zone identifier
- **WHEN** `normalize_ip` receives `fe80::1%eth0` with prefix 64, and `1.2.3.4%x`
- **THEN** it returns `fe80::/64` and `1.2.3.4`

#### Scenario: Value that is not an IP address
- **WHEN** `normalize_ip` receives `unknown`, `1.2.3.4:80` or `[::1]`
- **THEN** it returns the input unchanged

### Requirement: normalize_ip reduces IPv6 addresses to their prefix
When the input without its zone is a valid IPv6 address, `normalize_ip` SHALL return `::1` for the loopback address
whatever the prefix, the dotted IPv4 form for an IPv4-mapped address (`::ffff:0:0/96`), and for any other address the
address with every bit after the prefix cleared, in lowercase compressed form (the first longest run of two or more
zero groups replaced by `::`), followed by `/` and the prefix.

#### Scenario: Default prefix
- **WHEN** `normalize_ip` receives `2001:db8:1:2:ffff:ffff:ffff:ffff` with prefix 64
- **THEN** it returns `2001:db8:1:2::/64`

#### Scenario: Loopback and mapped addresses
- **WHEN** `normalize_ip` receives `::1` with prefix 0, and `::ffff:1.2.3.4` with prefix 64
- **THEN** it returns `::1` and `1.2.3.4`

#### Scenario: Full prefix
- **WHEN** `normalize_ip` receives `2001:db8:1:2::a` with prefix 128
- **THEN** it returns `2001:db8:1:2::a/128`

### Requirement: The IPv6 subnet prefix is configurable
`ThrottlerStorage` SHALL use `DEFAULT_IPV6_SUBNET_PREFIX` (64) as the IPv6 subnet prefix unless
`ThrottlerStorage::with_ipv6_subnet_prefix(prefix)` sets another one. Values above 128 SHALL be treated as 128. The
prefix SHALL apply to every configured throttler. A prefix of 128 SHALL keep each IPv6 address in its own counter.

#### Scenario: Opting out of grouping
- **WHEN** the storage is built with `with_ipv6_subnet_prefix(128)`, a route is limited to 1 request per minute, and
  requests come from `2001:db8:1:2::a` and then from `2001:db8:1:2::b`
- **THEN** both requests are accepted

#### Scenario: Larger allocation
- **WHEN** the storage is built with `with_ipv6_subnet_prefix(48)`, a route is limited to 1 request per minute, and
  requests come from `2001:db8:1:2::a` and then from `2001:db8:1:3::a`
- **THEN** the second request is rejected with status 429

#### Scenario: Out-of-range prefix
- **WHEN** the storage is built with `with_ipv6_subnet_prefix(200)`
- **THEN** it behaves as with `with_ipv6_subnet_prefix(128)`

#### Scenario: Factory-built storage
- **WHEN** `ThrottlerModule::for_root_async` returns a `ThrottlerStorage` built with `with_ipv6_subnet_prefix(56)`
- **THEN** the guard groups IPv6 clients by /56

### Requirement: The tracker follows the trust proxy setting
The client IP that `ThrottlerGuard` normalizes SHALL be `req.ip()`: the socket address by default, and the client
entry of `X-Forwarded-For` resolved by the `trust proxy` setting when one is configured. Normalization SHALL NOT
change `req.ip()`, `req.ips()` or the value of `#[ip]` parameters.

#### Scenario: IPv6 clients behind a trusted proxy
- **WHEN** `trust proxy` trusts the loopback range, a route is limited to 1 request per minute, and two requests from
  `127.0.0.1` carry `X-Forwarded-For: 2001:db8:1:2::a` and `X-Forwarded-For: 2001:db8:1:2::b`
- **THEN** the second request is rejected with status 429

#### Scenario: IPv4-mapped entry behind a trusted proxy
- **WHEN** `trust proxy` trusts the loopback range, a route is limited to 1 request per minute, and two requests from
  `127.0.0.1` carry `X-Forwarded-For: 1.2.3.4` and `X-Forwarded-For: ::ffff:1.2.3.4`
- **THEN** the second request is rejected with status 429

#### Scenario: Forwarded header without trust proxy
- **WHEN** no `trust proxy` setting is configured and requests from `127.0.0.1` carry different IPv6
  `X-Forwarded-For` values
- **THEN** they all share the counter of `127.0.0.1`

#### Scenario: `#[ip]` keeps the full address
- **WHEN** `trust proxy` trusts the loopback range and a request from `127.0.0.1` carries
  `X-Forwarded-For: 2001:db8:1:2::a` to a route that returns its `#[ip]` parameter
- **THEN** the response contains `2001:db8:1:2::a`

### Requirement: normalize_ip is public
`normalize_ip(ip: &str, ipv6_subnet_prefix: u8) -> String` and `DEFAULT_IPV6_SUBNET_PREFIX` SHALL be public under
`corvis::modules::throttler` and re-exported at the crate root next to `ThrottlerStorage`, so that code calling
`ThrottlerStorage::record_hit` directly can build the same tracker as the guard.

#### Scenario: Custom guard
- **WHEN** application code calls `corvis::normalize_ip("2001:db8:1:2::a", corvis::DEFAULT_IPV6_SUBNET_PREFIX)`
- **THEN** it receives `2001:db8:1:2::/64`

### Requirement: IP normalization matches @nestjs/throttler
For every address and prefix recorded in the differential fixture, `normalize_ip` SHALL return exactly the string that
`normalizeIp` of `@nestjs/throttler` 6.7.1 returns; for prefixes above 128 it SHALL return the recorded output for
128. The fixture SHALL cover IPv4, IPv6 in several spellings, IPv4-mapped addresses, loopback, zone identifiers and
values that are not addresses, at prefixes from 0 to 128.

#### Scenario: Replay of the recorded normalizations
- **WHEN** the Rust test replays `normalize-ip.json`, produced by running the original `normalizeIp` over the corpus
- **THEN** every output string is equal
