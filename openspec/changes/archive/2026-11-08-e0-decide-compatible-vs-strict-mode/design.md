# Design

## Context

Corvis reproduces what a client observes from an Express 5 application on the original Node stack (body parser,
multer, cors, cookies, errors). The HTTP/1 wire format itself is not reproduced: it is parsed by hyper 1.12.0, which
uses httparse 1.10.1 for the request head and its own decoder for chunked bodies (`Cargo.lock`). The audit
(`corvis-tests/tmp/security/http-input/FINDINGS.md`, section 9 and "Revisado y descartado") tested framing and
smuggling and found none; it left the questions this spike answers.

### What the original stack does (measured, not assumed)

The Node behavior below was measured on 2026-11-08 with a raw-socket script against `http.createServer` on Node
v24.13.0 (llhttp 9.3.0), the Node version houseplan's original runs on (`nest/houseplan` Dockerfile: `node:24-slim`;
consi and facten use `node:22`). The multer behavior comes from the installed packages of the three originals in
`corvis-tests/nest/*/node_modules`.

| Input | Node 24 (`http`) | Corvis today |
|---|---|---|
| Request line or header lines ending in bare LF | `400 Bad Request` | accepted (httparse `newline!` macro, `httparse-1.10.1/src/macros.rs:55-67`, takes `\n` alone as a line end) |
| Header block ended by `\r\n\n` | `400` | accepted (same macro) |
| Bare CR inside a header line | `400` | rejected (`Error::NewLine`, same macro) |
| Chunk size with leading zeros (`0002`, 26 digits) | accepted | accepted (`hyper-1.12.0/src/proto/h1/decode.rs:377-408`, `read_size` accumulates hex digits with `checked_mul`/`checked_add`; no digit count limit) |
| Chunk size over 64 bits (`10000000000000002`) | `400` | rejected (`or_overflow!`, `decode.rs:264-272`) |
| Whitespace after the chunk size (`2 \r\n`, `2\t\r\n`, `2 ;a=b\r\n`) | `400` | accepted (`read_size` → `SizeLws`, `decode.rs:398` and `410-425`) |
| Bare LF in the chunked framing (`2\nhi\n0\n\n`) | `400` | rejected (`read_size` only ends a size on `\r`, `decode.rs:400`; `read_size_lf`/`read_body_cr` require CRLF, `decode.rs:457-477`, `508-519`) |
| Bare LF inside a trailer line | `400` | accepted as trailer content up to the next CRLF (`read_trailer`, `decode.rs:533-552`); to be confirmed by the follow-up |
| Both `Content-Length` and `Transfer-Encoding: chunked` | `400` | processed as chunked, connection closed after the response (audit, `poc/smuggle.py`) |
| Chunk extension containing LF | `400` | rejected (`read_extension`, `decode.rs:426-455`) |

So the HTTP parser leniencies are hyper/httparse behavior; on all of them Node is equally or more strict. RFC 9112
allows a recipient to accept a bare LF (§2.2) and to process a message with both headers as chunked if it closes the
connection (§6.3), so hyper is within the RFC; the risk is a front proxy that disagrees on the same bytes.

### Multipart nesting limit

- `handle_field` (`crates/corvis/src/multer/interceptors.rs:376-402`) returns
  `MulterError(LimitFieldNesting, name)` when `limits.field_nesting_depth` is set and the name has more `[` than the
  limit (`:393-396`), as multer 2 does (`make-middleware.js`: `fieldname.split('[').length - 1 > limits.fieldNestingDepth`).
- `transform_multer_error` (`crates/corvis/src/multer/mod.rs:365-377`) maps every other multer code to `400`/`413`
  but `LimitFieldNesting` to `HttpException::internal(e)` (`:375`), i.e. `500 Internal server error`.
- In the originals, `transformException` (`@nestjs/platform-express/multer/multer/multer.utils.js`) differs by
  version: 11.1.28 (consi) has no `LIMIT_FIELD_NESTING` case, so the multer error falls through and becomes a `500`;
  11.2.1 (facten) and 12.0.4 (houseplan) list it with the other limits and answer
  `400 Field name nesting too deep - <field>`.
- Without `field_nesting_depth`, names are never rejected: `append_field` stores names deeper than
  `DEFAULT_MAX_FIELD_DEPTH = 32` (hard cap `MAX_FIELD_DEPTH = 256`) as literal keys
  (`crates/corvis/src/multer/append_field.rs:14-19`), tested by `deeply_nested_field_name_does_not_overflow`
  (`crates/corvis/tests/transport_security.rs:325-334`).

### Other behaviors with security relevance found in `http/`, `server.rs` and `multer/`

- **Client-triggered 500s from busboy configuration errors.** `process` maps every `ConfigError` except
  `BoundaryNotFound` to `HttpException::internal` (`interceptors.rs:297-301`): `Malformed content type` and
  `Unsupported content type: multipart/mixed` (`busboy.rs:36-55`) answer `500` on an upload route. The original does
  the same (busboy throws, `transformException` does not list those messages). The body is the generic
  `Internal server error`, so nothing leaks; the cost is that clients can produce 5xx at will (error-rate alerts,
  circuit breakers).
