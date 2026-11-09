# Spec Delta

## ADDED Requirements

### Requirement: Postgres test databases and roles are unique per run and cleaned up

Every scratch database and role that the corvis-orm Postgres tests create on the shared server SHALL carry a per-run
suffix made of a timestamp, the process id and a random component (`orm_test_<name>_<run>` and `orm_<name>_<run>`),
within Postgres' 63-byte identifier limit, so two runs against one server never share a name. Tests MUST get these
names from the shared helper instead of building them by hand. A run SHALL drop the databases and roles it created when
its process exits normally, and the helper MUST drop leftover `orm_test_*` databases and `orm_*` roles of earlier runs
once they are older than 3 hours, judged by the timestamp in the name, never touching other objects.

#### Scenario: Two runs at the same time
- **WHEN** two `cargo test -p corvis-orm` runs use the same Postgres server at the same time
- **THEN** each creates and drops only its own databases and roles, and both pass

#### Scenario: A run ends normally
- **WHEN** a run of the Postgres tests finishes and every test passed
- **THEN** the databases and roles it created no longer exist on the server

#### Scenario: Leftovers of a dead run
- **WHEN** a Postgres test starts and the server holds an `orm_test_*` database or `orm_*` role whose run id is older
  than 3 hours
- **THEN** it is dropped, while objects with a recent run id or without a run id are left alone
