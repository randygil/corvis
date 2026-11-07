---
status: documented
---

# CRUD generator

Almost every resource in an API starts the same way: a module, a controller with the five operations (create, list, find, update, delete), a service, the input DTOs and the entity. The CLI's `resource` schematic generates all of that at once.

```bash
corvis g resource cats
# or: corvis generate res cats
```

```text
CREATE src/cats/mod.rs
CREATE src/cats/cats_module.rs
CREATE src/cats/cats_controller.rs
CREATE src/cats/cats_service.rs
CREATE src/cats/dto/mod.rs
CREATE src/cats/entities/mod.rs
UPDATE src/main.rs
UPDATE src/app_module.rs
```

`main.rs` gets `mod cats;` and `AppModule` imports `CatsModule`. The result compiles and responds right away.

## What gets generated

**The controller** exposes the usual REST routes under `/cats`:

| Method | Route | Handler | Response |
|---|---|---|---|
| `POST` | `/cats` | `create(#[body] dto: CreateCatDto)` | The created entity (201) |
| `GET` | `/cats` | `find_all()` | All of them |
| `GET` | `/cats/:id` | `find_one(#[param("id", ParseIntPipe)] id: u32)` | One, or 404 |
| `PATCH` | `/cats/:id` | `update(id, #[body] dto: UpdateCatDto)` | The updated entity, or 404 |
| `DELETE` | `/cats/:id` | `remove(id)` | Empty, or 404 |

**The service** keeps the entities in memory (`#[default] items: RwLock<Vec<Cat>>`) and returns `NotFoundException` when the id does not exist. It is a starting point: replace the storage with your database (for example with [corvis-orm](/data/corvis-orm)).

**The DTOs** use `#[derive(Validate)]`. The update DTO is the version of the create DTO with all fields optional:

```rust
#[derive(Debug, Clone, Deserialize, Validate)]
pub struct CreateCatDto {
    #[is_string]
    #[is_not_empty]
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, Validate)]
pub struct UpdateCatDto {
    #[is_optional]
    #[is_string]
    pub name: Option<String>,
}
```

For them to be validated, register `ValidationPipe` (for example `app.use_global_pipes(ValidationPipe::new())`); see [Validation](/application/validation).

**The entity** uses the singular name (`cats` → `Cat`, `categories` → `Category`) with an `id: u32` and a `name: String`.

**The test** for the controller boots the module with `Test::create_testing_module()`, creates an item and checks the 201, the 200 and the 404:

```bash
cargo test cats
```

## Options

- `--path <module>` generates the resource in another folder under `src/`.
- If the folder already has a `mod.rs`, the command fails without touching anything.

`resource` does not support `--flat` or `--no-spec`. See [Usage](/cli/usages) for the rest of the schematics.
