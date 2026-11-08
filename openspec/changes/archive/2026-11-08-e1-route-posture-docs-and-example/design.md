# Design

## Context

The route posture spike (#25) planned this change as task group 5. #26 (check, `#[public]`, `PUBLIC_METADATA`,
`controls_access()`, exemptions, warn mode, `route_posture()`), #27 (SSE labels, gateway handlers, not-checked
surfaces) and #28 (report mode, `corvis posture`, the `corvis new` template, the CLI usage section) implement the
feature. This change documents it and enables it in an example. Current state, read in this worktree:

- **Docs where posture belongs.**
  - `docs/guards.md`: guard order (`:100-109`), the warning "What global guards do not cover" (`:111-113`), and
    "Public routes with a global guard" (`:150-180`), built on `IS_PUBLIC_KEY = "isPublic"` and
    `Reflector::get_all_and_override`.
  - `docs/websockets/guards.md:30`: global guards do not apply to gateways.
  - `docs/custom-decorators.md:79-116`: the `IS_PUBLIC_KEY` metadata example (`:84`, `:97`).
  - `docs/security/authentication.md:181-210` ("Enabling authentication on all routes", `IS_PUBLIC_KEY` at `:197-200`).
  - `docs/faq/common-errors.md`: one section per error message (`:10-119`).
  - The Security group of the sidebar (`docs/.vitepress/sidebar.mts:79-87`, `['English', 'Español', '/path']` entries)
    has authentication, authorization, encryption, helmet, CORS, CSRF and rate limiting. There is no posture page.
- **Docs rules** (`docs/README.md`): English page plus Spanish translation at the same path under `es/`, sidebar
  entry with both labels, absolute links per language (`/x` and `/es/x`), identical `status` frontmatter.
  `npm run docs:check` (`docs/scripts/check.mjs:1-8`) checks pages, structure (same headings per level and the same
  number of code fences in both languages), sidebar, links, language (no Spanish in English pages, no voseo) and
  naming.
- **Tertulia.** `create_app()` (`examples/tertulia/src/lib.rs:30-39`) sets the `api` prefix, global pipes and filters,
  and mounts `corvis::serve_static(PUBLIC_DIR)` as middleware (`:37`), which the posture check cannot see. Routes:
  - `AuthController` (`src/auth/auth_controller.rs:8-27`): `POST login` (open), `GET me` (`#[use_guards(AuthGuard)]`).
  - `RoomsController` (`src/rooms/rooms_controller.rs:11-56`): `GET` list, `GET :room/messages`, `GET :room/members`
    (open), `POST` create (`AuthGuard`), `POST :room/announcements` (`AuthGuard, RolesGuard`).
  - `StatsController` (`src/stats/mod.rs:10-36`): `GET` snapshot and `#[sse("stream")]`, both open.
  - `ChatGateway` (`src/chat/chat_gateway.rs:37-38`) has `#[use_guards(AuthGuard)]` at gateway level, so every
    `#[subscribe_message]` handler is guarded (`:53-132`). Its handshake has no `verify_client`; it authenticates in
    `handle_connection` and closes with 4401 (`:209-230`).
  - The firehose is a raw `on_upgrade` listener registered at bootstrap (`src/firehose.rs:15`, `:48`).
  - No global guards (`use_global_guards` / `app_guard` do not appear in `src/`).
  - Tests: `tests/chat.rs` (14 tests; `start()` at `:27-44` sets the environment once and calls
    `listen_in_background`), `tests/startup.rs` (single test, missing `JWT_SECRET`).
  - `README.md` is written in Spanish (lists `serve_static` at `:18` and the REST endpoints around `:98`).

## Goals / Non-Goals

**Goals:**
- One page a user can follow to enable posture on a new or existing application, with honest limits.
- Every page that says "global guards do not cover X" or shows the `"isPublic"` pattern points to it.
- A runnable example whose report is checked by a test, so the docs and the example cannot silently drift from the
  implementation.

**Non-Goals:**
- Changing framework code or the report format.
- Rewriting the existing `IS_PUBLIC_KEY` samples (see proposal "Out of scope").

## Decisions

1. **One page under Security, `security/route-posture.md`, status `documented`.** Sidebar entry
   `['Route posture', 'Postura de rutas', '/security/route-posture']` right after Authorization, since it is about
   who may call what. Proposed headings (identical structure in Spanish):
   `# Route posture` · `## The rule` · `## Enabling it` (enforce, warn, exemptions) · `## Marking routes public` ·
   `## Which guards count` · `## WebSockets and SSE` · `## What is not checked` · `## The error` ·
   `## Reviewing the posture` (`route_posture()` in a test, `corvis posture`, link to the CLI usage section) ·
   `## Migrating an existing application`.
   - Alternative: a section inside `guards.md`. Rejected: guards.md documents the guard API, and posture also covers
     exemptions, gateways, static files and the CLI; a long section would bury it. guards.md gets a short paragraph
     and a link instead.
   - Code samples come from tertulia where possible (the `#[public]` routes, `create_app()`), so task 3 keeps them
     true.

