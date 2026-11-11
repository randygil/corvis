## ADDED Requirements

### Requirement: A repeated state cookie is no state
When a request carries more than one cookie with the name of the state cookie, the strategy MUST read no flow from
them, as the session and the JWT cookie extractor treat a repeated cookie: a callback is refused as one without a state,
before any request to the provider, and a login start begins from an empty set of flows.

#### Scenario: Planted second cookie
- **WHEN** a callback carries the valid state cookie of its flow and a second cookie with the same name
- **THEN** the response is the 401 of a callback without a state, and the provider receives no request

#### Scenario: One cookie
- **WHEN** a callback carries only the valid state cookie of its flow
- **THEN** the flow completes as before
