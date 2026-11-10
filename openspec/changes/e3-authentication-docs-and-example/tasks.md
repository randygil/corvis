# Tasks

Prerequisite: #42, #41, #45, #44, #43 and #46 are merged into `dev`. If #46 deferred a finding of high or critical
severity, the maintainer decides how the docs treat it before this change starts.

## 1. Check the ground

- [ ] 1.1 Compare this change with the merged code and docs of the six changes: public names and options of each
  strategy, `AuthGuard::log_in`, `login_tracker`, the pages each change wrote and their current headings and
  statuses, the threat model's sections and findings register; update `design.md` (Context, decisions 1-6), the
  spec scenarios and this file where they differ, and record the adjustments under "Implementation notes" in
  `design.md`; verify `openspec validate e3-authentication-docs-and-example --strict` still passes
- [ ] 1.2 List every sentence in `docs/` and `docs/es/` that still says strategies, sessions, signed cookies or
  password hashing are missing or pending (search for the issue numbers #41-#45, "arrive", "does not include",
  "unavailable"); verify the list is attached to the pull request and each entry is assigned to a task below

## 2. The example

- [ ] 2.1 `examples/auth-app` skeleton: `Cargo.toml` (`publish = false`, corvis with the four auth features),
  `src/lib.rs` with `Settings` (`from_env`, `dev`), `create_app(Settings)`, `main.rs` with `--dev` binding
  `127.0.0.1`, `app_module.rs`, `.env.example` without working values; verify `cargo test -p auth-app --test
  startup` for "Missing secret", "Short secret" (error text without the value) and "Copying the example environment
  file"
- [ ] 2.2 Users and local login: `users.rs` (two argon2id users, one bcrypt user, roles), `auth/local.rs`
  (`verify_or_dummy`, rehash with `needs_rehash`), `auth/jwt.rs` (issuing with an expiry, `JwtValidator`),
  `POST /auth/login`, `GET /me`; verify tests for "Login for a token", "Wrong password and unknown user", "Legacy
  hash upgraded", "Valid token" and "Missing or expired token" in `tests/flows.rs`
- [ ] 2.3 API key and chain: `auth/api_key.rs` (`ExpectedApiKey` from `SERVICE_API_KEY`), `GET /reports` with
  `AuthGuard::chain(["jwt", "api-key"])`; verify tests for "Service key" and "Wrong key", and that a user token also
  reaches the route
- [ ] 2.4 Sessions: `SessionModule`, `auth/session.rs` (`SessionSerializer`), `POST /session/login` with
  `.log_in()`, `GET /session/me`, `POST /session/logout`; verify tests for "Session flow" and "Identifier changes at
  login"
- [ ] 2.5 OAuth2: `mock_provider.rs` (authorize, token and userinfo endpoints on a free loopback port, request
  counters), `auth/provider.rs` (generic options from `Settings`, state in a signed cookie, PKCE),
  `GET /auth/provider` and its callback with `.log_in()`; verify tests for "Full flow" and "Forged callback" with
  no network access
- [ ] 2.6 Combining: `GET /` and `GET /feed` (`#[public]`, optional chain), `auth/roles.rs` and `GET /admin/users`,
  `require_route_posture()` in `create_app`; verify tests for "Optional guard", "Role required" and "Posture
  report" in `tests/posture.rs`
- [ ] 2.7 Rate limiting: `ThrottlerGuard` through `Provider::app_guard`, per-client and per-account limits on the
  login routes with `login_tracker`, per-client limits on the OAuth2 routes; verify tests for "Attempts over the
  limit" and "Other routes keep the global limit" in `tests/throttle.rs`, and that startup logs no guard-order
  warning
- [ ] 2.8 `--dev` mode end to end: generated secrets, the mock provider started, `Secure` off with the framework's
  warnings, demo credentials printed; verify by running `cargo run -p auth-app -- --dev` and following every `curl`
  line of the README draft against `127.0.0.1`, then restarting and checking that the old token and cookie are
  refused
- [ ] 2.9 `examples/auth-app/README.md` (English): what it shows, the route table, the `curl` walkthrough with
  expected answers, running against a real provider, running without `--dev`; verify each command was run in 2.8
- [ ] 2.10 Record as proposed issues (text in the pull request, created by the maintainer's session) every place
  where the merged API did not allow what the designs described; verify the guide documents what works

## 3. Pages in English

- [ ] 3.1 Rewrite `docs/security/authentication.md` with the outline of design decision 2 (status stays
  `documented`), samples copied from `examples/auth-app` with the file path in a comment; keep anchors that other
  pages link to or update those links; verify every scenario of "The authentication page is the entry guide", "The
  guide explains how strategies combine" and "The guide states the limits" against the page
- [ ] 3.2 `docs/recipes/passport.md`: status `documented`, a new introduction (five strategies available, how the
  page is organized), one "Differences from the original" index linking to each strategy's list, the alternative
  section trimmed to what the guide links to; verify no section written by #42-#43 lost content (diff limited to
  the introduction, the index and the alternative section)
