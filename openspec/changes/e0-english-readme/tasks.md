# Tasks

## 1. Inventory

- [x] 1.1 For each section of the current `README.md`, list the docs page (English slug) it will link to and the facts
  that page lacks; record the table in this change's design.md (appendix) or in the PR description; verify every
  section has a target page or a reason to keep its text in the README
- [x] 1.2 Add the missing facts to their docs pages in English (`docs/`) and Spanish (`docs/es/`); verify with
  `npm run docs:check` in `docs/`

## 2. Doctest harness

- [ ] 2.1 Add `#[cfg(doctest)] #[doc = include_str!("../../README.md")] pub struct ReadmeDoctests;` to
  `crates/corvis/src/lib.rs` (with a one-line comment saying why); verify that a deliberately broken non-ignored block
  in a scratch copy of the README makes `cargo test --doc -p corvis` fail, then restore it (cargo verification left to the maintainer)

## 3. Rewrite

- [ ] 3.1 Write the top of the new README in English: tagline, hello-world (compiled, `no_run`), its log output,
  links to `https://corvis.randygil.com` and `https://corvis.randygil.com/es/`, install and CLI quick start (with the
  `corvis build` behavior the CLI has at that moment), repository layout; verify `cargo test --doc -p corvis` (cargo verification left to the maintainer)
- [x] 3.2 Translate the "coming from Node.js" mapping tables, fixing `#[global]` (below `#[module]`, or
  `#[module(global, ...)]`), `corvis::config::register_as` with `ConfigModule::for_feature`, `ServeStaticModule::for_root(vec![ServeStaticOptions::new(..)])`
  and "wildcards when `wildcard: true`"; verify each corrected entry against `module.rs:39-66`, `config.rs:30,288`,
  `serve_static.rs:37,123`, `event_emitter.rs:37-40`
- [ ] 3.3 Write the short feature examples (modules and providers with a compiled `#[global]` example, DTO
  validation, guards, configuration with `register_as`, events with wildcards enabled, scheduling, testing, WebSocket
  gateway, static files with `for_root(vec![...])`), each with a link to its docs page; verify the compiled ones with
  `cargo test --doc -p corvis` and that the events example matches `crates/corvis/tests/modules.rs:100-112` (cargo verification left to the maintainer)
- [x] 3.4 Write the ORM and OpenAPI summaries (`rust,ignore` blocks), the performance table (numbers copied from the
  current README and `bench/results.md`), the security summary, known limitations and license; verify the numbers match
  `bench/results.md`
- [x] 3.5 Remove the brand mentions (current lines 919, 1369, 1396, 1429) by describing behavior; verify
  `grep -nE '\b(Nest|NestJS|Prisma)\b|@nestjs/' README.md` prints nothing

## 4. Link check

- [x] 4.1 Check every `https://corvis.randygil.com/...` link of the README against `docs/**/*.md` slugs (and anchors
  against headings) with a one-off script or `npm run docs:check` extended to read the README; verify zero missing
  pages or anchors, and that relative links (`LICENSE`, `THIRD_PARTY_NOTICES.md`, `bench/results.md`) resolve

## 5. Definition of done

- [ ] 5.1 Builds with no warnings (cargo verification left to the maintainer)
- [ ] 5.2 `cargo clippy --workspace --all-targets -- -D warnings` passes (cargo verification left to the maintainer)
- [ ] 5.3 `cargo test --workspace` passes, including the README doctests (cargo verification left to the maintainer)
- [ ] 5.4 Regression tests: the README doctest harness (task 2.1) is the regression test for the examples (cargo verification left to the maintainer)
- [x] 5.5 Docs pages that received README content are updated in English (`docs/`) and Spanish (`docs/es/`);
  `npm run docs:check` in `docs/` passes
- [x] 5.6 No framework runtime code changes (only a `#[cfg(doctest)]` item): record that the corvis-parity check
  (three port apps, 0 apidiff differences) is not required for this change

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #3, tick it in epic #1 and move its card to Done (see the `corvis-task` skill).
