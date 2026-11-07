---
status: documented
---

# API reference

The complete reference of types, traits, and functions is generated from the code with `rustdoc`. From the repository root:

```bash
cargo doc -p corvis -p corvis-orm -p corvis-swagger --no-deps --open
```

The most useful entry points:

| Crate | Module | Contents |
|---|---|---|
| `corvis` | `corvis::prelude` | What a typical application imports (`use corvis::prelude::*;`). |
| `corvis` | `corvis::di` | `Provider`, `Token`, `DynamicModule`, `ModuleRef`, `ResolveCtx`, `Scope`. |
| `corvis` | `corvis::http` | `Request`, `Response`, `HttpException`, and the built-in exceptions. |
| `corvis` | `corvis::z` | Zod-style schemas and validation. |
| `corvis` | `corvis::testing` | `Test`, `TestingModule`, `request`. |
| `corvis-orm` | `corvis_orm::prelude` | Models, queries, and `OrmModule`. |
| `corvis-swagger` | root | `DocumentBuilder`, `SwaggerModule`, `#[api_*]` attributes. |

The attributes (`#[module]`, `#[injectable]`, `#[controller]`, `#[routes]`, `#[handlers]`, `#[websocket_gateway]`, `#[derive(Validate)]`) are documented in the `corvis-macros` crate and re-exported from `corvis`.
