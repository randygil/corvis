# Tasks

Start after #157 (`e0-write-deadline-for-stalled-clients`) is merged into `main`: this change edits the structures it
added to `crates/corvis/src/server.rs` (`GuardedBody` slicing, `BodyClock`, `watchdog`) and copies the raw h2c client
of `tests/send_timeout.rs`.

## 1. Failing tests

- [ ] 1.1 Create `crates/corvis/tests/response_memory.rs` with the raw h2c client of `tests/send_timeout.rs`
  (`:161-246`, copied), a raw HTTP/1 client, and routes: `/t/quarter` (256 KiB `String`), `/t/big` (1 MiB),
  `/t/huge` (8 MiB), `/t/small` (64 KiB), `/t/ok` (small JSON), `/t/counted` (256 KiB, increments a static
  `AtomicUsize` on each run); verify `cargo test -p corvis --test response_memory --no-run` builds
- [ ] 1.2 Add one test per scenario of `specs/http/response-memory/spec.md` (burst with window 0: at most four
  HEADERS and the rest `RST_STREAM` with code `0xb`; one response larger than the cap is sent; 100 concurrent 64 KiB
  requests with hyper's h2 client all answered; new streams wait while a stalled client holds the budget, observed
  with `/t/counted`; pipelined HTTP/1 runs one handler at a time; waiting request reset by the client; one
  progressing stream next to stalled ones; RST cycle of three rounds; disabled with `None`); verify with
  `cargo test -p corvis --test response_memory` that the cap tests fail and the legitimate-client tests pass
- [ ] 1.3 Add the scenarios of `specs/http/http2/spec.md` (600 streams reset after their HEADERS close the
  connection; 3000 completed plus 600 such resets do not) to `tests/http2_limits.rs`; verify the first fails on the
  current code and the second passes

## 2. Option and accounting

- [ ] 2.1 Add `max_pending_response_bytes: Option<usize>` (default `Some(8 * 1024 * 1024)`) and its setter to
  `ServerOptions`, with rustdoc: what counts, admission, reset with `ENHANCE_YOUR_CALM` over HTTP/2, one response
  always sent, HTTP/1 one response at a time, what is not counted, comparison with Node's `maxSessionMemory`; replace
  the "see #163" paragraph of the `send_timeout` rustdoc with a link to the new option; verify a unit test asserts the
  default and `cargo doc -p corvis --no-deps` has no broken links
- [ ] 2.2 Add `Budget { pending, running, warned }`, `ConnState::budget` and `ConnState::budget_freed` with pure
  methods `try_admit(share, cap) -> bool`, `finish_handler(charge, cap, http2) -> Keep | Reset` and
  `release(charge)` (design.md, Decisions 2-4); verify unit tests in `server.rs` for: admission from idle, the share
  rule, `pending == 0` always admits and always keeps, overshoot reset only over HTTP/2, responses within their share
  never reset
- [ ] 2.3 Record the uncompressed length in `finish_response` as a crate-private `PlainLen` extension before
  compression; give `GuardedBody` a `charge` (exact size or `PlainLen` for in-memory bodies, `rest.len()` for the
  others) added on creation and released on end or drop, with `budget_freed` notified; verify unit tests that a
  compressed 1 MiB body charges 1 MiB and a streamed body charges only its unsent `rest`

## 3. Admission and overshoot

- [ ] 3.1 In the `service_fn` closure of `serve`, wait for `try_admit` (on `budget_freed`) after the handler permit
  and before the second flood check; release the reservation and add the charge when `handle` returns; skip both when
  the option is `None`; verify the waiting, pipelined HTTP/1 and reset-while-waiting tests pass
- [ ] 3.2 Add `h2 = "0.4"` to `crates/corvis/Cargo.toml` and, on `Reset`, drop the response and return an error whose
  source chain holds `h2::Error::from(h2::Reason::ENHANCE_YOUR_CALM)`; log the first reset per connection; verify the
  burst, one-progressing-stream and RST-cycle tests pass and `Cargo.lock` gains no new package

## 4. Retention

- [ ] 4.1 Copy the HTTP/2 slices whose remainder is below `H2_TAIL` in `GuardedBody::poll_frame` (design.md,
  Decision 7); verify a unit test that the last 48 KiB of a 1 MiB body come in allocations outside the original
  (`as_ptr` range check) and the earlier slices do not
- [ ] 4.2 Set `http2().max_send_buf_size(H2_SEND_BUF)` (32 KiB) in `connection_builder`; measure a 64 MiB single-stream
  h2c download on loopback before and after (release build, five runs, median) and record it in design.md; if the drop
  is above 10 %, use 64 KiB; verify `tests/send_timeout.rs` (one-byte window, zero window) and `tests/http2_limits.rs`
  still pass

## 5. Rapid-reset accounting

- [ ] 5.1 Move the `Completed` record from `CancelWatch` to `GuardedBody` (completed on end, cancelled on drop before
  end), and record the overshoot reset as cancelled (design.md, Decision 6); verify the tests of 1.3 pass and the
  unit tests of the cancel window in `server.rs` still pass

## 6. Docs

- [ ] 6.1 `docs/http/performance.md`: a section "Response memory per connection (`max_pending_response_bytes`)" with
  what counts, admission, reset, HTTP/1, the per-connection bound with defaults (8 MiB or one response, plus about
  48 KiB per open HTTP/2 stream and the compression encoders), the global bound with `max_connections`, and the
  comparison with Node's `maxSessionMemory`; rewrite "What it does not bound" under `send_timeout` to point to it; same
  in `docs/es/http/performance.md`
- [ ] 6.2 `docs/security/http-parsing.md` (slow readers: memory now bounded per connection; streams reset after their
  response count against the rapid-reset limit) and `docs/deployment.md` (the option in the server options example);
  same in `docs/es/`
- [ ] 6.3 Run `npm run docs:check` in `docs/`; verify it passes

## 7. Definition of done

- [ ] 7.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 7.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] 7.3 `cargo test --workspace` passes, including the regression tests of 1.2 and 1.3
- [ ] 7.4 Docs updated in English and Spanish (`docs/` and `docs/es/`) and `npm run docs:check` passes in `docs/`
- [ ] 7.5 Framework runtime code changed: run the `corvis-parity` check and verify the three port apps keep 0 apidiff
  differences
- [ ] 7.6 Re-run `mixed`, `rstcycle` and `trickle` of `st157b/src/main.rs` (and `h1trickle`) against the branch with
  default options and record the peak RSS in design.md (expected: about 8 MiB above the baseline per connection
  instead of 103-114 MiB)

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #163, tick it in epic #1 and move its card to Done (see the `corvis-task` skill).
