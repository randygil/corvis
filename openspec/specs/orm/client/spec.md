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
