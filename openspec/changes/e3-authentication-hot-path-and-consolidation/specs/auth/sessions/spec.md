## MODIFIED Requirements

### Requirement: Session store contract
The framework SHALL define a `SessionStore` trait with `load`, `create`, `update`, `touch` and `destroy`, and the
middleware SHALL use the store given in the options for every operation. `create` MUST fail when the identifier
already exists. `touch` has no default: every store implements it and it MUST change only the expiry and the
timestamps, never the data; the record it receives holds only `cookie` and `__corvis`. `destroy` MUST report whether it
removed a record.

#### Scenario: Custom store
- **WHEN** an application provides its own `SessionStore` implementation in the options
- **THEN** the middleware uses it for every operation

#### Scenario: Identifier collision
- **WHEN** `create` is called with an identifier the store already has
- **THEN** it fails and the existing record is unchanged

#### Scenario: Touch receives the timestamps
- **WHEN** a request loads a session with data and changes nothing
- **THEN** the store's `touch` receives a record whose only keys are `cookie` and `__corvis`

## ADDED Requirements

### Requirement: An unchanged session is not copied
A request that reads its session and changes nothing MUST NOT copy or serialize the session data beyond what the store
needs to load it: the size limit SHALL be checked by counting bytes, the baseline for change detection SHALL be taken on
the first write, and the unchanged session SHALL be touched with its timestamps only. The saving rules do not change.

#### Scenario: Read-only request with a large session
- **WHEN** a request reads one value of a 64 KiB session and changes nothing
- **THEN** the bytes the session middleware allocates beyond the store's parse of the record are at most 4 KiB, and the
  store receives a `touch`

#### Scenario: Size limit at the boundary
- **WHEN** a stored record is exactly `max_session_bytes` long, and then one byte longer
- **THEN** the first loads as a session and the second is no session; a write of the same two sizes is saved and refused

#### Scenario: A value set to what it was
- **WHEN** a handler sets a key of a loaded session to the value it already has
- **THEN** the session is touched, not updated

### Requirement: An identifier is drawn only when needed
The identifier of a new session SHALL be drawn from the operating system's generator only when the session is written,
when `Session::id()` is called, or when the OAuth2 binding of a session that will be kept needs it. A request without a
session cookie that saves nothing MUST draw none. A failed draw MUST write nothing.

#### Scenario: Anonymous request
- **WHEN** a request without a session cookie reaches a handler that does not touch the session, with the default options
- **THEN** no identifier is drawn and no cookie is set

#### Scenario: First write
- **WHEN** a request without a session cookie sets a value
- **THEN** one identifier is drawn, of 32 base64url characters, and the response sets a cookie for it

#### Scenario: Failed draw
- **WHEN** the generator fails while a new session is written
- **THEN** the response is the 500 of a session failure and nothing is written to the store

### Requirement: The memory store purges in bounded batches
A write to `MemorySessionStore` SHALL remove expired records in batches of at most 64: one batch when it starts, and,
while it makes room for its record, further batches only until the record fits. Expired records MUST still go before
any live record is evicted.

#### Scenario: Store full of expired records
- **WHEN** a store with `max_sessions(10000)` holds 10 000 expired records and one session is created
- **THEN** the new session is stored and at least 9 872 of the expired records are still held

#### Scenario: Expired before live
- **WHEN** a full store holds live and expired records and a new session needs room
- **THEN** no live record is evicted while an expired one remains