- **CORS.** `OriginMatcher::Regex` uses `Regex::is_match` (`crates/corvis/src/router/cors.rs:28-37`), unanchored like
  JavaScript's `regex.test`, so `/example\.com/` also matches `https://example.com.evil.test`; `reflect_origin()`
  reflects any origin, `null` included (`cors.rs:229`), and `credentials(true)` adds
  `Access-Control-Allow-Credentials: true` regardless (`cors.rs:250`). This is the `cors` package's behavior and a
  configuration choice of the application.
- **Cookies.** `CookieOptions::default()` has `http_only: false`, `secure: false`, no `SameSite`
  (`crates/corvis/src/http/res.rs:142-160`), like Express `res.cookie`. Explicit per call; a default change would
  break parity on every `Set-Cookie`.
- **h2c.** `ServerOptions::default()` enables HTTP/2 with prior knowledge (`crates/corvis/src/server.rs:58`); Node's
  `http` module does not speak it. `ServerOptions::h2c(false)` turns it off (`server.rs:100-103`), enforced by
  `TimedIo::on_read` (`server.rs:472-488`). A Corvis default, not an Express one.
- **Head size.** 32 KiB by default (`server.rs:57`, `max_header_size` `:94-97`), Node 16 KiB. Bounded; not a quirk.

### Parity impact on the three port apps

- None of consi, facten or houseplan sets `field_nesting_depth` (grep over `corvis-tests/corvis/*/src`), so the
  nesting change cannot alter an apidiff check. consi's original (11.1.28) would still answer `500` there, but the
  path is unreachable in that app.
- apidiff sends requests with `node:http` (`corvis-tests/tools/apidiff/apidiff.mjs:10`), which always writes CRLF and
  never mixes `Content-Length` with `Transfer-Encoding`, so rejecting bare LF or such framing later cannot change an
  apidiff result either.
- consi and facten enable CORS with an explicit origin list and `credentials(true)` (consi `src/main.rs:61`, facten
  `src/app.setup.rs:27-36`); a warning for `reflect_origin()` + credentials would not fire for them.

## Goals / Non-Goals

**Goals:**
- A recorded decision on how Corvis handles lenient behaviors of the original stack and of its own HTTP parser.
- Every security-relevant leniency listed with its owner (Express package, hyper/httparse, Corvis default), the
  original's behavior and the chosen treatment.
- The nesting limit answered as a client error.
- Behaviors kept on purpose pinned by regression tests, and documented.

**Non-Goals:**
- Changing hyper's HTTP/1 parsing in this change (follow-up issue).
- Changing CORS, cookie or h2c defaults (see proposal, Out of scope).

## Decisions

1. **No global compatible/strict mode; per-behavior treatment with secure defaults (option C, recommended).**
   The options considered:
   - **A. Strict by default, one global opt-out** (`ServerOptions::compatible()`). Simple to explain, but it couples
     unrelated behaviors (multipart error codes, wire framing, CORS validation) behind one flag, so an application
     that needs one leniency for a legacy client gets all of them. Every feature would need tests and docs in both
     modes.
   - **B. Compatible by default, one global opt-in** (`ServerOptions::strict()`). Keeps today's behavior, but leaves
     the safer behavior to users who already know about it, and it is the default that the audit questioned. Same
     coupling and doubled test matrix as A.
   - **C. Per-behavior decisions, secure default, targeted opt-out only where a real client may need it.** The
     inventory shows few items and that, for most of them, the secure behavior is what current Node or current
     `@nestjs/platform-express` already does, so there is no parity to buy with a lenient default. An opt-out exists
     only for the wire-format leniency, named after Node's own escape hatch (`insecureHTTPParser`) so migrating users
     recognize it.
   Recommendation: C. It keeps one behavior per feature to document and test, and it follows the project rule
   "security over parity" without paying parity where the original is already strict.

