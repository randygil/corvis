---
status: documented
---

# Security

Security is described in two parts, as in any OpenAPI document:

1. The **schemes** (how authentication works: JWT in `Authorization`, an API key, a cookie...) are defined once with `DocumentBuilder` and go to `components.securitySchemes`.
2. The **requirements** (which operations require which scheme) are declared with attributes on controllers and handlers.

Swagger UI uses both to show the *Authorize* button and send the credentials on each try-out.

Documenting security does not protect anything: to require a token use a [guard](/guards).

## Schemes

| `DocumentBuilder` method | Default scheme | Name |
|---|---|---|
| `add_bearer_auth()` | `{ type: "http", scheme: "bearer", bearerFormat: "JWT" }` | `bearer` |
| `add_basic_auth()` | `{ type: "http", scheme: "basic" }` | `basic` |
| `add_api_key()` | `{ type: "apiKey", in: "header", name: "api_key" }` | `api_key` |
| `add_cookie_auth()` | `{ type: "apiKey", in: "cookie", name: "connect.sid" }` | `cookie` |
| `add_oauth2()` | `{ type: "oauth2", flows: {} }` | `oauth2` |

Each one has a `_with` variant to change the options and the name:

```rust
let config = DocumentBuilder::new()
    .set_title("Tertulia")
    .set_version("1.0")
    .add_bearer_auth()
    .add_api_key_with(Some(json!({ "type": "apiKey", "in": "header", "name": "X-Api-Key" })), "api_key")
    .add_cookie_auth_with("sid", None, "session")
    .build();
```

For any other scheme, `add_security(name, object)` writes it as is:

```rust
.add_security("openid", json!({ "type": "openIdConnect", "openIdConnectUrl": "https://auth.example.com/.well-known/openid-configuration" }))
```

## Per-operation requirements

```rust
#[controller("rooms")]
#[api_bearer_auth]                         // all the controller's routes
pub struct RoomsController { /* ... */ }

#[routes]
impl RoomsController {
    #[get("public")]
    #[api_security("api_key")]             // in addition, the API key
    fn public(&self) -> Value { /* ... */ }
}
```

| Attribute | Requirement |
|---|---|
| `#[api_bearer_auth]` / `#[api_bearer_auth("jwt")]` | The `bearer` scheme (or the name given) |
| `#[api_basic_auth]` | `basic` |
| `#[api_cookie_auth]` | `cookie` |
| `#[api_oauth2(["read", "write"])]`, `#[api_oauth2(["read"], "my-oauth")]` | `oauth2` with those scopes |
| `#[api_security("name")]`, `#[api_security("name", ["scope"])]` | Any scheme |
| `#[api_security(json!({ "a": [], "b": [] }))]` | Several schemes that must be satisfied at the same time |

The name must match that of the scheme defined in `DocumentBuilder`.

## Global requirements

To require a scheme across the whole document without annotating each controller:

```rust
let config = DocumentBuilder::new()
    .add_bearer_auth()
    .add_security_requirements("bearer", &[])
    .build();
```

`add_security_requirements_object(json!({ ... }))` accepts the complete requirements object.
