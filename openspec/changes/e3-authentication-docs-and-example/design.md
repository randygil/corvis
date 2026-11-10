# Design

## Context

This is the last change of epic #39. It writes no framework code: it ties together the documentation the other
changes leave behind and adds the example. Line numbers below were checked on `origin/dev` at `c837dcb`; statements
about #42, #41, #45, #44, #43 and #46 come from their artifacts in `openspec/changes/` (design plus Review notes)
and are designs until merged. Task 1.1 re-checks them.

### The documentation today (on `dev`, with only the strategy core merged)

- `docs/security/authentication.md` (`status: documented`, 260 lines): one flow built by hand: "Registering
  `JwtModule`" (line 24), "Signing the token" (68, with the plain-text comparison #41 replaces), "The login
  endpoint" (106), "The authentication guard" (133, a `CanActivate` over `JwtService`), "Enabling authentication on
  all routes" (182), "Reading the user in the handler" (214), "Authentication in WebSockets" (235), "What Corvis does
  not include" (255: "There are no Passport-style strategies ... or server-side sessions").
- `docs/recipes/passport.md` (`status: partial`, 157 lines): "The model" (11), "Writing a strategy" (28),
  "`AuthGuard`" (83), "Differences from the original" (115), "Differential testing" (123), "Alternative:
  `JwtModule` + guards" (127). Line 9 says the strategies "arrive one by one" and names their issues.
- `docs/security/authorization.md` (`status: documented`, 177 lines): role checks with a hand-written guard; the
  sample at line 105 is `#[use_guards(AuthGuard, RolesGuard)]`, where `AuthGuard` is the example's own type.
- `docs/security/encryption-and-hashing.md`, `docs/http/session.md` and `docs/security/csrf.md` are
  `status: unavailable`; #41 and #44 change the first two to `documented` and update the third.
- `docs/security/rate-limiting.md:105-121` shows a login override; #46 rewrites that part.
- `docs/migration-guide.md` (68 lines) has a table of structure equivalences and no row for authentication;
  `docs/recipes/express-migration.md` and `fastify-migration.md` are the existing migration recipes
  (`docs/.vitepress/sidebar.mts:206` and `214`); `express-migration.md:79-92` builds a session cookie by hand.
- `docs/faq/examples.md` has one section per example (`cats-app` at line 10, `blog-orm` at 49, `tertulia` at 69,
  "Example tests" at 91).
- Sidebar: the Security group is `docs/.vitepress/sidebar.mts:79-89`; Passport is in the recipes at line 203.
- `docs/scripts/check.mjs` (run by `npm run docs:check`, `docs/package.json:10`) checks that every page exists in
  both languages with the same `status`, the same headings per level and the same number of code fences, that the
  sidebars match, that links resolve, the language of each page and the public naming rule.

### What each change of the epic writes (as designed)

The table in `proposal.md` lists it. Three consequences for this change:

- `docs/security/authentication.md` is edited by #42 (main path becomes `AuthGuard::new("jwt")`), #41 (hashing),
  #44 (a pointer) and #43 (line 257). Its outline stays the one-flow walkthrough.
- `docs/recipes/passport.md` grows a section per strategy, each with its own options table and its own list of
  differences. It is the reference.
- `examples/cats-app` gains a JWT strategy, a hashed local login, a machine-client route, a session login and a
  GitHub login against a mock provider, inside an application that also shows versioning, OpenAPI, scheduling, a
  chat gateway and request scope (`docs/faq/examples.md:10-47`).
- #46 adds `docs/security/authentication-threat-model.md` (assets, boundaries, a table of threats per surface, the
  defaults table, the dependency inventory, the findings register), `corvis::auth::local::login_tracker` and the
  guard-order startup warning.

### Examples today

`examples/blog-orm`, `examples/cats-app`, `examples/tertulia`, all `publish = false`, members of the workspace
through `examples/*` (`Cargo.toml:11`), all binding `127.0.0.1` (`examples/cats-app/src/main.rs:21-22`,
`examples/tertulia/src/main.rs:18`). `cats-app` refuses to start without `JWT_SECRET`
(`openspec/specs/examples/cats-app/spec.md`, "No default JWT secret") and exposes `create_app()` from a library
crate so its tests build the same application the binary runs. `examples/tertulia/README.md` is the only example
README.

