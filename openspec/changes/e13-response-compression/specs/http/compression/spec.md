# Spec Delta

## Purpose

Response compression compatible with the `compression` middleware (1.8.2) of the original stack: when a response is
compressed, with which encoding, which headers change, how streamed bodies are handled, and where Corvis deliberately
differs. Compression is opt-in at build time (a cargo feature) and at run time (`app.enable_compression`).

## ADDED Requirements

### Requirement: Compression is opt-in
Response compression SHALL be available only with the `compression` cargo feature, which is not a default feature,
and SHALL apply only after `app.enable_compression(CompressionOptions)` has been called. Without that call, responses
MUST be byte-for-byte the same as without the feature.

#### Scenario: Feature on, not enabled
- **WHEN** the application is built with the `compression` feature, does not call `enable_compression`, and a client
  sends `Accept-Encoding: gzip` for a 10 KiB JSON response
- **THEN** the response has no `Content-Encoding` and no `Vary: Accept-Encoding` added

### Requirement: Compression runs on the final response
When enabled, compression SHALL run after the Express response defaults and the conversion to the wire response, in
the server and in the test client, so that it sees the final status, `Content-Type`, `Content-Length`, `ETag`,
`Cache-Control`, `Vary` and body, including headers set by middleware, interceptors, exception filters, `#[header]`
and default headers such as a `StreamableFile`'s `Content-Length`.

#### Scenario: Test client and server agree
- **WHEN** the same request with `Accept-Encoding: gzip` is sent through the test client and through a listening server
- **THEN** both responses have the same status, `Content-Encoding`, `Vary` and decoded body

### Requirement: Filtered and no-transform responses are left untouched
A response SHALL be left untouched (no `Vary` added, no encoding) when the filter rejects it or when its
`Cache-Control` has the `no-transform` directive (case-insensitive, as a comma-separated token).

#### Scenario: Image
- **WHEN** a 10 KiB `image/png` response is requested with `Accept-Encoding: gzip`
- **THEN** it is sent uncompressed, without `Vary: Accept-Encoding` added

#### Scenario: No-transform
- **WHEN** a 10 KiB JSON response carries `Cache-Control: private, no-transform`
- **THEN** it is sent uncompressed, without `Vary: Accept-Encoding` added

### Requirement: The default filter follows compressible
The default filter, `should_compress`, SHALL accept a response whose `Content-Type` media type (parameters removed,
lowercase) is marked compressible in mime-db 1.54.0, or, for types mime-db has no `compressible` flag for, matches
`^text/` or ends in `+json`, `+text` or `+xml`. It MUST reject responses without `Content-Type` and responses of type
`text/event-stream`.

#### Scenario: Octet stream
- **WHEN** a 10 KiB `application/octet-stream` response is requested with `Accept-Encoding: gzip`
- **THEN** it has `Content-Encoding: gzip`, as with the reference (mime-db marks the type compressible)

#### Scenario: Server-Sent Events
- **WHEN** a client with `Accept-Encoding: gzip, br` subscribes to an `Sse` route
- **THEN** the events are sent uncompressed and each one reaches the client as soon as it is produced

### Requirement: A custom filter replaces the default one
A filter set with `CompressionOptions::filter` SHALL be called instead of `should_compress`, with the request method,
path and headers and the response status and headers, and `should_compress` SHALL be public so a custom filter can
fall back to it.

#### Scenario: Custom filter
- **WHEN** compression is enabled with a filter that returns `false` for requests carrying `X-No-Compression` and
  `should_compress` otherwise, and a client sends that header with `Accept-Encoding: gzip`
- **THEN** a 10 KiB JSON response is sent uncompressed

### Requirement: Vary is added before the remaining checks
For a response that passes the filter and the `no-transform` check, `Accept-Encoding` SHALL be appended to `Vary`
(keeping existing values, without duplicates, and leaving `Vary: *` as is) whether or not the response ends up
compressed.

#### Scenario: Small response
- **WHEN** a 100-byte JSON response is requested without `Accept-Encoding`
- **THEN** it is sent uncompressed with `Vary: Accept-Encoding`

#### Scenario: Existing Vary
- **WHEN** CORS has set `Vary: Origin` on a 10 KiB JSON response requested with `Accept-Encoding: gzip`
- **THEN** the response carries `Vary: Origin, Accept-Encoding`

### Requirement: Responses below the threshold are not compressed
A response SHALL NOT be compressed when its `Content-Length` header is a number below the threshold, or, without that
header, when its body length is known and below the threshold. A streamed body of unknown length SHALL be compressed
whatever its size. The threshold defaults to 1024 bytes and is set with `CompressionOptions::threshold`.

