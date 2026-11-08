---
status: documented
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

El archivo generado no se edita a mano: se vuelve a generar. Sin `--schema`, busca el esquema primero en los lugares habituales de un proyecto con `schema.prisma` y, si no hay, en `db/schema.prisma` y luego en `db/schema` (`corvis-orm --help` muestra el orden completo); la carpeta de migraciones es por defecto `migrations`, junto al esquema. Opciones: `--no-docs` (no copia los comentarios `///`) y `--derive <Trait>` (un derive extra para cada modelo, se puede repetir).

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
| `max_connections(n)` | Tamaño del pool (tiene prioridad sobre `connection_limit` de la URL) |
| `explicit_connect(bool)` | Conectar en `on_module_init` (por defecto `true`) |
| `transaction_options(..)` | `maxWait`, `timeout` e `isolationLevel` por defecto de las transacciones |
| `push::<(A, B)>()` | Crea al arrancar las tablas e índices que falten (útil en desarrollo y pruebas) |

`push` no modifica ni elimina columnas existentes: para producción usa migraciones.

## Pool de conexiones

El pool se dimensiona y se limita en tiempo desde la URL de la base de datos, igual que lo hace el cliente original, así que una app que conserva su `DATABASE_URL` conserva su pool:

```text
postgresql://user:pass@host/db?connection_limit=5&pool_timeout=20&connect_timeout=10
file:./dev.db?connection_limit=1
```

| Parámetro | Significado | Por defecto |
|---|---|---|
| `connection_limit` | Número máximo de conexiones del pool. Un entero de al menos 1 | `2 * núcleos físicos de CPU + 1` |
| `pool_timeout` | Segundos que una consulta espera una conexión libre del pool antes de fallar con `P2024`. `0` espera sin límite | `10` |
| `connect_timeout` | Solo Postgres. Segundos para abrir una conexión (conectar, autenticar, la sentencia de preparación de `schema` y la comprobación de una conexión inactiva) antes de que la consulta falle con `P1001`. `0` espera sin límite | `5` |

- Funcionan con Postgres y con SQLite en archivo (`file:` y `sqlite:`). Se quitan de la URL antes de que llegue al driver; el resto de parámetros (`sslmode`, `application_name`, `options`, ...) se conservan tal cual.
- SQLite en memoria usa siempre exactamente una conexión: `connection_limit` se acepta y se ignora ahí. En SQLite, `pool_timeout` solo se aplica si es un número válido; uno inválido se ignora, como hace el cliente original.
- Un parámetro repetido conserva su último valor, y los valores pueden ir codificados con porcentaje.
- En SQLite, `connect_timeout` se quita de la URL y se ignora, sea cual sea su valor: abrir un archivo no espera a una red.
- Un valor inválido (`connection_limit=0`, `connection_limit=abc`, `pool_timeout=-1`, `connect_timeout=abc`) falla al crear el cliente, con un error de inicialización `P1013` que nombra el parámetro y muestra el valor solo si es corto y simple (letras, dígitos, `+ . _ -`), nunca el resto de la URL. `OrmClientOptions` muestra su URL sin la contraseña en la salida de `Debug`; un `acquire_timeout` en código de `Duration::MAX` significa sin límite. El cliente original trata `connection_limit=0` como sin límite; corvis-orm exige al menos 1.
- El tamaño del pool es como máximo 262143 (el mayor valor que Postgres acepta para su propio `max_connections`), en la URL y en código: el driver reserva un espacio por conexión por adelantado, así que un valor mayor agotaría la memoria. `connection_limit=262144` falla con `P1013`, y `max_connections(0)` o `max_connections(n)` por encima del límite falla al crear el cliente.
- Los valores puestos en código tienen prioridad sobre la URL: `OrmClientOptions::max_connections(n)` (y `OrmModuleOptions::max_connections(n)`) sobre `connection_limit`, y `OrmClientOptions::acquire_timeout` sobre `pool_timeout`. La URL tiene prioridad sobre los valores por defecto.
- `pool_timeout` y `acquire_timeout` limitan solo la espera de una conexión libre. Abrir una conexión lo limita `connect_timeout`, así que `pool_timeout=0` (esperar sin límite a un pool ocupado) nunca significa esperar para siempre a un servidor caído, mudo o lleno: la consulta falla con `P1001` "Can't reach database server at `host:port`" tras `connect_timeout`. La primera conexión de `connect()` y las comprobaciones de `migrate deploy` y `migrate status` tienen el mismo límite. `connect_timeout=0` quita el límite.
- Una conexión que no se puede preparar porque la base de datos rechaza la sentencia de `schema` (por ejemplo un nombre que la codificación de la base no puede representar) falla al instante con `P1001` y el mensaje de la base de datos, en lugar de reintentar hasta un tiempo límite.
- Diferencias con el cliente original: una conexión rechazada se reintenta dentro de `connect_timeout` (el original falla al instante), así que una app que arranca junto a una base de datos que aún está iniciando sigue funcionando; y todo fallo de conexión se informa como `P1001`, no con los códigos por causa del original (`P1000`, `P1003`, `P2037`).

::: warning Valores por defecto cambiados
Antes el pool tenía 10 conexiones en Postgres y 5 en archivos SQLite, y una consulta esperaba hasta 30 segundos por una conexión. Ahora los valores por defecto son `2 * núcleos físicos + 1` conexiones y 10 segundos. Define `connection_limit` y `pool_timeout` en la URL (o `max_connections` en código) para recuperar cualquier valor anterior. Con varias conexiones sobre un archivo SQLite, los escritores pueden competir por el bloqueo; usa `connection_limit=1` si tu app necesita un único escritor.
:::

