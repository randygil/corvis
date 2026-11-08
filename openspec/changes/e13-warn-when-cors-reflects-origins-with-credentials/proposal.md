# Proposal

Implements [randygil/corvis#134](https://github.com/randygil/corvis/issues/134) (epic
[#13](https://github.com/randygil/corvis/issues/13)).

## Why

`app.enable_cors(CorsOptions::new().reflect_origin().credentials(true))` copies every request's `Origin` into
`Access-Control-Allow-Origin` and adds `Access-Control-Allow-Credentials: true`. Any website the user visits can then
send requests with the user's cookies to the API and read the responses. It is what the `cors` package does with
`origin: true, credentials: true`, so Corvis keeps it for parity (decided in the compatible vs strict mode spike, #8),
but it is one of the most common CORS misconfigurations and today nothing points it out: the only hint is a warning
box in the CORS docs page.

## What Changes

- When the application initializes (`init()`, also reached through `listen()`, `listen_in_background()` and the
  testing utilities), Corvis logs one **warning** if the effective CORS options reflect the request origin
  (`reflect_origin()`, `origin_value(CorsOrigin::Reflect)`) and have `credentials(true)`. The message names both
  settings, says what they allow, suggests an explicit origin list and links to the CORS docs page.
- It is a warning, not an error: the application starts and answers exactly as before (parity with `cors`).
- No warning for explicit origins (`origin(..)`, `origins([..])`, `origin_regex(..)`), for `*` (browsers refuse
  credentialed responses with `*`), for `origin_fn` (its result cannot be known at startup) or without credentials.
- Docs (EN + ES): the CORS page turns the existing "`credentials` and open origins" box into a section with a stable
  anchor, mentions the startup warning, and keeps the advice about `origin_fn`.

## Capabilities

### New Capabilities

- `http/cors`: the startup warning for origin reflection combined with credentials.

### Modified Capabilities

None (no spec covers CORS yet).

## Impact

- `crates/corvis/src/router/cors.rs`: a crate-private check on `CorsOptions` returning the warning text.
- `crates/corvis/src/application.rs`: one call in `CorvisApplication::init()`.
- Tests: unit tests in `cors.rs`; new `crates/corvis/tests/cors_startup_warning.rs` with a capturing logger.
- Docs: `docs/security/cors.md`, `docs/es/security/cors.md`.
- Port apps: consi and facten use explicit origin lists with credentials, houseplan does not enable CORS; no warning
  and no HTTP change for any of them.

## Out of scope

- Turning the combination into an error or a strict-mode refusal (the strict mode itself is decided in #8).
- Other risky CORS settings: an unanchored `origin_regex` that also matches `https://example.com.evil.test`
  (`Regex::is_match`), a fixed or listed `"null"` origin, `origin_fn` that always returns `Reflect`; possible new issue.
- An option to acknowledge and silence the warning (see design, open questions).
- WebSocket origin checks (`GatewayOptions::allowed_origins`), which `enable_cors` does not affect.
