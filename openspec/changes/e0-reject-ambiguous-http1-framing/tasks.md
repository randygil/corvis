# Tasks

## 1. Decision

- [ ] 1.1 Maintainer reviews design.md (decision 1 and open questions 1-3) and records the outcome in a "Review notes"
  section at the end of design.md. If option 1 (upstream) is chosen instead of option 3, revise the proposal, the
  spec delta and tasks 2-4 with `/opsx:update` before continuing. If open question 2 is answered "yes", add the two
  inputs to the spec delta and to 2.3. Verify that proposal.md no longer says "pending maintainer review"

## 2. Fixtures and failing tests

- [ ] 2.1 Record the Node fixture: run `node probe.mjs --json --path /t/echo` from
  `openspec/changes/e0-reject-ambiguous-http1-framing/node-probe/` and save the output as
  `crates/corvis/tests/fixtures/http1_framing/node-24.13.0.json`; verify the file lists every case of
  `output-node-24.13.0.txt` with the same statuses
- [ ] 2.2 Add a test binary `crates/corvis/tests/http1_framing.rs` with an echo route on `/t/echo` (`GET` and `POST`,
  body read and returned), a route that counts its calls, and raw-socket helpers like those of
  `transport_security.rs` (`serve`, `read_head`, `closed_within`), plus a status collector that matches every
  `HTTP/1.x NNN` in what arrives until close or 1.5 s, as the probe does; verify it compiles and an `ok-get` smoke
  test passes (`cargo test -p corvis --test http1_framing`)
- [ ] 2.3 Add the differential test that replays the fixture and compares statuses with the rules of design.md,
  decision 7, including the listed expected differences with their reasons; verify it fails today on exactly the
  cases `bare-lf-head`, `bare-lf-request-line`, `crlf-lf-head-end`, `lf-crlf-head-end`, `chunk-size-space`,
  `chunk-size-tab`, `chunk-size-space-ext`, `trailer-bare-lf-end`, `cl-then-te` and `te-then-cl` (today the echo
  route answers `200` for the last two, Node `400`)
