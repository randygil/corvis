# Draft: reject ambiguous HTTP/1 framing by default (opt-out `insecure_http_parser`)

Parent epic: #1 (Foundations, security leftovers). Decided in #8 (change `e0-decide-compatible-vs-strict-mode`,
design.md decisions 2 and 3). Not created on GitHub yet: the maintainer opens it.

## Problem

Corvis parses HTTP/1 with hyper 1.12.0 and httparse 1.10.1. Three inputs are accepted that Node's `http` module (Node
24, llhttp 9.3.0) answers with `400`, and that a front proxy may read differently (request smuggling risk):

1. Request line or header lines ending in a bare LF, and a header block ended by `CRLF LF`.
2. Whitespace after a chunk size (`2 \r\n`, `2\t\r\n`, `2 ;a=b\r\n`).
3. `Content-Length` together with `Transfer-Encoding: chunked`: hyper processes the body as chunked and closes the
   connection after the response. RFC 9112 section 6.3 allows it, so this one is lower priority.

Also to confirm: a bare LF inside a trailer line is accepted as trailer content up to the next CRLF
(`hyper-1.12.0/src/proto/h1/decode.rs`, `read_trailer`); Node answers `400`.

Already rejected and out of this issue: bare CR in a header, chunk size over 64 bits, bare LF in the chunked framing,
chunk extension with LF. Leading zeros in chunk sizes stay accepted (Node accepts them).

## Proposal

- Reject the cases above with `400` and close the connection by default.
- Add `ServerOptions::insecure_http_parser(true)` as opt-out (named after Node's `insecureHTTPParser`), for legacy
  clients. Document it on `docs/security/http-parsing.md` (English and Spanish) and update the status column there.
- Parity: apidiff sends with `node:http`, which always writes CRLF and never mixes the two length headers, so the three
  ports are not affected.

## Options (design.md, decision 3)

1. Contribute an option to httparse (reject bare LF) and hyper (strict chunk-size line), then enable it from `serve`
   (`crates/corvis/src/server.rs`). Clean, depends on upstream review and releases.
2. Maintain a fork of httparse/hyper. Applications depending on the published `corvis` crate need the fork too, so a
   `[patch.crates-io]` in this workspace alone is not enough.
3. Rejected: scanning bytes in `TimedIo::on_read`. The IO layer cannot tell head bytes from body bytes without
   re-implementing the framing, so a pipelined second request would escape the check.

The issue decides between 1 and 2.

## Verification

- Differential test with the Node probe as oracle: `openspec/changes/e0-decide-compatible-vs-strict-mode/node-probe/probe.mjs`
  (raw sockets against `http.createServer`, `127.0.0.1` only) lists the expected status per input; the Corvis test sends
  the same bytes to a Corvis route and compares statuses.
- Regression tests per rejected input, and one with `insecure_http_parser(true)` showing the old behavior.
- Pipelined case: a rejected request must not let a second request on the same connection be processed.
- Docs: update `docs/security/http-parsing.md` and `docs/es/security/http-parsing.md`, write the issue number in them.
