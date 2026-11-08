---
status: documented
---

# Referencia de la API

La referencia completa de tipos, traits y funciones se genera desde el código con `rustdoc`. Desde la raíz del repositorio:

```bash
cargo doc -p corvis -p corvis-orm -p corvis-swagger --no-deps --open
```

Los puntos de entrada más útiles:

| Crate | Módulo | Contenido |
|---|---|---|
| `corvis` | `corvis::prelude` | Lo que importa una aplicación típica (`use corvis::prelude::*;`). |
| `corvis` | `corvis::di` | `Provider`, `Token`, `DynamicModule`, `ModuleRef`, `ResolveCtx`, `Scope`. |
| `corvis` | `corvis::http` | `Request`, `Response`, `HttpException` y las excepciones incluidas. |
| `corvis` | `corvis::z` | Schemas y validación estilo zod. |
| `corvis` | `corvis::testing` | `Test`, `TestingModule`, `request`. |
| `corvis-orm` | `corvis_orm::prelude` | Modelos, consultas y `OrmModule`. |
| `corvis-swagger` | raíz | `DocumentBuilder`, `SwaggerModule`, atributos `#[api_*]`. |

Los atributos (`#[module]`, `#[injectable]`, `#[controller]`, `#[routes]`, `#[handlers]`, `#[websocket_gateway]`, `#[derive(Validate)]`) se documentan en el crate `corvis-macros` y se reexportan desde `corvis`.
