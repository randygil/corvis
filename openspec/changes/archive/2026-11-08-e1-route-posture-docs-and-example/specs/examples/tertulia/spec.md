# Spec Delta

## Purpose

The `tertulia` example shows route posture enforced on a real application with REST, SSE, a gateway, a raw WebSocket
and static files.

## ADDED Requirements

### Requirement: Tertulia enforces route posture
`tertulia::create_app()` SHALL enable route posture in enforce mode, and every route and gateway message handler of
the example MUST be guarded or marked `#[public]`, so that the example starts and serves requests exactly as before.

#### Scenario: Application starts in enforce mode
- **WHEN** the tertulia test environment creates the application with `create_app()` and calls `init()`
- **THEN** `init()` succeeds

#### Scenario: Behavior unchanged
- **WHEN** the existing tertulia tests run against the application created by `create_app()`
- **THEN** they pass without changes to their assertions

### Requirement: Tertulia's report matches its routes
After `init()`, the posture report of tertulia MUST list `POST /api/auth/login`, `GET /api/rooms`,
`GET /api/rooms/:room/messages`, `GET /api/rooms/:room/members`, `GET /api/stats` and the SSE route
`/api/stats/stream` as public; `GET /api/auth/me`, `POST /api/rooms` and `POST /api/rooms/:room/announcements` and
every `/chat` message handler as protected; and the `/chat` handshake and the raw `upgrade` listener as not checked.
No entry may be unprotected or exempt.

#### Scenario: Reading the report in a test
- **WHEN** a tertulia test reads `app.route_posture()` after `init()`
- **THEN** the statuses are exactly those listed above
