# Spec Delta

## ADDED Requirements

### Requirement: The cache evicts its oldest live entry and keeps bounded bookkeeping
When storing an entry takes `CacheManager` over `max` entries, it SHALL evict the live entry stored first (a key stored
again while live keeps its place; a key stored after it expired or was deleted counts as new). Expired, deleted and
replaced entries MUST NOT be evicted in place of a live one, and the manager's internal bookkeeping MUST stay within a
constant multiple of `max` whatever the sequence of calls.

#### Scenario: A key stored again after expiring is not evicted at once
- **WHEN** with `max` 2, `a` and `x` are stored, `a` expires and is read, `y` is stored and then `a` is stored again
- **THEN** the cache holds `y` and `a`, and `x` was evicted

#### Scenario: Repeated expiry of one key
- **WHEN** one key is stored with a 1 ms ttl, read after it expired and stored again 10,000 times
- **THEN** the manager's bookkeeping holds at most `2 × max` items

### Requirement: The cache does not store a value it cannot serialize
`CacheManager::set` and `wrap` SHALL NOT store a value whose serialization fails: the key MUST then hold no entry (a
previous value is removed) and one error line SHALL name the key.

#### Scenario: A value that fails to serialize
- **WHEN** `set("k", value, None)` is called with a value whose `Serialize` implementation returns an error
- **THEN** `get_value("k")` returns `None` and an error line names `k`

### Requirement: A cron job that waits for completion never runs two callbacks at once
With `wait_for_completion`, a tick that starts while the job's callbacks run SHALL be skipped, also when two ticks start
at the same time, and the job MUST run again on its next tick after a callback panicked. The job's warnings and errors
SHALL go through the application's logger with context `Cron`.

#### Scenario: Two ticks at the same instant
- **WHEN** two `fire_on_tick` calls reach a `wait_for_completion` job at the same time
- **THEN** its callback runs once

#### Scenario: A callback panics
- **WHEN** a hand-built tick function panics and the job ticks again
- **THEN** the callback runs on the second tick

### Requirement: ConfigModule::for_feature runs its factory once
`ConfigModule::for_feature(factory)` SHALL call the factory once per application and use that value both for the
`ConfigService` namespace and for the `CONFIGURATION(<namespace>)` provider.

#### Scenario: Counting factory
- **WHEN** an application imports `ConfigModule::for_feature` with a factory that counts its calls
- **THEN** after `init()` the factory was called once and both lookups return the same value
