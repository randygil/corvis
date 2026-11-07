---
status: documentado
---

# Decoradores

Referencia de los atributos de `corvis-swagger`. Los de controlador y handler van debajo de `#[controller]` o de `#[get]`/`#[post]`...; los de propiedad, en los campos de un struct con `#[derive(ApiSchema)]`. Necesitan `use corvis_swagger::*;` en el archivo.

Los argumentos con nombre (`summary = "..."`) aceptan claves en snake_case o camelCase y se convierten a camelCase en el documento. Los valores son expresiones de Rust, excepto `type`, que recibe un tipo (`type = CatDto`, `type = [CatDto]`) o un nombre de tipo (`type = "string"`).

## Controlador y handler

| Atributo | Nivel | Descripción |
|---|---|---|
| `#[api_tags("a", "b")]` | Controlador, handler | Tags de la operación |
| `#[api_operation(summary, description, operation_id, deprecated, tags, external_docs)]` | Handler | Datos de la operación |
| `#[api_response(status = 418, ...)]` | Controlador, handler | Una respuesta con cualquier código |
| `#[api_default_response(...)]` | Controlador, handler | La respuesta `default` |
| `#[api_<estado>_response(...)]` | Controlador, handler | Atajos por código (ver abajo) |
| `#[api_param(name = "id", ...)]` | Controlador, handler | Parámetro de ruta |
| `#[api_query(name = "q", ...)]` | Controlador, handler | Parámetro de query |
| `#[api_header(name = "X-Tenant", ...)]` | Controlador, handler | Cabecera |
| `#[api_headers(json!([...]))]` | Controlador, handler | Varias cabeceras |
| `#[api_body(type = T, ...)]` | Handler | Cuerpo de la petición |
| `#[api_produces("application/pdf")]` | Controlador, handler | Tipos MIME de las respuestas |
| `#[api_consumes("multipart/form-data")]` | Controlador, handler | Tipos MIME del cuerpo |
| `#[api_bearer_auth]`, `#[api_basic_auth]`, `#[api_cookie_auth]` | Controlador, handler | Requisito de seguridad (nombre opcional) |
| `#[api_oauth2(["scope"], "nombre")]` | Controlador, handler | Requisito OAuth2 |
| `#[api_security("nombre", ["scope"])]` | Controlador, handler | Requisito de cualquier esquema |
| `#[api_extra_models(A, B)]` | Controlador, handler | Modelos añadidos a `components.schemas` |
| `#[api_extension("x-clave", valor)]` | Controlador, handler | Extensión `x-...` |
| `#[api_exclude_endpoint]` | Handler | Excluye la operación |
| `#[api_exclude_controller]` | Controlador | Excluye todo el controlador |
| `#[api_include_endpoint]` | Handler | Incluye la operación con `only_include_decorated_endpoints` |

## Atajos de respuesta

Cada uno equivale a `#[api_response(status = N, ...)]`:

| Código | Atributo |
|---|---|
| 100-103 | `api_continue_response`, `api_switching_protocols_response`, `api_processing_response`, `api_earlyhints_response` |
| 200-210 | `api_ok_response`, `api_created_response`, `api_accepted_response`, `api_non_authoritative_information_response`, `api_no_content_response`, `api_reset_content_response`, `api_partial_content_response`, `api_multi_status_response`, `api_already_reported_response`, `api_content_different_response` |
| 300-308 | `api_ambiguous_response`, `api_moved_permanently_response`, `api_found_response`, `api_see_other_response`, `api_not_modified_response`, `api_temporary_redirect_response`, `api_permanent_redirect_response` |
| 400-417 | `api_bad_request_response`, `api_unauthorized_response`, `api_payment_required_response`, `api_forbidden_response`, `api_not_found_response`, `api_method_not_allowed_response`, `api_not_acceptable_response`, `api_proxy_authentication_required_response`, `api_request_timeout_response`, `api_conflict_response`, `api_gone_response`, `api_length_required_response`, `api_precondition_failed_response`, `api_payload_too_large_response`, `api_uri_too_long_response`, `api_unsupported_media_type_response`, `api_requested_range_not_satisfiable_response`, `api_expectation_failed_response` |
| 418-456 | `api_i_am_a_teapot_response`, `api_misdirected_response`, `api_unprocessable_entity_response`, `api_locked_response`, `api_failed_dependency_response`, `api_precondition_required_response`, `api_too_many_requests_response`, `api_unrecoverable_error_response` |
| 500-508 | `api_internal_server_error_response`, `api_not_implemented_response`, `api_bad_gateway_response`, `api_service_unavailable_response`, `api_gateway_timeout_response`, `api_http_version_not_supported_response`, `api_insufficient_storage_response`, `api_loop_detected_response` |

## Propiedades de un modelo

En structs con `#[derive(ApiSchema)]` (ver [Tipos y parámetros](/es/openapi/types-and-parameters#dtos-con-derive-apischema)):

| Atributo | Descripción |
|---|---|
| `#[api_property(...)]` | Opciones de la propiedad |
| `#[api_property_optional(...)]` | Igual, con `required: false` |
| `#[api_response_property(...)]` | Igual, con `readOnly: true` |
| `#[api_hide_property]` | Excluye el campo |
| `#[api_schema(name = "...", ...)]` | En el struct: nombre del schema y otras claves |

## Funciones

Algunas piezas no tienen atributo propio y se usan como funciones del módulo `corvis_swagger::decorators`, que devuelven un `CustomDecorator`. Se aplican con `#[apply_decorators(...)]`:

| Función | Descripción |
|---|---|
| `zod_response::<Dto>(status, description)` | Código de estado + serialización con el schema del DTO + respuesta documentada |
| `zod_response_array::<Dto>(status, description)` | Lo mismo para una lista |
| `api_callbacks(vec![...])` | Callbacks de OpenAPI |

```rust
#[post]
#[apply_decorators(corvis_swagger::decorators::zod_response::<CatDto>(Some(201), None))]
fn create(&self, #[body] dto: CreateCatDto) -> Value { /* ... */ }
```

## Componer decoradores

Como todos los atributos producen un `CustomDecorator`, puedes agruparlos en una función y reutilizarla:

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

Ver también [Decoradores personalizados](/es/custom-decorators).
