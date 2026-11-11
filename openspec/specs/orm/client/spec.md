# orm/client Specification

## Purpose
The typed surface of the corvis-orm client: every documented delegate operation has a typed form, DTOs can be the
data of a write, and relation counts have typed helpers and results.

## Requirements

### Requirement: Every documented delegate operation is typed

`Delegate<M>` SHALL expose `create_many_and_return` and `update_many_and_return`, running the engine actions
`createManyAndReturn` and `updateManyAndReturn` and returning `Vec<M>`, with the same arguments, results and errors as
the same actions run through `OrmClient::query_json`, on SQLite and Postgres.

#### Scenario: Create many and return them
- **WHEN** `db.user().create_many_and_return(json!({ "data": [{ "email": "a@x.io" }, { "email": "b@x.io" }] }))` is awaited
- **THEN** it returns a `Vec<User>` with two users whose `id` and defaults are filled in, equal to the JSON result of
  `query_json(User::meta(), "createManyAndReturn", ...)` deserialized into `Vec<User>`

#### Scenario: Update many and return them
- **WHEN** `db.post().update_many_and_return(json!({ "where": { "published": false }, "data": { "published": true } }))`
  is awaited
- **THEN** it returns every updated `Post` with `published == true`

### Requirement: A serializable DTO can be the data of a write

`corvis-orm` SHALL provide `Data<T>` (a tuple struct over `T: Serialize`, also accepting `&T`) that converts into the
arguments of `create` (`{ data: <T as JSON> }`) and into the data of `.data(...)`, `.create(...)` and `.update(...)`
builders. Fields that `T` omits when serializing (for example `Option` fields with `skip_serializing_if`) MUST NOT be
sent. When `T` fails to serialize, the query MUST fail with a validation `OrmError` naming the type, without running
SQL.

#### Scenario: Create from a DTO
- **WHEN** `db.user().create(Data(&CreateUserDto { email: "a@x.io".into(), name: None }))` is awaited and the DTO skips
  `None` fields
- **THEN** the user is created exactly as with `create(json!({ "data": { "email": "a@x.io" } }))`

#### Scenario: Partial update from a DTO
- **WHEN** `db.user().update(user::id::equals(1)).data(Data(&UpdateUserDto { email: None, name: Some("Ana".into()) }))`
  is awaited
- **THEN** only `name` changes

### Requirement: Relation counts have typed helpers and results

For every to-many relation field `f` of a model `M`, `#[derive(Model)]` SHALL generate `<m>::f::count()`, an
`IncludeParam<M>` that requests `_count: { select: { f: true } }`. Adding several count includes to one query MUST
merge them into one `_count.select` object. The derive SHALL also generate `<m>::Count`, a struct with one
`Option<i64>` field per to-many relation (`#[serde(default)]`, absent counts are `None`), and
`<m>::WithCount = corvis_orm::WithCount<M, <m>::Count>`, where `WithCount<M, C>` deserializes and serializes the
model's fields flattened followed by `_count`.

#### Scenario: Count posts per user
- **WHEN** `db.user().find_many(()).include(user::posts::count()).exec_as::<Vec<user::WithCount>>()` runs for a user
  with three posts
- **THEN** the item has `model.id` set and `count.posts == Some(3)`

#### Scenario: Two counts merge
- **WHEN** a query adds `user::posts::count()` and `user::comments::count()`
- **THEN** the arguments sent to the engine contain one `_count` with `select: { posts: true, comments: true }`

#### Scenario: Same JSON as the untyped query
- **WHEN** the typed result of the first scenario is serialized with `serde_json::to_string`, for a model whose
  timestamp fields use `corvis_orm::DateTime`
- **THEN** it equals the string of the same query run with `.exec_json()`

### Requirement: Typed helpers never change a value into null
A value given to a generated helper (`equals`, `not`, `in_vec`, comparisons, `set`, arithmetic, list operations) that
cannot be serialized MUST make the query fail with a validation error before any SQL runs. A non-finite float MUST NOT
become `null`: it SHALL be sent as the original client sends it when that is a value, and be refused with a validation
error otherwise.

#### Scenario: NaN in a filter
- **WHEN** `score::equals(f64::NAN)` or `score::not(f64::NAN)` is used in a `find_many`, `update_many` or `delete_many`
- **THEN** the filter is not `IS NULL` or `IS NOT NULL`, and the rows read or changed are those recorded from the original
  client for the same filter, or the query fails with a validation error and changes nothing

#### Scenario: Value that fails to serialize
- **WHEN** a helper receives a value whose `Serialize` implementation returns an error
- **THEN** the query fails with a validation error naming the type, and no statement is sent

### Requirement: Conversions of caller input do not panic
Text SHALL become a `Decimal` only through `FromStr`, `TryFrom` or `Decimal::parse`, which return an error for invalid
text; no `From` conversion from text SHALL exist. A literal decimal SHALL be written with `decimal!`, checked when the
program is compiled. `RawSql::join` of an empty list MUST NOT panic: running the statement fails with a validation error.

#### Scenario: Invalid text from a request
- **WHEN** `"12,5".parse::<Decimal>()` or `Decimal::try_from("12,5")` runs
- **THEN** it returns an error and nothing panics

#### Scenario: Invalid literal
- **WHEN** a program contains `decimal!("1.2.3")`
- **THEN** it does not compile, and the compiler error names the invalid literal

#### Scenario: Empty join
- **WHEN** `_query_raw` runs a statement built with `RawSql::join` of an empty list
- **THEN** it fails with a validation error carrying the original's message, and no statement is sent

### Requirement: Arithmetic updates follow the recorded original on each dialect
`divide` by zero and an `increment` that leaves the range of an `Int` field SHALL give, on each dialect, the result, or
the error class, code and message, recorded from the original client on that dialect. The documentation SHALL list the
differences between the two dialects that remain.

#### Scenario: Divide by zero
- **WHEN** `update` sets `value: { divide: 0 }` on an `Int`, a `Float` and a `Decimal` field, on Postgres and on SQLite
- **THEN** each outcome equals the recording for that field type and dialect

#### Scenario: Integer overflow
- **WHEN** `update` sets `views: { increment: 1 }` on an `Int` field holding `2147483647`, on each dialect
- **THEN** the outcome equals the recording for that dialect
