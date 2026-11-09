# Spec Delta

## ADDED Requirements

### Requirement: Cats-app shows a request-scoped provider
The `cats-app` example SHALL register a request-scoped `RequestContext` provider that injects the current `Request` and
holds a request id taken from the `X-Request-Id` header or generated when the header is absent. An interceptor SHALL
read it through `Scoped<RequestContext>` and set the `X-Request-Id` response header, and at least one handler SHALL take
`Scoped<RequestContext>` as a parameter. The example's route posture checks MUST keep passing.

#### Scenario: Header echoed
- **WHEN** a client sends a request to a public cats-app route with `X-Request-Id: abc-123`
- **THEN** the response has `X-Request-Id: abc-123`

#### Scenario: Generated per request
- **WHEN** a client sends two requests without `X-Request-Id`
- **THEN** both responses have a non-empty `X-Request-Id` and the two values differ
