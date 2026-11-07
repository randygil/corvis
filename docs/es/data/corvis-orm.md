---
status: documentado
---

# corvis-orm

`corvis-orm` es el ORM del workspace. Está construido sobre `sqlx` 0.8, soporta **PostgreSQL** y **SQLite**, y se integra con la inyección de dependencias de Corvis mediante `OrmModule`, `OrmService` y `OrmExceptionFilter`. Su cliente acepta los argumentos de consulta en JSON (`where`, `include`, `select`, `orderBy`, ...) o con helpers tipados generados para cada modelo, y es compatible con proyectos que describen su base con un archivo `schema.prisma`: puede generar los modelos de Rust a partir de él y aplicar sus migraciones SQL.

Este capítulo cubre la integración con Corvis y lo esencial del cliente. La referencia completa del ORM (todos los filtros, escrituras anidadas, SQL crudo, `Decimal`) está en `crates/corvis-orm/README.md`.

## Instalación

Agrega el crate junto a `corvis`. La feature `corvis` (activa por defecto) incluye el módulo para la inyección de dependencias, y `tls-rustls` (también por defecto) habilita TLS para Postgres:

```toml
[dependencies]
corvis = { path = "../../crates/corvis" }
corvis-orm = { path = "../../crates/corvis-orm" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

(Las rutas son las de los ejemplos del repositorio; ajústalas a tu proyecto.)

## Modelos

Un modelo es un struct con `#[derive(Model)]`. Este es el de las salas de `examples/tertulia`:

```rust
use corvis_orm::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[serde(rename_all = "camelCase")]
#[model(table = "rooms")]
pub struct Room {
    #[id]
    pub name: String,
    pub topic: String,
    #[default(50)]
    pub max_members: i64,
    pub created_by: String,
    #[default(now)]
    pub created_at: corvis_orm::DateTime,
}
```

Y las relaciones, de `examples/blog-orm/src/models.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[serde(rename_all = "camelCase")]
#[model(table = "users")]
pub struct User {
    #[id]
    #[default(autoincrement)]
    pub id: i64,
    #[unique]
    pub email: String,
    pub name: Option<String>,
    #[default(now)]
    pub created_at: DateTime<Utc>,
    #[relation(has_many)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub posts: Option<Vec<Post>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[serde(rename_all = "camelCase")]
#[model(table = "posts")]
pub struct Post {
    #[id]
    #[default(autoincrement)]
    pub id: i64,
    pub title: String,
    pub content: Option<String>,
    #[default(false)]
    pub published: bool,
    #[default(0)]
    pub views: i64,
    #[default(now)]
    pub created_at: DateTime<Utc>,
    #[updated_at]
    pub updated_at: DateTime<Utc>,
    pub author_id: i64,
    #[relation(belongs_to, fields = "author_id", on_delete = "Cascade")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Box<User>>,
}
```

Atributos de campo: `#[id]`, `#[unique]`, `#[default(autoincrement | now | uuid | cuid | <literal>)]`, `#[updated_at]`, `#[map("columna")]`, `#[db("Uuid")]`, `#[db_type(...)]` y `#[relation(has_many | has_one | belongs_to | many_to_many, fields = "...", references = "...", on_delete = "...", on_update = "...", name = "...")]`. En el struct: `#[model(table = "...", id(fields = [...]), unique(fields = [...]))]`.

Las relaciones se declaran como `Option` porque solo se cargan cuando la consulta las incluye: `Option<Vec<T>>` para to-many y `Option<Box<T>>` para to-one. Con `skip_serializing_if` una relación no incluida no aparece en el JSON.

El derive genera tres cosas por modelo:

1. `impl Model for User` con los metadatos.
2. El trait `UserDelegate`, que añade `db.user()` al cliente. Impórtalo junto con el modelo: `use crate::models::{User, UserDelegate};`.
3. El módulo `user` con helpers tipados: `user::email::contains(..)`, `user::id::equals(..)`, `user::posts::include()`, ...

### Generar los modelos desde un esquema

Si ya tienes un `schema.prisma`, el binario `corvis-orm` escribe los modelos por ti:

