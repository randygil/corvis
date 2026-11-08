# Tasks

## 1. Reproduce

- [x] 1.1 Capture what the client reads when the test fails (bytes, error kind); verified with the compiled test
  binary looped 40 times (4 failures: `ConnectionAborted`, os error 10053, 0 bytes)

## 2. Server fix

- [x] 2.1 `TimedIo` owns `Option<TcpStream>` and the connection permit and leaves them in `ConnState` on drop;
  `finish_connection` runs `linger_close` (FIN, then bounded drain of 1 MiB / 2 s, 4 KiB buffer); verify
  `cargo build -p corvis` has no warnings
- [x] 2.2 No drain after `TimedIo` timeouts, refused h2c, hyper's own header timeout, upgraded connections and
  `SocketDestroyed` (design 2b); verify `silent_and_slow_header_connections_are_closed` and `h2c_can_be_disabled`
  still pass

## 3. Tests

- [x] 3.1 `early_error_response_survives_unread_input`: fails on the unfixed server on every run, passes with the fix
- [x] 3.2 `drain_after_an_early_error_response_is_bounded_in_bytes` and `_in_time`: an endless sender and a slow
  trickle are both cut off, and the permit is free by about 2 s
- [x] 3.3 `idle_keep_alive_connection_released_without_drain`, `slowloris_on_keep_alive_connection_released_without_drain`
  and `destroyed_upgrade_socket_is_closed_without_drain`: the permit is free within the timeout plus a small margin
  (each fails if the drain is applied)

## 4. Docs

- [x] 4.1 Docs pages: no documented behavior changes (see the docs check in 5.5); recorded here

## 5. Definition of done

- [x] 5.1 Builds with no warnings
- [x] 5.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 5.3 `cargo test --workspace` passes (375 passed, 0 failed)
- [x] 5.4 50 consecutive green runs of `cargo test -p corvis --test transport_security` on Windows (60 of 60 runs green on the compiled test binary)
- [x] 5.5 `npm run docs:check` in `docs/` passes
- [x] 5.6 Runtime code changed: the corvis-parity check is needed (the maintainer runs it); maintainer run on 82fd104: consi 832/832, facten 727/727, houseplan 598/598 (quiet-hours count), routes identical, databases equivalent. Security review: ship (both findings fixed and re-verified with the PoCs).

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #130, tick it in epic #1 and move its card to Done (see the `corvis-task` skill).