::: warning Cambio: `acquire_timeout` ya no limita la conexión
Un programa que puso `acquire_timeout` (o `pool_timeout`) en pocos segundos para fallar rápido ante un servidor Postgres inalcanzable ahora espera hasta `connect_timeout` (5 segundos por defecto). Añade `?connect_timeout=2` a la URL para recuperar el límite anterior.
:::

Los ajustes de pool que las versiones más nuevas del cliente reciben en código equivalen a `max_connections(n)` (tamaño máximo) y `acquire_timeout` (espera de una conexión libre); su tiempo de conexión equivale a `connect_timeout` en la URL.

## Esquema de la base de datos

Una URL de Postgres puede indicar el esquema que se usa, igual que en el cliente original:

```text
postgresql://user:pass@host/db?schema=tenant_a
```

- Cada conexión del pool ejecuta `SET search_path TO "tenant_a"` justo al abrirse, antes de cualquier otra sentencia. Las consultas generadas, el SQL crudo, las transacciones interactivas, `push` y las migraciones resuelven entonces los nombres sin calificar en ese esquema, y solo en ese. El nombre siempre se cita como identificador (se conservan mayúsculas y espacios), así que ningún valor de `schema` puede ejecutar otra sentencia.
- Sin `schema` no cambia nada: rige el `search_path` del servidor (`"$user", public` por defecto) y no se envía ningún `SET`.
- Los modelos y enums con esquema explícito (`#[model(schema = "audit")]`) conservan su nombre calificado.
- `migrate deploy`, `migrate status` y `push` crean el esquema si no existe, y `_prisma_migrations` vive en él. Un esquema existente nunca se vuelve a crear, así que un rol que solo tiene permisos sobre ese esquema puede desplegar. Crear un esquema que falta exige el privilegio `CREATE` sobre la base de datos.
- La ruta es exactamente ese esquema: los objetos de otros esquemas, como las funciones de extensiones instaladas en `public` o `extensions`, deben calificarse, y un `CREATE EXTENSION` en una migración instala la extensión en el esquema de la URL.
- Los scripts de migración y `db_execute` se ejecutan en una conexión que se cierra al terminar en lugar de volver al pool, así que un script que cambie la sesión (`SET search_path`, `RESET ALL`) no puede enviar consultas posteriores a otro esquema. `_prisma_migrations` siempre se escribe en el esquema de la URL con su nombre calificado.
- Los parámetros de la URL se leen como datos de formulario: la consulta termina en el primer `#`, las claves y los valores se decodifican (`schem%61=t` equivale a `schema=t`) y un `+` en un valor es un espacio (`My+Schema`).
- Un nombre vacío, `$user`, uno que empieza por `pg_`, con un carácter NUL o de más de 63 bytes falla al crear el cliente, con un error de inicialización `P1013` que nombra `schema` y nunca muestra la URL. El cliente original acepta un nombre vacío (y entonces no encuentra ninguna tabla) y deja que el servidor trunque los nombres largos; corvis-orm rechaza ambos.
- En SQLite el parámetro se acepta y se ignora. La opción `schema` del adaptador 7.x equivale a este parámetro de la URL.
- Un ajuste de sesión que cambie tu propio SQL (`SET search_path ...` fuera de una transacción) se queda en esa conexión del pool, como cualquier ajuste de sesión.

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

Con `?schema=nombre` en la URL (ver [Esquema de la base de datos](#esquema-de-la-base-de-datos)), `migrate deploy`, `migrate status` y `push` crean el esquema si no existe y `_prisma_migrations` queda dentro de él.

Desde código: `db.migrate_deploy(dir).await` y `db.migrate_status(dir).await`. No hay comandos para generar migraciones a partir de cambios en los modelos.

## Seguridad

- En producción con Postgres usa `sslmode=verify-full` en la URL; `prefer` (el valor por defecto) no protege contra un atacante en la red. El TLS viene de la feature `tls-rustls` (activa por defecto); los errores de conexión muestran solo `host:puerto`, nunca el usuario, la contraseña ni la query string.
- Pasar el JSON de una petición directamente a una consulta le da al cliente todos los operadores, `select` e `include`. Construye los argumentos con los campos que la ruta permite o valida el cuerpo con un schema estricto.
- Los argumentos con más de 125 niveles de anidamiento se rechazan antes de procesarse (`max_query_depth`, se cambia con `OrmClientOptions::max_query_depth(n)`).
- Un `where` con un carácter NUL (`\u0000`) en un string, en un segmento de `path` JSON o en una clave JSON es un error de validación en todas las bases de datos, antes de ejecutar SQL. PostgreSQL no puede guardar NUL en `text` ni en `jsonb`, y el cliente original responde con un error crudo de la base de datos; Corvis responde con un error de validación (`400` mediante `OrmExceptionFilter`).
- En los inputs únicos (`findUnique`, `update`, `upsert`, `delete`, ...) los campos `@id` y `@unique` solo aceptan el valor (`{ email: "a@b.c" }`), nunca un filtro como `{ equals }` o `{ not }` ni `null`.
- Los números fuera de rango son errores, nunca valores truncados: un `BigInt` de más de 64 bits, un byte de `Bytes` fuera de `0..=255`, un `Decimal` que no cabe en un `NUMERIC` de Postgres o un `limit` negativo en `update_many`/`delete_many` (`P2019`).
- `update`, `delete` y las operaciones `*_many` repiten su `where` en el `UPDATE`/`DELETE`, así que la concurrencia optimista (`where: { id, version }`) tiene un solo ganador.
- `log_queries` escribe los valores de los parámetros en los logs.

## Qué no está soportado

MySQL, SQL Server, MongoDB y CockroachDB; generación de migraciones (`migrate dev`, `diff`, `reset`); middlewares y extensiones del cliente; SQL crudo dentro de `_transaction_batch`.