```bash
cargo run -q -p corvis-orm --bin corvis-orm -- generate --schema db/schema.prisma --out src/models.rs
```

El archivo generado no se edita a mano: se vuelve a generar. Opciones: `--no-docs` (no copia los comentarios `///`) y `--derive <Trait>` (un derive extra para cada modelo, se puede repetir).

## Registrar el módulo

`OrmModule` ofrece tres formas de registrarse. La más habitual lee la URL desde `ConfigService` con una fábrica asíncrona, como en `examples/blog-orm`:

```rust
use corvis::prelude::*;
use corvis_orm::{OrmModule, OrmModuleOptions};

#[module(
    imports = [
        ConfigModule::for_root(ConfigModuleOptions { is_global: true, ..Default::default() }),
        OrmModule::for_root_async(true, |ctx| async move {
            let config = ctx.get::<ConfigService>().await?;
            let url: String = config.get_or("DATABASE_URL", "file:./dev.db".to_string());
            Ok(OrmModuleOptions::new(url)
                .log_queries(config.get_or("LOG_QUERIES", false))
                .push::<(User, Post)>())
        }),
        UsersModule,
        PostsModule,
    ],
)]
pub struct AppModule;
```

| Forma | Uso |
|---|---|
| `OrmModule::for_root(OrmModuleOptions::new(url).global(true))` | Opciones fijas |
| `OrmModule::for_root_async(global, \|ctx\| async move { ... })` | Opciones calculadas; la fábrica puede resolver otros providers |
| `OrmModule::for_root_from_config(global)` | Lee `DATABASE_URL` de `ConfigService` (que debe ser global) |

El primer argumento de `for_root_async` y `for_root_from_config` indica si el módulo es global. Con `true`, cualquier módulo puede inyectar `OrmService` sin importarlo.

Opciones de `OrmModuleOptions`:

| Método | Efecto |
|---|---|
| `new(url)` | `file:./dev.db`, `sqlite::memory:` o `postgresql://...` |
| `global(bool)` | Registra el módulo como global |
| `log_queries(bool)` | Registra cada consulta SQL con sus parámetros |
| `max_connections(n)` | Tamaño del pool |
| `explicit_connect(bool)` | Conectar en `on_module_init` (por defecto `true`) |
| `transaction_options(..)` | `maxWait`, `timeout` e `isolationLevel` por defecto de las transacciones |
| `push::<(A, B)>()` | Crea al arrancar las tablas e índices que falten (útil en desarrollo y pruebas) |

`push` no modifica ni elimina columnas existentes: para producción usa migraciones.

## Usar `OrmService`

`OrmService` implementa `Deref` hacia el cliente, así que los delegados de cada modelo se llaman directamente sobre él:

```rust
use corvis_orm::{OrmError, OrmService};
use crate::models::{Room, RoomDelegate};

#[injectable]
pub struct RoomsService {
    db: Arc<OrmService>,
    events: Arc<EventEmitter>,
}

impl RoomsService {
    pub async fn list(&self) -> Result<Vec<Room>, OrmError> {
        self.db.room().find_many(json!({ "orderBy": [{ "createdAt": "asc" }, { "name": "asc" }] })).await
    }

    pub async fn find(&self, name: &str) -> Result<Option<Room>, OrmError> {
        self.db.room().find_unique(json!({ "where": { "name": name } })).await
    }
}
```

La misma consulta se puede escribir con los helpers tipados. Así pagina `examples/blog-orm` los posts:

```rust
let posts = self
    .db
    .post()
    .find_many(filters)
    .order_by(post::created_at::order(SortOrder::Desc))
    .skip((page - 1) * limit)
    .take(limit)
    .include(post::author::include())
    .await?;
```

Operaciones disponibles en cada delegado: `find_many`, `find_first`, `find_unique`, `find_first_or_throw`, `find_unique_or_throw`, `create`, `create_many`, `create_many_and_return`, `update`, `update_many`, `update_many_and_return`, `upsert`, `delete`, `delete_many`, `count`, `aggregate` y `group_by`. Cada consulta admite `.await` directo, `.exec()`, `.exec_json()` (el JSON tal cual) o `.exec_as::<T>()`.

