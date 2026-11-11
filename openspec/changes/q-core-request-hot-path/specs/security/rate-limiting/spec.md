# Spec Delta

## ADDED Requirements

### Requirement: The stored key is the digest of the documented key string
For every request it counts, `ThrottlerGuard` SHALL store the hit under the SHA-256 digest of the key string
`<name length>:<name>:` followed by the source marker and what that source produced (`b:<class>:<handler>:<tracker>`,
`t:`, `m:` or `n<id length>:<id>:` plus the generator's output), however the digest is computed, so counters, the
separation between throttlers and sources, and the storage bounds do not change.

#### Scenario: Digest computed in parts
- **WHEN** the property test generates throttler names (multi-byte included), class and handler names, trackers and
  generator outputs for the four key forms
- **THEN** the digest the guard stores equals the SHA-256 digest of the concatenated key string

#### Scenario: A crafted tracker still cannot reach another source's counter
- **WHEN** a tracker returns a string that spells another route's built-in key
- **THEN** the two requests are counted under different digests and neither is limited by the other's hits
