# Spec Delta

## ADDED Requirements

### Requirement: Issuer, audience and critical headers in JwtService
When an issuer or audience is configured, `JwtService::verify` SHALL refuse a token whose `iss` is missing, not a string or not equal to it, or whose `aud` is missing, neither a string nor a list of strings, or does not contain it. It MUST refuse any token whose header has `crit` or is not a JSON object readable by `serde_json::Value`. With none configured, claims are not checked.

#### Scenario: Claim missing or of another type
- **WHEN** an issuer and an audience are configured and the token has no `iss`, a numeric `iss`, a list `iss`, no `aud` or a numeric `aud`
- **THEN** `verify` fails

#### Scenario: Matching claims
- **WHEN** the token's `iss` equals the issuer and its `aud` is the audience or a list containing it
- **THEN** `verify` succeeds

#### Scenario: Critical header
- **WHEN** the token's header contains `crit`, even with a member such as `1e400` that a strict parser refuses
- **THEN** `verify` fails