2. **Per-behavior treatment under option C.**

   | Behavior | Treatment | Where |
   |---|---|---|
   | `field_nesting_depth` exceeded → `500` | `400 Field name nesting too deep - <field>` (current upstream) | this change |
   | Bare LF line endings in the request head | reject with `400` and close; opt-out `ServerOptions::insecure_http_parser(true)` | follow-up issue |
   | Whitespace after a chunk size | reject, same opt-out | follow-up issue |
   | `Content-Length` + `Transfer-Encoding` | reject with `400` like Node, same opt-out (hyper's current handling is RFC-compliant, so lower priority) | follow-up issue |
   | Leading zeros in chunk sizes | keep (Node accepts them; RFC grammar `1*HEXDIG`; overflow already rejected) | pinned by tests here |
   | Bare LF in chunked framing, extension with LF | keep rejecting | pinned by tests here |
   | Busboy configuration errors → `500` | keep (parity; no leak) | open question 2 |
   | CORS regex unanchored, reflect + credentials | keep `cors` semantics; warn at startup for reflect + credentials | separate issue |
   | Cookie defaults (no `HttpOnly`/`Secure`/`SameSite`) | keep (Express defaults, explicit per call) | docs only |
   | h2c enabled by default | unchanged here | separate issue |

3. **The HTTP/1 leniencies are fixed upstream or in a dedicated change, not in this spike.** hyper's HTTP/1 builder
   (`hyper-1.12.0/src/server/conn/http1.rs:268-423`) exposes no option for bare LF or chunk-size whitespace, and
   httparse's `ParserConfig` only has `allow_*` switches that relax parsing further. Options for the follow-up:
   - Contribute an option to httparse (reject bare LF) and hyper (strict chunk-size line), then enable it from
     `serve` (`crates/corvis/src/server.rs:589-594`). Clean, but depends on upstream review and releases.
   - Scan the bytes in `TimedIo::on_read` (`server.rs:472-488`), which already inspects reads for the h2c preface.
     Rejected as the main fix: the IO layer cannot tell head bytes from body bytes once a request is in flight
     without re-implementing the framing, so a pipelined second request (exactly where a smuggled request would sit)
     would escape the check.
   - `[patch.crates-io]` with a forked httparse: only affects this workspace, not applications depending on the
     published `corvis` crate. Rejected.
   The follow-up decides between the first option and a maintained fork, and adds the `insecure_http_parser` opt-out.

4. **Nesting limit mapped like the other multer limits.** `LimitFieldNesting` joins the `BadRequestException` arm of
   `transform_multer_error`, which already formats `"<message> - <field>"`. Alternatives: `413` (the body is not too
   large) or `422` (the original never uses it for multer). The `MulterError` code and message stay the same, so a
   custom exception filter that inspects them keeps working.

5. **Documentation as one security page.** `docs/security/http-parsing.md` holds the table of decision 2 written for
   users (what is accepted, what is rejected, how it compares with Node, what changes with the follow-up), so the
   answer to "is Corvis strict?" has one place. `docs/http/file-upload.md` gains the status of the nesting limit in its
   limits table. The page avoids naming the original framework (public naming rule) and refers to "the original Node
   application" and to the Node packages (`multer`, `cors`, `llhttp`).

## Risks / Trade-offs

- [An application relied on the `500` for nesting (for example a filter that maps 5xx to alerts)] → release note; the
  crates are not published yet, and no port or example sets the limit.
- [The follow-up depends on upstream] → until it lands, the docs page states that bare LF is accepted and that a front
  proxy should normalize line endings; the audit found no smuggling with common proxies.
- [Option C means no single switch for compliance-minded users] → the docs page lists every behavior with its
  status, which is what such a user needs to audit anyway.
- [Node measurements were taken on Node 24; consi and facten originals run on Node 22] → task 2.3 repeats the probe on
  Node 22 and records any difference in the docs page.

## Open questions

1. **Approve option C** (no global mode; per-behavior secure defaults; `insecure_http_parser` opt-out for the wire
   format only)? Recommendation: yes. Approved (see Review notes).
2. **Busboy configuration errors** (`Malformed content type`, `Unsupported content type: ...`) answer `500` like the
   original. Keep parity, or answer `400`/`415` and document it as a deliberate divergence? Recommendation: keep for
   now; revisit with the route posture work (E1), which is where client-triggered 5xx matter most.
3. **Where the follow-up issue lives**: epic #1 (Foundations, security leftovers) or #13 (core fixes)?
   Recommendation: #1, next to this spike.
4. **Name of the opt-out**: `insecure_http_parser` mirrors Node's `insecureHTTPParser` exactly; alternative
   `lenient_http_parsing`. Recommendation: mirror Node.

## Review notes (2026-11-08)

- **Maintainer decision (2026-11-08):**
  1. Option C is approved: no global compatible/strict mode; per-behavior secure defaults as in decision 2.
  2. Busboy configuration errors (`Malformed content type`, `Unsupported content type: ...`) keep answering `500` for
     parity; revisit with the route posture work (E1).
  3. The follow-up issue for the protocol-level strictness goes in epic #1.
  4. The opt-out is named `insecure_http_parser`, mirroring Node's `insecureHTTPParser`.
  5. h2c stays enabled by default in this change; whether to keep it on is decided in a separate issue.
- Parity note for the decision: answering 400 for the multipart nesting limit diverges from the original version used
  by the consi port (11.1.28 answers 500) while matching the versions used by facten and houseplan.
- The follow-up issue for the protocol-level strictness (bare LF and the other chunked cases) goes in epic #1 once the
  decision is taken.
- Follow-up issue for the protocol-level strictness: #146 (epic #1), opened from `follow-up-issue.md`.
