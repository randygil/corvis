# Tasks

Start after `q-http-server-robustness`, `e3-authentication-hot-path-and-consolidation` and, preferably,
`q-http-request-hot-path` are merged into `dev` (design.md "Order with the other changes"). Shared Cargo target directory
only. Test servers and the benchmark bind to `127.0.0.1`. No test sleeps to synchronize or compares timings: a slow engine
in a test waits on a signal the test sends. Behavior must not change except where design.md "Behavior visible to
applications" says so.

## 1. Reconcile and measure

- [ ] 1.1 Re-check every line of design.md "Findings as checked" against `dev` with the prerequisites merged (in particular
  `BodySource` after `q-http-server-robustness`); record moved lines in a new "Implementation notes" section of design.md
  before any other task
- [ ] 1.2 `crates/corvis/tests/multer_memory.rs` with the two scenarios of design.md Decision 6 written against today's
  code, asserting the bounds of this change; verify they fail and record the peak heap they measured
- [ ] 1.3 `bench/http`: scenarios `upload-disk-8m` and `upload-memory-1m` (routes, the multipart request body for `oha`,
  the status check); before run of the two scenarios (`--label before`); copy the rows into Implementation notes

## 2. Parser (Decision 3)

- [ ] 2.1 `multer/busboy.rs`: `Event::FileData`, `FileEnd` without data, no `file.chunks`; the bounded boundary search
  across chunks with a reusable buffer; `Finder` built once; verify the busboy unit tests adapted to `FileData`
  (`streaming_is_split_invariant`, `discarded_file_keeps_no_data`, `file_size_limit_is_inclusive_like_busboy`,
  `parses_fields_and_files`)
- [ ] 2.2 `multer/fuzz.rs` properties adapted (kept bytes from `FileData` and fields); the counting-allocator unit test of
  Decision 3; run `PROPTEST_CASES=100000 cargo test -p corvis --lib multer::fuzz` once and record it; verify the default run

## 3. Stream contract (Decisions 1 and 2)

- [ ] 3.1 `multer/storage.rs`: `FileStream`, the new `handle_file` signature, `collect_file`; `MemoryStorage` through
  `collect_file`; `DiskStorage` writing chunk by chunk, name checked before creating, partial file removed on any failure;
  trait rustdoc with the contract and a streaming example; verify `cargo build -p corvis --all-targets`
- [ ] 3.2 `multer/interceptors.rs`: the file in progress in `read_events` (channel of one chunk, the engine's future polled
  in every wait, early completion, error item on limit / abort / timeout / parse error, `remove_file` after an `Ok` for a
  failed stream); verify `cargo test -p corvis --test multer --test transport_security`
- [ ] 3.3 Tests for the scenarios of the spec requirements "Storage engines receive the file as a stream" and "A failed
  upload leaves no partial file" (slow engine paced by the test, engine doing other async work, engine that stops early,
  file limit and client abort mid-file with disk storage); verify they pass 5 runs in a row
- [ ] 3.4 The rustdoc of `MulterOptions::body_limit` (Review note 2) and of `DEFAULT_MULTIPART_BODY_LIMIT`; verify
  `cargo doc -p corvis --no-deps` without warnings

## 4. Buffers with their request (Decision 4)

- [ ] 4.1 `multer/buffer.rs`: `BufferStore` in the request extensions, index handles, the task-local scope;
  `router/mod.rs` sets the scope around the pipeline and the filters in `execute_route`; `common/parse_file.rs` and
  `multer/file.rs` unchanged in their calls; remove the registry and `Lease`; rustdoc of `buffer_to_value` and
  `buffer_from_value`; verify `--test multer` (`single_file_and_fields`, `returned_file_serializes_like_express`,
  `parse_file_pipe`)
- [ ] 4.2 Tests for the scenarios of the spec requirement "File buffers resolve only within their request" (two concurrent
  uploads, a foreign handle in a JSON body, the store freed after the request, a custom pipe and a filter resolving a
  buffer); verify they pass

## 5. Cleanup visible (Decision 5)

- [ ] 5.1 Rate-limited warning on a failed removal in `Cleanup::run`, after a failed read, and in `DiskStorage`; test with
  an engine whose `remove_file` fails (same response, one line); verify `files_are_removed_when_a_pipe_or_the_handler_fails`

## 6. After, docs and review

- [ ] 6.1 `tests/multer_memory.rs` passes with the bounds of Decision 6; after run of the two upload scenarios
  (`--label after`); rows and the numbers each had to move in Implementation notes
- [ ] 6.2 Docs: `docs/http/file-upload.md` (memory per upload, backpressure, partial files removed, buffer handles scoped
  to the request) and `docs/application/file-storage.md` (the new `StorageEngine` contract, a streaming engine, the
  migration with `collect_file`); translate in `docs/es/`; verify `npm run docs:check` in `docs/`
- [ ] 6.3 Security review with the `security-reviewer` agent: the stream contract (an engine that ignores errors or stops
  early), limits and timeouts while a file is in progress, partial files on every failure path, the parser's boundary
  search across chunks (fuzz with adversarial splits), the drain of a half-read body, the scope of buffer handles; repeat
  rounds until no open finding; record findings and fixes under "Security review" in design.md

## 7. Definition of done

- [ ] 7.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 7.2 `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo clippy -p corvis --no-default-features
  -- -D warnings`
- [ ] 7.3 `cargo test --workspace` passes
- [ ] 7.4 Per-feature checks: `cargo check -p corvis --tests --features <f>` for websockets, compression, tls,
  auth-local, auth-api-key, auth-session, auth-oauth2 and socket-io, with no warnings
- [ ] 7.5 Regression tests: one per spec scenario, as listed in groups 2-5; the tests of 1.2 were seen failing before the
  change
- [ ] 7.6 Docs pages updated in English and Spanish, then `npm run docs:check` in `docs/`
- [ ] 7.7 Framework runtime code changed (`multer/`, `router/mod.rs`): run the `corvis-parity` check (consi's evidence
  upload through `disk_storage` is covered) and verify the three port apps keep 0 apidiff differences
- [ ] 7.8 `cargo deny check` and `node scripts/check-naming.mjs` pass
- [ ] 7.9 The security review of 6.3 is closed; a "Verification" table is added to design.md

## Workflow follow-up

- Archive the change with `/opsx:archive` (the new `http/multipart-storage` capability and the `bench/http-hot-path`
  delta applied).
- Close the issue, tick it in epic #187 and move the card to Done (see the `corvis-task` skill).
- Put the default multipart body limit to the maintainer as its own decision (design.md Open question 2).
