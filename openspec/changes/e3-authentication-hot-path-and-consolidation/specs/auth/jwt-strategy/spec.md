## ADDED Requirements

### Requirement: Key and token work is done once
The strategy SHALL build its verification settings once per key family when it is built, and check a token's header
once per request, before the key provider. A `JwtKey` SHALL parse its key material at most once, shared by its clones,
so a key provider that returns a clone of a cached key costs no parse. The length check of an HMAC secret a provider
returns MUST still run on every request.

#### Scenario: Cached provider key
- **WHEN** a key provider returns clones of one RSA `JwtKey` for 100 requests with valid tokens
- **THEN** the key is parsed once and every request is authenticated

#### Scenario: Header checked once
- **WHEN** a well-formed token goes through the strategy
- **THEN** its header is checked exactly once, and a token whose header has `crit` never reaches the key provider

#### Scenario: Short secret from a cache
- **WHEN** a key provider returns clones of one cached 31-byte secret
- **THEN** every request fails with the reason `key provider error`
