# docs/authentication Specification

## Purpose
Users can learn from the documentation site how to authenticate requests with the strategies of `corvis::auth`: which
one to choose, how to combine them, how to migrate a Passport setup, and what remains their responsibility, in
English and in Spanish. The per-strategy reference is written by the change that ships each strategy; these
requirements cover the pages that tie them together.

## Requirements

### Requirement: The authentication page is the entry guide
`/security/authentication` SHALL explain the strategy model in short, help the reader choose a strategy, show one
short working path for each built-in strategy, and link to the reference section of each. It MUST NOT present a
hand-written guard over `JwtService` as the main path.

#### Scenario: Reader choosing a strategy
- **WHEN** a reader opens `/security/authentication`
- **THEN** a table lists JWT, local, API key, session and OAuth2 with the client each one suits, where the credential
  travels, the cargo feature to enable and a link to its reference section

#### Scenario: Reader following one path
- **WHEN** a reader follows the JWT path of the guide
- **THEN** it shows the validator, its registration with `Provider::strategy`, `AuthGuard::new("jwt")` on a route
  and `User<T>` in the handler, and links to the JWT reference for the options

#### Scenario: Hand-written guard
- **WHEN** a reader looks for the `JwtService` guard the page used to build
- **THEN** it is described as an alternative with a link, after the strategy paths

### Requirement: The guide explains how strategies combine
The guide MUST document chains, optional guards, the default strategy, `#[public]` with `.skip_public()`, the order
of guards on a route, which user a handler sees when a session cookie and another credential arrive together, and
that an `AuthGuard` must come before any guard that reads the user.

#### Scenario: Session cookie and token together
- **WHEN** a reader looks for what happens when a request carries a session cookie and a bearer token
- **THEN** the guide states that the route's `AuthGuard` decides the user and shows the chain that accepts either

#### Scenario: Global guard and public routes
- **WHEN** a reader wants every route authenticated except a few
- **THEN** the guide shows a global `AuthGuard` built with `.skip_public()`, `#[public]` on the open routes, and
  links to the route posture page

### Requirement: The guide states the limits
The guide MUST say that `AuthGuard` denies WebSocket messages and how a gateway authenticates its handshake, and
MUST list what is not included (OpenID Connect ID token validation, CSRF tokens, multi-factor authentication,
account lockout, refresh-token storage) with a link to the tracking issue or page when one exists.

#### Scenario: Reader with a gateway
- **WHEN** a reader looks for authentication in WebSockets
- **THEN** the guide shows the handshake pattern, states that strategies do not run there, and links to the
  WebSocket guards page

### Requirement: The strategy reference is complete and marked documented
`/recipes/passport` SHALL have status `documented`, an introduction that describes the five built-in strategies as
available, and one index of the differences from the original that links to each strategy's own list. No page may
still say that strategies, sessions or password hashing are missing.

#### Scenario: Stale statements
- **WHEN** the documentation is searched for the sentences that announced strategies as pending or absent
- **THEN** none remains in English or Spanish

#### Scenario: Differences index
- **WHEN** a reader opens the differences index of `/recipes/passport`
- **THEN** each strategy has an entry that links to its list of deliberate differences

### Requirement: The authorization page builds on AuthGuard
`/security/authorization` SHALL show role checks as a guard that runs after `corvis::auth::AuthGuard` and reads the
authenticated user. It MUST NOT define a guard of its own named `AuthGuard`, and MUST warn that a guard reading the
user without an `AuthGuard` before it trusts whatever the session restored.

#### Scenario: Roles on a route
- **WHEN** a reader opens the role-based access control section
- **THEN** the sample lists `AuthGuard::new(..)` before the roles guard in `#[use_guards]` and reads the user through
  `User<T>` or `Request::user_as`

### Requirement: Migration page for Passport setups
The site SHALL have a page at `/recipes/passport-migration` and `/es/recipes/passport-migration`, status
`documented`, listed with the other migration recipes, that maps the classes, options and calls of the Node
authentication packages to Corvis and names the patterns with no equivalent.

#### Scenario: Strategy class
- **WHEN** a reader looks up a strategy class extending the original's passport strategy mixin with a `validate`
  method
- **THEN** the page shows the validator trait and `Provider::strategy` registration that replace it, for JWT and for
  local