## Goals / Non-Goals

**Goals**

- A reader who opens "Authentication" can choose a strategy, get it working, and find every detail by link.
- A reader with a Passport setup can translate it and knows every deliberate difference.
- A reader about to deploy knows what is theirs to do.
- One small example that runs offline with one command and backs the guide's samples.
- Every fact has one home; the new pages link to the reference and the threat model instead of copying them.

**Non-Goals**

- Framework code, a rewrite of the per-strategy reference, a front end, authorization features, changing
  `cats-app` (proposal Out of scope).

## Decisions

### 1. Page map: one home per fact

| Page | Role | Written by |
|---|---|---|
| `security/authentication.md` | Guide: choose, get started, combine | this change (rewrite) |
| `recipes/passport.md` | Reference: every option, default and difference per strategy | #136, #42, #41, #45, #44, #43; this change edits only the introduction, the status and the differences index |
| `recipes/passport-migration.md` | Migration: mapping from the Node packages, side-by-side procedure, all differences | this change (new) |
| `security/authentication-security.md` | Guidance for application authors | this change (new) |
| `security/authentication-threat-model.md` | Threat model for people judging the module | #46 |
| `security/authorization.md` | Role checks after `AuthGuard` | this change (samples and one warning) |
| `http/session.md`, `http/cookies.md`, `security/encryption-and-hashing.md`, `security/csrf.md`, `security/rate-limiting.md` | Topic reference | #44, #41, #46; linked only |

Rule for the writer: an option, a default or a limit is stated in the reference (or the threat model's defaults
table) and nowhere else; the guide and the guidance page name it and link. This keeps three pages from disagreeing
after the next default changes, and #46's defaults test then guards one table.

Alternative: fold the reference into `security/authentication.md` and delete the recipe. Rejected: the recipe is
where six changes put their sections and where people coming from Passport look; one page with all of it would be
several thousand lines.

### 2. Outline of the guide (`security/authentication.md`)

1. **How it works**: strategy, validator, `Provider::strategy`, `AuthGuard`, `User<T>`, the plain `401`; a diagram
   in text of one request.
2. **Choosing a strategy**: the table of the spec (client, where the credential travels, cargo feature, link).
3. **JWT**, **Username and password**, **API keys**, **Sessions**, **OAuth2**: each a short path of the same shape:
   enable the feature, the validator, the registration, the guard on a route, one `curl` line with its answer, then
   "Options and limits: see ...". Samples come from `examples/auth-app` (decision 5).
4. **Combining strategies**: `AuthGuard::chain`, `.optional()`, `PassportModule` and `AuthGuard::default()`, a
   session cookie together with a token (the route's guard decides), login routes that issue a token versus a
   session.
5. **Protecting everything by default**: a global guard with `.skip_public()`, `#[public]`, route posture (link),
   guard order and the throttler first (link to rate limiting).
6. **Reading the user**: `User<T>`, `Request::user()`, `Request::auth_info()`.
7. **Authorization** (two lines and a link), **WebSockets** (the handshake pattern, strategies do not run there,
   link), **Security** (link to the guidance page), **Migrating from Passport** (link).
8. **Without strategies**: the `JwtModule` plus hand-written guard path, kept short, linking to the section
   `recipes/passport.md` already has.
9. **What is not included**, with links to the tracking issues the earlier changes filed.

### 3. Migration page (`recipes/passport-migration.md`)

Title "Migrating from Passport", in the recipes group next to the Express and Fastify migrations. Sections:

1. **Before you start**: what carries over unchanged (strategy names, the `401` body, `req.user`), what must be
   decided (secret lengths, token expiry, cookie flags).
2. **Mapping tables**, one per package: the original framework's passport integration (strategy class and
   `validate`, `AuthGuard('x')`, `AuthGuard(['a','b'])`, `handleRequest`, `getRequest`, `PassportModule.register`,
   `defaultStrategy`, `req.user`, `req.authInfo`); `passport-jwt`; `passport-local`; `passport-headerapikey` and
   `passport-custom`; `express-session` with `passport.session()`, `serializeUser`, `deserializeUser`, `req.logIn`,
   `req.logout`; `passport-oauth2`, `passport-google-oauth20`, `passport-github2`, `passport-oauth2-refresh`;
   `bcryptjs`. Each row: the Node code, the Corvis code, a link to the reference.