#### Scenario: Default threshold
- **WHEN** JSON responses of 1023 and 1024 bytes are requested with `Accept-Encoding: gzip`
- **THEN** the first is sent uncompressed and the second with `Content-Encoding: gzip`

#### Scenario: Stream without length
- **WHEN** a `Response::stream` without `Content-Length` and with type `text/plain` sends 100 bytes to a client with
  `Accept-Encoding: gzip`
- **THEN** the response has `Content-Encoding: gzip`

### Requirement: Encoded, HEAD, bodyless and ranged responses are not compressed
A response SHALL NOT be compressed when it already has a `Content-Encoding` other than `identity`, when the request
method is `HEAD`, when its status is 1xx, 204 or 304, or when it has a `Content-Range` header. A `HEAD` response keeps
the `Content-Length` the `GET` would have had before compression. The `Content-Range` and status rules deliberately
differ from the reference, which compresses partial content.

#### Scenario: HEAD
- **WHEN** a `HEAD` request with `Accept-Encoding: gzip` targets a route whose `GET` returns 10 KiB of JSON
- **THEN** the response has no `Content-Encoding`, has `Vary: Accept-Encoding`, and keeps `Content-Length: 10240`

#### Scenario: Range request on a static file
- **WHEN** compression is enabled, a static 100 KiB `.txt` file is requested with `Range: bytes=0-49999` and
  `Accept-Encoding: gzip`
- **THEN** the response is `206` with `Content-Range: bytes 0-49999/102400`, no `Content-Encoding`, and 50,000 bytes of
  body

#### Scenario: Already encoded
- **WHEN** a handler returns a 10 KiB body with `Content-Encoding: gzip` already set
- **THEN** it is sent as it is, with `Vary: Accept-Encoding` added

