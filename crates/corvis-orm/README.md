# corvis-orm

ORM para Corvis, construido sobre `sqlx` 0.8 (runtime tokio), compatible con proyectos que usan archivos `schema.prisma`.
Soporta **PostgreSQL** y **SQLite**. En Postgres se prueba contra el cliente JavaScript de referencia con un oráculo diferencial
(ver [Pruebas](#pruebas)): mismos resultados, mismo orden de claves y mismos errores.

- Los modelos son structs con `#[derive(Model)]`, el equivalente de `schema.prisma`. También puedes generarlos desde
  un `schema.prisma` existente con `corvis-orm generate`.
- El cliente acepta argumentos de consulta en JSON (`where`, `include`, `select`, `orderBy`...) o helpers tipados.
  Las dos formas generan el mismo JSON interno.
- `corvis-orm migrate deploy` aplica las migraciones de `prisma/migrations` y las registra en la tabla `_prisma_migrations`,
  con el mismo formato que usan los proyectos `schema.prisma`: una base migrada con la herramienta original de esos
  proyectos también se puede migrar con corvis-orm, y al revés.
- `Decimal` con la semántica de decimal.js y `DateTime` equivalente al `Date` de JS.
- Los errores usan los códigos estándar `P2xxx` (`P2002`, `P2025`, `P2003`, `P2010`...) y los tipos
  `KnownRequestError`, `QueryValidationError`, `InitializationError` y `UnknownRequestError`, agrupados en `OrmError`.
- Para integrarlo con Corvis tienes `OrmModule`, `OrmService` y `OrmExceptionFilter`.

## Generar los modelos desde `schema.prisma`

```bash
cargo run -q -p corvis-orm --bin corvis-orm -- generate --schema prisma/schema.prisma --out src/models.rs
cargo run -q -p corvis-orm --bin corvis-orm -- migrate deploy --migrations prisma/migrations --url "$DATABASE_URL"
cargo run -q -p corvis-orm --bin corvis-orm -- migrate status --migrations prisma/migrations --url "$DATABASE_URL"
```

El archivo generado no se edita a mano: vuelve a generarlo. En el crate: `mod models; use models::*;`.
Sin `--out`, se escribe en `src/models.rs`.

- Los campos quedan en snake_case y el JSON sale con los nombres del esquema (`#[serde(rename = "createdAt")]`, `#[map("created_at")]`).
- El orden de los campos es el de los resultados con `include`: escalares en el orden del esquema, luego relaciones y luego `_count`.
- Tipos: `String`, `i32` (`Int`), `i64` (`BigInt`), `f64` (`Float`), `corvis_orm::Decimal`, `bool`, `corvis_orm::DateTime`
  (también `@db.Date` y `@db.Timestamptz`), `serde_json::Value` (`Json`), `Vec<u8>` (`Bytes`), `Option<T>` para `?` y `Vec<T>` para listas.
  `@db.Uuid` es `String`. Los enums son enums de Rust con `#[derive(OrmEnum)]` (enums nativos de Postgres, también en arrays).
- Relaciones: to-many `Option<Vec<T>>`, to-one requerida `Option<Box<T>>` y to-one opcional `Option<Option<Box<T>>>`
  (`None` = no incluida; `Some(None)` = incluida y `null`).
- `Unsupported(...)`, `@ignore` y `@@ignore` no aparecen en el cliente.

`db.migrate_deploy(dir)` y `db.migrate_status(dir)` hacen lo mismo desde código.

## Modelos escritos a mano

```rust
use corvis_orm::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[model(table = "users")]                // @@map("users"); por defecto se usa el nombre del struct ("User")
pub struct User {
    #[id] #[default(autoincrement)] pub id: i64,
    #[unique] pub email: String,
    pub name: Option<String>,               // Option<T> = campo opcional (String?)
    #[default(now)] pub created_at: corvis_orm::DateTime,
    #[relation(has_many)]                   // la FK se deduce del belongs_to de Post
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub posts: Option<Vec<Post>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
pub struct Post {
    #[id] #[default(autoincrement)] pub id: i64,
    pub title: String,
    #[default(false)] pub published: bool,
    pub author_id: i64,
    #[relation(belongs_to, fields = "author_id", references = "id", on_delete = "Cascade")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Box<User>>,
}
```

Atributos: `#[id]`, `#[unique]`, `#[default(autoincrement | now | uuid | cuid | <literal>)]`,
`#[updated_at]`, `#[map("columna")]`, `#[db("Uuid")]`, `#[db_type(String)]` (para tipos propios),
`#[model(table = "...", id(fields = [...]), unique(fields = [...]))]` y
`#[relation(has_many | has_one | belongs_to | many_to_many, fields = "...", references = "...", on_delete = "...", on_update = "...", name = "...")]`.

El derive genera tres cosas:
1. `impl Model for User`, con los metadatos estáticos.
2. El trait `UserDelegate`, que añade `db.user()` a `OrmClient`. Impórtalo junto al modelo: `use crate::models::{User, UserDelegate};`.
3. El módulo `user` con helpers tipados: `user::email::contains(..)`, `user::id::order(..)`, `user::posts::include()`, etc.

## Cliente

```rust
let db = OrmClient::new(std::env::var("DATABASE_URL")?)?;   // "postgresql://...", "file:./dev.db", "sqlite::memory:"

// Argumentos JSON
let users: Vec<User> = db.user().find_many(json!({
    "where": { "email": { "contains": "@corvis.dev" }, "OR": [{ "name": null }, { "name": { "startsWith": "A" } }] },
    "orderBy": { "id": "desc" }, "take": 10, "skip": 0,
    "include": { "posts": { "where": { "published": true } } }
})).await?;
let u = db.user().create(json!({ "data": { "email": "a@b.c", "posts": { "create": [{ "title": "Hola" }] } } })).await?;
let partial: Value = db.user().find_many(json!({ "select": { "id": true, "email": true } })).exec_json().await?;

// Tipado
let users = db.user()
    .find_many(user::email::contains("@corvis.dev"))
    .order_by(user::id::order(SortOrder::Desc))
    .take(10)
    .include(user::posts::include())
    .exec().await?;
db.user().update(user::id::equals(1)).data(vec![user::name::set("Ana B")]).await?;
```

Cada consulta es un `Query<M, Action>`. Puedes hacerle `.await` directamente o usar `.exec()`, `.exec_json()` (devuelve el JSON
tal cual, con el orden de claves del cliente de referencia) o `.exec_as::<T>()`.
Si quieres ver el SQL de una consulta, activa `log_queries` o usa `corvis_orm::debug_sql(&client, User::meta(), &where)`.

### `undefined`

En JS, una clave con `undefined` no se envía; `null` sí. Con `json!` usa `corvis_orm::undefined()` o `corvis_orm::or_undefined(opcion)`:

```rust
use corvis_orm::{or_undefined, undefined};
db.transaction().find_many(json!({
    "where": { "createdAt": { "gte": or_undefined(desde), "lte": or_undefined(hasta) }, "currency": undefined() }
})).await?;
```

### Transacciones

```rust
// $transaction(async (tx) => { ... })
let (user, post) = db._transaction(|tx| async move {
    let user = tx.user().create(json!({ "data": { "email": "x@y.z" } })).await?;
    let post = tx.post().create(json!({ "data": { "title": "Hola", "author_id": user.id } })).await?;
    Ok::<_, OrmError>((user, post))
}).await?;

// $transaction(fn, { maxWait, timeout, isolationLevel })
db._transaction_with(TransactionOptions::new().max_wait(15_000).timeout(30_000), |tx| async move { ... }).await?;
db._transaction_with(json!({ "isolationLevel": "Serializable" }), |tx| async move { ... }).await?;

// $transaction([a, b]): una tupla de hasta 12 consultas de cualquier tipo, o un Vec de consultas del mismo tipo
let (total, filas) = db._transaction_batch((
    db.user().count(json!({ "where": w })),
    db.user().find_many(json!({ "where": w, "take": 20 })),
)).await?;
```

- Una transacción interactiva espera como máximo `maxWait` (2000 ms) para obtener una conexión y se revierte
  al pasar `timeout` (5000 ms). Después de eso, la siguiente consulta o el commit fallan con `P2028`.
  Los valores por defecto se cambian con `OrmClientOptions::transaction_options(...)` o `OrmModuleOptions::transaction_options(...)`.
- Dentro de la closure, `tx` es un `OrmClient` ligado a la transacción. Las llamadas anidadas reutilizan la transacción externa.
- La closure hace commit si devuelve `Ok` y rollback si devuelve `Err`.

### SQL crudo

```rust
use corvis_orm::{sql, RawSql};

// $queryRaw`...${valor}...`  →  sql!("...{valor}...")
let filas: Vec<Value> = tx._query_raw(sql!(
    "SELECT * FROM \"Wallet\" WHERE \"merchantId\" = {merchant_id} AND currency = {currency}::\"Currency\" LIMIT 1 FOR UPDATE"
)).await?;
let saldos: Vec<Saldo> = db._query_raw_as(sql!("SELECT ... WHERE \"occurredAt\" > {desde}")).await?;   // $queryRaw<T>
let n: u64 = db._execute_raw(sql!("UPDATE contadores SET next = next + 1 WHERE id = {id}::uuid")).await?;

// join / raw / empty
let ids = RawSql::join([1, 2, 3], ",", "(", ")");
let orden = RawSql::raw("ORDER BY id");
let extra = if solo_activos { sql!("AND active = {true}") } else { RawSql::empty() };
db._query_raw(sql!("SELECT * FROM users WHERE id IN {ids} {extra} {orden}")).await?;

// $queryRawUnsafe / $executeRawUnsafe (placeholders nativos: $1 en Postgres, ? en SQLite)
db.query_raw("SELECT * FROM users WHERE id = $1", vec![json!(1)]).await?;
```

- Cada `{expr}` de `sql!` es un parámetro, nunca texto pegado en el SQL. Un `RawSql` dentro de `{}` se inserta como fragmento.
  Para llaves literales escribe `{{` y `}}` (`'{{1,2}}'::int[]`).
- Tipos de los parámetros (los mismos del cliente de referencia): enteros → `bigint`; `f64` y `Decimal` → `numeric`; `String`/`&str` → `text`;
  `bool`; `DateTime` y `chrono::DateTime<Utc>` → `timestamptz`; `NaiveDate` → `date`; `Value` objeto o arreglo → `jsonb`;
  `Vec<T>` → arreglo de Postgres (`= ANY({ids})`); `Option<T>` → `NULL` tipado; `corvis_orm::Bytes` → `bytea`.
  Si la columna es de otro tipo, castea en el SQL (`{id}::uuid`, `{tipo}::"Kind"`).
- Las filas salen como JSON: `numeric` como string decimal (se deserializa a `Decimal`), `bigint`/`int` como número,
  fechas como ISO 8601 y `json`/`jsonb` tal cual.
- Cualquier error de la base en SQL crudo es `P2010` (`Raw query failed. Code: ...`), con `meta.code` = SQLSTATE.

### Decimal y DateTime

`corvis_orm::Decimal` tiene la semántica de decimal.js (precisión 20, `ROUND_HALF_UP`, `toFixed`, `toDecimalPlaces`, `toString`...).
Se serializa como `toString()` (`"24.5"`) y se lee y escribe sin pérdida en columnas `NUMERIC`. `Decimal.clone({...})` es
`DecimalCtor::new(&CONFIG)`. `corvis_orm::DateTime` equivale al `Date` de JS: milisegundos y siempre `"2026-01-02T03:04:05.678Z"`.

## Integración con Corvis

```rust
#[module(imports = [
    ConfigModule::for_root(ConfigModuleOptions { is_global: true, ..Default::default() }),
    OrmModule::for_root_async(true, |ctx| async move {
        let config = ctx.get::<ConfigService>().await?;
        Ok(OrmModuleOptions::from_config(&config)?.log_queries(true))
    }),
])]
pub struct AppModule;

#[injectable]
pub struct UsersService { db: Arc<OrmService> }   // Deref a OrmClient: self.db.user()

app.use_global_filters(OrmExceptionFilter::new()); // P2002→409, P2025→404, validación→400, resto→500
```

- `OrmModule::for_root(OrmModuleOptions::new(url).global(true))`, `for_root_async(global, factory)` y `for_root_from_config(global)`.
  Este último lee `DATABASE_URL` del `ConfigService`.
- `OrmService` se conecta en `on_module_init`, se desconecta en `on_module_destroy` y escribe sus logs como `[OrmService]`.
  Con `.push::<Modelos>()` ejecuta `db push` al arrancar.
- Corvis ya define `impl<E: Error> From<E> for HttpException` (500), así que no se puede implementar `impl From<OrmError> for HttpException`:
  el compilador rechaza las dos implementaciones juntas por coherencia. Tienes dos alternativas:
  - Registrar `OrmExceptionFilter`. Detecta el error con `downcast_cause::<OrmError>()`, tanto en handlers `HttpResult<T>` como en `corvis::Result<T>`.
  - Usar `.http()` (trait `OrmResultExt`): `self.service.create(dto).await.http()` devuelve directamente un 409/404/400.

## Equivalencias con el cliente JavaScript

Si vienes del ecosistema de Node.js, esta tabla relaciona la API del cliente JavaScript de referencia con la de corvis-orm.

| Cliente JavaScript | corvis-orm |
|---|---|
| `generate` | `corvis-orm generate --schema ... --out src/models.rs` |
| `migrate deploy` / `migrate status` | `corvis-orm migrate deploy` / `migrate status`, o `db.migrate_deploy(dir)` |
| `db push` / `--force-reset` | `db.db_push::<(User, Post)>()` / `db_push_force_reset` |
| constructor del cliente / `$connect` / `$disconnect` | `OrmClient::new(url)?` / `connect()` / `disconnect()` |
| opciones `{ transactionOptions, omit }` | `OrmClientOptions::new(url).transaction_options(..).omit(json!(..))` |
| `client.user.findMany(args)` | `db.user().find_many(json!(args)).await` |
| `findFirst`, `findUnique`, `*OrThrow` | `find_first`, `find_unique`, `find_first_or_throw`, `find_unique_or_throw` |
| `create`, `createMany`, `createManyAndReturn`, `update`, `updateMany`, `updateManyAndReturn`, `upsert`, `delete`, `deleteMany` | los mismos nombres en snake_case |
| `count`, `aggregate`, `groupBy` | `count`, `aggregate`, `group_by` |
| `$transaction(async (tx) => ..., opciones)` | `db._transaction(\|tx\| ..)` / `_transaction_with(opciones, \|tx\| ..)` |
| `$transaction([a, b])` | `db._transaction_batch((a, b))` |
| `` $queryRaw`...${x}` `` / `$executeRaw` | `db._query_raw(sql!("...{x}"))` / `_execute_raw(..)` / `_query_raw_as::<T>(..)` |
| `sql` / `join` / `raw` / `empty` | `sql!(..)` / `RawSql::join(..)` / `RawSql::raw(..)` / `RawSql::empty()` |
| `$queryRawUnsafe` / `$executeRawUnsafe` | `query_raw(sql, params)` / `execute_raw(sql, params)` |
| `DbNull` / `JsonNull` / `AnyNull` | `json!({ "$type": "DbNull" })` / `"JsonNull"` / `"AnyNull"` |
| `undefined` | `corvis_orm::undefined()` / `or_undefined(opcion)` |
| `Decimal` | `corvis_orm::Decimal` |
| error de petición conocido (`code`, `meta`) | `OrmError::KnownRequest` (`.code()`, `.meta()`) |
| error de validación / error desconocido | `OrmError::Validation` / `OrmError::UnknownRequest` |

En Rust, `$` no es válido en un identificador, así que los métodos `$x` del cliente JavaScript se llaman `_x`.

## Qué está soportado

- Filtros `where`: `equals`, `not`, `in`, `notIn`, `lt`, `lte`, `gt`, `gte`, `contains`, `startsWith`, `endsWith`, `mode: "insensitive"`,
  `AND`/`OR`/`NOT` (también con objetos o listas vacías) y `null`; filtros de listas (`has`, `hasSome`, `hasEvery`, `isEmpty`, `equals`);
  filtros de `Json` (`equals`, `not`, `path`, `string_contains`, `array_contains`...); filtros de relación `some`/`every`/`none` e `is`/`isNot`
  (o la forma directa `{ author: { name } }`); claves únicas compuestas dentro de `where`.
- `orderBy`: por campo, por `{ sort, nulls }`, por campos de relaciones to-one (anidadas) y por `_count` de una relación to-many.
  Las columnas enum se ordenan por el orden de declaración del enum, como en Postgres.
- Paginación con `take` (también negativo), `skip`, `cursor` y `distinct`.
- `include`, `select` y `omit` anidados (incluido el `omit` global), con `where`, `orderBy`, `take`, `skip` y `_count` en las relaciones.
- `@@id` y `@@unique` compuestos en todas las operaciones (`where: { a_b: { a, b } }`), relaciones muchos a muchos implícitas y auto-relaciones.
- Escrituras anidadas: `create`, `createMany`, `connect`, `connectOrCreate`, `disconnect`, `set`, `update`, `updateMany`, `upsert`,
  `delete` y `deleteMany`, dentro de una transacción.
- Operaciones atómicas `set`, `increment`, `decrement`, `multiply` y `divide` (también sobre `Decimal`), y `push` en listas.
- `createMany` con `skipDuplicates`.
- `upsert` nativo con la misma regla del motor de consultas de referencia: si `where` tiene un solo campo único (o un único compuesto) con los
  mismos valores que `create`, no hay escrituras anidadas, `update` no está vacío y `select`/`include` no piden relaciones ni
  `_count`, se ejecuta un único `INSERT ... ON CONFLICT (...) DO UPDATE ... RETURNING`. Ese `INSERT` consume un valor de la
  secuencia aunque el registro ya exista, así que los `id` autoincrementales coinciden con los del cliente de referencia.
  En los demás casos se busca el registro y después se actualiza o se crea, dentro de una transacción.
- `count` (con `select`), `aggregate` (`_count`, `_sum`, `_avg`, `_min`, `_max`, con `where`, `orderBy`, `cursor`, `take`, `skip`) y
  `groupBy` con `having` y `orderBy` por agregados. `_sum`/`_avg` sobre `Decimal` devuelven `Decimal`.
- Validación de argumentos con sugerencias, por ejemplo: "Unknown argument `emial`. Did you mean `email`?".
- Errores: P2000, P2002, P2003, P2011, P2014, P2019, P2021, P2022, P2023, P2024, P2025, P2028, P2034 y P2010 (SQL crudo).
  Los errores de la base sin código `P2xxx` (CHECK, triggers con `RAISE EXCEPTION`, `EXCLUDE`...) llegan como
  `UnknownRequestError` con el mismo texto que el motor de consultas de referencia, que incluye el SQLSTATE y el nombre de la constraint.

## Qué no está soportado (por ahora)

- MySQL, SQL Server, MongoDB y CockroachDB.
- `migrate dev`/`diff`/`reset`. `db_push` solo crea las tablas e índices que faltan; no modifica ni elimina columnas.
- Middlewares, `$extends`, `$on` (eventos de log) y métricas.
- SQL crudo dentro de `_transaction_batch` (úsalo dentro de `_transaction`).
- PostgreSQL con TLS (no se activa ninguna feature TLS de sqlx).
- Los mensajes de error siguen a la versión 6 del cliente de referencia con su motor nativo. Con la versión 7 y driver
  adapters, algunos `meta` cambian (por ejemplo, `P2002` no trae `target`); el `code` es el mismo.

## Pruebas

```text
cargo test -p corvis-orm
```

- Las pruebas de SQLite usan una base en memoria. Cada pool con `sqlite::memory:` usa exactamente una conexión permanente,
  porque cada conexión a `:memory:` sería una base de datos distinta.
- Las pruebas de Postgres usan `CORVIS_ORM_PG` (por defecto `postgres://postgres:postgres@127.0.0.1:55432`) y crean bases `orm_test_*`.
  Si el servidor no responde, se omiten.
- `tests/pg_oracle.rs` es el oráculo diferencial: ejecuta los casos de `tests/fixtures/oracle/results.json`, grabados con
  el cliente JavaScript de referencia (6.19) sobre `tests/fixtures/oracle/schema.prisma`, y exige el mismo resultado (texto y orden de claves),
  el mismo código, `meta` y mensaje de error.
- `tests/decimal_js.rs` compara `Decimal` con decimal.js 10.6 (2500 casos por operación) y `tests/codegen_schemas.rs` comprueba
  que los modelos generados de varios esquemas compilan.
