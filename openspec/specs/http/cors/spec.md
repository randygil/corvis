# http/cors Specification

## Purpose
CORS keeps the semantics of the `cors` package, but the configuration that lets any website make credentialed
requests is reported when the application starts.

## Requirements

### Requirement: Warn at startup when the origin is reflected with credentials
When an application initializes with CORS options whose `origin` is `CorsOrigin::Reflect` and whose `credentials` is
`true`, Corvis SHALL log exactly one `warn` message with the context `CorsOptions` during `init()`, before the server
accepts connections, whether the options came from `enable_cors(..)` or from `CorvisApplicationOptions::cors`.

#### Scenario: Reflection with credentials
- **WHEN** an application calls `app.enable_cors(CorsOptions::new().reflect_origin().credentials(true))` and then
  `listen_in_background(..)`
- **THEN** exactly one `warn` message with context `CorsOptions` containing `reflect_origin()` and `credentials(true)`
  is logged before the call returns

#### Scenario: Options given at creation
- **WHEN** an application is created with `CorvisApplicationOptions { cors: Some(CorsOptions::new().reflect_origin().credentials(true)), .. }`
  and initialized
- **THEN** the same warning is logged once

#### Scenario: Initialized twice
- **WHEN** `init()` is called again after a successful `init()`, or a test handles several requests
- **THEN** no additional warning is logged

### Requirement: The warning explains the risk and the fix
The CORS warning MUST name `reflect_origin()` and `credentials(true)`, state that any website can make credentialed
requests, recommend an explicit origin list and contain the URL
`https://corvis.randygil.com/security/cors#credentials-and-open-origins`.

#### Scenario: Message content
- **WHEN** the warning is logged for `CorsOptions::new().origin_value(CorsOrigin::Reflect).credentials(true)`
- **THEN** the message contains `reflect_origin()`, `credentials(true)`, `origins([...])` and the docs URL

### Requirement: No warning for safe or undecidable configurations
Corvis MUST NOT log the CORS warning when `credentials` is `false`, when the origin is `Any`, `Fixed`, `List` (exact
strings or regexes), `Disabled` or `Dynamic`, or when CORS is not enabled.

#### Scenario: Explicit origin list with credentials
- **WHEN** an application enables `CorsOptions::new().origins(["https://app.example.com"]).credentials(true)` and
  initializes
- **THEN** no warning with context `CorsOptions` is logged

#### Scenario: Reflection without credentials
- **WHEN** an application enables `CorsOptions::new().reflect_origin()` and initializes
- **THEN** no warning with context `CorsOptions` is logged

#### Scenario: Origin function
- **WHEN** an application enables `CorsOptions::new().origin_fn(|_| CorsOrigin::Reflect).credentials(true)` and
  initializes
- **THEN** no warning with context `CorsOptions` is logged

### Requirement: The warning does not change CORS responses
With the warning logged, the application SHALL start and every CORS response MUST be identical to the one produced
without it (the `cors` package behavior: the request origin in `Access-Control-Allow-Origin`, `Vary: Origin` and
`Access-Control-Allow-Credentials: true`).

#### Scenario: Reflected response unchanged
- **WHEN** an application with `reflect_origin().credentials(true)` receives `GET /health` with
  `Origin: http://z.io`
- **THEN** the response has `Access-Control-Allow-Origin: http://z.io`, `Vary: Origin` and
  `Access-Control-Allow-Credentials: true`