3. **Not provided**: flash messages, `failureRedirect` and `successRedirect` on the guard, `passReqToCallback` (the
   request is always passed), the `property` option, `assignProperty`, `session: false` (sessions are opt-in per
   guard with `.log_in()`), `authInfo` shapes of serialized errors.
4. **What answers differently, on purpose**: one table per strategy (original, Corvis, reason). Built from the
   `divergence` marks of `crates/corvis/tests/fixtures/auth/*.json` (#136 decision 11 gives every case a
   `divergence` field) plus the startup errors that have no request to record. Task 3.3 lists the marks with a
   one-line script and checks each has a row.
5. **Running both side by side**: sessions (`connect.sid`, the same secrets of at least 32 bytes, the record
   format, a store both can reach, `__corvis` keys visible to Node); passwords (bcrypt hashes verify as they are,
   `needs_rehash` on login, what Node can no longer read once a hash is argon2id, and the bcrypt option that keeps
   both sides reading); tokens (same secret and algorithm, `exp` required, issuer and audience lists); API keys
   (the default name changed to `"api-key"`); OAuth2 (the redirect URI must be registered for the new host);
   moving routes behind a reverse proxy one prefix at a time.
6. **Checklist**.

The public naming rule applies: the page names the Node packages (`passport`, `passport-jwt`, ...) and refers to
the framework integration as "the original passport integration", as `recipes/passport.md:7` does.

Alternative: a section inside `migration-guide.md`. Rejected: that page is a one-screen syntax table; this is a
procedure with seven tables. `migration-guide.md` gets a row that links here.

### 4. Security guidance page (`security/authentication-security.md`)

Title "Securing authentication". Written for the person deploying an application, in the second person, short.

