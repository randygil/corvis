# Proposal

Part of epic [#187](https://github.com/randygil/corvis/issues/187) (code quality audit); implements the issue filed from
`quality/q-http-request-hot-path.issue.md`. Source: the engineering-quality review of the HTTP layer, the server and the
multipart module, read against `dev` at `d46758a`. Third of four changes (see `q-http-server-robustness/proposal.md`). It
lands after `openspec/changes/e3-authentication-hot-path-and-consolidation/` (which edits `http/request.rs`), preferably
after `openspec/changes/e2-socket-io-hot-path-and-api-hygiene/` (which shares the handshake `Request` behind an `Arc` and
adds a constructor for polling requests), and after `q-http-response-consistency` (which changes the same response code).

## Why

Every JSON request pays work proportional to its body several times over, and every response pays copies it does not
need:

- **The request is deep-copied to work around the borrow checker.** `Request` derives `Clone` over its headers, parsed
  query and parsed body (`crates/corvis/src/http/request.rs:14-41`, the doc comment says so). With CORS on, the router
  clones the whole request to apply CORS afterwards (`router/mod.rs:589`); with any middleware it clones it again for the
  error context (`router/mod.rs:624`); `body_as` clones the body to deserialize it (`request.rs:159`), `user_as` clones the
  user (`request.rs:313-315`).
- **JSON bodies go through UTF-16.** `body_parser::decode` turns the UTF-8 body into a `Vec<u16>` twice its size, copies
  it again when it starts with a BOM (`http/body_parser.rs:245-254`); the parser allocates a `Vec<u16>` per literal
  (`http/json_parse.rs:492-493`), one per string before its `String`, and a `HashMap` that clones every key of every object
  (`json_parse.rs:214-216`). The purpose is real: error messages identical to V8's, with UTF-16 positions. Only errors
  need it.
- **Finishing a response copies it.** `express::finalize` clones every text and HTML body (`http/express.rs:300`), turns
  the content type into a `String` (`express.rs:332`) that `into_hyper` may parse back (`server.rs:321-338`), and validates
  the `X-Powered-By` value on every request (`express.rs:290-295`).
- **Smaller costs on every request:** `Request::cookie(name)` builds the map of every cookie plus a joined copy of the
  header to read one (`request.rs:231-243`); `hostname()` and `ip()` format the socket address into a `String` to parse it
  back (`request.rs:193-199,207`); `file_stream` zeroes a fresh 64 KiB buffer per chunk (`http/response.rs:33`); static
  files canonicalize the served folder on every request (`http/serve_static.rs:263-264`).

There is no benchmark of these paths, so the first task group builds one, and every performance decision names the number
it must move.

## What Changes

- **Benchmark.** `bench/http/`: a Corvis-only, release-mode benchmark (`corvis-http-bench`) driven by `oha` on
  `127.0.0.1`, reporting requests per second on all workers and on one worker, latency percentiles, and the server's
  allocations and bytes allocated per request. Scenarios for text and JSON responses, JSON bodies of 1 KiB and 16 KiB
  (ASCII and not), the same with CORS and a middleware, a cookie read, a static file and a streamed file. Run before and
  after; the numbers go to design.md.
- **A request that clones in constant time.** The parsed body, the parsed query and the headers are held behind `Arc`
  (copy on write for `headers_mut` and `set_body`, so clones stay independent as today), so the router's clones for CORS
  and for middleware errors cost a few reference counts; `body_as` and `user_as` deserialize from a reference. Removing the
  CORS clone altogether and sharing the route path are `q-core-request-hot-path`'s (it owns the router's per-request work
  and lands after this change).
- **JSON parsed on UTF-8.** The parser runs over the UTF-8 bytes when the body is valid UTF-8 with the same grammar, the
  same depth bound and the same value semantics; on any error it parses again over UTF-16 to produce today's message and
  position. Other charsets keep the UTF-16 path. Literals need no allocation and small objects no hash map.
- **Fewer copies when finishing.** The text body is moved into the response bytes; a body whose type header is set
  carries no type string; `X-Powered-By` is prepared once per application.
- **Small per-request costs.** `cookie(name)` scans with an early exit; the trust-proxy checks take the socket's `IpAddr`;
  `file_stream` reads into a reused buffer; the served folder is canonicalized once, again only when the cached form stops
  matching.
- **Measured, not changed here:** the time of one compression step at the default settings, to decide whether a follow-up
  moves compression off the worker; the ETag cost is documented in the performance page.

## Capabilities

### New Capabilities

- `bench/http-hot-path`: the HTTP benchmark (scenarios, what it reports, where it builds, how results are recorded).

### Modified Capabilities

- `http/request-parsing`: added requirements that the JSON result does not depend on the parsing path and that a request
  shares its parsed parts with its clones.

## Impact

- New: `bench/http/{Cargo.toml,src/main.rs,run.sh,README.md}` (workspace member, `publish = false`),
  `crates/corvis/tests/request_clone.rs`.
- Edited: `crates/corvis/src/http/{request,body_parser,json_parse,express,response,serve_static,proxy_addr}.rs`,
  `crates/corvis/src/router/{mod,cors}.rs`, `crates/corvis/src/server.rs` (or `server/response.rs` after
  `q-http-server-robustness`), `crates/corvis/src/application.rs` (the prepared `X-Powered-By`), `Cargo.toml` (workspace
  member), `.gitignore` (`bench/http/results/`), `bench/README.md` (link), and in `docs/` and `docs/es/`:
  `http/performance.md`.
- Public API: nothing changes signature. `Request`'s fields are crate-private; `headers_mut` and `set_body` keep their
  signatures with copy-on-write semantics. The rustdoc of `Request` changes from "cloning copies" to "cloning shares".
- Behavior visible to applications and clients: none intended; design.md "Behavior visible to applications" lists the one
  edge (a static folder replaced while the server runs is re-resolved on the first mismatch).
- No new dependency.
- Framework runtime code changes: the corvis-parity check runs, and the JSON error fixture (`fixtures/v8_json_parse.json`)
  must pass unchanged.

## Out of scope

- The pipe API: pipes own the `Value` they transform (`PipeTransform::transform(Value, ..)`), so `#[body]` keeps one copy
  of the body for its pipes (design.md Decision 2).
- A JSON body kept as serialized bytes (rejected in `q-http-response-consistency` Decision 5).
- The watchdog's timer per response and compression on the worker: measured or recorded, changed in follow-ups only if the
  numbers ask for it.
- The multipart path (`q-http-multipart-streaming`).
