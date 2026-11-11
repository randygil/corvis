# Spec Delta

## Purpose

How `migrate deploy` behaves when several processes run it against one database at the same time, and what checksum it
records. Introduced by the ORM quality change `q-orm-api-hygiene` (epic
[#187](https://github.com/randygil/corvis/issues/187)).

## ADDED Requirements

### Requirement: Concurrent deploys apply each migration once
On Postgres, `migrate_deploy` SHALL hold, for its whole run, the session advisory lock recorded from the original's
deploy, on a connection of its own bounded by `connect_timeout`, and SHALL read the migration history only after taking
it. It MUST release the lock when it ends, whether it succeeded or failed.

#### Scenario: Two replicas start together
- **WHEN** two `migrate_deploy` calls on the same directory and database start at the same time
- **THEN** both succeed, each migration is applied once, and the history holds one finished row per migration and no
  failed row

#### Scenario: Lock held elsewhere
- **WHEN** another session holds the recorded advisory lock and `migrate_deploy` starts
- **THEN** the deploy waits on that lock, and finishes after the other session releases it

#### Scenario: Failing migration
- **WHEN** a migration fails during a deploy
- **THEN** the deploy returns `P3018` and releases the lock, and a second deploy returns `P3009` instead of waiting

### Requirement: The recorded checksum is that of the script that ran
`migrate_deploy` SHALL run exactly the bytes it hashed for the history row, read once per migration. A migration file that
is not valid UTF-8 MUST fail the deploy with an initialization error naming the file, before its history row is written.

#### Scenario: File read once
- **WHEN** a migration is applied
- **THEN** the checksum stored in its history row is the SHA-256 of the script that was executed
