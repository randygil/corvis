# Design

## Context

See proposal.md (Why) for the four inputs. Versions from `Cargo.lock`: hyper 1.12.0, httparse 1.10.1, hyper-util
0.1.21. `serve` (`crates/corvis/src/server.rs:669-735`) builds a hyper-util `auto::Builder` (HTTP/1 plus h2c by prior
knowledge) and hands it `TokioIo<TimedIo>`. `TimedIo` (`server.rs:451-667`) wraps the `TcpStream`; its `poll_read`
(`:612-627`) calls `on_read` (`:560-577`) on every read, which today only watches for the HTTP/2 preface when h2c is
off. Non-multipart bodies are read by `from_hyper` before routing (`server.rs:246-302`), so a body that fails to
decode is answered `400 request aborted` (`request_aborted_error`, `server.rs:195-197`) even on an unknown route.
Multipart bodies are read later by the file interceptor.

### Where each input is accepted today

Paths are relative to `~/.cargo/registry/src/index.crates.io-*/`. Corvis behavior is measured with
`node-probe/probe.mjs` against the `cats-app` example (`node-probe/output-corvis-hyper-1.12.0.txt`). Node's is in
`node-probe/output-node-24.13.0.txt`.

| Input | Accepted at | Node 24 | Corvis today |
|---|---|---|---|
| Bare LF ending the request line | `httparse-1.10.1/src/lib.rs:500` calls `newline!`, whose `b'\n'` arm (`src/macros.rs:62-64`) takes LF alone as a line end | `400` | accepted, next request served |
| Bare LF ending a header line | `httparse-1.10.1/src/lib.rs:1206-1209` (value line end, `b == b'\n'` at `:1208`) and `:1184-1185` (empty value) | `400` | accepted |
| Head ended by `CRLF LF` or `LF CRLF` | `httparse-1.10.1/src/lib.rs:1107-1116`: at the start of a line a lone `\n` (`:1113`) ends the head. hyper's partial-read fast path `is_complete_fast` (`hyper-1.12.0/src/proto/h1/role.rs:98-115`) also treats `\n\n` and `\n\r\n` as a head end (hyper 1.11.1, PR #4147) | `400` | accepted |
| Empty lines before the request line | `httparse-1.10.1/src/lib.rs:558-582` (`skip_empty_lines`, bare LF at `:567`) | accepted | accepted (kept) |
| Whitespace after a chunk size | `hyper-1.12.0/src/proto/h1/decode.rs:398` (`read_size`: `b'\t' \| b' '` goes to `SizeLws`) and `:410-425` (`read_size_lws` accepts more whitespace, then `;` or `\r`) | `400` | accepted, body decoded |
| `Content-Length` + `Transfer-Encoding: chunked` | `hyper-1.12.0/src/proto/h1/role.rs:275-294` (TE wins; an earlier `Content-Length` is removed from the headers at `:285-287`), `:295-299` (a later `Content-Length` is skipped), `:359-361` (`if is_te && is_cl { keep_alive = false; }`) | `400` | decoded as chunked, connection closed after the response. The handler never sees `Content-Length` |
| Bare LF inside the trailer section | `hyper-1.12.0/src/proto/h1/decode.rs:533-552` (`read_trailer` keeps any byte except `\r` in the trailer), `:554-586` (lines are counted only on CRLF), `:642-680` (`decode_trailers` parses the buffer with `httparse::parse_headers`, which splits on bare LF as above, into as many slots as CRLF lines) | `400` | `X-A: b\nX-C: d`: two fields for one slot, `TooManyHeaders`, so the body fails and Corvis answers `400`. `X-A: b\n\nGARBAGE`: httparse stops at `\n\n` (`lib.rs:1113`), returns one field, `GARBAGE` is dropped, the request is served and the pipelined request too |

Already rejected, kept: bare CR in a header (`macros.rs:59`, `lib.rs:1206-1207`), chunk size over 64 bits
(`decode.rs:264-272`), bare LF in the chunked framing (`decode.rs:400`, `:457-477`, `:508-519`), LF in a chunk
extension (`decode.rs:426-455`), obsolete line folding (httparse rejects a header line that starts with whitespace in
requests), `HTTP/1.0` with `Transfer-Encoding` (`role.rs:281-284`).

