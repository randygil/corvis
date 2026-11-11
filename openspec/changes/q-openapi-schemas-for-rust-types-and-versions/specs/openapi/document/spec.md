# Spec Delta

## ADDED Requirements

### Requirement: Generic DTOs get one component per instantiation
The component name of a type with generic arguments SHALL be the short name of the base type followed by `_` and the
component name of each type argument (`Page<User>` is `Page_User`), with every character outside `[A-Za-z0-9._-]`
replaced by `_`. The name of a type without generic arguments MUST NOT change, and `#[api_schema(name = ..)]` MUST
take precedence.

#### Scenario: Two instantiations in one document
- **WHEN** an application documents responses of type `Page<User>` and `Page<Post>`
- **THEN** the document has the components `Page_User` and `Page_Post`, each response refers to its own, and no
  duplicate warning is logged

### Requirement: Generic DTOs bound their type parameters
`#[derive(ApiSchema)]` SHALL bound each type parameter used by a field with `ApiSchema`, unless
`#[api_schema(bound = "...")]` gives the bounds, so a field of a type parameter is documented as the type it is
instantiated with, whatever the order in which types are first registered.

#### Scenario: A wrapper over a scalar
- **WHEN** `Page<T>` derives `ApiSchema` with `#[api_schema(bound = "T: ::corvis_swagger::SwaggerType")]` and a
  response has type `Page<String>`
- **THEN** the component `Page_String` has `items` of type `string`

### Requirement: Foreign types can be described per document
`SwaggerDocumentOptions::with_type::<T>(schema)` SHALL make every property and parameter of type `T` that has no other
OpenAPI description use `schema` in that document. A type that still has no description SHALL be documented as an
empty object, as today, and the generator MUST log one warning per type and document naming the Rust type and the ways
to describe it.

#### Scenario: A decimal from another crate
- **WHEN** a document is created with `with_type::<rust_decimal::Decimal>(json!({ "type": "string", "format": "decimal" }))`
  and a DTO has `price: rust_decimal::Decimal`
- **THEN** the property is `{ type: "string", format: "decimal" }`

#### Scenario: An undescribed type
- **WHEN** a DTO field's type has no OpenAPI description and no `with_type` entry
- **THEN** the field refers to an empty object component and one warning names the type

### Requirement: Corvis keywords are written for the declared version
Keywords that Corvis adds and the original library does not (exclusive bounds of float fields, the nullability of
`Option` fields) SHALL be written in the form of the version in `doc.openapi`: `nullable: true` and boolean
`exclusiveMinimum` / `exclusiveMaximum` with the bound in `minimum` / `maximum` in 3.0; type arrays with `"null"` and
numeric exclusive bounds in 3.1. A document without such keywords MUST come out of the generator byte for byte as before.

#### Scenario: Nullable reference in 3.0
- **WHEN** a DTO has `owner: Option<UserDto>` in a `3.0.0` document
- **THEN** the property is `{ allOf: [{ $ref: "#/components/schemas/UserDto" }], nullable: true }`

#### Scenario: Nullable string in 3.1
- **WHEN** a DTO has `nick: Option<String>` in a `3.1.0` document
- **THEN** the property is `{ type: ["string", "null"] }`

#### Scenario: Recorded documents unchanged
- **WHEN** the documents of the recorded fixture are generated
- **THEN** their JSON text is identical with and without the finishing step

### Requirement: Forms inherited from the original are kept by default
By default the generator and `cleanup_open_api_doc` SHALL produce the same document as the original library for the
same application, including forms that are not valid for the declared version: a `nullable` response as `oneOf` with
`{ type: "null" }`, a `search` operation for routes on every method, `nullable` in a 3.1 document, and references the
zod cleanup does not rename.

#### Scenario: Nullable response
- **WHEN** a route has `#[api_ok_response(type = CatDto, nullable = true)]`
- **THEN** the response schema is `{ oneOf: [{ $ref: "#/components/schemas/CatDto" }, { type: "null" }] }`, as the
  recorded fixture shows for the original

#### Scenario: Route on every method
- **WHEN** a route is declared with `#[all("any")]`
- **THEN** its path item has the operations `get`, `post`, `put`, `delete`, `patch`, `options`, `head` and `search`

### Requirement: Strict cleanup makes the document valid for its version
`try_cleanup_open_api_doc` with `CleanupOptions { strict: true, .. }` SHALL rename references to renamed zod schemas
wherever they appear and in every operation, rewrite the inherited forms for the declared version, remove `search`
operations with a warning that lists them, and return `SwaggerError::DanglingRef` naming the JSON pointer when a
`$ref` does not resolve, or `SwaggerError::LeftoverMarker` when an internal zod key remains.

#### Scenario: Renamed DTO under items
- **WHEN** a zod DTO with `meta({ id: "Kitten" })` is the item type of an array response and the document is cleaned
  with `strict`
- **THEN** the response's `items.$ref` is `#/components/schemas/Kitten`

#### Scenario: Nullable response in 3.0
- **WHEN** a `3.0.0` document with a nullable response is cleaned with `strict`
- **THEN** the response schema has no `oneOf` with `{ type: "null" }` and is valid against the official OpenAPI 3.0
  schema

#### Scenario: Default cleanup unchanged
- **WHEN** the same document is cleaned without `strict`
- **THEN** it equals the recorded original output byte for byte

### Requirement: The document matches the original for the recorded app
For each document of the `document_forms` fixture, recorded by running the original generator and its zod integration
on the fixture's TypeScript app, the document Corvis generates for the equivalent app SHALL equal the recorded JSON text
byte for byte.

#### Scenario: Fixture replay
- **WHEN** `cargo test -p corvis-swagger --test document_forms` runs
- **THEN** each generated document's `to_json()` equals the recorded string

### Requirement: The duplicate component warning names the Rust types
When two different Rust types produce the same component name with different schemas, the generator SHALL keep the
original's behavior (the last definition is written) and log a warning that names both Rust type paths and
`#[api_schema(name = ..)]` as the way to rename one.

#### Scenario: Two DTOs with one name
- **WHEN** `users::CreateDto` and `posts::CreateDto` both appear in a document
- **THEN** the warning names `users::CreateDto`, `posts::CreateDto` and `#[api_schema(name = ..)]`

### Requirement: A served document with zod markers is reported
`SwaggerModule::setup` and `scalar::mount` with a document SHALL log one warning, at mount time, when the document
still holds internal zod keys, naming `cleanup_open_api_doc`. The document served MUST NOT change.

#### Scenario: Cleanup forgotten
- **WHEN** an application mounts the Swagger UI with a document of zod DTOs that was not cleaned
- **THEN** one warning naming `cleanup_open_api_doc` is logged and the document is served as given
