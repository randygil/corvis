---
status: documented
---

# HTTP parsing and compatibility

Corvis does not have a global "compatible" or "strict" mode. Each behavior that matters for security has its own
treatment, with the safe choice as the default. This page lists what Corvis accepts and rejects on the wire and in
multipart bodies, how that compares with Node's `http` module, and how to turn the stricter checks off.

The HTTP/1 wire format is parsed by hyper (and `httparse` for the request head). hyper and `httparse` have no option
to refuse the ambiguous inputs below, so Corvis puts a small framing check in front of hyper on every connection (see
[How the check works](#how-the-check-works)). The Node figures below come from a raw-socket probe against
`http.createServer` on Node 24.13 (`llhttp` 9.3.0).

## What Corvis does

| Input | Corvis | Node `http` | Status |
|---|---|---|---|
| Request line or headers ending in a bare LF | `400`, connection closed | `400` | rejected (`insecure_http_parser` accepts it) |
| Header block ended by `CRLF LF` (or the last header ended by `LF CRLF`) | `400`, connection closed | `400` | rejected (`insecure_http_parser` accepts it) |
| Empty line (CRLF or bare LF) before the request line | ignored | ignored | kept |
| Bare CR inside a header line | rejected | `400` | kept |
| Obsolete line folding in a header | rejected | `400` | kept |
| Chunk size with leading zeros (`0002`) | accepted | accepted | kept |
| Chunk size that does not fit in 64 bits | rejected | `400` | kept |
| Whitespace after a chunk size (`2 \r\n`, `2\t\r\n`, `2 ;a=b\r\n`) | rejected | `400` | rejected (`insecure_http_parser` accepts it) |
| Bare LF in the chunked framing (`2\nhi\n0\n\n`) | rejected | `400` | kept |
| Chunk extension containing LF | rejected | `400` | kept |
| Bare LF inside the trailer section of a chunked body (`0\r\nX-A: b\n\nGARBAGE\r\n\r\n`) | rejected | `400` | rejected (`insecure_http_parser` accepts it) |
| `Content-Length` and `Transfer-Encoding` together, in either order | `400`, connection closed | `400` | rejected (`insecure_http_parser` processes it as chunked and closes the connection afterwards) |
| Two identical `Content-Length` headers | accepted | `400` | accepted: one possible reading, so not ambiguous; a difference from Node |
| `Transfer-Encoding: chunked, chunked` | accepted | `400` | accepted: one possible reading, so not ambiguous; a difference from Node |
| `HTTP/1.0` request with `Transfer-Encoding` | `400` | accepted | kept (stricter than Node) |
| `field_nesting_depth` exceeded in multipart | `400 Field name nesting too deep - <field>` | n/a | changed |
| Busboy configuration errors (`Malformed content type`, `Unsupported content type: ...`) | `500` | `500` | kept for parity |

Every row is covered by a regression test. The rows that name `insecure_http_parser` changed in issue #146
(Foundations epic #1): before it, Corvis accepted those inputs. A rejected request gets a `400` and the connection is
closed, and nothing sent after it on that connection is processed, so a proxy that read the same bytes differently
cannot get a second request past Corvis.

Clients are not affected unless they hand-write requests: Node's `http`, browsers, curl, reqwest and proxies always
end lines with CRLF, never put whitespace after a chunk size and never send both length headers.

## How the check works

A small state machine reads each connection before hyper does and follows where every request starts and ends: the
head, then the body by `Content-Length` or by chunks (sizes, extensions, trailers), then the next head. It refuses the
four inputs above and passes everything else to hyper unchanged. Header syntax, limits and status codes stay with
hyper. Inputs it does not fully understand (a `Transfer-Encoding` that does not end in `chunked`, differing or invalid
`Content-Length` values, `HTTP/1.0` with `Transfer-Encoding`, an unknown HTTP version, a head over the size limit) are
handed to hyper as they are and nothing after that request is read, so hyper answers them as before. After a request
with an `Upgrade` header the bytes that follow are held until the request is answered, because after a `101` they are
not HTTP. A property test replays generated and mutated pipelined streams through the check and through hyper and fails
if they disagree on where a request ends.

## Turning the checks off

`ServerOptions::new().insecure_http_parser(true)` turns off the four checks and only those: bare LF line endings in
the head and in trailers, whitespace after a chunk size, and `Content-Length` together with `Transfer-Encoding`. It is
named after Node's `insecureHTTPParser`, and it is narrower: inputs that hyper itself rejects (a bare CR, a chunk size
over 64 bits, a bare LF in the chunk framing, an LF in a chunk extension, obsolete line folding, a bad
`Transfer-Encoding`) stay rejected, and h2c, timeouts and size limits are not touched. Corvis logs one warning when the
server starts listening with it on.

```rust
use corvis::ServerOptions;

app.set_server_options(ServerOptions::new().insecure_http_parser(true));
```

Use it only for legacy clients that cannot be fixed (devices that send bare LF, for example). With it on, a bare LF
is a line ending again, which is a risk when a proxy in front of Corvis reads the same bytes differently: let the
proxy normalize requests (most do, and reject bare LF themselves).

## Multipart limits

`limits.field_nesting_depth(n)` rejects a text field name that has more than `n` opening brackets. The request is
answered with a client error:

```json
{ "message": "Field name nesting too deep - a[b][c][d]", "error": "Bad Request", "statusCode": 400 }
```

Without that limit, a name nested more than 32 levels is stored as a literal key instead of being rejected. See
[File upload](/http/file-upload).

Malformed or unsupported multipart content types (`multipart/mixed`, for example) answer `500` with the generic
`Internal server error` body, like the original Node application. Nothing leaks, but a client can trigger a 5xx on
purpose; keep that in mind for error-rate alerts. This is revisited with the route posture work.

## Other defaults worth knowing

- Cookies: `CookieOptions::default()` sets no `HttpOnly`, `Secure` or `SameSite`, as `res.cookie` in Express does.
  Set them on each call.
- CORS: a regular expression origin is unanchored, like JavaScript's `regex.test`, and `reflect_origin()` with
  `credentials(true)` reflects any origin. See [CORS](/security/cors).
- HTTP/2 without TLS (h2c) is off by default, like Node's `http` module, which does not speak it.
  `ServerOptions::new().h2c(true)` turns it on for a proxy or platform configured to send it; the HTTP/2 limits then
  differ from the HTTP/1 ones. See [Performance](/http/performance).
- The request head is limited to 32 KiB by default (Node: 16 KiB); `ServerOptions` has `max_header_size`.
