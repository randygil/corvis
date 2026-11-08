# Proposal

Implements [randygil/corvis#2](https://github.com/randygil/corvis/issues/2) (epic [#1](https://github.com/randygil/corvis/issues/1)).

## Why

The security audit fixed 70 findings with example-based regression tests, but hostile clients do not send examples:
they send whatever breaks the parser. Corvis needs its robustness guarantees stated as behavior and checked with
generated input, so that a later change cannot silently reopen a crash, a memory blow-up, a file read outside the
served folder or an injectable query. Part of this already exists (13 properties, uncommitted); this change finishes
it and makes the guarantees part of the specs.

## What Changes

- Commit the existing property-based fuzzing of the request parsers: `JSON.parse` compatibility, `qs`, the body
  parser, multipart (busboy) and field-name nesting, JWT verification, route paths and trust proxy (13 properties,
  2,000 cases each by default, `PROPTEST_CASES` to run longer).
- Keep the multipart behavior the 100,000-case run surfaced: an empty boundary (`boundary=""`) is accepted, exactly
  like the original busboy, and parsing with it stays total and bounded.
- New: end-to-end fuzzing of static file serving through a running application (generated request paths against a
  served folder with a secret file next to it).
- New: fuzzing of corvis-orm `where` filters on in-memory SQLite (generated hostile filters on a seeded table).
- A long run of every property (`PROPTEST_CASES=100000`) as part of verification, with any failing seed kept in
  `proptest-regressions/` so it is replayed on every later run.

Out of scope:
- Coverage-guided fuzzing with `cargo fuzz` (needs nightly; planned for CI on Linux, issue #6).
- Fixing behavior the new properties might expose beyond small, local fixes: anything larger becomes its own issue.
- Fuzzing the WebSocket frame parser and the CLI.

## Capabilities

### New Capabilities

- `http/request-parsing`: hostile request data (JSON, urlencoded, query strings, multipart) never crashes the server
  and produces output proportional to the input.
- `http/static-files`: static file serving never exposes files outside the served folder and honors the dotfiles
  policy, whatever the request path.
- `http/trust-proxy`: client address resolution from `X-Forwarded-For` is total and can only add trusted hops.
- `auth/jwt`: token verification rejects forged, malformed and tampered tokens.
- `orm/query-filters`: query filters are always parameterized; hostile filters yield validation errors or results,
  never SQL syntax errors, crashes or wider results.

### Modified Capabilities

None (no specs exist yet).

## Impact

- `crates/corvis`: test-only modules `src/fuzz_tests.rs` and `src/multer/fuzz.rs`, a new integration test for static
  files, `proptest-regressions/` seeds.
- `crates/corvis-orm`: a new integration test for filters.
- Dev-dependency `proptest` in both crates (no runtime dependency change).
- `CONTRIBUTING.md` (new): how to build, test and run the fuzzing longer.
- Test time: about 10 s more for `cargo test --workspace` at the default case count.
