# Spec Delta

## ADDED Requirements

### Requirement: Requests outside the documentation cost nothing
The middleware that `SwaggerModule::setup` and `scalar::mount` install SHALL pass a request it does not serve to the
next handler without allocating memory, comparing the request path with mount paths it lowercased at mount time.

#### Scenario: An API route with the Swagger UI mounted
- **WHEN** an application with the Swagger UI mounted at `docs` receives `GET /cats`
- **THEN** the middleware allocates nothing for that request, as measured by a counting allocator

#### Scenario: A route with the Scalar page mounted
- **WHEN** an application with `scalar::mount` at `/reference` receives `POST /api/cats`
- **THEN** the middleware allocates nothing for that request

### Requirement: Documentation bodies are built once
When no `patch_document_on_request` function is set, the JSON document, the YAML document, the Swagger UI init script
and pages, and the Scalar page SHALL each be serialized at most once per mount and served from shared bytes. With a
patch function they MUST be built per request, as the patched document depends on it.

#### Scenario: Two requests for the JSON document
- **WHEN** `GET /docs-json` is requested twice
- **THEN** both responses carry the same shared buffer

### Requirement: Served responses do not change
The status, headers and body of every response the documentation middleware serves SHALL be byte for byte those served
before the bodies were cached, for every route it serves, with mixed-case paths, trailing slashes and `HEAD`.

#### Scenario: Snapshot of the served routes
- **WHEN** the serving test requests the UI page, both init script paths, `-json`, `-yaml`, `LICENSE`, a redirected
  asset, a custom asset and the Scalar page on two mounts
- **THEN** each response equals the snapshot recorded from the code before this change

### Requirement: Documentation serving does not block the runtime
A custom Swagger UI asset SHALL be confined and read on a blocking thread, and a lazily built document SHALL be built
once on a blocking thread while concurrent first requests wait for the same result and other routes keep being served.

#### Scenario: Concurrent first requests
- **WHEN** 20 requests for `-json` arrive before the lazy document's factory returns, on a single-threaded runtime
- **THEN** the factory runs once, another route answers while it runs, and all 20 requests get the document

#### Scenario: Factory failure
- **WHEN** the lazy document's factory panics
- **THEN** that request answers status 500, the panic is logged, and the next request runs the factory again
