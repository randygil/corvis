# Spec Delta

## ADDED Requirements

### Requirement: Concurrent first requests share one connection attempt
When several requests need the pool of a client that is not connected, they SHALL wait for a single attempt to open it,
holding no lock while they wait. Every waiter MUST receive the outcome of that attempt (the pool, or the same error), and
a request that arrives after a failed attempt has been delivered SHALL start a new one.

#### Scenario: Database down
- **WHEN** 50 queries start at the same time on a new client whose server accepts connections and never answers, with
  `connect_timeout=1&pool_timeout=0`
- **THEN** the server accepts at most 5 connections (one attempt and its retries), and every query fails with the same
  `P1001` error

#### Scenario: Database up
- **WHEN** 50 queries start at the same time on a new client whose server is reachable
- **THEN** the pool is opened once and every query succeeds

#### Scenario: Retry after a failure
- **WHEN** a first attempt has failed and been reported, the server becomes reachable, and a new query runs
- **THEN** the new query starts a new attempt and succeeds