- [ ] 2.4 Add one regression test per scenario of the spec delta (requirements "Request head lines end with CRLF",
  "Chunk size line has no whitespace", "Content-Length with Transfer-Encoding is rejected", "Trailer lines end with
  CRLF", "A framing rejection ends the connection") and tests that pin what stays: empty line before the request line
  accepted, leading zeros accepted, bare CR, overflow, bare LF in chunk lines and LF in an extension rejected; verify
  the new-behavior tests fail today and the pinned ones pass
- [ ] 2.5 If Node 22 is available, run `node probe.mjs` on it and save `node-probe/output-node-22.x.txt`; verify the
  statuses match Node 24, or record the differences in design.md (if Node 22 is not installed, write that in the
  probe README, as #8 did)

## 3. Framing check

- [ ] 3.1 Create the sans-IO module (design.md, decisions 2 and 5): states for empty lines, head, `Content-Length`
  body, chunk size, extension, chunk data, chunk end, trailer, held bytes after an upgrade request, and the HTTP/2
  preface prefix; outcomes forward / hold / reject / end of stream after a head; verify with unit tests for every
  input of the spec delta, every input handed to hyper (bad `Transfer-Encoding`, differing `Content-Length`,
  `HTTP/1.0` with `Transfer-Encoding`, unknown version, oversized head) and every stream split at every offset
  (`cargo test -p corvis --lib`)
- [ ] 3.2 Add the property test of design.md, decision 7 (generated and mutated pipelined streams, random splits,
  core versus `hyper::server::conn::http1` over `tokio::io::duplex`); verify it passes with at least 10,000 cases
  locally (`PROPTEST_CASES=10000`) and that unmutated streams are always accepted entirely
- [ ] 3.3 Add a unit test per chunked decoder state and for the head position showing that hyper (the version in
  `Cargo.lock`) rejects `\r\0` with a parse error at a head (automatic `400`) and a body error mid-body; verify it
  passes
- [ ] 3.4 Wire the check into `TimedIo` (`crates/corvis/src/server.rs`): `Option` field built in `TimedIo::new`,
  `poll_read` as in design.md decision 5, held bytes served first, no `Ready` with zero bytes before the real end of
  input, `on_read` still run on socket reads; add the finished-requests counter to `ConnState` and the wake-up for
  held bytes; verify the tests of 2.3 and 2.4 now pass
- [ ] 3.5 Handle upgrades and h2c as in design.md decision 4; verify the existing WebSocket and h2c tests pass
  unchanged (`cargo test -p corvis --test websockets --test ws_security --test ws_enhancers --test transport_security
  --test http_adapter`) and add a test where an `Upgrade` request answered with a non-`101` is followed by a pipelined
  bare-LF request that gets rejected
- [ ] 3.6 Delay the rejection bytes at a head boundary until no request is in flight (design.md decision 3); verify
  the "Valid request followed by a rejected one" scenario gets its `200` with a route whose response body streams in
  several chunks
- [ ] 3.7 Add a debug assertion in `serve` that `ignore_invalid_headers` is not enabled together with the check, and
  a comment pointing to design.md decision 4; verify `cargo test -p corvis` passes

## 4. Opt-out

- [ ] 4.1 Add `ServerOptions::insecure_http_parser(bool)` (default `false`, rustdoc as in design.md decision 6) and
  build `TimedIo` without the check when it is `true`; verify opt-out tests: the four inputs behave as
  `node-probe/output-corvis-hyper-1.12.0.txt` records, and the already-rejected inputs stay rejected
- [ ] 4.2 Log one startup warning when the option is on, through the framework logger, with its own test binary in
  the style of `crates/corvis/tests/cors_startup_warning.rs`; verify the warning is logged once with the option and
  never without it

## 5. Documentation and measurements

- [ ] 5.1 Update `docs/security/http-parsing.md` as design.md decision 8 says (status column, new rows, the opt-out
  section replacing "Until the stricter parser lands", the probe's Node version); verify no row still says "planned"
  for the four inputs and the page names issue #146
- [ ] 5.2 Add `insecure_http_parser` to the `ServerOptions` section of `docs/http/performance.md`, with a link to
  `/security/http-parsing`; verify the link resolves
- [ ] 5.3 Translate both changes into `docs/es/security/http-parsing.md` and `docs/es/http/performance.md` with the
  same headings, tables and code blocks; verify `npm run docs:check` in `docs/` passes
- [ ] 5.4 Measure throughput with the check on and off (`insecure_http_parser(true)`) on `GET /` and a chunked
  `POST` with the existing benchmark setup in `bench/` (shared target dir, release build), and record both numbers in
  design.md under "Review notes"; verify the difference is written down and flagged to the maintainer if it is
  above 5% on `GET /`

## 6. Definition of done

- [ ] 6.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 6.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] 6.3 `cargo test --workspace` passes, including `crates/corvis/tests/http1_framing.rs`, the startup warning test
  and the property test with its default case count
- [ ] 6.4 Regression tests exist for every requirement of the `http/request-parsing` delta (2.3, 2.4, 3.5, 3.6, 4.1,
  4.2)
- [ ] 6.5 Docs updated in English and Spanish (`docs/` and `docs/es/`); `npm run docs:check` in `docs/` passes
- [ ] 6.6 Security review: run the `security-reviewer` agent on the diff (framing check, `TimedIo` changes, opt-out)
  with design.md and the spec delta as context, including an attempt to make the check and hyper disagree on a
  request boundary; fix or answer every finding in design.md "Review notes"
- [ ] 6.7 corvis-parity: framework runtime code changed (`server.rs`, the new module), so run the `corvis-parity`
  check; the three port apps (consi, facten, houseplan) keep 0 apidiff differences (not run by the implementer: the
  maintainer runs it)

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #146, tick it in epic #1 and move its card to Done (see the `corvis-task` skill).
