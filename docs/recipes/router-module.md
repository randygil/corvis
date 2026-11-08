---
status: unavailable
---

# Router module

Corvis **has no router module** that assigns a prefix to all the controllers of a module (and of its child modules) from a separately declared route tree. Each controller's route is defined in the controller itself.

## Alternatives

**Include the prefix in the controller.** This is the most direct option and keeps the full route in sight:

```rust
#[controller("admin/users")]
pub struct AdminUsersController { /* ... */ }

#[controller("admin/reports")]
pub struct AdminReportsController { /* ... */ }
```

A constant avoids repeating the prefix:

```rust
pub const ADMIN: &str = "admin";

#[controller(path = [ADMIN])]
pub struct AdminController { /* ... */ }
```

**Global prefix.** For a prefix common to the whole application, `app.set_global_prefix("api")`, with `app.set_global_prefix_with_exclude("api", &["health"])` if some routes must stay out. See [Global prefix](/faq/global-prefix).

**Versioning.** If the prefix you want is an API version (`/v1/...`), use `app.enable_versioning(VersioningOptions::uri())` and `#[controller(path = "cats", version = "1")]`. See [Versioning](/http/versioning).

**Several routes for one controller.** `#[controller(path = ["cats", "gatos"])]` registers the same routes under both prefixes.
