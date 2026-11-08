# Tasks

Prerequisite: #26, #27 and #28 are merged (`e1-public-metadata-and-startup-check`,
`e1-route-posture-for-websockets-and-sse`, `e1-posture-report-in-the-cli`).

## 1. Route posture page

- [x] 1.1 Write `docs/security/route-posture.md` (frontmatter `status: documented`) with the headings of design
  decision 1, covering every item of the `docs/route-posture` spec: rule, `require_route_posture()` /
  `set_route_posture(RoutePostureOptions::warn())` / exemptions with a reason (including the middleware and reverse
  proxy case), `#[public]` on a route and on a controller and `PUBLIC_METADATA` with an existing global guard,
  `controls_access()` and the two framework guards that return `false`, gateways (global guards do not count) and SSE,
  the not-checked surfaces, the error message (copied from the implementation), `route_posture()` in a test,
  `corvis posture` with its exit codes, and the migration path (warn → fix → enforce); verify each code sample
  against the merged API (compile it in a scratch test or take it from tertulia)
- [x] 1.2 Translate it to `docs/es/security/route-posture.md` with the same headings, code fences and status; add
  `['Route posture', 'Postura de rutas', '/security/route-posture']` to the Security group of
  `docs/.vitepress/sidebar.mts` after Authorization; verify `npm run docs:check` in `docs/`

## 2. Cross-links

- [x] 2.1 `docs/guards.md`: link from the "What global guards do not cover" warning and add the `#[public]` note to
  "Public routes with a global guard"; `docs/websockets/guards.md` (global guards and gateway posture);
  `docs/custom-decorators.md` and `docs/security/authentication.md` (`#[public]` writes `"isPublic"`); same edits in
  the `docs/es/` pages with `/es/` links; verify `npm run docs:check`
- [x] 2.2 `docs/cli/usages.md` (`corvis posture` section from #28) links to the page; `docs/faq/common-errors.md` gets
  the `"Route posture check failed: ..."` section (cause, fixes, link); same in `docs/es/`; verify `npm run docs:check`

## 3. Tertulia

- [x] 3.1 In `examples/tertulia/src/lib.rs` `create_app()`, call `app.require_route_posture()` with a comment pointing
  to the docs page; add `#[public]` to `AuthController::login`, `RoomsController::{list, messages, members}` and below
  `#[controller("stats")]` in `src/stats/mod.rs`; verify `cargo test -p tertulia` passes unchanged (all existing
  tests now run in enforce mode)
- [x] 3.2 Add `route_posture_matches_the_routes` to `examples/tertulia/tests/chat.rs`: `start()`, then
  `t.app.route_posture()` and assert the statuses of `specs/examples/tertulia/spec.md` (public, protected, SSE kind for
  `/api/stats/stream`, every `/chat` handler protected, the handshake and one raw `upgrade` listener not checked, no
  unprotected or exempt entry); verify `cargo test -p tertulia --test chat route_posture`
- [x] 3.3 Temporarily remove `#[public]` from `RoomsController::list` and check that `cargo test -p tertulia` fails
  with the posture error naming `GET /api/rooms`; restore it; record the output in the PR description
- [x] 3.4 Add the route posture paragraph of design decision 5 to `examples/tertulia/README.md` (in the README's
  language); verify `corvis posture` from `examples/tertulia` prints the report described there and exits 0

## 3b. cats-app (review notes)

- [x] 3b.1 Replace `IS_PUBLIC_KEY` with `#[public]` and `PUBLIC_METADATA` in `examples/cats-app` (controllers, the
  public `ChatGateway`, `AuthGuard`), remove the constant, make the global pass-through `RolesGuard` return `false`
  from `controls_access()`, and call `app.require_route_posture()` in `create_app()`
- [x] 3b.2 Add `examples/cats-app/tests/posture.rs` (enforce mode starts, report statuses, `AuthGuard` is the only
  counted guard); verify `corvis posture` exits 0 in `examples/cats-app` and `examples/tertulia`
- [x] 3b.3 Migrate the docs samples that showed `IS_PUBLIC_KEY` (authentication, terminus, discovery service, examples
  FAQ) to `#[public]` / `PUBLIC_METADATA`, EN and ES

## 4. Definition of done

- [x] 4.1 `cargo build --workspace --all-targets` with no warnings
- [x] 4.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 4.3 `cargo test --workspace` passes
- [x] 4.4 Regression tests: `examples/tertulia/tests/chat.rs` covers every scenario of
  `specs/examples/tertulia/spec.md`; the `docs/route-posture` scenarios are covered by the page content and
  `npm run docs:check` (list the mapping in the PR description)
- [x] 4.5 Docs pages added and updated in English and Spanish (`docs/` and `docs/es/`), then `npm run docs:check` in
  `docs/` passes
- [x] 4.6 No framework runtime code changes in this change (only docs and the example), so the corvis-parity check does
  not apply; confirm with `git diff --stat` that nothing under `crates/` changed

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #29, tick it in epic #24 and move its card to Done (see the `corvis-task` skill). With #26–#29 done,
  close epic #24 if nothing else remains in it.