Measured but not in the issue: two identical `Content-Length` headers and `Transfer-Encoding: chunked, chunked` are
accepted by hyper (`role.rs:295-311`; `headers.rs:159-168` only looks at the last coding) and answered `400` by Node.
`HTTP/1.0` with `Transfer-Encoding` is the reverse: Node accepts it, hyper answers `400`.

### No upstream switch exists

- httparse 1.10.1 (latest release, 2025-03-03): `ParserConfig` (`src/lib.rs:209-218`) only has `allow_*` and
  `ignore_invalid_headers_*` switches, which relax parsing. The free function `parse_headers` that hyper uses for
  trailers takes no config at all.
- hyper 1.12.0 (latest release, 2026-10-06): the server `http1::Builder` (`src/server/conn/http1.rs:243-430`) passes
  only `allow_multiple_spaces_in_request_line_delimiters` (`:293`) and `ignore_invalid_headers` (`:306`) to
  httparse. The chunked decoder and the length-header logic have no options. hyper's changelog from 1.6 to 1.12 adds
  no strictness option, and 1.11.1 (PR #4147) made the fast path recognize `\n\r\n`: bare LF is supported on purpose.
- hyper-util 0.1.21 mirrors the hyper builder methods in `auto` (`src/server/conn/auto/mod.rs:777` for
  `ignore_invalid_headers`), so any new hyper option also needs a hyper-util release before `serve` can use it.

## Goals / Non-Goals

**Goals:**
- Reject the four inputs by default, without changing the status of any input hyper already rejects.
- Guarantee that after a rejection no later byte of the connection is parsed as a request.
- Work for every application that depends on the published `corvis` crate, with no fork and no `[patch]`.
- A one-line opt-out that restores today's behavior exactly.

**Non-Goals:**
- A general HTTP/1 parser. The new check only decides where each request ends. Header syntax, token rules, limits and
  status codes stay with hyper.
- HTTP/2.
- Rejecting the inputs that are not ambiguous (identical duplicate `Content-Length`, `chunked, chunked`; open
  question 2).

## Decisions

### 1. Where the check lives: a framing check in Corvis' connection reader (recommended)

Three options were compared. Options 1 and 2 are the ones #146 listed; option 3 is the one #8's design set aside, and
the reason for setting it aside is answered below.

**Option 1. Upstream options in httparse and hyper.** What would have to change:
- httparse: a new opt-in `ParserConfig` switch (default lenient, for semver), for example
  `allow_bare_lf_in_requests(false)`. It must reach the `newline!` arm (`macros.rs:62-64`, used by the request line
  at `lib.rs:500`), the header loop (`lib.rs:1113`, `:1185`, `:1208`, and `:1090` on the `ignore_invalid_headers`
  path), and a new config-taking variant of `parse_headers` for trailers. `skip_empty_lines` (`:567`) stays lenient.
- hyper: an `http1::Builder` method that sets it, then strict modes for `read_size`/`read_size_lws`
  (`decode.rs:398`, `:410-425`) and `read_trailer` (`decode.rs:533-552`; the line end must be strict in the state
  machine, a strict `decode_trailers` alone does not move the message boundary), and a `400` instead of
  `keep_alive = false` at `role.rs:359-361`. The decoder gets its settings from `Conn` (`conn.rs:310-330`), so a
  config has to be threaded into `Decoder::chunked`.
- hyper-util: mirror the new builder method in `auto`.
- Timeline: three repositories, three reviews, three releases. httparse has not released for 20 months. hyper
  released five times in 2026 (1.9.0 in March to 1.12.0 in October). hyper has accepted opt-in parser switches before
  (`ignore_invalid_headers` in 1.6.0, `allow_multiple_spaces_in_request_line_delimiters` in 1.7.0), but all of them
  relax parsing, and #4147 points the other way. A realistic estimate is two to six months, with no guarantee of
  acceptance, and the docs keep saying "planned" until then.

**Option 2. A maintained fork.** `[patch.crates-io]` only applies to the workspace that declares it, so applications
depending on the published `corvis` would still build stock hyper. The fork would have to be published under new
names (for example `corvis-httparse`, `corvis-hyper`, `corvis-hyper-util`, since hyper-util depends on hyper by name)
and used through `package =` renames. Costs: every hyper and hyper-util release (HTTP/1 and HTTP/2, security fixes
included) has to be merged and republished by Corvis; RustSec advisories for hyper do not match the renamed crates,
so users lose `cargo audit` coverage; applications that also use reqwest or another hyper user compile two hypers;
the public `UpgradedIo` (`crates/corvis/src/http_adapter.rs:51`, `TokioIo<hyper::upgrade::Upgraded>`) becomes the
fork's type. Quick to start, expensive forever.

**Option 3. A framing check in front of hyper (recommended).** A per-connection state machine that sees every byte
`TimedIo::poll_read` reads, before hyper does, and follows the request framing: head, then body by
`Content-Length` or chunked, then the next head. #8 set aside "scanning the bytes in `TimedIo::on_read`" because
the IO layer cannot tell head bytes from body bytes without re-implementing the framing, so a pipelined second
request would escape. That objection holds for a scanner. It does not hold for a check that does track the framing,
and the framing to track is small because the check only has to handle the strict grammar:
- it accepts only CRLF line endings, at most one kind of length header, decimal `Content-Length` values, and chunk
  lines of the form `HEXDIG+ [";" ext] CRLF`;
- anything it rejects for a reason in this change gets a `400` (decision 3);
- anything else it does not fully understand (a bad `Transfer-Encoding`, an invalid or differing `Content-Length`,
  `HTTP/1.0` with `Transfer-Encoding`, an unknown version, an oversized head) is handed to hyper unchanged and the
  stream ends right after that head. hyper answers it exactly as today, and since nothing follows, there is no later
  request whose start the two parsers could disagree on.

Because every accepted message has one possible reading, hyper finds the same boundaries. A property test checks
that on generated and mutated streams (decision 7). Compared with option 1, it ships now and does not depend on
anyone else's release. Compared with option 2, nothing has to be merged from upstream. It also covers what an
httparse switch alone would not cover: the trailer boundary and `Content-Length` + `Transfer-Encoding`. Its costs are
a second, small framing parser that must stay stricter than hyper, one scan of heads and chunk lines, and the care
needed around upgrades (decision 4).

Other approaches discarded: a check in `handle` cannot see line endings or chunk lines, and hyper removes
`Content-Length` when `Transfer-Encoding` is present (`role.rs:285-287`, `:297-299`), so not even the length-header
case is visible there. Replacing hyper's HTTP/1 server is out of proportion.

**Recommendation: option 3.** Fallback, if the maintainer rejects it or the property test finds a disagreement that
cannot be fixed: option 1, keeping the docs status "planned" until hyper, hyper-util and httparse release. Option 2
is not recommended. Once option 3 is in place, proposing option 1 upstream would let Corvis delete the check later
(open question 3).

### 2. What the check accepts

A private module (`crates/corvis/src/server/h1_framing.rs` or `crates/corvis/src/h1_framing.rs`, whichever fits the
module layout) with a sans-IO core: `feed(&mut self, input: &[u8])` returns how many bytes may go to hyper and
whether the rest must be held, rejected or followed by end of stream. The core never allocates per byte and uses
`memchr` (already a dependency) to find line ends.

- **Before a head:** CRLF or bare LF empty lines are skipped (as httparse and Node do). A CR not followed by LF is a
  rejection.
- **Head:** buffered until the empty line that ends it, bounded by the limit hyper uses (`max_buf_size`, i.e.
  `options.max_header_size.max(8 * 1024)`, `server.rs:682`). Every LF must follow a CR and every CR must precede an LF,
  otherwise reject. The request line must end in ` HTTP/1.1` or ` HTTP/1.0`. Header names are compared
  case-insensitively and values trimmed of SP and HTAB, as httparse does. The check only reads `Content-Length`,
  `Transfer-Encoding`, `Upgrade` and the method (`CONNECT`). Other header syntax is hyper's job.
- **Body length,** following hyper's rules (`role.rs:275-361`) for the cases that have one reading:
  - neither header: no body;
  - both headers: reject (new);
  - `Content-Length` only, every value `1*DIGIT`, all equal, at most `u64::MAX - 2` (hyper's `MAX_LEN`): that many
    bytes, forwarded without scanning;
  - `Transfer-Encoding` only, `HTTP/1.1`, and the last coding of the last `Transfer-Encoding` line is `chunked`
    (hyper's `is_chunked_`): chunked;
  - anything else: hand the head to hyper and end the stream.
- **Chunked body:** chunk size `1*HEXDIG` (leading zeros allowed, overflow over 64 bits handed to hyper), then
  immediately `;` or CRLF. Whitespace there: reject (new). Extension bytes up to CRLF, LF inside: reject (hyper also
  rejects). Chunk data is forwarded by count, then CRLF. After the last chunk, trailer lines up to an empty line, each
  ending in CRLF with no bare LF or CR: otherwise reject (new). Then the next head.
- A connection's first bytes that are a prefix of the HTTP/2 preface are held until they match the whole preface or
  stop matching (decision 4).

The check accepts the same bytes whatever the read boundaries are. A test feeds every stream split at every offset.

### 3. How a rejection reaches the client

The check forwards the bytes before the offending message (or, mid-body, the valid bytes of the current body), then
gives hyper two bytes, `\r\0`, and then end of stream:
- At the start of a head, httparse sees `\r` not followed by `\n` and returns `Error::NewLine`. hyper maps it to
  `Parse::Header` (`hyper-1.12.0/src/error.rs:653-659`) and answers `400 Bad Request` itself
  (`role.rs:493-511`, `conn.rs:836-850`), then closes.
- Inside a chunked body, every decoder state that is not chunk data rejects `\r` followed by `\0` (`read_size_lf`,
  `read_body_lf`, `read_trailer_lf`, `read_end_lf` all require `\n` after `\r`). The body fails, hyper stops reading
  the connection (`conn.rs`, `poll_read_body` sets `Reading::Closed`), Corvis answers `400 request aborted`, and the
  connection closes after the response. A unit test per decoder state checks that hyper rejects the two bytes.
- At a head boundary after a pipelined request whose response is still being written, the two bytes are held until
  that request is no longer in flight (`ConnState::inflight`, woken through `ConnState::waker`, `server.rs:374-412`).
  hyper only writes its own `400` when it is not writing another response (`conn.rs:836-840`); otherwise it would
  close without the `400`.
- A rejection is an early response like the others, so the closing drain of `http/connection-close` applies
  unchanged: `linger_close` reads the rest of the socket directly, not through the check.

An I/O error was considered instead of the two bytes. hyper treats an I/O error at a head boundary as a closed
connection and sends nothing (`conn.rs:339-363`), so the client would not get the `400`.

### 4. Upgrades, h2c and edge cases

- **Upgrade and `CONNECT`.** After a request with an `Upgrade` header or the `CONNECT` method (and its body, if any),
  the bytes that follow may not be HTTP. The check holds them, without parsing or forwarding them, until every request
  it has forwarded has finished. If the response was `101`, the service already sets `ConnState::upgraded`
  (`server.rs:715-717`); the check then forwards the held bytes and everything after them unchanged. If it was not a
  `101`, hyper goes on parsing HTTP (it only upgrades on a `101`), so the check resumes and validates the held bytes
  as the next head. Turning the check off for good after any `Upgrade` header would let a client skip it by adding
  `Upgrade: x` to its first request. A counter of finished requests in `ConnState` (incremented with the in-flight
  counter) tells the check when the forwarded requests are done.
- **h2c.** With h2c on, a connection whose first 24 bytes are the HTTP/2 preface is passed through unchanged
  (HTTP/2 frames, not HTTP/1). With h2c off, the existing refusal in `on_read` stays as it is.
- **Oversized head.** When a head reaches hyper's buffer limit without ending, the check forwards what it has and ends
  the stream. hyper answers `431` as today.
- **End of input.** At end of input the check forwards whatever it holds. hyper sees the same partial bytes as
  today.
- **Expect: 100-continue.** The head is forwarded once complete. The client waits for hyper's `100` before sending
  the body, as today.
- If `ignore_invalid_headers` is ever turned on in `serve`, httparse would skip header lines that the check reads.
  The module documents that this combination is not allowed, and a debug assertion in `serve` enforces it.

### 5. Integration in `TimedIo`

`TimedIo` gets an `Option<FramingCheck>`: `Some` by default, `None` with `insecure_http_parser(true)`. With `None`,
`poll_read` is exactly today's code. With `Some`, `poll_read` reads from the socket into hyper's buffer, runs the
check over the new bytes, keeps the bytes that may be forwarded, and moves the rest (an incomplete head, held bytes)
into a small buffer of the check, shrinking `filled` (`ReadBuf::set_filled`). The next call serves that buffer first.
If a read produced only held bytes, `poll_read` reads again until it has something to return or the socket returns
`Pending`, so it never returns `Ready` with zero bytes unless it is the real end of input. `on_read` (timeouts, h2c
refusal) keeps running on what the socket returns, before the check.

Content-Length bodies and chunk data are forwarded in place without a copy. Only an incomplete head and held bytes are
copied.

### 6. `ServerOptions::insecure_http_parser`

```rust
/// Accept ambiguous HTTP/1 framing that Corvis rejects by default: bare LF line endings in the request head and in
/// chunked trailers, whitespace after a chunk size, and `Content-Length` together with `Transfer-Encoding`. Named
/// after Node's `insecureHTTPParser`. Default `false`. Only for legacy clients that cannot be fixed; prefer a proxy
/// that normalizes requests.
pub fn insecure_http_parser(mut self, enabled: bool) -> Self
```

- Scope: only the four checks of this change. It does not re-enable what hyper already rejects (bare CR, chunk size
  overflow, bare LF in chunk lines, LF in a chunk extension, obsolete line folding, bad `Transfer-Encoding`), and it
  does not touch h2c, timeouts or size limits. That is narrower than Node's flag, which also accepts invalid header
  characters. The docs say so.
- With it on, `listen` logs one warning through the framework logger, as Node prints a warning for
  `--insecure-http-parser`. Same mechanism and test style as the CORS startup warning
  (`crates/corvis/tests/cors_startup_warning.rs`).
- Name and default as decided in #8 (Review notes, item 4).

### 7. Tests

- **Differential against Node.** `node probe.mjs --json` writes the fixture
  `crates/corvis/tests/fixtures/http1_framing/node-24.13.0.json` (cases, raw bytes, Node statuses). A Rust test
  serves a route that reads and echoes the body for `GET` and `POST` on the probe's path, sends each case on a raw
  socket, collects statuses the same way the probe does, and compares: Node `400` matches Corvis `400` or a close
  without a response, Node `200` matches `200`, and the number of responses must match. Expected differences are
  listed in the test with their reason: `pipelined-valid-then-bare-lf` (Node does not answer the valid first
  request, see `node-probe/README.md`), `http10-te` (hyper is stricter), `dup-cl-identical` and `te-chunked-twice`
  (open question 2). Node is not needed to run the test, only to regenerate the fixture.
- **Regression per input:** one test per scenario of the spec delta, in the style of the chunked tests of #8 in
  `crates/corvis/tests/transport_security.rs` (`serve`, `read_head`, `closed_within`).
- **Pipelining:** a route that counts its calls. A rejected head, a rejected chunk line and a rejected trailer are
  each followed in the same write by a valid request, and the counter must not move. A valid request followed by a
  rejected one gets its `200`.
- **Opt-out:** the same inputs with `insecure_http_parser(true)` behave as `node-probe/output-corvis-hyper-1.12.0.txt`
  records (bare LF accepted and the next request served, chunk whitespace decoded, both length headers decoded as
  chunked with the connection closed, the `\n\n` trailer accepted). The already-rejected inputs stay rejected.
- **Property test against hyper** (`proptest`, already a dev-dependency): generate one to three pipelined requests
  (`GET`/`POST`, `Content-Length` or chunked bodies with random chunk sizes, extensions and trailers, optional empty
  lines before the request line), then mutate them (insert, delete or replace CR, LF, SP or HTAB; duplicate or
  reorder length headers), and split the stream at random read boundaries. Run the core on the stream, then run hyper
  (`hyper::server::conn::http1` over `tokio::io::duplex`, with a service that records method, path, body and
  trailers) on the bytes the core forwards. hyper must record exactly the requests the core accepted, with the same
  bodies, and never one more. Unmutated streams must be accepted entirely.
- **Existing suites** must keep passing unchanged: WebSocket upgrades (`tests/websockets.rs`, `tests/ws_*.rs`), h2c,
  `http/connection-close`, and `transport_security.rs`. One added test sends an `Upgrade` request that gets a
  non-`101` answer, followed by a pipelined bare-LF request, which must be rejected.

### 8. Documentation

`docs/security/http-parsing.md` and `docs/es/security/http-parsing.md`: the status column of the four rows becomes
"rejected (`insecure_http_parser` accepts it)"; new rows for the trailer section, empty lines before the request
line, obsolete line folding, identical duplicate `Content-Length`, `chunked, chunked` and `HTTP/1.0` with
`Transfer-Encoding`; the section "Until the stricter parser lands" becomes a section about the opt-out (what it turns
off, what it does not, the warning, when to use it). `docs/http/performance.md` and its translation list the option
with the other `ServerOptions`.

## Risks / Trade-offs

- [The check rejects a valid request that hyper would accept (false positive)] → it only rejects for the reasons in
  the spec, and hands everything else to hyper. The property test asserts that unmutated streams pass. apidiff
  exercises the path on every request of the three ports. The opt-out exists for clients it breaks.
- [The check and hyper disagree on where a request ends (a new desync inside Corvis)] → accepted messages have one
  reading by construction, unclear heads end the stream, and the property test compares boundaries with hyper on
  mutated input. A hyper upgrade that changes framing rules would show up there.
- [A hyper release changes its parse-error handling, so `\r\0` no longer produces a `400`] → the per-state unit
  tests run against the hyper in `Cargo.lock` and fail on such an upgrade. The client would still get a closed
  connection, which the spec allows mid-body but not at a head.
- [Throughput] → one extra pass over heads and chunk lines, no copy of `Content-Length` bodies or chunk data.
  Measured by task 5.4 against the published benchmark (`bench/results.md`: 119k req/s on `GET /`).
- [Upgrade handling holds bytes too long, or forever] → bytes are held only after an `Upgrade` or `CONNECT` request,
  only until the forwarded requests finish, and the headers timeout still runs. Clients that send WebSocket frames
  before the `101` (not allowed by RFC 6455) are delayed, not broken.
- [Parity: apidiff cannot see the change] → `node:http` always sends CRLF and one length header, so apidiff only
  shows false positives, not the new rejections. The new behavior is covered by the differential test against Node.
- [Legacy clients that send bare LF break] → behavior change in the release notes and the docs, and the opt-out.
- [Node 22 was never probed (consi and facten originals run on it)] → task 2.5 runs the probe on Node 22 when it is
  available. Node 22 also ships llhttp 9, so the same statuses are likely, but they have not been measured.

## Migration Plan

- Default behavior changes for the four inputs. The crates are not published yet (`e0-crate-packaging`), so the
  release notes of the first published version include it.
- Rollback for an application: `ServerOptions::new().insecure_http_parser(true)`. Rollback for Corvis: revert the
  change; nothing is stored or migrated.

## Open Questions

Answered by the maintainer on 2026-11-08, see "Review notes" below.

1. **Approve option 3 (framing check in `TimedIo`) over options 1 and 2?** Recommendation: yes. It goes back on #8's
   reason for setting it aside, for the reason given in decision 1. If not, the fallback is option 1 and
   tasks 2 to 4 are rewritten with `/opsx:update`.
2. **Also reject two identical `Content-Length` headers and `Transfer-Encoding: chunked, chunked`, as Node does?**
   Recommendation: not in this change. They have one reading for hyper and for any proxy that reads them, so they
   are not smuggling vectors. The docs list them. Adding them later is a few lines in the check, under the same
   opt-out.
3. **Propose the strict options upstream (option 1) once this lands, so the check can be deleted later?**
   Recommendation: yes, as a separate issue in epic #1, opened by the maintainer, with the property test as evidence.
   No upstream issue or PR is opened by this change.

## Review notes (2026-11-08)

1. Option 3 is approved: the framing check in `TimedIo`, on by default, turned off by
   `ServerOptions::insecure_http_parser(true)`. #8 set aside a plain byte scan; this check follows the message framing,
   which answers that objection.
2. Two identical `Content-Length` headers and `Transfer-Encoding: chunked, chunked` are not rejected in this change.
   The docs list them as a difference from Node.
3. Proposing the strict options upstream is tracked as a separate issue in epic #1. This change opens nothing
   upstream.

## Implementation notes (2026-11-08)

Decisions taken while implementing, where the design left room, and the places where the code differs from it.

1. **Module and API.** The check is `crates/corvis/src/server/h1_framing.rs` (tests in `h1_framing/tests.rs`), declared
   from `server.rs`. Core API: `passthrough_prefix` (bytes forwarded in place), `feed` (everything else, appended to an
   output buffer), `eof`, `release` (upgraded), `resume` (upgrade answered without `101`), and a pending terminal
   (`Reject` or `End`, with the number of finished requests to wait for). `TimedIo` owns the check, an output queue and
   the socket's end-of-input flag; `ConnState` gets a `finished` counter, bumped when a request's response body is dropped.
2. **Copies.** Design decision 5 says `Content-Length` bodies and chunk data are forwarded in place. That holds for the
   bytes that follow a read boundary inside a body. Bytes that share a read with a head (a small POST arrives in one
   read) go through the output queue, so they are copied once. A head is always copied (buffered until its empty
   line). Measured cost: see item 9.
3. **Waiting before the end of the connection.** A rejection, and the end of the stream after a head handed to hyper,
   are delayed until `finished >= wait`, where `wait` is the number of requests forwarded before the offending one
   (`heads` at a head boundary, `heads - 1` in the middle of a body, because the request whose body is being read
   cannot finish). I compared with the design's "no request in flight": the finished counter also covers the window
   in which hyper has the bytes of a request but has not called the service yet. The wait registers the connection
   task in `ConnState::waker`, checks again after registering (a request may finish in between), and falls back to
   `poll_idle`, so the header and keep-alive timeouts still apply. Without this wait hyper reads end of input while it
   is still answering and fails with `IncompleteMessage`; the property test hit that with a bare duplex stream.
4. **`Upgrade` hold.** Any `Upgrade` header (any value, either HTTP version) and the `CONNECT` method hold the bytes
   that follow, a superset of what hyper upgrades (hyper needs HTTP/1.1 and a `101`, or a `2xx` to `CONNECT`).
   The held bytes are capped at 64 KiB: past it the check stops reading the socket until the request is answered.
   `ConnState::upgraded` (set when a response is `101`) releases the held bytes unchanged; `finished` reaching the
   forwarded count resumes validation. No Corvis route answers a `CONNECT` with a tunnel.
5. **What the check hands to hyper (stream ends after the head).** A request line that does not end in ` HTTP/1.1` or
   ` HTTP/1.0`; `Transfer-Encoding` that is not chunked at the end (last line, last coding), or on `HTTP/1.0`; `Content-Length`
   values that are not a plain `1*DIGIT` number (corrected in the security review follow-up: a list such as `5, 5` is
   NOT accepted, hyper's server reads the value with `from_digits` and answers `400`); a number above `u64::MAX - 2`;
   a head that reaches `max_header_size` without ending. Two identical `Content-Length` header lines are one number,
   and hyper accepts them too (`role.rs`, `prev != len`). Both length headers together are rejected even when one
   of them is invalid.
6. **Chunked framing.** An empty chunk size (`\r\n` or `;a` first) is rejected by the check; hyper reads it as size 0.
   The grammar asks for `1*HEXDIG`, and Node rejects it too. A chunk size that overflows 64 bits is rejected by the
   check (hyper rejects it as well). Bytes of a chunked line that were already valid are forwarded before the
   rejection bytes, so hyper fails the body at the same position.
7. **Startup warning.** It is logged in `bind` (so for `listen` and `listen_in_background`), not in `init`, because
   `set_server_options` may be called after `init()`. Logger context `ServerOptions`, one line per listen.
8. **`ignore_invalid_headers`.** `serve` now sets it explicitly from a constant that is `false`, and a `debug_assert!`
   states that it must not be on while the check is. The constant documents the rule where the builder is configured.
9. **Throughput (task 5.4).** Release build of `bench/corvis-app` (temporary `INSECURE` switch, not committed),
   `oha` 100 connections for `GET /`, 10 s per run after a 3 s warm-up, three runs per mode in alternating order, same
   machine as `bench/results.md`, client and server competing for CPU:

   | Mode | `GET /` req/s (3 runs) | average | chunked `POST /users` req/s (3 runs) | average |
   |---|---|---:|---|---:|
   | check on | 98,406 / 89,500 / 96,831 | 94,912 | 55,922 / 52,045 / 41,534 | 49,833 |
   | check off (`insecure_http_parser(true)`) | 105,336 / 100,648 / 85,968 | 97,317 | 50,524 / 53,338 / 46,130 | 49,997 |

   Difference: -2.5% on `GET /` and -0.3% on the chunked `POST`, both inside the run-to-run noise of this setup (single
   runs vary by about 10%). Below the 5% threshold, so nothing to flag. The chunked load came from a small Node raw-socket
   generator (64 connections, `oha` cannot send chunked bodies), the same for both modes; absolute numbers are
   limited by it.
10. **Tests that differ from the design.** The "response body streams in several chunks" scenario uses a route that
    streams five chunks over about 400 ms. The property test uses hyper's `half_close(true)` so that the test's own end
    of input does not cut the answers; it compares with hyper only up to the first HTTP/1.0 request, which closes the
    connection (hyper then reads less than the check, the safe direction). It asserts that hyper never reads a request
    the check did not complete, and that both read the same requests when hyper finishes without an error. I checked
    that it catches a deliberately broken trailer rule (an empty line accepted as a bare LF) at 10,000 cases.
11. **Fixture.** `fixtures/http1_framing/node-24.13.0.json` has the 27 cases of the probe. Regenerating it from Git Bash
    needs `MSYS_NO_PATHCONV=1` (see `node-probe/README.md`). Node 22 is not installed, so task 2.5 is recorded as not
    measured.
12. **Not done here.** `npm run docs:check` (no `docs/node_modules` in the worktree) and the `corvis-parity` check
    (6.7). The security review (6.6) was run afterwards, see the follow-up below.

## Security review follow-up (2026-11-08)

The security review found nothing exploitable. Findings and what was done:

1. **A rejection after an empty line closed without `400` (low, spec violation).** `Start` forwarded empty lines to
   hyper one read at a time; hyper kept a lone `\r\n` buffered as the possible start of a head, and the rejection bytes
   `\r\0` that followed were then waited on until end of input (`IncompleteMessage`, no response). Fixed by dropping the
   empty lines in `Start` instead of forwarding them (the other option, `\r\0\r\n\r\n`, would have kept sending bytes that
   hyper has to interpret). hyper now never sees an empty line between messages, so nothing sits in front of the
   rejection bytes; a lone CR at the end of the input is dropped too. Design decision 2 ("empty lines are skipped") is
   now literal. Tests: unit tests for every kind of rejected head after each kind of empty line, a unit test that runs
   those streams through hyper and expects `400`, and raw-socket tests (one write, two writes, and a keep-alive request
   followed by the empty line and the rejected head).
2. **`Content-Length: 3, 3` (info).** The check accepted a list; hyper's server (`headers::content_length_parse`,
   `from_digits`) does not and answers `400`. The check now requires `1*DIGIT` (anything else goes to hyper, stream ends
   after the head). Note 5 above is corrected. Two identical header lines are accepted by hyper (checked in `role.rs`
   and with a raw-socket test), so the docs row "Two identical `Content-Length` headers: accepted" stays true; the docs
   never mentioned lists.
3. **Read into an empty buffer (info).** `poll_read_checked` returns `Ready(Ok(()))` without reading when
   `buf.remaining() == 0`, so it cannot set `sock_eof` by mistake.
4. **Order of loads (info).** `finished` is read before `upgraded` (a 101 sets `upgraded` before its request finishes), so
   a request seen as finished cannot make the check call `resume` instead of `release`.
5. **Test gaps.**
   - The check records, in test builds only, each forwarded head, each finished message with its decoded body and its
     trailer bytes. The mutated-stream property test compares, per request, method, path and body with what hyper
     serves (not only the count), and no longer skips hyper's error responses: an error status or a failed body must be
     explained by the rejection, a handed-over head, a message still open or a head still buffered at the end of the
     stream, or by header, URI or trailer syntax that fails an independent grammar check in the test. Anything else is
     reported as a disagreement. With a deliberately re-broken `content_length` (lists accepted) the test fails.
   - New generated shapes: `Upgrade` (held, then resumed as if answered without `101`), `CONNECT`, HTTP/1.0 with
     keep-alive, `Expect: 100-continue`, the largest `Content-Length`, empty lines before every request, and spliced
     headers such as duplicate or list `Content-Length`, `Content-Length: +2`, `Content-Length: 2 ` and
     `Connection: close`.
   - Unmutated streams (with those shapes) must be accepted entirely, forwarded without their empty lines, and read by
     hyper as exactly the generated requests with no error status.
   - One integration test drives the real `TimedIo` and `auto::Builder` with a pipelined valid stream and a pipelined
     rejected stream, each cut at every byte (two writes with a pause between them).
