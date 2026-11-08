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
