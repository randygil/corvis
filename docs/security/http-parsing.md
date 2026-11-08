---
status: documented
---

# HTTP parsing and compatibility

Corvis does not have a global "compatible" or "strict" mode. Each behavior that matters for security has its own
treatment, with the safe choice as the default. This page lists what Corvis accepts and rejects on the wire and in
multipart bodies, how that compares with Node's `http` module, and what is planned to change.

The HTTP/1 wire format is parsed by hyper (and `httparse` for the request head), not by Corvis. The Node figures below
come from a raw-socket probe against `http.createServer` on Node 24.13 (`llhttp` 9.3.0).

## What Corvis does

| Input | Corvis | Node `http` | Status |
|---|---|---|---|
| Request line or headers ending in a bare LF | accepted | `400` | planned to be rejected |
| Header block ended by `CRLF LF` | accepted | `400` | planned to be rejected |
| Bare CR inside a header line | rejected | `400` | kept |
| Chunk size with leading zeros (`0002`) | accepted | accepted | kept |
| Chunk size that does not fit in 64 bits | rejected | `400` | kept |
| Whitespace after a chunk size (`2 \r\n`) | accepted | `400` | planned to be rejected |
| Bare LF in the chunked framing (`2\nhi\n0\n\n`) | rejected | `400` | kept |
| Chunk extension containing LF | rejected | `400` | kept |
| `Content-Length` and `Transfer-Encoding: chunked` together | processed as chunked, connection closed afterwards | `400` | planned to be rejected |
| `field_nesting_depth` exceeded in multipart | `400 Field name nesting too deep - <field>` | n/a | changed |
| Busboy configuration errors (`Malformed content type`, `Unsupported content type: ...`) | `500` | `500` | kept for parity |

"Kept" means the behavior is covered by a regression test. "Planned" means a follow-up change (#146, Foundations epic
#1) will reject it by default and add an opt-out named `insecure_http_parser`, after Node's `insecureHTTPParser`
option. The opt-out does not exist yet.

## Until the stricter parser lands

Corvis accepts a bare LF as a line ending in the request head. RFC 9112 allows a recipient to do that, but it is a
risk when a proxy in front of Corvis reads the same bytes differently. If Corvis sits behind a proxy or load balancer,
let the proxy normalize requests (most do, and reject bare LF themselves). No request smuggling was found with common
proxies.

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
- HTTP/2 without TLS (h2c) is on by default, and Node's `http` module does not speak it. Whether it stays on by
  default is tracked in issue #144; `ServerOptions::new().h2c(false)` turns it off today. See
  [Performance](/http/performance).
- The request head is limited to 32 KiB by default (Node: 16 KiB); `ServerOptions` has `max_header_size`.
