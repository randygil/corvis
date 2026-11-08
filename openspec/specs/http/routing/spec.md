# http/routing Specification

## Purpose
How route paths are built and matched: the global prefix and its exclusions, module middleware route patterns under
URI versioning, and optional route parameters.

## Requirements

### Requirement: Global prefix exclusions match the full route path
`set_global_prefix_with_exclude(prefix, exclude)` SHALL compare each exclusion pattern with the full path of each
route without the prefix and without the version segment (controller path joined with the handler path), using the
same pattern rules as `for_routes` (literals, `:param`, wildcards, ASCII case-insensitive). A route whose path matches
an exclusion MUST be published without the prefix; every other route MUST be published with it.

#### Scenario: Excluding a single handler
- **WHEN** the prefix is `api` with exclusion `status`, and a controller without a path has `#[get("status")]` and
  `#[get("users")]`
- **THEN** `GET /status` and `GET /api/users` answer, and `GET /api/status` and `GET /users` answer 404

#### Scenario: Excluding a whole controller
- **WHEN** the prefix is `api` with exclusion `health/*`, and `#[controller("health")]` has `#[get]` and
  `#[get("live")]`
- **THEN** `GET /health` and `GET /health/live` answer without the prefix

#### Scenario: Exclusion naming only the controller path
- **WHEN** the prefix is `api` with exclusion `health`, and `#[controller("health")]` has `#[get]` and `#[get("live")]`
- **THEN** `GET /health` answers without the prefix and `GET /api/health/live` answers with it

### Requirement: Middleware route patterns cover URI versions
When URI versioning is enabled, a text pattern given to `for_routes` or `exclude` of module middleware SHALL match a
route published at `<prefix>/<version prefix><version>/<pattern>` for every version the application publishes, in
addition to the unversioned path `<prefix>/<pattern>`.

#### Scenario: Middleware on a versioned route
- **WHEN** URI versioning is enabled with default version `1`, `#[controller("cats")]` has `#[get]`, and a module
  applies a middleware with `for_routes("cats")`
- **THEN** a request to `GET /v1/cats` runs the middleware

#### Scenario: Excluding a versioned route
- **WHEN** URI versioning is enabled, a middleware is applied with `.exclude("cats").for_routes("*")`, and
  `#[controller("cats")]` is published as version `2`
- **THEN** a request to `GET /v2/cats` does not run the middleware

#### Scenario: Neutral routes
- **WHEN** URI versioning is enabled, `#[controller("health")]` is marked `VERSION_NEUTRAL`, and a middleware is
  applied with `for_routes("health")`
- **THEN** a request to `GET /health` runs the middleware

### Requirement: Optional route parameters
A route path segment written as `:name?` SHALL make that segment optional: the route MUST answer both with and
without the segment, and when the segment is absent the parameter MUST be missing (a `#[param("name")]` of type
`Option<T>` receives `None`). A path with optional segments MUST NOT be registered as if the parameter were required.

#### Scenario: Request without the optional segment
- **WHEN** `#[controller("cats")]` has `#[get(":id?")]` with `#[param("id")] id: Option<String>`
- **THEN** `GET /cats` answers with `id` equal to `None`, and `GET /cats/7` answers with `id` equal to `Some("7")`

#### Scenario: Optional segment in the middle
- **WHEN** a route is declared as `#[get("files/:dir?/list")]`
- **THEN** both `GET /files/list` and `GET /files/docs/list` reach the handler

#### Scenario: Static routes keep priority
- **WHEN** a controller declares `#[get("cats/count")]` and `#[get("cats/:id?")]`
- **THEN** `GET /cats/count` reaches the `count` handler, and `GET /cats` and `GET /cats/7` reach the optional route
