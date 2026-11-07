# Design

## Context

See proposal.md for motivation; the guarantees are in the five spec deltas. Current state:

- `crates/corvis/src/fuzz_tests.rs` (10 properties) and `crates/corvis/src/multer/fuzz.rs` (3 properties) exist,
  pass at 2,000 cases and, after one fix to the test's own assumption about empty boundaries, at 100,000 cases. They
  live inside the crate because the parsers are `pub(crate)` (`json_parse`, `body_parser::qs`, `busboy`,
  `append_field`, `router::to_matchit`, `proxy_addr::all_addrs`), and `busboy`/`append_field` are private modules of
  `multer`, so their properties sit inside `multer`.
- `crates/corvis/tests/static_response_security.rs` already builds a served folder with `.env`, `.git/config` and an
  `outside/secret.txt` next to it, plus an app with several `serve_static` mounts; the end-to-end property reuses
  that shape.
- `crates/corvis-orm/tests/security.rs` already creates an in-memory SQLite client with an `Account` model and
  `query_json(meta, action, args)`; the filter property reuses that shape.
- Only stable Rust is installed, on Windows.

## Goals / Non-Goals

**Goals:**
- Every requirement in the spec deltas is checked by at least one property.
- Default runs stay fast enough for every `cargo test --workspace`; longer runs are one environment variable away.
- A failing case found once is replayed forever.

**Non-Goals:**
- Coverage-guided fuzzing and sanitizers (`cargo fuzz`), planned for Linux CI (issue #6).
- Differential fuzzing against the Node originals (body-parser, qs, busboy) — a later step of the hardening plan.

## Decisions

1. **proptest on stable instead of `cargo fuzz`.** `cargo fuzz` needs nightly and libFuzzer, which is unreliable on
   Windows MSVC, and it would add another target dir. proptest runs inside `cargo test`, shrinks failures to minimal
   inputs and stores seeds. Alternative considered: `bolero` (can drive both), rejected for now to keep one dependency
   and one model; revisit with the CI work.
2. **Structured generators, not random bytes only.** Each generator mixes fragments of real inputs (multipart
   headers and delimiters, JSON tokens, qs brackets and indexes, traversal segments, SQL metacharacters) with random
   bytes, so most cases go deep into the parser instead of failing on the first byte.
3. **Case counts.** 2,000 cases per property by default; `PROPTEST_CASES` overrides it through one helper
   (`fuzz_tests::config`) shared by both in-crate modules. The long verification run uses 100,000.
4. **Static files end to end, through the application.** The property sends requests through the test client of a
   real application with `serve_static` mounts (default policy, `deny`, `allow`) rather than calling the internal
   resolver, so routing, percent-decoding and middleware order are covered too. The app and a tokio runtime are built
   once and reused across cases with proptest's `TestRunner`, because building an app per case would dominate the run
   time. Paths are generated from segments that are valid in a URI (`..`, `.`, `%2e%2e`, `%2f`, `%5c`, `%00`, `C:`,
   `ENV~1`, `::$DATA`, names inside and outside the folder), since the test client turns an invalid URI into `/`.
   The oracle is simple and strict: no response body contains the secret file's text or the dotfiles' text, except
   on the `allow` mount, where dotfiles inside the folder may be served; and no response has a 5xx status, because a
   hostile path is a client error (a 500 there usually means an I/O error from a malformed path leaking through).
5. **ORM filters on in-memory SQLite.** No server needed, so it runs everywhere. The table is seeded with a few known
   rows; generated filters combine real and unknown fields, operators (`equals`, `in`, `notIn`, `contains`,
   `startsWith`, `lt`, `not`, `AND`/`OR`/`NOT`, `mode`) and hostile values. Checks per case: no panic; any error is a
   validation or known-request error and its message does not contain `syntax error`; for `email: { equals: s }`
   every returned row has email `s`. After the run, the table is compared with the seed. Postgres is not fuzzed here:
   the SQL builder is shared, and the Postgres security tests need a server.
6. **Seeds are committed.** `proptest-regressions/` files go into version control so every failure becomes a
   permanent regression case.

## Risks / Trade-offs

- [Random testing can miss deep bugs] → structured generators, long runs before releases, and coverage-guided
  fuzzing in Linux CI later.
- [Flaky failures from a shared runtime or app state] → the static app is read-only and the ORM checks only use
  read queries after seeding; any write found in a filter path is itself a finding.
- [Test time grows] → default case counts keep the added time around 10 s; heavy runs are opt-in.
- [A property may expose a real bug] → small local fixes go in this change with a regression test; anything larger
  becomes its own issue in the right epic and the property is marked with a reference to it.
