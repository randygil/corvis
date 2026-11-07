---
status: documented
---

# Decorators

Reference for the `corvis-swagger` attributes. Controller and handler attributes go below `#[controller]` or `#[get]`/`#[post]`...; property attributes go on the fields of a struct with `#[derive(ApiSchema)]`. They need `use corvis_swagger::*;` in the file.

Named arguments (`summary = "..."`) accept snake_case or camelCase keys and are converted to camelCase in the document. Values are Rust expressions, except `type`, which takes a type (`type = CatDto`, `type = [CatDto]`) or a type name (`type = "string"`).

## Controller and handler

| Attribute | Level | Description |
|---|---|---|
| `#[api_tags("a", "b")]` | Controller, handler | Operation tags |
| `#[api_operation(summary, description, operation_id, deprecated, tags, external_docs)]` | Handler | Operation data |
| `#[api_response(status = 418, ...)]` | Controller, handler | A response with any code |
| `#[api_default_response(...)]` | Controller, handler | The `default` response |
| `#[api_<status>_response(...)]` | Controller, handler | Shortcuts by code (see below) |
| `#[api_param(name = "id", ...)]` | Controller, handler | Path parameter |
| `#[api_query(name = "q", ...)]` | Controller, handler | Query parameter |
| `#[api_header(name = "X-Tenant", ...)]` | Controller, handler | Header |
| `#[api_headers(json!([...]))]` | Controller, handler | Several headers |
| `#[api_body(type = T, ...)]` | Handler | Request body |
| `#[api_produces("application/pdf")]` | Controller, handler | MIME types of the responses |
| `#[api_consumes("multipart/form-data")]` | Controller, handler | MIME types of the body |
| `#[api_bearer_auth]`, `#[api_basic_auth]`, `#[api_cookie_auth]` | Controller, handler | Security requirement (optional name) |
| `#[api_oauth2(["scope"], "name")]` | Controller, handler | OAuth2 requirement |
| `#[api_security("name", ["scope"])]` | Controller, handler | Requirement for any scheme |
| `#[api_extra_models(A, B)]` | Controller, handler | Models added to `components.schemas` |
| `#[api_extension("x-key", value)]` | Controller, handler | `x-...` extension |
| `#[api_exclude_endpoint]` | Handler | Excludes the operation |
| `#[api_exclude_controller]` | Controller | Excludes the whole controller |
| `#[api_include_endpoint]` | Handler | Includes the operation with `only_include_decorated_endpoints` |

## Response shortcuts

Each one is equivalent to `#[api_response(status = N, ...)]`:

| Code | Attribute |
|---|---|
| 100-103 | `api_continue_response`, `api_switching_protocols_response`, `api_processing_response`, `api_earlyhints_response` |
| 200-210 | `api_ok_response`, `api_created_response`, `api_accepted_response`, `api_non_authoritative_information_response`, `api_no_content_response`, `api_reset_content_response`, `api_partial_content_response`, `api_multi_status_response`, `api_already_reported_response`, `api_content_different_response` |
| 300-308 | `api_ambiguous_response`, `api_moved_permanently_response`, `api_found_response`, `api_see_other_response`, `api_not_modified_response`, `api_temporary_redirect_response`, `api_permanent_redirect_response` |
| 400-417 | `api_bad_request_response`, `api_unauthorized_response`, `api_payment_required_response`, `api_forbidden_response`, `api_not_found_response`, `api_method_not_allowed_response`, `api_not_acceptable_response`, `api_proxy_authentication_required_response`, `api_request_timeout_response`, `api_conflict_response`, `api_gone_response`, `api_length_required_response`, `api_precondition_failed_response`, `api_payload_too_large_response`, `api_uri_too_long_response`, `api_unsupported_media_type_response`, `api_requested_range_not_satisfiable_response`, `api_expectation_failed_response` |
| 418-456 | `api_i_am_a_teapot_response`, `api_misdirected_response`, `api_unprocessable_entity_response`, `api_locked_response`, `api_failed_dependency_response`, `api_precondition_required_response`, `api_too_many_requests_response`, `api_unrecoverable_error_response` |
| 500-508 | `api_internal_server_error_response`, `api_not_implemented_response`, `api_bad_gateway_response`, `api_service_unavailable_response`, `api_gateway_timeout_response`, `api_http_version_not_supported_response`, `api_insufficient_storage_response`, `api_loop_detected_response` |

## Model properties

On structs with `#[derive(ApiSchema)]` (see [Types and parameters](/openapi/types-and-parameters#dtos-with-derive-apischema)):

| Attribute | Description |
|---|---|
| `#[api_property(...)]` | Property options |
| `#[api_property_optional(...)]` | Same, with `required: false` |
| `#[api_response_property(...)]` | Same, with `readOnly: true` |
| `#[api_hide_property]` | Excludes the field |
| `#[api_schema(name = "...", ...)]` | On the struct: schema name and other keys |

## Functions

Some pieces have no attribute of their own and are used as functions of the `corvis_swagger::decorators` module, which return a `CustomDecorator`. They are applied with `#[apply_decorators(...)]`:

| Function | Description |
|---|---|
| `zod_response::<Dto>(status, description)` | Status code + serialization with the DTO's schema + documented response |
| `zod_response_array::<Dto>(status, description)` | The same for a list |
| `api_callbacks(vec![...])` | OpenAPI callbacks |

```rust
#[post]
#[apply_decorators(corvis_swagger::decorators::zod_response::<CatDto>(Some(201), None))]
fn create(&self, #[body] dto: CreateCatDto) -> Value { /* ... */ }
```

## Composing decorators

Since all attributes produce a `CustomDecorator`, you can group them in a function and reuse it:

```rust
use corvis::CustomDecorator;
use corvis_swagger::decorators::{api_bearer_auth, api_status_response};

fn authenticated() -> CustomDecorator {
    CustomDecorator::new()
        .and(api_bearer_auth(None))
        .and(api_status_response(401, json!({ "description": "Missing or invalid token" })))
}

#[get("me")]
#[use_guards(AuthGuard)]
#[apply_decorators(authenticated())]
fn me(&self, user: ChatUser) -> ChatUser { user }
```

See also [Custom decorators](/custom-decorators).