### Transacciones

```rust
let (user, post) = db._transaction(|tx| async move {
    let user = tx.user().create(json!({ "data": { "email": "x@y.z" } })).await?;
    let post = tx.post().create(json!({ "data": { "title": "Hola", "authorId": user.id } })).await?;
    Ok::<_, OrmError>((user, post))
}).await?;
```

La closure hace commit si devuelve `Ok` y rollback si devuelve `Err`. `_transaction_with(opciones, ..)` acepta `TransactionOptions::new().max_wait(ms).timeout(ms)` o el nivel de aislamiento, y `_transaction_batch((a, b))` ejecuta varias consultas en una sola transacción. Los métodos llevan `_` delante porque reproducen las funciones con `$` del cliente JavaScript de referencia, y `$` no es válido en un identificador de Rust.

### SQL crudo

`sql!("SELECT ... WHERE id = {id}")` construye una consulta parametrizada: cada `{expr}` es un parámetro, nunca texto concatenado. Se ejecuta con `db._query_raw(..)`, `_query_raw_as::<T>(..)` o `_execute_raw(..)`.

## Errores HTTP

Las consultas devuelven `OrmError`, con los códigos `P2xxx` del cliente de referencia (`P2002` clave única duplicada, `P2025` registro no encontrado, ...). Hay dos formas de convertirlos en respuestas:

**Filtro global.** `OrmExceptionFilter` reconoce los `OrmError` que llegan como causa de un `HttpException` y responde con el estado adecuado. Con él basta usar `?` en el handler:

```rust
// main.rs
app.use_global_filters(OrmExceptionFilter::new()); // P2002 → 409, P2025 → 404, validación → 400, resto → 500

// controlador
#[get(":id")]
async fn find_one(&self, #[param("id", ParseIntPipe)] id: i64) -> HttpResult<User> {
    Ok(self.users_service.find_one(id).await?)
}
```

`OrmExceptionFilter::new().map_code("P2000", 422)` cambia el estado de un código concreto.

**Conversión explícita.** El trait `OrmResultExt` añade `.http()`, que devuelve directamente el `HttpException` adecuado sin depender del filtro:

```rust
use corvis_orm::OrmResultExt;

#[post]
async fn create(&self, #[body] dto: CreateUserDto) -> HttpResult<User> {
    self.users_service.create(dto).await.http()
}
```

No existe `impl From<OrmError> for HttpException`: Corvis ya convierte cualquier `std::error::Error` en un `500`, y Rust no permite las dos implementaciones a la vez. Por eso `?` produce un `500` con el error como causa, y el filtro lo reinterpreta.

## Migraciones

El binario aplica las migraciones SQL de una carpeta (`<carpeta>/<nombre>/migration.sql`) y lleva el registro en la misma tabla de control que usan los proyectos con `schema.prisma`, así que una base migrada con la herramienta original se puede seguir migrando con `corvis-orm`, y al revés:

```bash
cargo run -q -p corvis-orm --bin corvis-orm -- migrate deploy --migrations db/migrations --url "$DATABASE_URL"
cargo run -q -p corvis-orm --bin corvis-orm -- migrate status --migrations db/migrations --url "$DATABASE_URL"
```

Desde código: `db.migrate_deploy(dir).await` y `db.migrate_status(dir).await`. No hay comandos para generar migraciones a partir de cambios en los modelos.

## Seguridad

- En producción con Postgres usa `sslmode=verify-full` en la URL; `prefer` (el valor por defecto) no protege contra un atacante en la red.
- Pasar el JSON de una petición directamente a una consulta le da al cliente todos los operadores, `select` e `include`. Construye los argumentos con los campos que la ruta permite o valida el cuerpo con un schema estricto.
- Los argumentos con más de 125 niveles de anidamiento se rechazan antes de procesarse.
- `log_queries` escribe los valores de los parámetros en los logs.

## Qué no está soportado

MySQL, SQL Server, MongoDB y CockroachDB; generación de migraciones (`migrate dev`, `diff`, `reset`); middlewares y extensiones del cliente; SQL crudo dentro de `_transaction_batch`.
