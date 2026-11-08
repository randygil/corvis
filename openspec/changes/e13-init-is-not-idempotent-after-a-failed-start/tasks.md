# Tasks

## 1. Failure state in init()

- [ ] 1.1 In `crates/corvis/src/application.rs`, move the body of `init()` (`:365-420`) into a private
  `async fn init_steps(&mut self) -> crate::Result<()>`; add the private field `init_error: Option<String>` (set to
  `None` in `bootstrap`); make `init()` return the stored failure first, then `Ok(())` when `initialized`, otherwise
  run `init_steps`, setting `initialized` on success and `init_error` + `self.shared = None` on failure (design.md,
  Decision 2); verify the existing `cargo test -p corvis --test enhancer_order --test app` still pass
- [ ] 1.2 Add `pub struct InitFailedError` with `first_error()`, `Display` (message in design.md, Decision 3),
  `Debug` and `std::error::Error`; return it from the second and later calls; re-export it from
  `crates/corvis/src/lib.rs`; verify with the unit test of 2.1

## 2. Regression tests

- [ ] 2.1 Unit test in `application.rs` (`#[cfg(test)]`): an application with `Provider::app_guard`,
  `app_interceptor`, `app_pipe`, `app_filter` and an `on_module_init` that returns `Err`; call `init()` twice;
  assert the first error is the hook's, the second downcasts to `InitFailedError` and contains the hook's message,
  and each of `config.router.global_guards` / `global_interceptors` / `global_pipes` / `global_filters` holds one
  entry; verify `cargo test -p corvis --lib application`
- [ ] 2.2 New `crates/corvis/tests/init_failure.rs`: failing `on_module_init` then second `init()` →
  `InitFailedError`, and an `on_application_bootstrap` counter is still 0; `handle(request)` and
  `listen_in_background("127.0.0.1:0")` after the failure → `InitFailedError`; verify
  `cargo test -p corvis --test init_failure`
- [ ] 2.3 In the same file: a static `AtomicBool` makes the hook fail only on its first run; after the failed
  application, a fresh application from the same module initializes, and one request runs the global
  `CountingGuard` exactly once; two successful `init()` calls on one application run `on_module_init` once; verify
  `cargo test -p corvis --test init_failure`
- [ ] 2.4 `TestClient::new` (`crates/corvis/src/testing.rs:255-258`) panics with the failed-init message when
  `app.init_failed()`; add a `#[should_panic(expected = "init() failed")]` test in `init_failure.rs`; verify
  `cargo test -p corvis --test init_failure`
- [ ] 2.5 Bind retry after a successful `init()`: bind a `TcpListener` on `127.0.0.1:0`, call
  `listen_in_background` on that address (error), then on `127.0.0.1:0` (ok) with `on_module_init` counted once;
  verify `cargo test -p corvis --test init_failure`

## 3. Docs

- [ ] 3.1 `docs/fundamentals/lifecycle-events.md:63`: replace "`init()` is idempotent" with both cases (after
  success, later calls do nothing; after a failure, later calls return `InitFailedError` and the application must be
  recreated, because hooks that already ran cannot be undone); add a sentence near `:80` (hook errors) linking to
  it; in `docs/fundamentals/testing.md` near `:168`, note that `request(&app)` also panics after a failed `init()`;
  verify the text matches the final error message
- [ ] 3.2 Same changes in `docs/es/fundamentals/lifecycle-events.md` and `docs/es/fundamentals/testing.md`; verify
  `npm run docs:check` in `docs/`
- [ ] 3.3 Update the doc comment of `CorvisApplication::init` with the single-use rule; verify
  `cargo doc -p corvis --no-deps` shows no warnings for it

## 4. Definition of done

- [ ] 4.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 4.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] 4.3 `cargo test --workspace` passes, including the regression tests of group 2
- [ ] 4.4 Docs updated in English and Spanish (`docs/` and `docs/es/`); `npm run docs:check` in `docs/` passes
- [ ] 4.5 corvis-parity: the three port apps (consi, facten, houseplan) keep 0 apidiff differences (framework
  runtime code changed in `application.rs`)

## Workflow follow-up

- [ ] Archive the change with `/opsx:archive`
- [ ] Close issue [#123](https://github.com/randygil/corvis/issues/123)
- [ ] Tick #123 in its epic [#13](https://github.com/randygil/corvis/issues/13)
- [ ] Move the card to Done in the Corvis Roadmap project
