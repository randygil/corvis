# Tasks

## 1. Failing tests first

- [x] 1.1 Add `#[cfg(test)] mod tests` to `crates/corvis-macros/src/controller.rs` with unit tests that build an `ItemImpl` with `syn::parse_quote!` and call `expand_routes`: guard on a helper method (exact message from the spec), `#[http_code(204)]` on a helper method, a plain helper and one with `#[allow(dead_code)]` (both `Ok`), the route method unchanged (`Ok`), two offending methods (two messages through `syn::Error::into_iter`); verify with `cargo test -p corvis-macros` that the error cases fail on the current code because `expand_routes` returns `Ok`
- [x] 1.2 Same in `crates/corvis-macros/src/handlers.rs` for `handlers::expand`: guard on a helper method (exact message), `#[set_metadata(k, v)]` on a helper method, a plain helper (`Ok`), the existing `#[on_event]` + enhancer error unchanged, two offending methods; verify with `cargo test -p corvis-macros` that the new error cases fail on the current code

## 2. Macros

- [x] 2.1 In `expand_routes`, record the first consumed non-route attribute of each method (every `match` branch at `controller.rs:191-243` except the `keep` fallback, and decorators returned by `decorator_tokens`); at the `continue` for methods without a route, build the error from Decision 3 with the route list taken from `METHODS`, push it into an accumulated `Option<syn::Error>` with `combine`, and return it after the loop; verify the group 1.1 tests pass
- [x] 2.2 In `handlers::expand`, at the `continue` for methods without a kind (`handlers.rs:118`), turn a recorded `enhancer_attr` into the error from Decision 3, accumulating like 2.1; keep the existing `#[subscribe_message]`-only error (`:119-123`) as is; verify the group 1.2 tests pass
- [x] 2.3 If #26 or #27 has landed, remove their `#[public]`-only check for methods without a route/handler attribute, make sure `#[public]` is recorded by the new check, and update their tests to the new message; verify `cargo test -p corvis-macros` and the #26/#27 tests pass (skip and note it here if neither has landed) (skipped: neither #26 nor #27 has landed in this branch)
- [x] 2.4 Build the examples and the `corvis` tests that use `#[routes]` and `#[handlers]` to confirm no existing code trips the check; verify `cargo test -p corvis` compiles and passes

## 3. Doctests

- [x] 3.1 Create `crates/corvis/src/doctests.rs` with one doc-commented item per case: `#[routes]` helper with `#[use_guards]`, `#[handlers]` helper with `#[use_guards]` on a plain `#[injectable]` provider; each with a ` ```compile_fail ` block and a passing ` ``` ` twin that only adds `#[get("purge")]` / `#[subscribe_message("kick")]`; declare it in `crates/corvis/src/lib.rs` as `#[cfg(doctest)] mod doctests;` next to `ReadmeDoctests`; verify with `cargo test --doc -p corvis` that the four blocks pass, and that removing the route attribute from a passing twin makes it fail

## 4. Docs

- [x] 4.1 `docs/faq/common-errors.md`, "Attribute compile errors": add the two messages with their cause and fix ("add the route attribute or remove the enhancer"); same in `docs/es/faq/common-errors.md`; verify both tables have the same rows
- [x] 4.2 `docs/controllers.md` (after the "Verbs" table): one sentence saying that a method without a verb attribute is a plain helper and that putting enhancers or route attributes on it is a compile error; `docs/websockets/gateways.md` (section `#[handlers]` and `#[subscribe_message]`): the same for methods without `#[subscribe_message]`; same in `docs/es/`; verify both languages say the same
- [x] 4.3 Run `npm run docs:check` in `docs/`; verify it passes

## 5. Definition of done

- [x] 5.1 `cargo build --workspace --all-targets` with no warnings
- [x] 5.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 5.3 `cargo test --workspace` passes, including the regression tests of groups 1 and 3 (`cargo test --doc -p corvis` runs the `compile_fail` doctests)
- [x] 5.4 Docs updated in English and Spanish (`docs/` and `docs/es/`) and `npm run docs:check` passes in `docs/`
- [x] 5.5 corvis-parity: only macro expansion changes for methods that do not compile anymore, and no framework runtime code changes, so the three port apps are not affected; confirm by building them (`cargo build` in each port, as in the `corvis-parity` skill, step 1). Run the full apidiff check only if group 2 ended up touching code that methods with a route or handler attribute go through (not run by the implementer: the maintainer runs it; the ports' own path dependency does not point at this worktree) (maintainer batch run on batch/phase03-c cd79e44: consi 832/832, facten 727/727, houseplan 598/598 quiet-hours count; routes identical, databases equivalent)

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #124, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).
