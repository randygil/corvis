# Spec Delta

## Purpose

The `cats-app` example uses the `#[public]` marker and enforces route posture.

## ADDED Requirements

### Requirement: Cats-app marks open routes with #[public] and enforces route posture
`cats_app::create_app()` SHALL enable route posture in enforce mode, its open routes, its SSE stream and its chat
gateway MUST be marked `#[public]` (with no `IS_PUBLIC_KEY` constant left), and its global guards that let requests
through without a decision MUST declare that they do not control access, so that the example starts and serves
requests exactly as before.

#### Scenario: Application starts in enforce mode
- **WHEN** the cats-app tests create the application with `create_app()` and call `init()`
- **THEN** `init()` succeeds and the report has no unprotected entry

#### Scenario: Only the authentication guard counts
- **WHEN** the report is read for a protected route such as `POST /api/v1/cats`
- **THEN** `AuthGuard` is the only guard that controls access; `ThrottlerGuard` and `RolesGuard` do not