2. **Cross-links are short.** Each linked page gets one or two sentences and a link, not a copy of the rules:
   - `guards.md`: the warning (`:111-113`) gains "To make the application refuse to start when a route has no guard,
     see Route posture". "Public routes with a global guard" notes that `#[public]` writes the same `"isPublic"`
     key, so the shown guard works with it unchanged.
   - `websockets/guards.md:30`: global guards do not count for gateway posture either; link.
   - `custom-decorators.md` and `security/authentication.md`: same note as guards.md about `#[public]`.
   - `cli/usages.md`: link from the `corvis posture` section written by #28 to the page.
   - `faq/common-errors.md`: new section headed `"Route posture check failed: ..."`, with cause, the three fixes and
     a link.

3. **Tertulia: route-level and controller-level `#[public]`.** `create_app()` calls `app.require_route_posture()`
   (so `main`, which uses `create_app()`, and all tests run in enforce mode). `#[public]` goes on `login`, `list`,
   `messages` and `members`, and below `#[controller("stats")]` for `StatsController`, so the example shows both
   placements and an SSE route covered by its controller. No exemption is needed. `serve_static` stays middleware (not
   checked, as the page explains). Switching to `ServeStaticModule` just to show it in the report would change how
   the example serves files.
   - Alternative: an exemption for the stats endpoints ("read-only public numbers"). Rejected: `#[public]` is the
     honest statement here; exemptions are for protection the framework cannot see.

4. **The test lives in `tests/chat.rs`.** One new test reuses `start()` (environment, `listen_in_background`, so init
   hooks have run and the firehose listener is counted), reads `t.app.route_posture()` and asserts the statuses of
   the spec, that no entry is unprotected or exempt, and the not-checked surfaces (the `/chat` handshake without
   `verify_client`, one raw `upgrade` listener). Adding it to the existing binary avoids linking another test
   executable in the shared target directory. The assertions use the report API of #26/#27 (kind, method, path,
   status), not the text table.

5. **README.** One short paragraph in tertulia's README, in its existing language (Spanish), saying the example runs
   with route posture enforced, which routes are public and why, and how to print the report (`corvis posture` from
   `examples/tertulia`).

## Risks / Trade-offs

- [The page documents APIs from three changes; if any of them changed during review, samples drift] → tasks verify
  every sample against the merged code; tertulia's test pins the example's behavior.
- [Enforce mode in tertulia makes a future unguarded route fail every tertulia test] → intended: that is the
  example's point, and the error explains the fix.
- [Spanish structure check] → both pages are written with the same headings and the same number of code fences;
  `npm run docs:check` enforces it.

## Open questions

Both were settled in the review notes below: the page goes after Authorization, and `examples/cats-app` moves to
`#[public]` in this change.

## Review notes (2026-11-08)

- The new page goes right after Authorization in the sidebar.
- Migrate `examples/cats-app` to `#[public]` in this change (it runs after #21 and #116, which also touch cats-app).

## Implementation notes

Differences between this design and what was implemented, after #26, #27 and #28 were merged and archived:

- **Existing docs already covered parts of the feature.** `docs/guards.md` ("Require a guard on every route",
  "Pass-through global guards", "Report mode", "What the check does not see"), `docs/websockets/guards.md`,
  `docs/cli/usages.md` and `docs/first-steps.md` were written by the earlier changes. The new page is now the one place
  that explains the feature end to end (rule, enabling, exemptions, `#[public]`, counted guards, gateways and SSE,
  limits, error, review tools, migration). The guards page keeps a short "Require a guard on every route" section that
  links to it, and keeps the sections about pass-through guards, report mode and unseen surfaces, which the new page
  links to instead of copying. The gateway section of the WebSocket guards page keeps the gateway-specific details.
- **Final API**: the page documents the literal exemption matcher (`*` is one segment, `**` the rest), `exempt_message`,
  `access_guards()`, `public_source`, the `unchecked` surfaces, `RoutePostureError`, and `corvis posture` with exit
  codes 0, 1 and 2 (the application's own report-mode exit codes 40 and 41, and the warning not to set
  `CORVIS_ROUTE_POSTURE_REPORT` in production or test jobs). It also says that `corvis g guard` generates a guard that
  denies by default and that `corvis new` enables the check.
- **Static files in tertulia are not in the report.** The example serves them with `serve_static` (middleware), which
  the report never lists; only `ServeStaticModule` mounts appear under "Not checked". The tertulia test asserts the
  handshake and the raw `upgrade` listener, and the README says `serve_static` is invisible to the report.
- **Wrong statement fixed**: `docs/security/authentication.md` said that global guards do not apply to WebSocket
  gateways; they do (unless `apply_global_enhancers_to_gateways(false)`). It now says so and links to the page.
- **cats-app**: besides `#[public]`, its global `RolesGuard` returns `Ok(true)` when a route declares no roles, so it now
  overrides `controls_access()` to return `false`. Without that the check would count it as access control for every
  route. `ChatGateway` keeps being public as a whole, now with `#[public]`. `create_app()` enables
  `require_route_posture()`, and `tests/posture.rs` pins the report. `scripts/smoke-examples.mjs` is unchanged
  (32/32); `corvis posture` was run by hand in both examples (exit 0).
- **Docs samples**: the samples built on cats-app (authentication, terminus, discovery service, examples FAQ) now show
  `#[public]` and `PUBLIC_METADATA`.
- **No framework code changed** (`git diff --stat` shows nothing under `crates/`), so the corvis-parity check does not
  apply.