### Requirement: The encoding is negotiated like the reference
The encoding SHALL be the result of negotiator 0.6.4's `encoding(['br', 'gzip', 'deflate', 'identity'],
['br', 'gzip'])` for the request's `Accept-Encoding`. Without an `Accept-Encoding` header, or when the result is
`identity` or nothing, the response SHALL NOT be compressed, and no `406` is sent.

#### Scenario: No header
- **WHEN** a 10 KiB JSON response is requested without `Accept-Encoding`
- **THEN** it is sent uncompressed with `Vary: Accept-Encoding`

### Requirement: Accept-Encoding is parsed like negotiator 0.6.4
Entries SHALL be split on commas, each with its `q` parameter (default 1), names compared case-insensitively; `*`
MUST match any encoding with lower specificity than an exact name; `identity` MUST be acceptable at the lowest listed
quality unless listed; entries with `q` 0 or an unparseable `q` MUST be excluded; the order SHALL be higher `q` first,
then `br`, then `gzip`, then the header's order.

#### Scenario: Preference on ties
- **WHEN** a 10 KiB JSON response is requested with `Accept-Encoding: gzip, deflate, br`
- **THEN** it has `Content-Encoding: br`

#### Scenario: Quality wins
- **WHEN** it is requested with `Accept-Encoding: br;q=0.5, gzip`
- **THEN** it has `Content-Encoding: gzip`

#### Scenario: Deflate only
- **WHEN** it is requested with `Accept-Encoding: deflate`
- **THEN** it has `Content-Encoding: deflate` and the body is in the zlib format (RFC 1950)

#### Scenario: Nothing acceptable
- **WHEN** it is requested with `Accept-Encoding: identity;q=0, gzip;q=0`
- **THEN** it is sent uncompressed with status 200

### Requirement: Headers of a compressed response
A compressed response SHALL carry `Content-Encoding` with the chosen encoding, MUST NOT carry `Content-Length`, and
SHALL keep its status, `ETag`, `Accept-Ranges` and every other header. Its decoded body MUST equal the uncompressed
body.

#### Scenario: JSON with ETag
- **WHEN** a 10 KiB JSON response with a weak `ETag` is requested with `Accept-Encoding: gzip`
- **THEN** it has `Content-Encoding: gzip`, no `Content-Length`, the same `ETag` as the uncompressed response, and its
  gunzipped body equals the JSON

### Requirement: Streamed bodies are compressed incrementally
A streamed body SHALL be compressed as it is read, feeding the encoder at most 64 KiB at a time, and the encoder MUST
be flushed whenever the source has no data ready, so that every chunk the source produced is sent without waiting for
more input. The response MUST NOT be read into memory as a whole, and an error of the source MUST end the response
as it does without compression.

#### Scenario: Large static file
- **WHEN** a 5 MiB static `.json` file is requested with `Accept-Encoding: gzip`
- **THEN** the response is compressed, its decoded body equals the file, and the server never holds more than a bounded
  amount of it (the 64 KiB read chunk plus the encoder's buffers) for that response

#### Scenario: Slow stream
- **WHEN** a `text/plain` stream sends one 2 KiB chunk, waits one second and sends another, to a client with
  `Accept-Encoding: gzip`
- **THEN** the client can decode the first chunk before the second is produced

### Requirement: Compression levels are bounded
`CompressionOptions::level` SHALL be clamped to 0..=9 (default 6) and `CompressionOptions::brotli_quality` to 0..=11
(default 4). A single compression step MUST NOT process more than 64 KiB of input, so one large body does not hold a
worker thread for the whole body.

#### Scenario: Out of range
- **WHEN** compression is enabled with `level(20)` and `brotli_quality(50)`
- **THEN** gzip uses level 9 and brotli quality 11

### Requirement: Compressions in progress are capped
The number of responses being compressed at the same time SHALL be limited by `CompressionOptions::max_concurrent`
(at least 1, default 8 times the available parallelism), and `br` SHALL use at most half of those slots (at least one).
A slot MUST be held until the compressed body ends, fails or is dropped. One connection SHALL hold at most
`CompressionOptions::max_per_connection` slots (at least 1, default 4); a response over its connection's cap MUST be
sent uncompressed, and requests without a connection (the in-process test client) are only subject to the global cap. When no slot is available for the negotiated
encoding, a response that would use `br` SHALL use the best other encoding the client accepts (`gzip` or `deflate`) if
a slot is free for it, and otherwise MUST be sent uncompressed. A response that was eligible (it passed the filter and
`no-transform` checks) MUST carry `Vary: Accept-Encoding` whether or not a slot was available. The brotli window MUST
NOT exceed 2^19 bytes, whatever the body length, which deliberately differs from the reference's fixed 2^22.

#### Scenario: Cap reached
- **WHEN** the cap is 2 and the first response with `Accept-Encoding: br, gzip` is still being sent
- **THEN** the second response with the same header uses `gzip`, the third is sent uncompressed with
  `Vary: Accept-Encoding`, and after the first body is dropped a new request gets `br` again

#### Scenario: Per-connection cap
- **WHEN** one HTTP/2 connection (h2c) has four compressed responses in progress, which default
  `CompressionOptions::max_per_connection` allows, and opens a fifth stream with `Accept-Encoding: gzip`
- **THEN** the fifth response is sent uncompressed with `Vary: Accept-Encoding`, other connections still get compressed
  responses, and when a response of the first connection ends or is dropped the next one is compressed again

#### Scenario: Slot released
- **WHEN** a compressed body ends, or is dropped before it ends
- **THEN** its slot is free for the next response

#### Scenario: Large body brotli window
- **WHEN** a 10 MiB body is compressed with `br`
- **THEN** the encoder uses a window of 2^19 bytes and the decoded body equals the original

### Requirement: Behavior matches the compression middleware
For the same routes and requests, the status, `Content-Encoding`, `Vary`, presence of `Content-Length`, `ETag` and
decoded body of Corvis responses SHALL match those of the original stack with `app.use(compression())` (`compression`
1.8.2), verified by a differential test against a recorded fixture, except for the documented divergences
(`text/event-stream`, `Content-Range`).

#### Scenario: Replay of the recorded requests
- **WHEN** the Rust test replays the requests recorded against the original stack (small and large JSON, text, HTML,
  `image/png`, `application/octet-stream`, `no-transform`, an existing `Content-Encoding`, `HEAD`, `304` with
  `If-None-Match`, a static file, a stream without length, each with several `Accept-Encoding` values)
- **THEN** every response matches the recorded one on those headers and on the decoded body

### Requirement: Negotiation and type checks match their libraries exactly
The encoding negotiation and the compressible check SHALL return, for every recorded input, exactly what negotiator
0.6.4 and compressible 2.0.18 with mime-db 1.54.0 returned.

#### Scenario: Compressible table
- **WHEN** the unit test runs the recorded types (with parameters, mixed case, types without a mime-db flag, unknown
  types) through the compressible check
- **THEN** every result equals what compressible 2.0.18 returned

#### Scenario: Negotiation table
- **WHEN** the unit test runs the recorded `Accept-Encoding` values (empty, `*`, `identity`, `gzip;q=0`, mixed case,
  q-values with three decimals, invalid q-values, unknown encodings) through the negotiation
- **THEN** every result equals the encoding negotiator 0.6.4 returned
