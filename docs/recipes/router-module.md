---
status: no-disponible
---

# Módulo de rutas

Corvis **no tiene un módulo de rutas** que asigne un prefijo a todos los controladores de un módulo (y de sus módulos hijos) desde un árbol de rutas declarado aparte. La ruta de cada controlador se define en el propio controlador.

## Alternativas

**Incluir el prefijo en el controlador.** Es lo más directo y deja la ruta completa a la vista:

```rust
#[controller("admin/users")]
pub struct AdminUsersController { /* ... */ }

#[controller("admin/reports")]
pub struct AdminReportsController { /* ... */ }
```

Una constante evita repetir el prefijo:

```rust
pub const ADMIN: &str = "admin";

#[controller(path = [ADMIN])]
pub struct AdminController { /* ... */ }
```

**Prefijo global.** Para un prefijo común a toda la aplicación, `app.set_global_prefix("api")`, con `app.set_global_prefix_with_exclude("api", &["health"])` si algunas rutas deben quedar fuera. Ver [Prefijo global](/faq/global-prefix).

**Versionado.** Si el prefijo que buscas es una versión de la API (`/v1/...`), usa `app.enable_versioning(VersioningOptions::uri())` y `#[controller(path = "cats", version = "1")]`. Ver [Versionado](/http/versioning).

**Varias rutas para un controlador.** `#[controller(path = ["cats", "gatos"])]` registra las mismas rutas bajo los dos prefijos.
