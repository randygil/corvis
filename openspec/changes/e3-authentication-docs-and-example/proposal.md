# Proposal

Implements [randygil/corvis#47](https://github.com/randygil/corvis/issues/47) (epic
[#39](https://github.com/randygil/corvis/issues/39)). Planned in the #40 spike
(`openspec/changes/archive/2026-11-08-e3-strategy-module-design/`, task group 8) and re-scoped here against what
the other changes of the epic already write.

**Depends on #42, #41, #45, #44, #43 and #46 being merged into `dev`.** It is the last change of the epic: #136
(merged) → #42 → #41 → #45 → #44 → #43 → #46 → **#47**. Their APIs are described here as designed, not as shipped;
the first task compares them with the merged code. A finding of #46 that is deferred with high or critical severity
blocks this change until the maintainer decides how the docs treat it.

## Why

Every change of the epic documents its own piece where the piece lives:

| Change | Docs it writes | Example it touches |
|---|---|---|
| #136 (merged) | `docs/recipes/passport.md` (the model, `AuthGuard`, writing a strategy), `docs/guards.md` | none |
| #42 | JWT section of `recipes/passport.md`; `security/authentication.md` switches to `AuthGuard::new("jwt")` | `cats-app` |
| #41 | `security/encryption-and-hashing.md`; hashing in `security/authentication.md`; local section of `recipes/passport.md` | `cats-app` |
| #45 | "API keys" and "Writing a strategy" in `recipes/passport.md`; `openapi/security.md`; `interceptors.md` | `cats-app` |
| #44 | `http/session.md`, `http/cookies.md`, `security/csrf.md`; sessions section of `recipes/passport.md` | `cats-app` |
| #43 | OAuth2 section of `recipes/passport.md`; `security/authentication.md` | `cats-app` |
| #46 | `security/authentication-threat-model.md`; login section of `security/rate-limiting.md` | none |

After seven changes the reference exists, section by section, but four things do not:

- **A guide.** `docs/security/authentication.md` is today a walkthrough of one flow with a hand-written guard over
  `JwtService` (260 lines; "The authentication guard" at 133, "What Corvis does not include" at 255 still says there
  are no strategies). Six changes edit it in place. Nobody arriving at "Authentication" is told which strategy to
  pick, how the strategies combine, or where each detail lives. `docs/recipes/passport.md` is `status: partial` and
  its introduction says the strategies "arrive one by one" (line 9). `docs/security/authorization.md:105` still
  uses a hand-written type named `AuthGuard`, which now collides with `corvis::auth::AuthGuard`.
- **Migration notes.** The positioning of the project is migrating Node backends with verified parity. Each
  section maps its own options, but there is no page for someone holding a Passport setup: what each class,
  option and call becomes, what answers differently on purpose, and how to run both stacks side by side (shared
  sessions, bcrypt hashes, the same token secret).
- **Security guidance for application authors.** The threat model of #46 is written for someone judging the module.
  An application author needs the other half: what the framework does not do for them, and the mistakes the
  combination allows (an optional guard as the only guard, a roles guard without an `AuthGuard`, a throttler after
  authentication).
- **One runnable example.** `cats-app` gains a piece of every strategy, spread over a large tour of the framework.
  There is no small application that is only about authentication, runs with one command without any external
  account, and whose code the guide's samples are taken from.

## What Changes

- **The guide**: `docs/security/authentication.md` rewritten as the entry page (which strategy to choose, the model
  in short, one short path per strategy taken from the example, combining strategies, public routes and global
  guards, reading the user, WebSockets, what is not included), linking to the reference sections instead of
  repeating them. `docs/recipes/passport.md` becomes `status: documented` with a consolidated introduction and one
  index of the differences from the original. `docs/security/authorization.md` uses `corvis::auth::AuthGuard`
  followed by the roles guard, and `User<T>`.
- **Migration page**: `docs/recipes/passport-migration.md` ("Migrating from Passport"): mapping tables per Node
  package, the patterns with no equivalent, the index of deliberate differences (built from the `divergence` marks
  of the recorded fixtures), and a side-by-side migration procedure.
- **Security guidance page**: `docs/security/authentication-security.md` ("Securing authentication"): a deployment
  checklist, what is the application's job per strategy, the common mistakes with their fix, each item linked to
  the threat model row that explains it.
- **Example** `examples/auth-app`: local login with hashed passwords (argon2id, one legacy bcrypt hash upgraded on
  login), JWT access tokens, an API key route for machine clients, session login and logout, an OAuth2 login
  against a bundled mock provider on `127.0.0.1`, a chain, an optional guard, a roles guard, login rate limiting
  and enforced route posture; tests for every flow; a README with a `curl` walkthrough; a section in
  `docs/faq/examples.md`.
- **Spanish pages** for everything above under `docs/es/`, sidebar entries in both languages, and cross-links from
  the pages that mention authentication.

No framework code changes.

## Capabilities

### New Capabilities

- `docs/authentication`: what the authentication guide, the strategy reference, the migration page and the security
  guidance page must contain, where they are linked from, and that their samples match the example.
- `examples/auth-app`: the authentication example: its flows, its configuration rules and its tests.

### Modified Capabilities

None. `docs/localization` (`openspec/specs/docs/localization/spec.md`) applies as it is to the new pages.

## Impact

- Docs (English and `docs/es/`): `security/authentication.md` (rewritten), `security/authorization.md`,
  `recipes/passport.md` (introduction, status, differences index), new `recipes/passport-migration.md`, new
  `security/authentication-security.md`, `faq/examples.md`, `migration-guide.md` and
  `recipes/express-migration.md` (links), `websockets/guards.md` (link), `docs/.vitepress/sidebar.mts`.
- New `examples/auth-app/` (`Cargo.toml`, `src/`, `tests/`, `README.md`, `.env.example`); it joins the workspace
  through the existing `examples/*` member glob (`Cargo.toml:11`).
- No change under `crates/`. The three port apps are not affected and the corvis-parity check does not apply
  (recorded in the definition of done).
- `examples/cats-app` is not changed: its auth pieces stay as the earlier changes left them.

## Out of scope

- Any framework API. A gap found while writing the guide or the example (a sample that cannot be written the way
  the reference says) is recorded as an issue, and the page documents what works.
- Rewriting the per-strategy reference sections of `recipes/passport.md`, `http/session.md`,
  `security/encryption-and-hashing.md` and `security/csrf.md`: they belong to #42-#44 and are only linked, apart
  from the introduction, status and differences index of `recipes/passport.md`.
- The threat model and the login rate limiting recipe (#46): linked, not repeated.
- Removing the auth pieces from `examples/cats-app` now that a dedicated example exists: possible new issue.
- A front end for the example (login forms, a "Sign in with" button): the walkthrough uses `curl`; possible new
  issue.
- WebSocket handshake authentication in the example: `examples/tertulia` shows it, and strategies at the handshake
  are tracked by the issue #46 files.
- Authorization features (a roles decorator, policies, CASL-style abilities): the authorization page only stops
  showing a guard whose name collides; possible new issues.
- A database for the example's users and sessions (`corvis-orm`, Redis): the session stores for both are follow-up
  issues of #44.
- Courses, videos and a blog post announcing the epic.
