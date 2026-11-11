# Design

## Context

Source: the engineering-quality review of the HTTP layer, the server and the multipart module (issue filed from
`quality/q-http-request-hot-path.issue.md`, epic #187). Code read on `dev` at `d46758a` (2026-11-11). Nothing was measured
by the review; every performance claim below is either confirmed by reading what is copied or allocated, and then moved
by a number of the benchmark of Decision 1, or left for that benchmark to decide.

### Findings as checked

| # | Finding | Where now | Verdict |
|---|---|---|---|
| 4 | `Request` is deep-copied; the body is cloned several times | `request.rs:14-41` (`#[derive(Clone)]` over `headers: HeaderMap`, `body: Value`, `query: Map`, `params: HashMap`, `route_path: Option<String>`; the doc comment at 14-17 says cloning copies them); `router/mod.rs:589` (`req_for_cors`, whole request when CORS is on), `router/mod.rs:624` (`req_for_errors` when any middleware applies), `request.rs:159` (`body_as`: `from_value(self.body.clone())`) | Confirmed. Narrowed: `extract_param` (`router/mod.rs:902`) clones the body because pipes own their input (`PipeTransform::transform(Value, ..)`), so one copy for `#[body]` is inherent to the pipe API and stays; `query_as` (`request.rs:177`) clones a query bounded by the URI length and `common::lenient` owns its input, so it stays. Found while confirming: `user_as` (`request.rs:313-315`) clones the stored user; `route_path` is a `String` copied into every request (`router/mod.rs:702`, left to `q-core-request-hot-path` Decision 4); and `body_error.clone()` (`router/mod.rs:593`) copies an exception on the error path only (left) |
| 5 | JSON parsing goes through UTF-16 and over-allocates | `body_parser.rs:245-254` (`String::from_utf8_lossy(body).encode_utf16().collect()`, then `units[1..].to_vec()` with a BOM); `json_parse.rs:52` (`parse(&str)` encodes to UTF-16 as well); `json_parse.rs:214-216` (`JsObject` with a `HashMap<String, usize>` index cloning each key); `json_parse.rs:492-493` (`scan_literal` collects a `Vec<u16>` per `true` / `false` / `null`) | Confirmed by reading; the cost needs the benchmark (Decision 3 names the number). The UTF-16 path exists for V8-identical messages with UTF-16 positions, pinned by `tests/json_parse.rs` over 1,000+ cases recorded from Node 24.13.0 (`fixtures/v8_json_parse.json`) |
| 7 | `finalize` copies text bodies; content type round trip; `X-Powered-By` validated per request | `express.rs:300` (`Bytes::from(t.clone())`), `express.rs:332` (`to_string()` of the type for every sent body), `server.rs:321-338` (`into_hyper` builds a `String` per response and parses it with `HeaderValue::from_str` when the header is missing), `express.rs:290-295` (`HeaderValue::from_str(p)` per request) | Confirmed. Narrowed: `into_hyper`'s parse runs only for raw bodies (`Response::bytes`, streams) because `finalize` sets the header of every sent body; the `String` is still built for every response |
| 12 | `serve_static` repeats per request work that can be done once | `serve_static.rs:263-264` (`canonicalize(root)` per request inside `check_on_disk`), `serve_static.rs:353-359` (one `spawn_blocking` per request under the default `Dotfiles::Ignore`) | Confirmed for the root. The review's second idea, one `canonicalize` of the target instead of the walk per segment, is rejected: the walk is what refuses symlinks and junctions inside the folder with `follow_symlinks(false)` and finds 8.3 aliases and trailing dots on Windows (first security audit, static-response; `serve_static_short_names_do_not_bypass_dotfiles`, `serve_static_follow_symlinks_false_confines_to_root`) |
| — | Reading one cookie parses them all | `request.rs:231-243` (`cookie(name)` = `cookies().remove(name)`: a `Vec<String>`, a joined `String`, the whole map) | Confirmed |
| — | IP addresses as strings | `request.rs:193-199` (`socket_ip()` formats the address), `request.rs:207` (`hostname()` passes it to `trusts(&str)`, which parses it back, `proxy_addr.rs:85-91`) | Confirmed. `ip()` and `ips()` return `String` by API and keep doing so |
| — | `file_stream` zeroes a 64 KiB buffer per chunk | `response.rs:33` (`vec![0u8; want]` per read) | Confirmed |
| — | Compression runs inline on the worker | `compression/body.rs:181-195` (one 64 KiB slice per poll; brotli at the configured quality) | Needs a measurement: Decision 7 records the time of one step; a follow-up moves it off the worker only if the number asks for it |
| — | ETag SHA-1 on every JSON response | `express.rs:320-330` | Confirmed; parity with the original's default. Documented in `docs/http/performance.md` with `EtagMode::Disabled` |
| 6b | The typed result is copied before it is serialized | `__private.rs:285-293` | Fixed by `q-http-response-consistency` Decision 5 (lands before this change); this benchmark's "before" run already includes it |

## Goals / Non-Goals

**Goals:**

- A benchmark of the request and response paths that every later performance change can reuse.
- Work proportional to the body done once per request where the API allows it, each removal moved by a named number.
- Identical observable results: values, error messages and positions, headers, status codes.

**Non-Goals:**

- Changing the pipe API, the `Body` enum or the `Request` accessors' signatures.
- The multipart path, compression off the worker, the watchdog.

## Decisions

### 1. An HTTP benchmark, run before and after

`openspec/specs/bench` has `bench/synthetic` (Corvis against Node on four endpoints) and `bench/port-vs-original`; neither
has CORS, middleware, large or non-ASCII bodies, cookies or static files, and adding Corvis-only routes to the synthetic
app would break its cross-framework comparison. As the authentication and Socket.IO hot-path changes did for their
modules, this change adds a Corvis-only benchmark, `bench/http/`, with the same layout as `bench/auth/` if that one has
landed (copy its `run.sh` structure; no shared crate):

- Workspace member `corvis-http-bench` (`publish = false`, `corvis` with default features plus `compression` for the
  measurement of Decision 7). The binary serves one route per scenario on `127.0.0.1`, installs a counting global
  allocator and a request counter, and answers `GET /__bench/stats` with `{requests, allocations, bytes}`.
- `bench/http/run.sh`: builds with `cargo build --release -p corvis-http-bench` in the workspace target directory (none of
  its own), checks each scenario's status and body with `curl` first, and per scenario runs a warmup, reads the stats, runs
  `oha` closed-loop (`CONNS`, `DURATION`, `OHA_THREADS` as in `bench/run.sh`), reads the stats again, then repeats with the
  server started with `TOKIO_WORKER_THREADS=1`. It writes `bench/http/results/<label>.json` (ignored by git) and prints a
  Markdown table. `--label before` / `--label after`.

| Scenario | Request | What it exercises |
|---|---|---|
| `text-16k` | `GET /text`, 16 KiB text answer | the copy of text bodies in `finalize` |
| `json-get` | `GET /json`, a 2 KiB struct answer | serialization and the response path |
| `post-1k`, `post-16k` | `POST /echo-size` with a 1 KiB / 16 KiB JSON object (ASCII), `#[body] Value`, answers its key count | JSON parsing, the request |
| `post-16k-utf8` | same with half the strings non-ASCII (accents, CJK, an emoji) | the UTF-8 path with multi-byte characters |
| `post-16k-cors-mw` | `post-16k` with CORS enabled and one global no-op middleware | the two request clones |
| `cookie-read` | `GET /cookie` with a 4 KiB `Cookie` header of 40 cookies, the handler reads the 20th | `cookie(name)` |
| `static-4k` | `GET /static/a.txt` (4 KiB) with the default dotfiles policy | `check_on_disk` |
| `file-8m` | `GET /file` through `StreamableFile::from_path` of an 8 MiB file | `file_stream` |

The benchmark informs; nothing asserts its timings. What each decision must move (figures from the `after` run against
the `before` run on the same machine, recorded in "Implementation notes"):

| Decision | Scenario | Number it must move |
|---|---|---|
| 2 | `post-16k-cors-mw` | bytes allocated per request fall to at most those of `post-16k` plus 2 KiB (the two clones removed) |
| 2 | `post-16k` | bytes allocated per request do not rise |
| 3 | `post-16k`, `post-16k-utf8` | bytes allocated per request fall by at least twice the body length (the UTF-16 copy); requests per second on one worker do not fall |
| 3 | `post-1k` | allocations per request fall by at least the number of `true` / `false` / `null` literals of the body |
| 4 | `text-16k` | bytes allocated per request fall by at least 16 KiB |
| 4 | `json-get` | allocations per request fall by at least 2 (the type `String` built by `finalize` and the `X-Powered-By` header value built per request) |
| 5 | `cookie-read` | allocations per request fall by at least 40 (one map entry per cookie) |
| 5 | `file-8m` | requests per second do not fall; bytes allocated per request do not rise |
| 6 | `static-4k` | requests per second on one worker rise by at least 5 %; if not, Decision 6 is reverted and the result recorded |

Alternative considered: Criterion microbenchmarks of the parser and of `Request::clone`. Rejected as the first measurement:
the costs at stake are allocations and copies per request under load; microbenchmarks can be added later for regressions.

### 2. A request that clones in constant time

- `Request` holds `headers: Arc<HeaderMap>`, `body: Arc<Value>` and `query: Arc<Map<String, Value>>`. `headers()`,
  `body()`, `query()` keep returning references. `headers_mut()` and `set_body()` use `Arc::make_mut` / replacement, so a
  clone that is changed stops sharing: clones stay independent, as today. `params` stays a `HashMap` (a few short strings,
  filled after routing). `raw_body` is already `Bytes`. The rustdoc says that a clone shares the parsed parts and the
  extensions.
- The router's clones (`req_for_cors`, `router/mod.rs:589`; `req_for_errors`, `router/mod.rs:624`) become a few reference
  counts plus `params`. Removing the CORS clone altogether (computing the CORS headers before the request moves into the
  pipeline) and sharing the route path (`CompiledRoute::path` as `Arc<str>`) belong to `q-core-request-hot-path`
  (Decisions 2 and 4 there), which owns what the router does per request and lands after this change; they are not done
  twice.
- `body_as::<T>()` deserializes with `T::deserialize(&*self.body)`; `user_as::<T>()` deserializes from the stored user under
  the read lock instead of cloning it. Both keep their error text: `serde_json`'s errors from `&Value` and from `Value` are
  produced by the same visitors; a unit test compares the messages of both forms for a table of mismatches (missing field,
  wrong type, unknown variant, wrong length) so a client never sees a different `400` message.
- `extract_param` keeps cloning the body for `#[body]` (pipes own their input); the copy is documented next to it.

How it composes with the other two changes on `Request`:

- e3 (`Request::insert`/`get`/`remove` tolerate a poisoned lock; `Request::typed_user`): independent fields; this change
  rebases on it.
- e2 (`Arc<Request>` for the Socket.IO handshake; `Request::for_engine_poll` builds a request with an empty query and
  body): still useful after this change, since a shared `Arc<Request>` also avoids copying `params` and the extensions
  handle; whichever lands second builds the `Arc` fields in `for_engine_poll`. e2's figures for the handshake copies are
  measured against today's `Request`, so e2 lands first when both are ready.

Tests (`tests/request_clone.rs`): a clone of a request with a 16 KiB body shares the body, the query and the headers
(pointer identity through `std::ptr::eq` on what `body()`, `query()`, `headers()` return); `headers_mut` on the clone
leaves the original's headers unchanged and makes them stop sharing; `set_body` likewise; a counting-allocator test: a
request with CORS on and one middleware allocates no copy of its body during dispatch (bytes allocated by `dispatch` for a
16 KiB body below 4 KiB beyond those of the same request without CORS and middleware).

Alternative considered: share the whole request (`Arc<Request>` everywhere). Rejected: `ExecutionContext::new`, the
middleware chain and `execute_route` take `Request` by value and set `params` and `route_path` on it; sharing the parts
keeps every signature.

How it composes with `q-core-request-hot-path` (the core review's hot-path change, which states that it is written against
the contract "cloning a `Request` is O(1)" delivered here): that change adds an in-process benchmark (`bench/pipeline/`)
that counts allocations per request through the test client's path, and owns CORS, middleware matching, the router's
lookups and the route path. The two benchmarks measure different layers (this one drives sockets with `oha`); each change
names its numbers in its own benchmark.

### 3. JSON parsed on UTF-8, UTF-16 only for errors

- `json_parse` becomes generic over its input: a `Source` trait with two implementations, `Utf8<'a>(&'a [u8])` and
  `Utf16<'a>(&'a [u16])`, giving the parser the next code unit, a peek, the position and a way to read a run of string
  characters. The grammar, the depth bound, the duplicate-key rule (the last value wins at the first key's position) and
  the treatment of lone surrogate escapes (`\uD800` is written as U+FFFD, as today) are shared code.
- `body_parser`: for `utf-8` / `utf8` (and no charset), the body after an optional UTF-8 BOM is validated with
  `std::str::from_utf8` (no copy) and parsed on `Utf8`. If validation fails, or parsing fails for any reason, the body is
  decoded to UTF-16 exactly as today and parsed on `Utf16`, whose error (message and UTF-16 position) is the one returned.
  A body that the UTF-8 path accepts is never parsed twice. `utf-16le`, `utf-16be`, `latin1` keep the UTF-16 path.
- Strings: the UTF-8 path copies runs without escapes straight from the input into the `String`; escapes are decoded as
  today. Literals compare against byte strings (no `Vec<u16>`). Objects: `JsObject` keeps a linear list of keys and builds
  its `HashMap` index only past 8 keys.
- `json_parse::parse(&str)` (public, used by `tests/json_parse.rs`) uses the same two-step rule.

Why the two paths cannot disagree on a valid body: both run the same state machine over the same code points; UTF-16
positions only appear in messages, which always come from the UTF-16 path. A property test checks it rather than this
argument.

Tests: `tests/json_parse.rs` passes unchanged (every recorded case, through `parse`); a property test in `fuzz_tests.rs`,
`json_paths_agree`: for any input from `json_soup()` plus inputs with non-ASCII characters, surrogate escapes, invalid UTF-8
bytes and a BOM, the result of `body_parser` equals the result of the UTF-16 path alone (value or error message);
`json_parse_is_total`, `json_parse_accepts_what_serde_accepts`, `json_parse_round_trips`, `json_depth_is_bounded` pass
unchanged; a unit test parses an object of 20 keys with duplicates (index built) and one of 3 (no index) with the same
results as today.

Alternative considered: parse with `serde_json` and translate its errors. Rejected: `serde_json` accepts and refuses other
inputs than V8 (`__proto__` handling aside, it reports line and column in bytes), so the messages could not be rebuilt.

### 4. Finishing a response without copies

- `finalize` takes the body with `mem::replace(&mut resp.body, Body::Empty)`: a text or HTML body becomes `Bytes` without a
  copy (`Bytes::from(String)` reuses the allocation); JSON is written with `serde_json::to_vec` once, as today.
- The sent body is stored as `Body::Bytes { data, content_type: String::new() }`: the header is already set by `finalize`,
  and `into_hyper` skips an empty type (it already does, `server.rs:333`). `into_hyper` builds the type as
  `HeaderValue::from_static` for its fixed JSON and text types and parses only the type of a raw body.
- `X-Powered-By`: `AppShared` keeps the validated `Option<HeaderValue>` computed at `init` from `HttpSettings::x_powered_by`
  (public field, unchanged), and `finalize` receives it with the settings. `HttpSettings` gains no field (adding a private
  one would break struct literals of applications).

Tests: the existing response suites (`express_parity.rs`, `typed_responses.rs`, `empty_results.rs`, `res.rs` with
`fixtures/res_express.json`) pass unchanged; a unit test that a 16 KiB text body reaches the hyper body with the same
pointer it had in `Body::Text`.

### 5. Small per-request costs

- `Request::cookie(name)`: `scan_cookies` over each `Cookie` header value in turn, stopping at the first match (the first
  occurrence wins, as `cookie.parse`), decoding only that value; no joined string, no map. `cookies()` keeps building the
  map. The `cookie_values` of e3 keeps its own rule (duplicates refused).
- Trust proxy: `TrustProxy::trusts_ip(IpAddr, hop)` (crate-private) used by `hostname()` and the proxy-address walk with
  the socket's `IpAddr`; `trusts(&str, hop)` (public) parses and calls it. `ip()` / `ips()` still return `String`.
- `file_stream`: `tokio_util::io::ReaderStream::with_capacity(file.take(len), 64 KiB)` (tokio-util is a direct dependency
  after `q-http-server-robustness`; `io` feature added) or an equivalent `BytesMut` with `read_buf`; the `UnexpectedEof` on a
  file that shrinks is kept (a wrapper checks the bytes read against `len`).

Tests: `cookies_first_occurrence_wins` and the cookie parsing tests pass unchanged, plus a unit test that `cookie(name)`
with 40 cookies and the name at position 20, and with the name twice, returns what `cookies().remove(name)` returns;
`tests/trust_proxy.rs` and `trust_proxy_is_total_and_bounded` pass unchanged; `streamable_file_from_path_streams` and
`serve_static_ranges_are_exact` pass unchanged, plus a test that a file truncated while it is streamed ends with an error.

### 6. The served folder canonicalized once

`ServeStatic` canonicalizes its root when it is built and keeps it (`Option<PathBuf>`, `None` if the folder does not exist
yet). `check_on_disk` uses the cached form; when it is `None`, or when the target's canonical path does not start with it
(the folder was replaced or moved, for example a deploy that swaps a symlinked release folder), it canonicalizes the root
again, updates the cache and decides with the fresh value. The walk per segment is unchanged.

Gate: if the `static-4k` figure of Decision 1 does not rise by 5 %, this decision is reverted and the measurement recorded.

Tests: the `serve_static_*` tests of `static_response_security.rs` and `tests/fuzz_static.rs` pass unchanged; a test that
replaces the served folder (rename the folder and create a new one at the same path) while the application runs still
serves the new folder's file and still refuses a file outside it.

### 7. Measured, not changed

- Compression: `corvis-http-bench compress-step` encodes 64 KiB slices of a JSON and a text sample with the default
  `CompressionOptions` (gzip level and brotli quality as configured) and prints the median and p99 time per slice for each
  coding. If the p99 exceeds 1 ms, a follow-up issue proposes encoding off the worker (`block_in_place` or a blocking pool);
  either way the figure goes to Implementation notes.
- ETag: `docs/http/performance.md` says that the default weak ETag hashes every `res.send()` body with SHA-1, as the
  original does, and that `EtagMode::Disabled` turns it off.

## Security invariants that must hold

| Invariant | Source | Decisions near it | Pinned by |
|---|---|---|---|
| JSON parsing is total, nesting bounded, no stack exhaustion | request-parsing spec; first audit http-input #1 | 3 | `deeply_nested_json_is_rejected_and_the_server_survives`, `json_parse_is_total`, `json_depth_is_bounded`, `parse_body_is_total` |
| JSON messages and positions identical to V8 | e0 parsing work | 3 | `json_parse_matches_v8` (1,000+ recorded cases), the new `json_paths_agree` |
| A body over the parser limit is refused before it is read | first audit | 3 (the parser), none on reading | `json_over_the_parser_limit_is_rejected_before_reading` |
| Clones share the extensions (a guard's user reaches the handler); the request's `Debug` is the redacted URL | request docs; e13 redaction | 2 | `tests/redact_allocations.rs`, the auth suites, `redacted_urls_never_hold_a_marked_value` |
| Cookies read like `cookie.parse`: first occurrence wins; a duplicated token cookie is no token | first audit http-input #6; e3 | 5 | `cookies_first_occurrence_wins`, `a_jwt_cookie_sent_twice_is_no_token` |
| `X-Forwarded-For` ignored unless `trust proxy`; trust rules exact | first audit http-input #4 | 5 | `forwarded_for_is_ignored_by_default`, `tests/trust_proxy.rs`, `trust_proxy_is_total_and_bounded` |
| Static files never leave the folder; dotfiles policy; symlinks refused with `follow_symlinks(false)`; 8.3 aliases and streams on Windows | first audit static-response; static-files spec | 6 | `serve_static_short_names_do_not_bypass_dotfiles`, `serve_static_rejects_alternate_data_streams`, `serve_static_follow_symlinks_false_confines_to_root`, `tests/fuzz_static.rs` |
| Files are streamed, never held whole; ranges exact | first audit static-response #3 | 5 | `streamable_file_from_path_streams`, `serve_static_ranges_are_exact`, `bodies_are_streamed_from_disk` |
| CORS answers as configured; a reflected origin with credentials warns | e13 CORS | 2 (the CORS clone becomes cheap; its logic is untouched) | the CORS tests of `tests/app.rs`, `tests/cors_startup_warning.rs` |

## Behavior visible to applications

| What | Before | After | Consequence |
|---|---|---|---|
| Static folder replaced while the server runs | Re-resolved on every request | Re-resolved on the first request whose target does not match the cached form | Same answers; one extra `canonicalize` on that request |
| `Request::clone` | Copies the parsed parts | Shares them until one side changes them | Cheaper; same semantics |
| Everything else | — | — | No intended change; the response suites, the JSON fixture and the parity check verify it |

## Order with the other changes

- After `e3-authentication-hot-path-and-consolidation` (same file, `http/request.rs`).
- Preferably after `e2-socket-io-hot-path-and-api-hygiene` (Decision 2); if it lands first, e2's task 4.2 builds the
  `Arc` fields in `Request::for_engine_poll`.
- After `q-http-response-consistency` (same response files; its removal of the copy of typed results is part of the
  "before" run here).
- Before `q-http-multipart-streaming`, which adds upload scenarios to this benchmark.
- Before `q-core-request-hot-path` (core review, same epic), which is written against "cloning a `Request` is O(1)" and
  owns the CORS clone, the route path and the router's lookups.

## Risks / Trade-offs

- Two paths in the JSON parser can drift → shared state machine, the recorded fixture, and the agreement property test
  that runs in CI's fuzz job with 100,000 cases before merge.
- Copy on write can surprise code that mutated a clone's headers expecting the original to see it → it never did (clones
  were copies); the semantics are unchanged.
- Caching the canonical root can serve stale decisions → any mismatch re-resolves; the walk per segment still runs.
- Benchmark noise on one machine → before and after on the same machine; the named numbers are allocations, which are
  deterministic, except the static rate, which has a revert rule.

## Open questions

1. Keep the `HashMap` index of `JsObject` past 8 keys or switch to `indexmap` (already a dependency)? Recommendation:
   `IndexMap` is not needed: the list keeps insertion order and the index only speeds up the duplicate check; keep the
   threshold rule, measure in `post-16k`.
2. Should `Request::query_as` avoid its clone too (a borrowed `Lenient`)? Recommendation: no; the query is bounded by the
   URI, and a borrowed deserializer is a refactor of `common::lenient` for no measured gain.
3. Decision 6 with a cached root: should a folder that did not exist at startup be cached when it appears? Recommendation:
   yes, on the first request that finds it.

## Review notes

These settle the open questions and override the design where they differ.

1. `JsObject` keeps a list plus a `HashMap` index past 8 keys; no new dependency.
2. `query_as` keeps its clone; its rustdoc says it deserializes a copy of the query.
3. A root that appears after startup is cached on the first request that canonicalizes it.
4. The security review covers the JSON parser's two paths (totality, depth, agreement), the copy-on-write `Request`, the
   cached root of static files, and `cookie(name)`.
