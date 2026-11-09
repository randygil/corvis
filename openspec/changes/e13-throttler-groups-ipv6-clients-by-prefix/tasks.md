# Tasks

Prerequisite: #120 (`e13-rate-limit-headers-in-the-throttler`) and #135
(`e13-throttler-block-duration-like-the-original`) are implemented and archived, so
`openspec/specs/security/rate-limiting/spec.md` exists, the guard loop calls `record_hit`, and
`crates/corvis/tests/fixtures/throttler/` has a `package.json` pinning `@nestjs/throttler`. If they are not, stop and
implement them first.

## 1. Reference behavior and failing tests

- [ ] 1.1 Check the `@nestjs/throttler` version pinned by `crates/corvis/tests/fixtures/throttler/package.json`; if it
  is not 6.7.1, re-read its `dist/ip.js` and the `getTracker` / `ipv6SubnetPrefix` parts of `dist/throttler.guard.js`
  against design.md (Context, Original) and record any difference there; verify by quoting the version in design.md
- [ ] 1.2 Add `crates/corvis/tests/fixtures/throttler/normalize-ip.mjs`: import `normalizeIp` from
  `@nestjs/throttler`, run it over the corpus and prefixes of design.md, Decision 7, and write `normalize-ip.json`
  (`[{ ip, prefix, out }]`); run it once and commit the JSON (not `node_modules`); verify the JSON has
  `2001:db8:1:2::/64` for `2001:db8:1:2:ffff:ffff:ffff:ffff` at 64, `1.2.3.4` for `::ffff:1.2.3.4`, `::1` for `::1`
  at 0, and `unknown` unchanged
- [ ] 1.3 Add failing tests: the fixture replay and the guard tests of design.md, Decision 7 in
  `crates/corvis/src/modules/throttler.rs`, and the `X-Forwarded-For` tests in `crates/corvis/tests/trust_proxy.rs`
  (one per scenario of the spec delta that needs the HTTP stack); verify they fail to compile or fail for the
  expected reason with `cargo test -p corvis --lib throttler` and `cargo test -p corvis --test trust_proxy`

## 2. Normalization helper

- [ ] 2.1 Add `pub const DEFAULT_IPV6_SUBNET_PREFIX: u8 = 64` and `pub fn normalize_ip(ip: &str,
  ipv6_subnet_prefix: u8) -> String` to `crates/corvis/src/modules/throttler.rs` (design.md, Decision 2), with a
  private formatter that mirrors `formatIpv6`; rustdoc maps it to the original's `normalizeIp` and lists the rules of
  the spec delta; verify the fixture replay of 1.3 passes, including prefixes 129 and 255 against the output for 128
- [ ] 2.2 If the replay shows a disagreement between the `std` parsers and `net.isIP`, adjust `normalize_ip` to follow
  `net.isIP` and record the case in design.md (Decision 2); verify the replay passes with no skipped entries
- [ ] 2.3 Export `normalize_ip` and `DEFAULT_IPV6_SUBNET_PREFIX` in the `modules::throttler` re-export of
  `crates/corvis/src/lib.rs` (not the prelude); verify an integration test reaches `corvis::normalize_ip`

## 3. Storage setting and guard

- [ ] 3.1 Add the `ipv6_subnet_prefix: u8` field (default `DEFAULT_IPV6_SUBNET_PREFIX`) and
  `ThrottlerStorage::with_ipv6_subnet_prefix(prefix: u8)` clamping to 128, with rustdoc naming the original's
  `ipv6SubnetPrefix`, the opt-out with 128 and the shared-/64 trade-off; verify with a unit test that 200 behaves as
  128
- [ ] 3.2 In `ThrottlerGuard::can_activate`, compute `normalize_ip(&req.ip(), self.storage.ipv6_subnet_prefix)` once
  before the throttler loop and use it in place of `req.ip()` in the key (design.md, Decision 5); update the
  `ThrottlerGuard` rustdoc ("keyed by client IP + handler") to say the IP is normalized; verify the guard tests of
  1.3 pass (IPv4, one /64, two /64s, IPv4-mapped, prefix 128, prefix 48)
- [ ] 3.3 Verify the `trust_proxy.rs` tests of 1.3 pass, and that `crates/corvis/tests/trust_proxy.rs`,
  `crates/corvis/tests/input_hardening.rs`, `crates/corvis/tests/modules.rs` and #120 / #135's throttler tests and
  replays still pass unchanged
- [ ] 3.4 Add a test that a `for_root_async` factory returning `ThrottlerStorage::new(..).with_ipv6_subnet_prefix(56)`
  groups by /56, next to `throttler_factory_can_return_a_storage` in `crates/corvis/tests/modules.rs`; verify it
  passes

## 4. Docs

- [ ] 4.1 `docs/security/rate-limiting.md`: in "How counting works", say the key uses the client IP normalized like
  the original (IPv4 as is, IPv4-mapped as IPv4, IPv6 by /64 prefix, `::1` alone); add an "IPv6 clients" section with
  `with_ipv6_subnet_prefix`, the opt-out with 128, smaller prefixes (56, 48) for sensitive routes, the shared-/64
  trade-off, `normalize_ip` for custom guards that call `record_hit`, and that `req.ip()` / `#[ip]` keep the full
  address; in "Behind a proxy", say the normalized value is the resolved `X-Forwarded-For` client and that forged
  non-IP entries are kept as they are; same in `docs/es/security/rate-limiting.md`
- [ ] 4.2 Run `npm run docs:check` in `docs/`; verify it passes

## 5. Definition of done

- [ ] 5.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 5.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] 5.3 `cargo test --workspace` passes, including the regression tests of 1.3, 3.1 and 3.4 and the differential
  replay of `normalize-ip.json`
- [ ] 5.4 Docs updated in English and Spanish (`docs/` and `docs/es/`) and `npm run docs:check` passes in `docs/`
- [ ] 5.5 Framework runtime code changed: run the `corvis-parity` check and verify the three port apps keep 0 apidiff
  differences (none uses the throttler)

## Workflow follow-up

- Archive the change with `/opsx:archive` after review (after the changes of #120 and #135 are archived).
- Close issue #154, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).