#### Scenario: Guard overrides
- **WHEN** a reader looks up `handleRequest`, `getRequest` and `AuthGuard(['a', 'b'])`
- **THEN** the page maps them to `.optional()` / `.on_failure(..)`, to "not needed", and to `AuthGuard::chain`

#### Scenario: Session calls
- **WHEN** a reader looks up `serializeUser`, `deserializeUser`, `req.logIn`, `req.logout` and `passport.session()`
- **THEN** the page maps each to `SessionSerializer`, `AuthGuard::..log_in()`, `Authenticator::log_out` and the
  `"session"` strategy

#### Scenario: Pattern with no equivalent
- **WHEN** a reader looks up flash messages, `failureRedirect`, `passReqToCallback` or the `property` option
- **THEN** the page says each is not provided and what to do instead

### Requirement: The migration page lists every deliberate difference
The migration page MUST list every behavior that differs from the original on purpose, grouped by strategy, with
what the original does, what Corvis does and why. Every case marked as a divergence in the recorded fixtures under
`crates/corvis/tests/fixtures/auth/` MUST be covered by an entry.

#### Scenario: Token without expiry
- **WHEN** a reader migrating tokens that have no `exp` claim reads the JWT differences
- **THEN** the entry says they are refused by default and names the opt-out with its startup warning

#### Scenario: Fixture divergence without an entry
- **WHEN** the divergence marks of the fixtures are listed and compared with the page
- **THEN** every mark corresponds to an entry

### Requirement: The migration page explains running both stacks side by side
The migration page MUST explain how a Node application and a Corvis application share state during a migration:
session cookies and records, password hashes with rehash on login, token secrets and their minimum length, and the
order in which to move routes.

#### Scenario: Shared sessions
- **WHEN** a reader wants users to stay logged in while routes move one by one
- **THEN** the page gives the cookie name, the secret rule, the record format and what the store must be

#### Scenario: Password hashes
- **WHEN** a reader has hashes made by `bcryptjs`
- **THEN** the page shows that they verify as they are and how `needs_rehash` upgrades them on login

### Requirement: Security guidance page for application authors
The site SHALL have a page at `/security/authentication-security` and `/es/security/authentication-security`,
status `documented`, in the Security section of the sidebar, with a deployment checklist, the application's
responsibilities per strategy, and the common mistakes with their fix.

#### Scenario: Deployment checklist
- **WHEN** a reader opens the checklist
- **THEN** it covers at least HTTPS and cookie flags, secret length, storage and rotation, login rate limiting,
  credentials in URLs and logs, CSRF for cookie-borne credentials, session store choice, redirect targets and token
  lifetime

#### Scenario: Common mistakes
- **WHEN** a reader opens the common mistakes section
- **THEN** it covers an optional guard as the only guard, a guard that reads the user without an `AuthGuard` before
  it, a missing `.skip_public()`, a throttler placed after authentication, and linking accounts by an unverified
  e-mail

### Requirement: Guidance links to the threat model instead of restating it
Every item of the security guidance page that relies on a framework mitigation MUST link to the threat model, and
the page MUST NOT repeat its tables. Findings of the security review that were deferred MUST appear as guidance with
their interim workaround.

#### Scenario: Deferred finding
- **WHEN** the findings register of the threat model has an entry recorded as an issue
- **THEN** the guidance page tells the reader what to do until it is fixed and links to the entry

### Requirement: Samples match the example
Every code sample of the guide, the migration page and the security guidance page that shows Corvis code MUST be
taken from `examples/auth-app` or compile against the merged API, in both languages.

#### Scenario: Sample checked against the example
- **WHEN** a sample of the guide is compared with the file of `examples/auth-app` it names
- **THEN** the code is the same apart from elided lines marked with a comment

### Requirement: Authentication pages are linked and translated
The new pages SHALL be in the sidebar in both languages, and the guards, WebSocket guards, session, encryption and
hashing, rate limiting, examples and migration guide pages MUST link to the guide or to the page that covers their
topic. Spanish pages have the same headings, samples and status.

#### Scenario: From the migration guide
- **WHEN** a reader of `/migration-guide` looks for authentication
- **THEN** a row links to `/recipes/passport-migration`

#### Scenario: Docs checks
- **WHEN** `npm run docs:check` runs in `docs/`
- **THEN** it passes: page pairs, structure, sidebar, links, language and naming
