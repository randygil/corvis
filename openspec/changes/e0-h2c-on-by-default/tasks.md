# Tasks

## 1. Default and rustdoc

- [x] 1.1 Set `h2c: false` in `ServerOptions::default()` (`crates/corvis/src/server.rs:63`) and rewrite the rustdoc of
  `ServerOptions::h2c` and the struct summary (`:33`) as in design.md Decision 5

## 2. Tests

- [x] 2.1 Set `.h2c(true)` in `silent_and_slow_header_connections_are_closed`
  (`crates/corvis/tests/transport_security.rs`), which relied on the old default
- [x] 2.2 Add `h2c_is_off_by_default` and `h2c_can_be_enabled` next to `h2c_can_be_disabled`; verify
  `cargo test -p corvis --test transport_security` passes

## 3. Docs

- [x] 3.1 Update `docs/http/performance.md`, `docs/security/http-parsing.md`, `docs/deployment.md` and
  `docs/faq/multiple-servers.md` (default is HTTP/1.1 only, `h2c(true)` opt-in and its limits; remove the references
  to the undecided issue #144)
- [x] 3.2 Update the Spanish copies under `docs/es/`
- [x] 3.3 Check for a CHANGELOG to note the behavior change: the repository has none, so nothing to edit
- [x] 3.4 Check whether the preface handling of #146 simplifies when h2c is off: it stays as is (design.md Decision 3)

## 4. Definition of done

- [x] 4.1 `cargo build --workspace --all-targets` with no warnings
- [x] 4.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 4.3 `cargo test --workspace` passes
- [x] 4.4 Regression tests exist for every requirement of the `http/connection-close` delta (2.1, 2.2)
- [x] 4.5 Docs pages in English and Spanish updated; `npm run docs:check` in `docs/` passes
  (docs updated; `docs:check` not run: `docs/node_modules` does not exist in this worktree)
- [ ] 4.6 corvis-parity: run by the maintainer; the change only flips a server default that the three port apps do not
  set or use

## Workflow follow-up

- [ ] Archive the change with `/opsx:archive`
- [ ] Close issue [#144](https://github.com/randygil/corvis/issues/144)
- [ ] Tick #144 in its epic [#1](https://github.com/randygil/corvis/issues/1)
- [ ] Move the card to Done in the Corvis Roadmap project
