# Design

## Context

Source: the engineering-quality review of the HTTP layer, the server and the multipart module (issue filed from
`quality/q-http-response-consistency.issue.md`, epic #187). Code read on `dev` at `d46758a` (2026-11-11). The original
packages are not vendored in the repository (no `node_modules` is committed), so every claim about what the original
does is settled by a recorded fixture, never by memory (Decision 1).

### How the existing differential fixtures are recorded

`crates/corvis/tests/fixtures/<area>/` holds a `package.json` pinning exact versions of the original packages, its
`package-lock.json`, a `generate.mjs` that runs them (servers on `127.0.0.1` only) and writes `<area>.json` with the Node
and package versions it ran, and the Rust tests replay the JSON (`compression/`, `throttler/`, `auth/` with
`signed_cookies.mjs`, `empty_results/`). Two older files predate that layout: `res_express.oracle.mjs` (reads an
application's `node_modules` by absolute path) and `v8_json_parse.json` (no generator committed). The new fixture follows
the newer layout. The auth fixture's lock shows what express 5.3.0 resolves: `cookie` 0.7.2, `encodeurl` 2.0.1, `fresh`
2.0.0, `send` 1.2.1, `serve-static` 2.2.1, `escape-html` 1.0.3, `vary` 1.1.2, `etag` 1.8.1.

### Findings as checked

| # | Finding | Where now | Verdict |
|---|---|---|---|
| 8a | Two cookie serializers diverge | `response.rs:273-308` (`Cookie::try_to_string`: `NON_ALPHANUMERIC`, order Path, Domain, Max-Age, HttpOnly, Secure, SameSite; no `Expires`; `SameSite` written as typed; `"true"` refused; the value never validated) against `res.rs:246-302` (`serialize_cookie`: `encodeURIComponent`, order Max-Age, Domain, Path, Expires, HttpOnly, Secure, Partitioned, Priority, SameSite; `Expires` from `max_age`; `"true"` is `Strict`) | Confirmed. `tests/static_response_security.rs:299-301` even pins the divergence (`SameSite=lax`, no `Expires`) |
| 8b | Two `encodeurl` diverge | `res.rs:339-379` (kept set `0x21, 0x23-0x3B, 0x3D, 0x3F-0x5F, 0x61-0x7A, 0x7C, 0x7E`; a malformed escape is one unit with the bytes after it) against `serve_static.rs:205-222` (kept set alphanumerics and ``!#$&'()*+,-./:;=?@[]_~``; `\`, `^`, `|` encoded; a malformed `%` encoded alone) | Confirmed. The unit tests of each copy claim to match the package (`res.rs` `encodeurl_matches_the_package` keeps `\^|`; `serve_static.rs` `helpers_match_the_node_packages`); at most one is right. Used by `Response::redirect`, `Res::location` and the static directory redirect |
| 8c | Two `fresh` diverge | `express.rs:137-163` (token match of `no-cache`; returns at once for `If-None-Match: *`; returns the ETag result without looking at `If-Modified-Since`) against `serve_static.rs:587-612` (`c.contains("no-cache")`; checks both headers; seconds instead of milliseconds) | Confirmed, and wider than reported: besides `no-cache-x`, the two copies disagree on `*` and on a request with both headers. Which one matches the original is for the fixture |
| 8d | Other duplicates | `escape_html` `res.rs:906-918` and `serve_static.rs:189-202` (identical); `parse_http_date` `express.rs:189-194` (ms, RFC 2822 then a `GMT` pattern) and `serve_static.rs:234-236` (s, RFC 2822 only); `vary_append` `router/cors.rs:331-346` (rebuilds the list, always `Some`) and `compression/mod.rs:309-321` (keeps the original text); `latin1` `body_parser.rs:136-138` and `busboy.rs:926-928` (identical); `js_number` `body_parser.rs:159-175` (`Option<f64>`, ASCII whitespace, no `0o` / `0b`) and `compression/negotiate.rs:78-95` (`f64`, JS whitespace, all prefixes) | Confirmed. `escape_html` and `latin1` are copies without divergence; the others diverge |
| 9 | Public API panics on run-time input | `response.rs:313-320` (`Display`), `response.rs:166-174` (`with_cookie` / `set_cookie` through `to_string`), `res.rs:246-299` (`serialize_cookie`), `res.rs:531-542` (`header_name`, `header_value`) reached by `set_header` 591, `set_header_values` 605, `set` 621, `header` 629, `append` 666, `type_` 677, `content_type` 692, `vary` 696, `links` 704, `attachment` 714 (only through `type_` with an extension, always valid), `cookie` 733, `cookie_json` 737, `clear_cookie` 741; `status` 570-575 and through it `send_status` 581 and `redirect_with` 823 | Confirmed. The panics are caught by the router's `catch_unwind` and answered `500` (pinned by `invalid_cookies_answer_500_without_the_header`), but a panic in a `Display` used by a log line, under `panic = "abort"`, or outside a handler is not caught |
| 6a | Serialization errors answered `200 null` | `response.rs:98`, `res.rs:793`, `sse.rs:30`; `Json<T>` propagates (`response.rs:411`) | Confirmed. Found while confirming: `__private.rs:285-293` builds a `Value` and hands it to `Response::json`, which serializes the `Value` into a new one: a deep copy of every typed handler result. Siblings outside this layer: `websockets.rs:98,160,207,262,487,515`, `modules/cache.rs:88` (follow-up) |
| 6b | Responses serialized twice (T → `Value` → bytes) | `response.rs:47`, `express.rs:299` | Narrowed, the redesign rejected (Decision 5) |
| 11 | `IntoStatus for i32` truncates | `exception.rs:351-355` (`self as u16`); `u16` falls back to `500` without saying so (`exception.rs:346-350`) | Confirmed |
| — | `same_site` and `priority` as strings | `res.rs:154-157`, `response.rs:225`; an enum `SameSite` already exists in `auth/session/options.rs:36-63` | Confirmed; Decision 4 |
| — | `Res::send` takes the state lock four times | `res.rs:772-790` (`has_header`, `type_` → `set` → `set_header`, `get`, `finish`) | Confirmed; folded into Decision 3, which rewrites these methods |

## Goals / Non-Goals

**Goals:**

- One implementation of each original helper, and a recorded fixture that says which behavior is right.
- No public function of this layer panics on a value a caller can pass at run time; errors are values.
- A value that cannot be serialized is an error, never `null`.

**Non-Goals:**

- Performance of the response path (`q-http-request-hot-path`).
- Changing what the three port apps answer: the fixture decides only between copies that already diverge; where a port
  app's traffic hits a changed helper, the parity check decides (Risks).

## Decisions

### 1. A fixture recorded from the original packages

`crates/corvis/tests/fixtures/node_compat/`:

- `package.json` (`"private": true`, `"type": "module"`, a `generate` script) pinning `express` 5.3.0, `cookie` 0.7.2,
  `encodeurl` 2.0.1, `fresh` 2.0.0, `serve-static` 2.2.1, `send` 1.2.1, `escape-html` 1.0.3 and `vary` 1.1.2 (what the
  auth fixture's lock resolves for express 5.3.0); `package-lock.json` committed.
- `generate.mjs` writes `node_compat.json` with `node`, the package versions read from `node_modules`, and one array per
  helper:
  - `cookie`: an express app on `127.0.0.1` with one route that calls `res.cookie(name, value, options)` from the query
    (or `res.clearCookie`) and answers the `Set-Cookie` values, or the thrown error's `name` and `message`. Cases: names
    with each class of character of the name grammar and the separators; values with `-_.!~*'()`, space, `;`, `"`, `%`,
    non-ASCII, the empty string; `maxAge` positive, negative, fractional, zero; `expires`; `domain` valid, invalid,
    leading dot; `path` default, empty, with space, with `;`; `httpOnly`, `secure`, `partitioned`; `priority` in each case
    and invalid; `sameSite` `true`, `false`, `'lax'`, `'Strict'`, `'none'`, invalid; `encode` set to the identity (Corvis'
    `raw`). `Expires` depends on the clock: the recorder stores it as the number of seconds after the response's `Date`
    header, and the replay checks that difference within one second.
  - `encodeurl`: `encodeurl(s)` for every ASCII character alone and inside a path, `%` followed by zero, one and two hex
    digits and by non-hex characters, lone surrogate-free non-ASCII; plus `res.location(s)` and the `Location` of
    `express.static`'s directory redirect for a folder whose name has a space, `^`, `|` and `\` (on Windows the backslash
    case is recorded as not applicable).
  - `fresh`: `fresh(reqHeaders, resHeaders)` for a table crossing `If-None-Match` (absent, `*`, one tag, a list, weak and
    strong forms, extra spaces), `If-Modified-Since` (absent, equal, earlier, later, malformed, other date formats),
    `Cache-Control` (`no-cache`, `no-cache-x`, `x, no-cache`, `NO-CACHE`, `max-age=0`) and response `ETag` /
    `Last-Modified`; plus the status (`200` or `304`) of `express.static` on a temporary folder for the same request
    headers.
  - `vary`: `vary.append(header, field)` for empty, `*`, mixed case, lists with and without spaces, a field list.
  - `escape_html`: `escape-html` over the five characters and mixed text.
  - `number`: `Number(s)` for the strings a header can hold: empty, spaces of each JS whitespace class, signs, decimals,
    exponents, `0x`, `0o`, `0b` with valid and invalid digits, `Infinity` with signs, letters.
- Replay: `crates/corvis/tests/node_compat.rs` for what is public (`serialize_cookie`, `Cookie`, `Res` through a test
  application, `Response::redirect`, `serve_static` through a test application) and unit tests in `http/node_compat.rs`
  for the crate-private helpers, reading the same file with `include_str!`. Each case matches byte for byte, or carries a
  `divergence` string in a small Rust table next to the replay (never in the recorded JSON), and the test fails when a
  divergence entry no longer differs.
- `generate.mjs` header comment: usage (`npm ci && node generate.mjs`), the loopback rule, and what each array records.

The fixture is recorded first (task 1.1); its result decides each copy in Decision 2, and the decisions record the
outcome in "Implementation notes".

### 2. One implementation of each helper

`crates/corvis/src/http/node_compat.rs` (crate-private) holds `encode_url`, `fresh(req: &HeaderMap, etag: Option<&str>,
last_modified: Option<&str>) -> bool`, `vary_append(header: &str, field: &str) -> String`, `escape_html`, `http_date`,
`parse_http_date` (one unit, milliseconds), `latin1` and `js_number(&str) -> f64`. Every copy is deleted and its callers
use these:

- `encode_url`: `Response::redirect`, `Res::location` and the static directory redirect. `http::res::encode_url` stays
  public as a re-export (it is public today).
- `fresh`: `express::finalize` and `ServeStatic::is_fresh`. `serve_static` keeps its own options (`etag(false)`,
  `last_modified(false)`) by passing `None` for the header it does not send, which is what the original static middleware
  does when it does not set the header; the fixture's `express.static` cases check it.
- `vary_append`: `Res::vary`, CORS and compression. `corvis::router::cors::vary_append` is public today (both `router`
  and `cors` are `pub` modules) and returns an `Option` that is always `Some`; it stays for one release as a
  `#[deprecated]` wrapper over the new helper, with a note that `Res::vary` is the public way.
- `js_number`: the body parser's `!isNaN(content-length)` and the compression negotiation.
- `escape_html`, `parse_http_date`, `latin1`: their callers.

The behavior kept is the fixture's. Where the fixture shows that neither copy was right, the helper follows the fixture
and the change is listed in "Behavior visible to applications" with the case.

### 3. Errors instead of panics, in the types of the original's exceptions

- `CookieError` (`#[non_exhaustive]` enum: `InvalidName`, `InvalidValue`, `InvalidDomain`, `InvalidPath`,
  `InvalidSameSite`, `InvalidPriority`), `Display` = the original `TypeError` text (`argument name is invalid`, ...),
  `std::error::Error`. `serialize_cookie(..) -> Result<String, CookieError>`.
- `Cookie`: `try_to_string(&self) -> Result<String, CookieError>` builds a `CookieOptions` from its fields and calls
  `serialize_cookie`, so both APIs produce the same header (Decision 1 decides that header). `impl Display` is removed: a
  type whose rendering can fail is not `Display`, and a `Display` that returns `fmt::Error` makes `to_string()` panic.
  `Response::with_cookie(self, Cookie) -> Result<Self, CookieError>`, `Response::set_cookie(&mut self, Cookie) ->
  Result<(), CookieError>`.
- `ResError` (`#[non_exhaustive]`: `InvalidStatus(u16)`, `InvalidHeaderName(String)`, `InvalidHeaderValue { name }`,
  `Cookie(CookieError)`, `Json(serde_json::Error)`), `Display` = the original text (`RangeError: Invalid status code: ...`,
  `TypeError: Invalid character in header content ["x"]`, ...). The `Res` methods that validate input return
  `Result<&Self, ResError>`: `status`, `set_header`, `set_header_values`, `set`, `header`, `append`, `type_`,
  `content_type`, `vary`, `links`, `cookie`, `cookie_json`, `clear_cookie`; `send_status`, `redirect_with` and `json`
  return `Result<(), ResError>`. The methods whose input cannot be invalid keep their signatures: `send`, `end`,
  `end_empty`, `redirect` (fixed status, encoded location), `location` (encoded), `attachment` (the type comes from a
  known extension and the disposition is escaped), the getters.
- `?` in a handler: both errors implement `std::error::Error`, so `HttpException`'s blanket `From` turns them into the
  generic `500` with the error logged, which is what the router answers for today's panic. A handler that ignores the
  `Result` gets a `#[must_use]` warning; nothing invalid reaches the wire either way (the header is not added).
- `Res::send` validates and writes under one lock acquisition (`has_header`, the default type and `finish` in one
  critical section).

Alternative considered: keep the panicking methods and add `try_*` twins. Rejected: the house rule (a public function must
not panic on run-time input) would still be broken by the methods applications already call, and two names per method
double the surface of the most used type of a migration. Alternative considered: record the error in the `Res` and answer
`500` when the handler returns. Rejected: the handler would keep running after the point where the original stopped (a
database write after an invalid cookie would happen), while `?` stops it where the original's `throw` did.

### 4. Cookie attributes as enums

`SameSite { Strict, Lax, None }` moves from `auth/session/options.rs` to `http/res.rs`, re-exported as `http::SameSite`
and, unchanged, as `auth::session::SameSite`. New `CookiePriority { Low, Medium, High }`. `CookieOptions::same_site`,
`CookieOptions::priority` and `Cookie::same_site` hold them; the builder methods take them. Both implement `FromStr`
(case-insensitive; `"true"` parses as `Strict`, as the original's `sameSite: true`), returning `CookieError`, for values
read from configuration. With enums the two "invalid option" errors can only come from `FromStr`.

### 5. No silent `null`, one copy less, and the bytes redesign rejected

- `Response::json(value: impl Into<Value>) -> Self`: cannot fail; a `Value` passed in is moved, not re-serialized.
  `Response::try_json(value: impl Serialize) -> Result<Self, serde_json::Error>` for other types. `SerializeTag::convert`
  passes its `Value` to `Response::json`, which removes the deep copy of every typed handler result.
- `MessageEvent::new(data: impl Into<Value>)` and `MessageEvent::try_new(data: impl Serialize) -> Result<Self,
  serde_json::Error>`.
- `Res::json(value: impl Serialize) -> Result<(), ResError>` (Decision 3).
- The review's medium-term proposal, a JSON body kept as serialized bytes and turned into a `Value` only when an
  interceptor calls `map_data`, is rejected: two of the three port apps call `map_data` on every response (consi's global
  `TransformInterceptor`, houseplan's API interceptor) and read `json_body()`, so they would pay a parse on every request
  on top of today's work; `Body::Json(Value)`, `json_body()` and `map_data` are public API that interceptors are written
  against; and an error found when the bytes are written, after the filters ran, could no longer be answered by them.
  Writing the `Value` straight to bytes once is what the hot-path change measures instead.

Tests: a `Serialize` type that fails (a map with non-string keys) through `Response::try_json`, `Res::json`,
`MessageEvent::try_new` and a typed handler result: each is a `500` (or an `Err`), never `200 null`; `Response::json(v)`
with a `Value` keeps the same allocation (a unit test comparing the address of a string inside the value before and after).

### 6. Run-time statuses

`IntoStatus for i32`: `u16::try_from(self).map_or(INTERNAL_SERVER_ERROR, IntoStatus::into_status)`. The trait's rustdoc
says that a code outside `100..=999` is `500`. Test: `65736`, `-1`, `1000` and `99` give `500`; `404` gives `404`.

## Security invariants that must hold

From the first security audit (static-response and http-input areas, pinned by `tests/static_response_security.rs` and
`tests/input_hardening.rs`) and `e0-finish-fuzzing-serve-static-end-to-end-and-orm-filters`.

| Invariant | Source | Decisions near it | Pinned by |
|---|---|---|---|
| Outgoing cookie name, value, `Domain` and `Path` are validated like `cookie` 0.7 (no attribute injection, CVE-2024-47764) | static-response #5, auth-ws #11 | 1, 3, 4 | `outgoing_cookie_attributes_are_validated` (rewritten from panics to `Err` values, same cases) |
| An invalid cookie is never written; the request is answered `500` | static-response #5 | 3 | `invalid_cookies_answer_500_without_the_header` (the two handlers use `?`) |
| `Location` is encoded like `encodeurl` (no CR/LF, no raw `<`, `"`) | static-response #10 | 1, 2 | `redirect_location_is_encoded`, the fixture's `encodeurl` cases |
| The static directory redirect never reflects an unescaped path into HTML | static-response | 2 | `escape_html` unit cases, `fuzz_static.rs` |
| No file outside the folder, dotfiles policy, totality of static serving | static-files spec | 2 (the freshness change touches `serve_static`) | `tests/fuzz_static.rs`, `serve_static_*` tests of `static_response_security.rs` |
| Cookies read like `cookie.parse` (first occurrence wins) | http-input #6 | none (request side untouched) | `cookies_first_occurrence_wins` |
| Signed cookies: constant-time verification, size limit before HMAC | e3 sessions change | 3 (callers of `serialize_cookie`) | `tests/signed_cookies.rs` replay, the `HMAC_RUNS` tests |
| SSE fields cannot inject lines | static-response | 5 (`MessageEvent` constructors) | `sse_splits_cr_and_strips_newlines_from_fields` |

## Behavior visible to applications

| What | Before | After | Consequence |
|---|---|---|---|
| `Response::with_cookie(Cookie::new("a", "x-y").same_site(..))` | `a=x%2Dy; Path=/; SameSite=lax`, no `Expires` with `max_age` | As `res.cookie` (fixture): `a=x-y; Path=/; SameSite=Lax`, `Expires` with `max_age`, the original attribute order | Same header from both APIs; browsers read both forms alike |
| Static files with `Cache-Control: no-cache-x` and a matching `If-None-Match` | `200` | As the fixture says (expected `304`) | Fewer bytes; parity |
| `encodeurl` of `\`, `^`, `|` in a static redirect or in `Res::location` | Copies disagreed | One behavior, the fixture's | One of the two paths changes |
| Conditional `GET` with `If-None-Match: *` or with both conditional headers | The two copies disagreed | One behavior, the fixture's | One of the two paths changes |
| `Content-Length: 0b1` on a JSON request | No body (NaN) | As `Number()` | Edge case only |
| A value that cannot be serialized (`Response::try_json`, `Res::json`, typed result, SSE) | `200 null` | `500` / `Err` | Bugs surface |
| `Response::json(struct)` | Compiles | Does not compile: use `Response::try_json(&s)?` or `Json(s)` | Breaking, compile-time |
| `res.cookie(..)`, `res.set_header(..)`, `res.status(..)`, ... | Panic on invalid input, `500` by `catch_unwind` | `Result`; `?` gives the same `500` | Breaking, compile-time (`#[must_use]` warns on an ignored result) |
| `format!("{cookie}")` | Panics on an invalid cookie | Does not compile; `try_to_string()` | Breaking, compile-time |
| `CookieOptions::new().same_site("lax")` | String | `same_site(SameSite::Lax)` or `"lax".parse()?` | Breaking, compile-time |
| An `i32` status such as `65736` (`with_status`, `#[http_code]` built from an `i32`) | `200` (`65736 as u16`) | `500` | No wrapped statuses |

## Order with the other changes

- **After `e3-authentication-hot-path-and-consolidation`**: e3 adds `has_cookie_prefix` to `http/res.rs`, edits the
  session manager and the OAuth2 state cookie, which call `serialize_cookie`, and `Request::insert`/`get`/`remove`. This
  change then adapts those callers to `Result` and to `SameSite` in one place. Starting before e3 merges would conflict on
  every cookie caller.
- **After `q-http-server-robustness`**, which edits `res.rs` (`finish`, `wait_end`, `Drop`); no overlap in logic.
- **Before `q-http-request-hot-path`**, which edits `response.rs`, `express.rs` and `res.rs` for performance; its benchmark
  then measures the code this change leaves.
- Independent of `e2-socket-io-hot-path-and-api-hygiene` (no shared file).

## Risks / Trade-offs

- A helper that changes to match the fixture changes what a port app answers → the parity check runs; a difference that
  appears is either a parity gain on a route the apidiff suites cover (the original app answers like the fixture) or a
  sign that the fixture is incomplete, investigated before merging.
- The breaking signatures touch many tests and two port app call sites → the build lists every site; task 8.3 edits the
  port apps; the migration guide gets a section.
- `Date.parse` accepts more date formats than RFC 2822 → the fixture records them; formats Corvis does not parse are
  listed as divergences with their reason (V8-specific heuristics).

## Open questions

1. Return `Result` from the validating `Res` methods, or add `try_*` twins and keep panicking ones? Recommendation:
   `Result` (Decision 3).
2. Keep `http::res::encode_url` and `router::cors::vary_append` public? Recommendation: keep `encode_url` (documented,
   used in examples); deprecate `vary_append` for one release, since `corvis::router::cors` is reachable.
3. Should `Response::set_header` / `with_header` also return `Result`? Recommendation: no; they are builders that drop what
   they cannot write, which is now documented; enhancers have their own validated path (`core/enhancers` "Invalid headers
   are rejected").

## Review notes

These settle the open questions and override the design where they differ.

1. The validating `Res` methods return `Result`; no panicking twin is kept.
2. `encode_url` stays public; `router::cors::vary_append` is `#[deprecated]` for one release, then removed.
3. `Response::set_header` and `with_header` keep their signatures; their rustdoc says an invalid name or value is ignored.
4. The fixture is recorded and committed before any helper is changed; the replay test lands first with the cases that
   fail today marked, so the diff of each later task shows which cases it fixed.
5. The security review covers the cookie serializer, the encoders used in `Location` and HTML, and the freshness rule of
   static files.
