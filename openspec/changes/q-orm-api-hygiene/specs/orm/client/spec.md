# Spec Delta

## ADDED Requirements

### Requirement: Errors keep their cause and stay small
An `OrmError` SHALL occupy at most two machine words. An error produced from a driver error SHALL return that error from
`std::error::Error::source`, and its `Display` MUST NOT contain the connection URL's user, password or query parameters.
`code()`, `message()`, `meta()` and `name()` SHALL return what they returned before.

#### Scenario: Unmapped database error
- **WHEN** a model query fails with a Postgres error that has no client code
- **THEN** the error's `source()` downcasts to `sqlx::Error` and its SQLSTATE is readable, and its message is unchanged

#### Scenario: Size
- **WHEN** `std::mem::size_of::<OrmError>()` is evaluated
- **THEN** it is at most `2 * std::mem::size_of::<usize>()`

### Requirement: Database errors are mapped as the original maps them
A deadlock (SQLSTATE 40P01), a serialization failure (40001) and a statement timeout (57014) during a model query SHALL
produce the error class, code, meta and message recorded from the original client for the same case.

#### Scenario: Deadlock
- **WHEN** two interactive transactions lock two rows in opposite order and Postgres aborts one of them with 40P01
- **THEN** the aborted call fails with the recorded error

### Requirement: Public enums can grow
The public enums of corvis-orm (`OrmError`, `FieldKind`, `DefaultValue`, `RelationKind`, `ReferentialAction`, `Dialect`,
`SqlValue`, `RawArg`, `IsolationLevel`, `SortOrder`, `Rounding`) and the error structs SHALL be `#[non_exhaustive]`, so
that adding a variant or a field is not a breaking change. Internal variants SHALL NOT appear in the documentation.

#### Scenario: Exhaustive match outside the crate
- **WHEN** code outside corvis-orm matches one of these enums without a wildcard arm
- **THEN** it does not compile

### Requirement: Generated ids use the full randomness of their format
`cuid()`, `cuid(2)`, `nanoid()` and `ulid()` defaults SHALL draw every random character from the operating system's
generator, so that each random position can take every symbol of its alphabet. A failure of the generator MUST be an
error of the query, never a panic.

#### Scenario: Every symbol in every random position
- **WHEN** 10,000 ULIDs are generated
- **THEN** each of the 16 positions of the random part takes each of the 32 symbols at least once