1. **What the framework does for you** (ten lines, each a link to the threat model row).
2. **Checklist before going live**: HTTPS and `Secure` cookies; secrets (length, source, rotation with
   `CookieKeys`); token lifetime and what a stolen token allows; login rate limiting per client and per account
   (link to #46's recipe); credentials in URLs and what ends up in proxy logs; CSRF when a credential travels in a
   cookie; a real session store for more than one process; redirect targets (`return_to`); linking accounts only by
   `verified_email()`; the unverified `id_token`; API keys stored as digests; password policy; trust proxy for
   client addresses.
3. **Per strategy: your part** (a table: the framework's job, your job).
4. **Common mistakes**, each with the wrong and the right sample: an optional guard as the only guard of a route
   that needs a user; a guard that reads `Request::user()` with no `AuthGuard` before it (it trusts the session
   user); a global guard without `.skip_public()` or `#[public]` routes that expect a user; `ThrottlerGuard` after
   `AuthGuard` (the startup warning of #46); a wrapper guard that does not forward `as_any`; returning `Pass` from a
   custom strategy; echoing `auth_info()` to the client; comparing secrets with `==`.
5. **Known limitations**: the deferred findings of #46's register, each with its workaround and a link.
6. **Reporting a vulnerability** (how to reach the maintainer; the text the repository already uses, if any).

It restates no table of the threat model. Alternative: make this a section of the threat model. Rejected: different
readers; a deployment checklist buried under assets and trust boundaries is not read.

### 5. Samples come from the example

The samples of the guide and of the guidance page are copied from `examples/auth-app`, with the file path in a
comment on the first line and elided parts marked `// ...`. Task 6.2 compares them. The Spanish pages carry the same
samples (the docs check requires the same number of code fences).

Alternative: VitePress snippet imports (`<<< @/../examples/auth-app/src/..#region`). Rejected: `check.mjs` counts
code fences in the Markdown source, the example would need region markers, and the Spanish pages could not
translate comments.

### 6. The example: `examples/auth-app`

Package `auth-app`, library `auth_app` with `create_app(Settings)` and a thin `main.rs`, `publish = false`, like
`cats-app`. The spike called it `examples/auth`; `auth-app` follows `cats-app` and avoids a crate named `auth` next
to `corvis::auth`. Dependencies: `corvis` with `auth-local`, `auth-api-key`, `auth-session`, `auth-oauth2` (and the
default `jwt`), `serde`, `serde_json`; no database.

```
src/
  lib.rs            create_app(Settings), Settings::from_env(), Settings::dev()
  main.rs           --dev flag, binds 127.0.0.1
  app_module.rs     ThrottlerModule, SessionModule, PassportModule, Provider::app_guard::<ThrottlerGuard>()
  users.rs          in-memory users: two argon2id hashes, one bcrypt hash (upgraded on login), roles
  auth/
    local.rs        LocalAuth: LocalValidator with verify_or_dummy and needs_rehash
    jwt.rs          JwtAuth: JwtValidator (bearer extractor), token issuing with expires_in
    api_key.rs      ServiceKey: ApiKeyValidator over ExpectedApiKey
    session.rs      UserSerializer: SessionSerializer
    provider.rs     ProviderAuth: OAuth2Validator, generic OAuth2Options from Settings, state in a signed cookie
    roles.rs        RolesGuard (reads User<T>; listed after AuthGuard)
  controllers.rs    the routes below
  mock_provider.rs  a minimal OAuth2 provider (authorize, token, userinfo) for --dev and the tests
tests/
  flows.rs  startup.rs  posture.rs  throttle.rs
README.md  .env.example
```

| Route | Guard | Shows |
|---|---|---|
| `GET /` | `#[public]` | a public route |
| `POST /auth/login` | `AuthGuard::new("local")`, per-client and per-account throttle | local login, returns an access token |
| `GET /me` | `AuthGuard::new("jwt")` | token route, `User<T>` |
| `GET /reports` | `AuthGuard::chain(["jwt", "api-key"])` | a chain; machine clients |
| `POST /session/login` | `AuthGuard::new("local").log_in()`, throttled | session login |
| `GET /session/me` | `AuthGuard::new("session")` | session route |
| `POST /session/logout` | `AuthGuard::new("session")` | `Authenticator::log_out` |
| `GET /auth/provider`, `GET /auth/provider/callback` | `AuthGuard::new("provider").log_in()`, throttled | OAuth2 with PKCE, state cookie, session login |
| `GET /feed` | `#[public]` + `AuthGuard::chain(["jwt", "session"]).optional()` | optional authentication |
| `GET /admin/users` | `AuthGuard::chain(["jwt", "session"])`, `RolesGuard` | authorization after authentication |

Choices:

- **No global `AuthGuard`.** Each controller states its guard, and `require_route_posture()` makes a forgotten one
  a startup error. The global-guard pattern is shown in the guide and in `cats-app`; here the point is to see each
  strategy on its route.
- **`ThrottlerGuard` global through `Provider::app_guard`**, so it precedes every `AuthGuard`
  (`crates/corvis/src/application.rs:532-534`) and #46's warning stays silent.
- **Settings.** `Settings::from_env()` requires `JWT_SECRET`, `SESSION_SECRET`, `SERVICE_API_KEY` (32 bytes or
  more each) and `OAUTH_AUTHORIZE_URL`, `OAUTH_TOKEN_URL`, `OAUTH_USERINFO_URL`, `OAUTH_CLIENT_ID`,
  `OAUTH_CLIENT_SECRET`, `OAUTH_REDIRECT_URI`; errors name the variable, never the value. `Settings::dev()`
  generates the three secrets from the operating system generator, starts `mock_provider` on a free loopback port,
  points the OAuth2 settings at it, turns `Secure` off for the session and state cookies (the framework logs its
  startup warnings for that) and prints the demo credentials and the service key for this run. There is no secret
  in the source or in `.env.example`.
- **Mock provider inside the example**, not a dependency on the test helper of `crates/corvis/tests/`: an example
  cannot depend on another crate's test code. It issues one fixed user, checks `state`, `code_verifier` and the
  client secret, and counts requests for the tests. The OAuth2 client accepts plain HTTP only for loopback
  addresses (#43), which is what makes the offline run possible.
- **Password hashes** are computed when the user table is built at startup with the default argon2id parameters
  (#41 sets `opt-level = 3` for the hashing crates in the dev profile); the legacy user's bcrypt hash is a constant
  with a comment saying how it was made. No plain password is stored; the demo passwords are printed by `--dev` and
  listed in the README.
- **README** in English: what the example shows, `cargo run -p auth-app -- --dev`, a `curl` walkthrough per route
  with the expected answer, how to point it at a real provider, how to run it without `--dev`.

Alternative: extend `cats-app` and call it the example. Rejected: it already carries every strategy as one topic
among fifteen; the guide needs short files to quote. Alternative: a real GitHub client in the example. Rejected: it
could not run offline or in CI.

### 7. Order of work and review

Example first (it validates that the merged APIs compose as the designs say, and any surprise is cheaper to find
before the prose), then the guide, the migration page, the guidance page, the cross-links, then the Spanish pages
with the `docs-writer` agent, then the checks. Technical review of the guidance page against #46's threat model is
its own task, because that page makes security claims.

## Risks / Trade-offs

- **Written against designs.** Any API that shipped differently changes samples and tables. Task 1.1 reconciles
  first; the example is built before the pages for the same reason.
- **Overlap with the reference.** The guide's short paths repeat a few lines of each reference section. Kept to one
  sample each, with the options left to the reference (decision 1).
- **Two examples with authentication** (`cats-app` and `auth-app`). `faq/examples.md` says which to read for what;
  trimming `cats-app` is out of scope.
- **`--dev` turns `Secure` off.** It is the only way a browser or `curl` keeps the cookies over plain HTTP on
  loopback; the flag is explicit, the framework warns at startup, and the README says what production needs.
- **The differences index can go stale** when a later change adds a divergence to a fixture. Task 3.3 leaves the
  listing script in the page's maintenance note; a test is not added (no framework code in this change; Open
  question 4).
- **Long pages in two languages.** About four pages of new prose, translated; the docs check keeps the structure
  aligned, not the meaning. The Spanish pages are reviewed against the neutral-Spanish rule of the site.
- **A deferred high-severity finding of #46** would make the guidance page document a workaround instead of a fix.
  The change waits for the maintainer's decision in that case.

## Open questions

1. **Example name.** `examples/auth-app` (this design) versus the spike's `examples/auth`. Recommended: `auth-app`.
2. **Global guard in the example.** None, with route posture enforced (decision 6), versus the global
   `AuthGuard::default().skip_public()` pattern most migrated applications use. Recommended: none here; the guide
   and `cats-app` show the global pattern.
3. **`--dev` mode** that generates secrets and starts the mock provider, versus requiring a `.env`. Recommended:
   keep `--dev`; without it the example needs nine variables before it shows anything.
4. **A test that the differences index covers every fixture divergence.** It would live under `crates/corvis/tests/`
   and read a docs page, as #46's source checks do. Recommended: add it to `auth_source_checks.rs` only if #46
   shipped that file as designed; otherwise keep the manual listing of task 3.3.
5. **Trimming `cats-app`** back to JWT and local login once `auth-app` exists. Recommended: a new issue, not this
   change (the specs of #42-#43 require those pieces there).
6. **"Reporting a vulnerability"** on the guidance page needs a contact or a policy; the repository has no
   `SECURITY.md` today. Recommended: the maintainer provides the text, or the section is left out and an issue is
   filed for a security policy.
7. **Spike task 7.4** (the list of divergences) lands here as section 4 of the migration page, as #46's Open
   question 6 proposes. Recommended: confirm.
8. **ROADMAP.md** needs no rename: both change names match it (`e3-authentication-security-review`,
   `e3-authentication-docs-and-example`). If #46's threat model page or this change's new pages are renamed during
   review, the sidebar entries of both changes must be updated together.

## Review notes

These settle the open questions and override the design where they differ.

1. The example is `examples/auth-app`.
2. The example has no global `AuthGuard`; route posture is enforced, and the guide points to `cats-app` for the
   global pattern.
3. `--dev` mode is kept (generated secrets, bundled mock OAuth2 provider on 127.0.0.1); without it there are no
   default secrets and startup fails when one is missing.
4. The test that the differences index covers every fixture divergence is added to `auth_source_checks.rs` only if
   #46 shipped that file as designed; otherwise the listing stays manual.
5. Trimming the authentication pieces of `cats-app` is out of scope; an issue is filed.
6. The security page has no "Reporting a vulnerability" section: the repository has no security policy yet. An issue
   is filed for the maintainer to decide one, and the section is added when it exists.
7. The full divergence list lives in the migration page.
8. Implemented after #46 lands; a deferred high or critical finding of #46 blocks this change.
