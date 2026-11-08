---
status: documented
---

# Seguridad

La seguridad se describe en dos partes, como en cualquier documento OpenAPI:

1. Los **esquemas** (cómo se autentica: JWT en `Authorization`, una API key, una cookie...) se definen una vez con `DocumentBuilder` y van a `components.securitySchemes`.
2. Los **requisitos** (qué operaciones exigen qué esquema) se declaran con atributos en los controladores y handlers.

Swagger UI usa ambas cosas para mostrar el botón *Authorize* y enviar las credenciales en cada prueba.

Documentar la seguridad no protege nada: para exigir un token usa un [guard](/es/guards).

## Esquemas

| Método de `DocumentBuilder` | Esquema por defecto | Nombre |
|---|---|---|
| `add_bearer_auth()` | `{ type: "http", scheme: "bearer", bearerFormat: "JWT" }` | `bearer` |
| `add_basic_auth()` | `{ type: "http", scheme: "basic" }` | `basic` |
| `add_api_key()` | `{ type: "apiKey", in: "header", name: "api_key" }` | `api_key` |
| `add_cookie_auth()` | `{ type: "apiKey", in: "cookie", name: "connect.sid" }` | `cookie` |
| `add_oauth2()` | `{ type: "oauth2", flows: {} }` | `oauth2` |

Cada uno tiene una variante `_with` para cambiar las opciones y el nombre:

```rust
let config = DocumentBuilder::new()
    .set_title("Tertulia")
    .set_version("1.0")
    .add_bearer_auth()
    .add_api_key_with(Some(json!({ "type": "apiKey", "in": "header", "name": "X-Api-Key" })), "api_key")
    .add_cookie_auth_with("sid", None, "session")
    .build();
```

Para cualquier otro esquema, `add_security(nombre, objeto)` lo escribe tal cual:

```rust
.add_security("openid", json!({ "type": "openIdConnect", "openIdConnectUrl": "https://auth.example.com/.well-known/openid-configuration" }))
```

## Requisitos por operación

```rust
#[controller("rooms")]
#[api_bearer_auth]                         // todas las rutas del controlador
pub struct RoomsController { /* ... */ }

#[routes]
impl RoomsController {
    #[get("public")]
    #[api_security("api_key")]             // además, la API key
    fn public(&self) -> Value { /* ... */ }
}
```

| Atributo | Requisito |
|---|---|
| `#[api_bearer_auth]` / `#[api_bearer_auth("jwt")]` | El esquema `bearer` (o el nombre indicado) |
| `#[api_basic_auth]` | `basic` |
| `#[api_cookie_auth]` | `cookie` |
| `#[api_oauth2(["read", "write"])]`, `#[api_oauth2(["read"], "mi-oauth")]` | `oauth2` con esos scopes |
| `#[api_security("nombre")]`, `#[api_security("nombre", ["scope"])]` | Cualquier esquema |
| `#[api_security(json!({ "a": [], "b": [] }))]` | Varios esquemas que deben cumplirse a la vez |

El nombre debe coincidir con el del esquema definido en `DocumentBuilder`.

## Requisitos globales

Para exigir un esquema en todo el documento sin anotar cada controlador:

```rust
let config = DocumentBuilder::new()
    .add_bearer_auth()
    .add_security_requirements("bearer", &[])
    .build();
```

`add_security_requirements_object(json!({ ... }))` acepta el objeto de requisitos completo.
