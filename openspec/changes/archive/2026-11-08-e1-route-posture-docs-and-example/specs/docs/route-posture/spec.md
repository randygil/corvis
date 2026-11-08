# Spec Delta

## Purpose

Users can learn from the documentation site how route posture works, how to enable it, how to mark open routes, what
it cannot see and how to review it, in English and in Spanish.

## ADDED Requirements

### Requirement: Route posture page
The documentation site SHALL have a route posture page at `/security/route-posture` and `/es/security/route-posture`,
listed in the Security section of the sidebar in both languages, with status `documented`. Every code sample on it
MUST match the implemented API.

#### Scenario: Spanish translation
- **WHEN** a reader opens `/es/security/route-posture`
- **THEN** the same content is shown in Spanish with the same headings and code samples, and the docs checks pass

### Requirement: The page explains how to enable posture and open routes
The page MUST document the acceptance rule (guarded, public or exempt), `require_route_posture()` and
`set_route_posture()` with warn mode and exemptions with a reason, `#[public]` and `PUBLIC_METADATA` and their
interaction with global guards that read `"isPublic"`, and which guards count (`controls_access()`).

#### Scenario: Reader looking for how to open a route on purpose
- **WHEN** a reader opens `/security/route-posture`
- **THEN** the page shows `#[public]` on a route and on a controller, and explains that it writes the `"isPublic"`
  metadata existing guards already read

#### Scenario: Reader migrating an existing application
- **WHEN** a reader with an application that has unguarded routes reads the page
- **THEN** it explains how to start in warn mode, read the list, add guards, `#[public]` or exemptions, and switch to
  enforce mode

### Requirement: The page states the limits and the review tools
The page MUST document WebSocket gateways and SSE, the surfaces that are not checked (static files, raw `upgrade`
listeners, Swagger and Scalar UIs, middleware, gateway handshakes), the error message, `route_posture()` and
`corvis posture`.

#### Scenario: Reader relying on middleware authentication
- **WHEN** a reader whose routes are protected by middleware or a reverse proxy reads the page
- **THEN** it states that the check cannot see middleware and shows an exemption with a reason for that case

### Requirement: Pages that mention unprotected surfaces link to the route posture page
The guards page (its warning about what global guards do not cover and its public routes section), the WebSocket
guards page, the custom decorators page, the authentication page, the CLI usage page (`corvis posture`) and the common
errors page MUST link to the route posture page in both languages. The common errors page MUST have an entry for the
route posture error message.

#### Scenario: From the global guards warning
- **WHEN** a reader reads "What global guards do not cover" on the guards page
- **THEN** the warning links to the route posture page

#### Scenario: Error lookup
- **WHEN** a reader searches the common errors page for "Route posture check failed"
- **THEN** an entry explains the cause and links to the route posture page
