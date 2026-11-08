# Design

## Context

See proposal.md for motivation; the guarantees are in the `docs/readme` spec delta. Current state, verified in the code:

- `README.md` has 1,518 lines in Spanish. Top-level sections: hello-world (lines 1-62), table of contents (66),
  repository layout (79), CLI quick start (99), "Equivalencias para quien viene de Node.js" (121-227), guide (229-935:
  modules, DTO validation, guards, events, schedule, testing, WebSocket gateways, `HttpAdapterHost`, CORS/ETag, body
  parser and router, `@Res()`, multer, static files), zod-style schemas (936), ORM (1116), OpenAPI (1169), performance
  (1231), security (1261-1491), known limitations (1492), license (1514).
- `crates/corvis/Cargo.toml:9` has `readme = "../../README.md"`, so the root README is also the crate's README.
  Nothing includes it with `include_str!`, so its code blocks are never compiled (`git grep 'cfg(doctest)'` finds
  nothing).
- Broken examples the docs review found, checked against the code:
  - `README.md:131` says "`#[global]` encima de `#[module]`" (above). `#[global]` is not an attribute macro of its
    own (`crates/corvis-macros/src/lib.rs` exports no `global`); `#[module]` removes it from the item's attributes
    (`crates/corvis-macros/src/module.rs:59-66`), which only works when it is written below `#[module]`.
    `#[module(global)]` and `#[module(global = true)]` are also parsed (`module.rs:39-45`). `docs/modules.md:169-171`
    already documents the correct position.
  - `README.md:218` lists `ConfigModule::register_as`. The function is `corvis::config::register_as`
    (`crates/corvis/src/config.rs:30`); it returns a `ConfigFactory`, consumed by `ConfigModule::for_feature`
    (`config.rs:288`) or `ConfigModuleOptions::load` (`config.rs:51`). It is not in the prelude.
  - `README.md:225` shows `ServeStaticModule::for_root(ServeStaticOptions {...})`; the signature is
    `for_root(options: Vec<ServeStaticOptions>)` (`crates/corvis/src/modules/serve_static.rs:123`) and options are
    built with `ServeStaticOptions::new(root_path)` plus builder methods (`serve_static.rs:37-67`).
  - `README.md:344` uses `#[on_event("cat.*")]` with no emitter options, and `README.md:220` says "(con comodines)".
    `EventEmitterOptions::default()` has `wildcard: false` (`crates/corvis/src/modules/event_emitter.rs:37-40`), and
    `matches` returns `false` for a non-identical pattern when wildcards are off (`event_emitter.rs:71-76`).
    `crates/corvis/tests/modules.rs:100-112` is a working module that enables wildcards and calls `for_root(vec![...])`.
  - Brand mentions: `README.md:919` (`@nestjs/serve-static`), `1369` ("Nest 11"), `1396` (`@nestjs/jwt`), `1429`
    ("en Nest"). Words in the mapping tables such as `@Global()` or `ConfigModule.forRoot` are API names of the Node
    model, not banned words; they stay.
- `README.md:111` says `corvis build` runs `cargo build --release`; `crates/corvis-cli/src/main.rs:340-341` adds
  `--release` only with `--release` or `--prod`. That contradiction is issue #20.
- The docs site is published at `https://corvis.randygil.com` (`docs/README.md:6`), English at the root and Spanish
  under `/es/`, with pages for every README topic (`docs/modules.md`, `docs/application/events.md`,
  `docs/recipes/serve-static.md`, `docs/http/file-upload.md`, `docs/security/*`, `docs/data/corvis-orm.md`,
  `docs/openapi/*`, `docs/http/performance.md`, ...).

## Goals / Non-Goals

**Goals:**
- An English README a newcomer can read in a few minutes and that sends them to the right docs page.
- No example in it that fails to compile or silently does nothing.
- A guard so README code cannot drift from the API unnoticed.

**Non-Goals:**
- Changing framework behavior (the README follows the code, including today's `corvis build` default).
- A Spanish README; packaging metadata for crates.io.

## Decisions

1. **Shorter landing page instead of a full translation.** The README keeps: tagline and hello-world with its log
   output, links to the docs site (both languages), install and CLI quick start, repository layout, the "coming from
   Node.js" mapping tables, one short example per main feature (modules and providers, DTO validation, guards,
   events and scheduling, testing, WebSocket gateway, ORM, OpenAPI), the performance table, a security summary of
   about ten bullets, known limitations and license. The long reference sections (CORS/ETag defaults, body parser and
   router, `@Res()`, multer tables, static file options, zod equivalences, the 230-line security list) become one
   paragraph each with a link. Alternative considered: translate all 1,518 lines; rejected because the same content
   now lives in 173 docs pages in two languages, and three copies drift (the broken examples above are that drift).
