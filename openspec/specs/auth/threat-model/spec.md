# auth/threat-model Specification

## Purpose
The threat model of the authentication module: one document that states what is protected, from whom, by which
mechanism and which test proves it, kept next to the code it describes so that a change to a mitigation or a default
is visible.

## Requirements

### Requirement: Threat model document
The documentation site SHALL have a threat model page for authentication at
`/security/authentication-threat-model` and `/es/security/authentication-threat-model`, listed in the Security
section of the sidebar, with status `documented`.

#### Scenario: Spanish translation
- **WHEN** a reader opens `/es/security/authentication-threat-model`
- **THEN** the same content is shown in Spanish with the same headings and tables, and the docs checks pass

### Requirement: The document states scope, assets, boundaries and attackers
The page MUST list the assets (credentials, signing secrets, sessions, tokens, user identity), the trust boundaries
(browser, reverse proxy, application, session store, OAuth2 provider, logs), the attackers considered, and what is
out of scope with the reason.

#### Scenario: Reader checking an assumption
- **WHEN** a reader looks for whether a compromised session store is in scope
- **THEN** the page answers it under the trust boundaries, with what an attacker who reads the store obtains

### Requirement: Every threat has a mitigation, a test and a residual risk
For each surface (strategy core, JWT, passwords and local login, API keys, signed cookies and sessions, OAuth2,
combinations of strategies, guard order and public routes, WebSocket and Socket.IO handshakes, login rate limiting)
the page MUST have a table of threats with the mitigation, the test file or spec scenario that verifies it, and the
residual risk.

#### Scenario: Threat without a framework mitigation
- **WHEN** a threat is left to the application or is not mitigated
- **THEN** its row says so, names the issue that tracks it when there is one, and states what the application must do

#### Scenario: Interaction threats
- **WHEN** a reader opens the table for combinations of strategies
- **THEN** it covers at least session with token guards, chain fallthrough, shared credential places, OAuth2 state
  with session login, and the session user on routes without an `AuthGuard`

### Requirement: Test references resolve
A test SHALL read the English threat model page and fail when a test file it references does not exist in the
repository.

#### Scenario: Renamed test file
- **WHEN** a test file named in the page is renamed without updating the page
- **THEN** `cargo test` fails and names the missing path

### Requirement: Defaults table
The page MUST contain one table of every security-relevant default of the module (value, where it is set, the
original's value when it differs), in a form the defaults test of `auth/security-invariants` reads.

#### Scenario: Default changed in code only
- **WHEN** the default idle timeout of sessions is changed in the code and not in the table
- **THEN** the defaults test fails and names the row

### Requirement: Findings register
The page MUST list every finding of the cross-cutting review with an identifier, severity, the affected components,
and its outcome: fixed in this change with the regression test, or recorded as an issue with its number and the
interim guidance. Findings of the earlier per-change reviews are linked, not copied.

#### Scenario: Finding fixed in the change
- **WHEN** a reader looks up a fixed finding
- **THEN** the row names the regression test that fails without the fix

#### Scenario: Finding deferred
- **WHEN** a finding was not fixed in this change
- **THEN** the row has the issue number, the severity and what an application should do until it is fixed

### Requirement: Findings are triaged by a written rule
A finding SHALL be fixed in this change when its fix stays inside the authentication modules and changes no
requirement of another capability. Any other finding MUST be recorded as an issue. A finding of high or critical
severity that is deferred MUST be reported to the maintainer before the change is archived.

#### Scenario: Fix that needs a router change
- **WHEN** a finding can only be fixed by changing how the router runs guards
- **THEN** it is recorded as an issue labeled for security, and the register links it

#### Scenario: Fix that changes a specified behavior
- **WHEN** a fix inside the authentication modules changes a requirement of an archived change
- **THEN** this change gains a `MODIFIED` delta for that requirement before the fix is merged
