---
status: documentado
---

# Generador CRUD

Casi todos los recursos de una API empiezan igual: un módulo, un controlador con las cinco operaciones (crear, listar, buscar, actualizar, borrar), un servicio, los DTOs de entrada y la entidad. El schematic `resource` de la CLI genera todo eso de una vez.

```bash
corvis g resource cats
# o: corvis generate res cats
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

`main.rs` recibe `mod cats;` y `AppModule` importa `CatsModule`. El resultado compila y responde de inmediato.

## Qué se genera

**El controlador** expone las rutas REST habituales bajo `/cats`:

| Método | Ruta | Handler | Respuesta |
|---|---|---|---|
| `POST` | `/cats` | `create(#[body] dto: CreateCatDto)` | La entidad creada (201) |
| `GET` | `/cats` | `find_all()` | Todas |
| `GET` | `/cats/:id` | `find_one(#[param("id", ParseIntPipe)] id: u32)` | Una, o 404 |
| `PATCH` | `/cats/:id` | `update(id, #[body] dto: UpdateCatDto)` | La entidad actualizada, o 404 |
| `DELETE` | `/cats/:id` | `remove(id)` | Vacío, o 404 |

**El servicio** guarda las entidades en memoria (`#[default] items: RwLock<Vec<Cat>>`) y devuelve `NotFoundException` cuando el id no existe. Es un punto de partida: reemplaza el almacenamiento por tu base de datos (por ejemplo con [corvis-orm](/data/corvis-orm)).

**Los DTOs** usan `#[derive(Validate)]`. El de actualización es la versión con todos los campos opcionales del de creación:

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

Para que se validen, registra `ValidationPipe` (por ejemplo `app.use_global_pipes(ValidationPipe::new())`); ver [Validación](/application/validation).

**La entidad** usa el nombre en singular (`cats` → `Cat`, `categories` → `Category`) con un `id: u32` y un `name: String`.

**El test** del controlador levanta el módulo con `Test::create_testing_module()`, crea un elemento y comprueba el 201, el 200 y el 404:

```bash
cargo test cats
```

## Opciones

- `--path <módulo>` genera el recurso en otra carpeta de `src/`.
- Si la carpeta ya tiene un `mod.rs`, el comando falla sin tocar nada.

`resource` no admite `--flat` ni `--no-spec`. Ver [Uso](/cli/usages) para el resto de schematics.