- [ ] 3.3 New `docs/recipes/passport-migration.md` (status `documented`) with the six sections of design decision
  3; build the differences tables from the `divergence` marks of `crates/corvis/tests/fixtures/auth/*.json` (list
  them with a one-line Node script, kept in the pull request description) plus the startup errors; verify every
  mark has a row, and every scenario of the three migration requirements against the page
- [ ] 3.4 New `docs/security/authentication-security.md` (status `documented`) with the sections of design decision
  4, each framework claim linking to its row of `/security/authentication-threat-model`, deferred findings of #46's
  register under "Known limitations"; verify every scenario of the two guidance requirements, and that no table of
  the threat model is copied
- [ ] 3.5 `docs/security/authorization.md`: samples use `corvis::auth::AuthGuard::new(..)` followed by the roles
  guard and read the user with `User<T>`; add the warning about guards that read the user without an `AuthGuard`;
  update the WebSocket section's wording if it names the old guard; verify the sample matches
  `examples/auth-app/src/auth/roles.rs`
- [ ] 3.6 Cross-links and sidebar: `docs/.vitepress/sidebar.mts` (the guidance page in Security after the threat
  model; the migration page next to the Express and Fastify migrations), `docs/migration-guide.md` (a row),
  `docs/recipes/express-migration.md` (pointer from its hand-made session to sessions), `docs/guards.md`,
  `docs/websockets/guards.md`, `docs/http/session.md`, `docs/security/encryption-and-hashing.md`,
  `docs/security/rate-limiting.md` (links to the guide), `docs/faq/examples.md` (a section for `auth-app` and one
  line on which example to read for what); verify the stale sentences listed in 1.2 are gone
- [ ] 3.7 Technical review of 3.1, 3.3 and 3.4 against the merged code and #46's threat model: every statement
  about a default, a limit or a behavior is checked against the reference or a test; verify the reviewer's list of
  corrections is empty or applied

## 4. Spanish pages

- [ ] 4.1 `docs/es/security/authentication.md`, `docs/es/recipes/passport.md` and
  `docs/es/security/authorization.md` with the `docs-writer` agent: same headings, samples and status, neutral
  Spanish; verify `npm run docs:check` in `docs/`
- [ ] 4.2 `docs/es/recipes/passport-migration.md` and `docs/es/security/authentication-security.md`; verify
  `npm run docs:check`
- [ ] 4.3 The cross-links of 3.6 in `docs/es/` (links stay under `/es/`) and the sidebar's Spanish labels; verify
  `npm run docs:check`

## 5. Samples and consistency

- [ ] 5.1 Compare every Corvis code sample of the three new or rewritten pages with the file of
  `examples/auth-app` named in its first-line comment (same code apart from `// ...` elisions), in both languages;
  verify with a listing of sample, file and result in the pull request
- [ ] 5.2 Check the one-home rule of design decision 1: no option table, default value or limit is stated in the
  guide or the guidance page without a link to the reference or the threat model; verify by searching the new pages
  for the default values listed in the threat model's defaults table
- [ ] 5.3 Read the built site (`npm run docs:build` in `docs/`, served on `127.0.0.1`) for the five pages in both
  languages: tables render, anchors work, the sidebar order is right; verify with a note in the pull request

## 6. Definition of done

- [ ] 6.1 `cargo build --workspace --all-targets` with no warnings (the new example included)
- [ ] 6.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] 6.3 `cargo test --workspace` passes, including the tests of `auth-app`, with no network and no Node
- [ ] 6.4 Regression tests: one test per scenario of `examples/auth-app`; the `docs/authentication` scenarios are
  covered by the page content, the sample comparison of 5.1 and `npm run docs:check`
- [ ] 6.5 Docs pages updated in English (`docs/`) and Spanish (`docs/es/`); `npm run docs:check` in `docs/` passes
- [ ] 6.6 No framework runtime code changed (`git diff --stat origin/dev -- crates/` is empty), so the
  corvis-parity check does not apply; record it in the archive notes. If a fix under `crates/` became necessary, it
  goes to its own issue and change, not here
- [ ] 6.7 `cargo deny check` passes with the example in the workspace (no new crate versions expected)

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #47, tick it in epic #39, and move its card to Done (see the `corvis-task` skill).
- With #47 closed every task of epic #39 is done: close the epic, and link from it the follow-up issues the epic
  produced (session stores, redirect URI allowlist, strategies at the WebSocket handshake, the findings of #46).
