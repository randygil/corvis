# orm/db-push Specification

## Purpose
What `db push` (`OrmClient::db_push`, `db_push_models`, `db_push_force_reset` and `OrmModuleOptions::push`) creates, what
it refuses and how it fails. Introduced by the ORM quality change `q-orm-correctness` (epic
[#187](https://github.com/randygil/corvis/issues/187)).

## Requirements

### Requirement: db push creates the reference schema
For every model it is given whose table does not exist, `db_push` SHALL create the table with the columns, types,
nullability and defaults of the original's migration for the same models, the primary key over all `@id` or `@@id`
fields, unique and plain indexes, implicit many-to-many join tables, Postgres enum types and schemas, and foreign keys,
with the names that migration uses.

#### Scenario: Compound primary key
- **WHEN** `schema_sql` runs for a model with `@@id([a, b])`
- **THEN** the table's primary key constraint covers `"a", "b"`, and two rows that share `a` but not `b` can be created

#### Scenario: Implicit many-to-many
- **WHEN** `Category` and `Post` with an implicit many-to-many relation are pushed and a post is created with
  `categories: { create: [{ name: "tech" }] }`
- **THEN** the join table `_CategoryToPost` exists with its key, its `B` index and both cascading foreign keys, and the
  create succeeds

#### Scenario: Postgres enum
- **WHEN** a model with an enum field mapped to `role_t` is pushed on Postgres and a row is created with that field
- **THEN** the enum type `role_t` exists with the mapped values in schema order, and the create succeeds

#### Scenario: Native types and scalar lists on Postgres
- **WHEN** a model with `String @db.Uuid`, `DateTime @db.Date`, `Int[]` and `Role[]` fields is pushed on Postgres
- **THEN** the columns are `uuid`, `date`, `integer[]` and `role_t[]`, and filters on them succeed

#### Scenario: Catalog equal to the reference migration
- **WHEN** one database is built from the oracle's reference migration and another by `db_push` of the oracle models,
  on Postgres and on SQLite
- **THEN** their tables, columns, keys, indexes, foreign keys and enum types are listed identically

### Requirement: db push refuses what it cannot build
When a model uses a construct that the reference migration would create and `db_push` cannot express (a native type, an
index type or an index option outside the documented list), `db_push` MUST fail with a validation error that names the
model, the field and the construct, before it runs any statement.

#### Scenario: Unknown native type
- **WHEN** a model has a field with a native type outside the documented list and is pushed
- **THEN** `db_push` fails with a validation error naming the model, the field and the type, and no table was created

### Requirement: db push is atomic
`db_push_models` and `db_push_force_reset` SHALL run every statement, including the creation of the URL schema, in one
transaction on both dialects, so a failure leaves the database as it was.

#### Scenario: A statement fails midway
- **WHEN** a push of three models fails on the statement of the third one
- **THEN** none of the three tables exists afterwards and the error is returned

### Requirement: Existing tables are not altered
`db_push` SHALL NOT alter, drop or re-create a table that already exists; `db_push_force_reset` drops the tables of the
given models first. The documentation SHALL say so.

#### Scenario: Table pushed by an earlier version
- **WHEN** a table already exists with older column types and `db_push` runs
- **THEN** the table keeps its columns and the push succeeds for the tables that were missing