2. **Nothing is lost.** Before a section is shortened, each of its facts is checked against the docs page it will
   link to. Facts missing from the docs are added to that page in English and Spanish in this change (expected to be
   small, mostly security bullets). If a fact needs a new page, it becomes an issue in the docs epic and the README
   keeps the paragraph meanwhile.
3. **Doctest harness.** Add to `crates/corvis/src/lib.rs`
   `#[cfg(doctest)] #[doc = include_str!("../../README.md")] pub struct ReadmeDoctests;` so that
   `cargo test --doc -p corvis` compiles the README. The hello-world and the global module, `register_as`,
   events-with-wildcards and `ServeStaticModule` examples are complete compiled blocks (`no_run` where they would
   start a server). Blocks that need other crates (`corvis-orm`, `corvis-swagger`) or are long fragments are tagged
   `rust,ignore`; `text`, `bash` and `toml` blocks are not Rust. Alternative considered: making the README the crate's
   top-level docs (`#![doc = include_str!(...)]`); rejected because the docs.rs front page should stay API-oriented
   and the README's relative links break there. Alternative considered: a separate test crate; heavier for the same
   result.
4. **Correct forms of the four examples.**
   - Global module: `#[module(providers = [...], exports = [...])]` followed by `#[global]` on the next line; the
     table also mentions `#[module(global, ...)]`.
   - Configuration: `use corvis::config::register_as;` and
     `ConfigModule::for_feature(register_as("database", || json!({ ... })))`, in the table and in an example.
   - Static files: `ServeStaticModule::for_root(vec![ServeStaticOptions::new("public").serve_root("/static")])`.
   - Events: the module imports
     `EventEmitterModule::for_root(EventEmitterOptions { wildcard: true, ..Default::default() })`, and a comment says
     that without it `"cat.*"` only matches an event literally named `cat.*`.
5. **Brand mentions are rephrased by behavior.** "As in `@nestjs/jwt`, a non-empty `secret` wins" becomes "a non-empty
   `secret` takes priority over the keys"; the SSE framing note describes the framing itself. The mapping tables keep
   Node API names (`@Global()`, `ConfigModule.forRoot`) because they are what migrating users search for.
6. **Absolute links to the published site.** Docs links use `https://corvis.randygil.com/<slug>` so they work on
   GitHub and crates.io alike; links to repository files (`LICENSE`, `THIRD_PARTY_NOTICES.md`, `bench/results.md`)
   stay relative.

## Risks / Trade-offs

- [Readers relied on the long README as a single-page reference] → every shortened section links to a page with the
  same content, and the docs site has search.
- [The doctest harness adds compile time to `cargo test --workspace`] → a handful of small doctests; heavy blocks are
  `ignore`.
- [The README states a `corvis build` behavior that #20 may change] → whichever change lands second updates the
  README line.
- [Docs links break if pages are renamed] → `npm run docs:check` covers only internal links; a task checks every
  README docs link against an existing page slug.

## Open questions

- Confirm the shorter landing page (Decision 1) instead of a full translation of every section.
- Confirm that no `README.es.md` is wanted (the Spanish docs at `/es/` replace it).
- Should the README state a minimum supported Rust version? The workspace declares no `rust-version`; left to #10.

## Review notes (2026-11-08)

- The short English README that links to the published site is accepted; no `README.es.md` (the Spanish reader has
  `/es/` on the site).
- Minimum Rust version: left to #10 (crate packaging).
- The README's brand mentions are removed here; #22 runs after this change and its naming check must pass on the new
  README.

## Implementation notes

- **Doctest path.** The design wrote `include_str!("../../README.md")`, but `include_str!` resolves relative to
  `crates/corvis/src/lib.rs`, so the harness uses `"../../../README.md"` (`Cargo.toml`'s `readme` is relative to the
  crate directory, hence the difference). The item only exists under `cfg(doctest)`, so `cargo package` and docs.rs
  builds never read that path.
- **Compiled blocks.** Five README blocks are compiled (`rust,no_run`): the hello-world, the global module example,
  configuration with `register_as` + `ConfigModule::for_feature`, events with `wildcard: true` plus a cron handler,
  and static files (`ServeStaticModule::for_root(vec![...])` plus the `serve_static(..).mount(..)` middleware). Each
  has its own `#[corvis::main] async fn main()`, so the items stay at module level (rustdoc would otherwise wrap them
  in a function) and no hidden `# ` lines are needed (GitHub would show them). Their forms are copied from code that
  is already tested (`crates/corvis/tests/modules.rs`, `tests/app.rs`, `tests/express_parity.rs`) or read from the
  macros (`corvis-macros/src/module.rs` for `#[global]`). Every other Rust block is `rust,ignore` (fragments, or they
  need `corvis-orm`/`corvis-swagger`). Cargo could not be run in this session, so no compiled block has been built
  yet (see the unticked tasks).
- **Sections kept vs. linked.** Kept in the README: hello-world, CLI quick start, repository layout (no docs page lists
  the crates and folders), the five "coming from Node.js" tables, one short example per feature, a short HTTP
  behavior list, ORM and OpenAPI summaries, the performance table, a ten-bullet security summary, known limitations
  and license. Replaced by links: the long CORS/ETag, body parser/router, `#[res]`, multer, static files, zod and
  security sections, and the WebSocket/`ws`, multer, zod and OpenAPI mapping tables (the docs carry them in
  `websockets/*`, `http/file-upload`, `application/validation` and `openapi/decorators`).
- **Facts moved into the docs (EN + ES).** See the appendix. Most README content was already documented. The largest
  addition is in `application/validation`: the zod 4 and Node.js-integration equivalence tables and the sections
  "differences specific to Rust", "behaviors inherited from zod" and "how fidelity is tested", which existed only in
  the README (five new `###` headings in each language). `npm run docs:check` passes (173 + 173 pages).
- **Benchmark text.** Every number in the table matches `bench/results.md`. The all-cores ratio said "8–12"; it is
  now "7.7–11.7" (7.67x–11.72x in the ratio table). Memory said "~15–35 times less"; it is now "about 15–40 times
  less" (Express 165.7–205.3 MB and Fastify up to 344.1 MB against Corvis's 8.6–11.4 MB). The "110–120k req/s"
  ceiling became "about the same rate as a bare hyper server", because `bench/results.md` says ~100–110k while the
  measured runs reach 119k–126k. The POST variance between runs (57k–100k req/s) is kept from the old README;
  `bench/results.md` records one run (56,972 req/s), so its upper bound cannot be checked there. `bench/results.md`
  itself (Spanish, original framework name in its title) is generated by `bench/run.sh` and left to #22.
- **`corvis build`.** The README states today's behavior: `cargo build` (debug), with `--release` or `--prod` for an
  optimized build (`crates/corvis-cli/src/main.rs:339-341`). If #20 changes the default, that change updates the line.
- **Corrections found while moving facts.** The old README said out-of-range `take` and `limit` are ORM errors; the
  code only rejects a negative `limit` (`P2019`), so only that case was documented. The claim that the facten port
  "serves the same `/reference` byte for byte" could not be verified from this repository and was neither kept nor
  moved. The old zod section's JSON-depth link pointed to an unrelated security subsection.
- **Link check (task 4.1).** A one-off Node script (kept outside the repository) read every link in the README prose
  and, after `npm run docs:check`, resolved each `https://corvis.randygil.com/<slug>#anchor` against
  `docs/.vitepress/dist/<slug>.html` and its `id=` attributes; it checked relative links as files and `#` links
  against the README headings (GitHub slugs). Result: 68 links, 0 missing. A deliberately wrong anchor was reported,
  so the check is not vacuous.
- **Parity (task 5.6).** The only Rust change is a `#[cfg(doctest)]` item that no normal build compiles, so the
  framework runtime is unchanged and the corvis-parity check of the three port apps is not required for this change.

## Appendix: README inventory (task 1.1)

| Old README lines | Topic | Docs page (English slug) | Facts added to the docs (EN + ES) |
|---|---|---|---|
| 1-62 | Hello-world and log output | `first-steps` | kept in README |
| 79-97 | Repository layout | none | kept in README (no page lists the crates) |
| 99-119 | CLI quick start | `cli/usages`, `first-steps` | `--corvis-version` must be a Cargo version requirement; `--corvis-path` rejects quotes and control characters; `generate` applies the same name rule |
| 121-227 | "Coming from Node.js" tables | `migration-guide` | kept in README (translated, four entries corrected) |
| 231-267 | Modules and providers | `modules#encapsulation`, `modules#global-modules`, `providers`, `fundamentals/*` | none missing |
| 269-290 | DTO validation | `application/validation` | none missing |
| 292-327 | Guards and custom decorators | `guards`, `custom-decorators` | none missing |
| 329-415 | Events and `corvis::schedule` | `application/events#wildcards`, `application/task-scheduling` | `CronTime` methods; test file and the 48 × 9 × 7 comparison with `cron` 4.4 |
| 417-438 | Testing | `fundamentals/testing` | none missing |
| 440-534 | WebSocket gateways | `websockets/gateways`, `websockets/guards`, `websockets/pipes`, `websockets/interceptors` | none missing |
| 536-625 | `HttpAdapterHost` and `corvis::ws` | `websockets/adapter`, `faq/http-adapter` | none missing |
| 627-703 | CORS, Express default headers, `StreamableFile`, `ParseUUIDPipe` | `security/cors#behavior`, `controllers#responses`, `http/streaming-responses#streamablefile`, `pipes` | expose headers in preflight, `Vary` merging; ETag, charset, raw bodies, HEAD, `X-Powered-By`; in-memory `Content-Length`; `Response::bytes`; `ParseUUIDPipe` version and non-string messages |
| 705-746 | Body parser and Express router | `faq/raw-body`, `controllers`, `recipes/express-migration`, `http/cookies` | parser details (BOM, empty body, V8 messages, depth limits, 400/408/413/415, latin1 headers, errors through CORS and filters); `%XX` rule; query parser details; IPv6 `hostname` |
| 747-796 | `#[res]` / `#[res(passthrough)]` | `controllers#the-response-object-res` | initial status and headers, headers kept on error, `send`/`end` behavior, `redirect` negotiation, `cookie` validation, `get_response()` |
| 798-894 | File upload (multer) | `http/file-upload` | returned `UploadedFile` serialization, full error list, latin1 file names, `body_limit` details, 408, empty `raw_body()`, detected types, `buffer_from_value`, `attach` |
| 896-921 | Static files | `recipes/serve-static` | `.mount` matching, fallthrough, 301 body, headers and ETag format, 304/412/206/416, chunked reads, the module uses the middleware |
| 923-934 | `status_code()`, `http_error`, enhancer order | `faq/request-lifecycle#response`, `exception-filters#unexpected-errors` | `status_code()` is set before the route runs; catch-all filters see `http_error` as unknown |
| 936-1114 | `corvis::z` | `application/validation#schemas-with-corvisz` | zod 4 and integration equivalence tables, Rust differences, inherited behaviors, fidelity tests, versions, `format_error` |
| 1116-1167 | `corvis-orm` | `data/corvis-orm` | none missing |
| 1169-1229 | `corvis-swagger` | `openapi/introduction`, `openapi/decorators`, `openapi/other-features` | `setup` routes, `custom_swagger_ui_path` rules, Scalar page details and escaping |
| 1231-1259 | Performance | `faq/corvis-vs-fastify-express-hono#measured-performance` | kept in README |
| 1263-1291 | Security: HTTP input | `faq/raw-body`, `deployment#behind-a-reverse-proxy` | trust-proxy list formats and the right-to-left rule |
| 1293-1349 | Security: transport and uploads | `deployment#server-limits`, `http/file-upload` | `None` disables a timeout, accept queue, 32 KiB headers; field-name holes and nesting, Windows file-name rules, `preserve_path`, orphan files, `FileTypeValidator` caveats |
| 1351-1380 | Security: static files and responses | `recipes/serve-static#security`, `http/cookies#validation`, `controllers#redirects`, `http/server-sent-events`, `application/logger` | dotfiles against real names, NTFS streams, symlinks; `encodeurl` and the open-redirect warning |
| 1382-1434 | Security: auth, config, WebSockets | `application/configuration`, `security/authentication`, `middleware`, `security/rate-limiting`, `websockets/gateways`, `websockets/adapter`, `websockets/guards`, `guards` | JWT empty-key and `nbf`/claim-type errors; gateway panics and pings; `corvis::ws` buffer and close details; what global guards do not cover; a static file shadows a protected endpoint |
| 1436-1477 | Security: validation, Swagger, CLI, dependencies | `application/validation#security-limits`, `openapi/other-features`, `cli/usages`, `devtools/ci-cd-integration` | depth limit details, linear cost, regex step cap, `#[array_unique]`; RUSTSEC-2023-0071 ignore and its reason; `forbid(unsafe_code)` |
| 1479-1490 | Security: ORM | `data/corvis-orm#security` | TLS feature, `host:port` errors, `max_query_depth`, unique inputs, negative `limit`, `where` repeated in UPDATE/DELETE |
| 1492-1512 | Known limitations | `fundamentals/injection-scopes`, `faq/raw-body`, `http/file-upload`, `pipes`, `controllers`, `interceptors`, `middleware` | kept in README; none missing in the docs |
| 1514-1518 | License | none | kept in README |
