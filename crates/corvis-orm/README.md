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

`or_undefined` escribe un `NaN` o un infinito como `null`, igual que el cliente original envía `NaN`; un valor que no se puede
serializar hace fallar la consulta con un error de validación antes de enviar SQL.

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
- `RawSql::join` de una lista vacía no entra en pánico: la sentencia lleva el error del cliente de referencia ("Expected
  `join([])` to be called with an array of multiple elements, but got an empty array"), también si está anidada en otra, y
  `_query_raw`, `_query_raw_as` y `_execute_raw` fallan con un error de validación sin enviar nada. `RawSql::error()` lo consulta.

### Decimal y DateTime

`corvis_orm::Decimal` tiene la semántica de decimal.js (precisión 20, `ROUND_HALF_UP`, `toFixed`, `toDecimalPlaces`, `toString`...).
Se serializa como `toString()` (`"24.5"`) y se lee y escribe sin pérdida en columnas `NUMERIC`. `Decimal.clone({...})` es
`DecimalCtor::new(&CONFIG)`. `corvis_orm::DateTime` equivale al `Date` de JS: milisegundos y siempre `"2026-01-02T03:04:05.678Z"`.

`Decimal::parse` y `decimal!` aceptan lo mismo que decimal.js (comprobado contra él en `tests/fixtures/decimal_parse.json`): números `0x`, `0b` y `0o` con fracción y exponente binario (`0x1.8p3`) y un `_` entre dos dígitos (`1_000`).

El texto se convierte en `Decimal` solo con conversiones que informan del error: `Decimal::parse`, `"12.5".parse::<Decimal>()`
y `Decimal::try_from(..)` (de `&str` o `String`) para texto que llega en tiempo de ejecución, y el literal `decimal!("1.50")`,
que se comprueba al compilar (un literal inválido no compila). Ya no existe `From<&str>` ni `From<String>`: entraban en pánico
con texto inválido, y como los helpers tipados reciben `impl Into<Decimal>`, un texto de una petición podía tumbar el handler.

```rust
use corvis_orm::{decimal, Decimal};

let tasa = decimal!("40.50");                        // antes: Decimal::from("40.50")
let monto: Decimal = entrada.parse()?;               // antes: Decimal::from(entrada.as_str()), que entraba en pánico
let unitario = D.parse("1234.5678")?;                // antes: D.of("1234.5678")
db.wallet().find_many(vec![wallet::amount::gte(decimal!("100"))]).await?;
```

En SQLite, una columna `Decimal` se compara, ordena y agrega como número, como con la migración del cliente de referencia
(afinidad numérica, 15 dígitos significativos): `db_push` la crea como `DECIMAL`, y las comparaciones, `orderBy`, cursores,
`_min`/`_max`/`_sum`/`_avg`, `having` y la aritmética leen la columna con `CAST(.. AS NUMERIC)`, así que una columna `TEXT`
creada por una versión anterior de `db_push` también se compara como número. Ese `CAST` impide usar un índice simple sobre la
columna en un rango. En Postgres no cambia nada.

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
| `include: { _count: { select: { posts: true } } }` | `.include(user::posts::count())`, leído con `.exec_as::<Vec<user::WithCount>>()` |
| `data` armado desde un objeto propio | `create(Data(&dto))` / `.data(Data(&dto))` (`corvis_orm::Data`) |
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

## Seguridad

### Conexión y TLS

- La feature `tls-rustls` (activa por defecto) compila sqlx con TLS (rustls y los certificados raíz
  de Mozilla). El modo se elige con `sslmode` en la URL, como en libpq:
  - `sslmode=prefer` (por defecto en sqlx): intenta TLS y, si el servidor no lo ofrece, sigue en
    texto plano. No protege contra un atacante en la red.
  - `sslmode=require`: exige TLS pero no verifica el certificado.
  - `sslmode=verify-full`: exige TLS, verifica la cadena y el nombre del host. **Es el modo
    recomendado en producción**. Con una CA propia: `&sslrootcert=/ruta/ca.pem`.
  - Para compilar sin TLS: `default-features = false, features = ["corvis"]`.
- Los errores de conexión (`P1001`) muestran solo `host:puerto`, nunca el usuario, la contraseña ni
  la query string de la URL. Un protocolo no soportado tampoco repite la URL.

### Argumentos de consulta que vienen del cliente

`query_json` y los helpers aceptan los mismos argumentos que el cliente JavaScript de referencia,
así que pasar el `Value` de una request directamente a una consulta le da al cliente el poder de esos
argumentos. corvis-orm limita lo que podría tumbar el proceso o corromper datos:

- **Profundidad.** Los argumentos con más de 125 niveles de objetos o listas (el mismo límite que el
  motor de referencia) se rechazan con un `QueryValidationError` antes de recorrerlos, así que un
  `where` anidado no desborda la pila. Se cambia con `OrmClientOptions::max_query_depth(n)`; subirlo
  mucho vuelve a exponer la pila.
- **`WhereUniqueInput`.** En `findUnique`, `update`, `upsert`, `delete`, `cursor`, `connect`... los
  campos `@id`, `@unique` y los de una clave compuesta solo aceptan el valor (`{ email: "a@b.c" }`),
  nunca un filtro (`{ equals }`, `{ not }`, `{ startsWith }`) ni `null`, igual que la versión 5+ del
  cliente de referencia. Los demás campos siguen aceptando filtros.
- **`Decimal`.** El parseo es lineal y respeta el rango de decimal.js (exponente hasta ±9e15: más
  allá es `Infinity` o `0`). Un valor que no cabe en un `NUMERIC` de Postgres (más de 131072 dígitos
  enteros o 65535 decimales) falla con el mismo error de conversión del motor de referencia
  (`ConversionError("Couldn't convert an integer (possible overflow).")`) en vez de truncarse. En
  Postgres la precisión y la escala de la columna las aplica la base (`numeric field overflow`,
  redondeo a la escala). En SQLite la columna se trata como `@db.Decimal(p, s)` o, por defecto,
  `Decimal(65, 30)`: al escribir se redondea a `s` decimales y se rechaza (error de validación) un
  valor cuya parte entera no cabe. `db_push` crea los `Decimal` de Postgres como `DECIMAL(65,30)` (o el `@db.Decimal(p, s)`
  del campo) y los de SQLite como `DECIMAL`.
- **Helpers tipados.** Un valor que no se puede serializar, o un `f32`/`f64` `NaN` o infinito (solo, en una lista o en un
  `Option`), hace fallar la consulta con un error de validación antes de enviar SQL; antes se convertía en `null` y el filtro
  pasaba a ser `IS NULL` / `IS NOT NULL`. Para `null` usa `is_null()`, `is_not_null()` o `set_null()`.
- **Números.** Un `BigInt` fuera de 64 bits o un byte de `Bytes` fuera de `0..=255` es un error de
  validación, nunca un valor saturado o truncado. En SQL crudo, un `u64` mayor que `i64::MAX` se
  envía como `numeric`.
- **Paginación.** `take` admite cualquier `i64`; `limit` negativo en `updateMany`/`deleteMany` da
  `P2019` (`Provided limit (-1) must be a positive integer.`).
- **Listas `in`/`notIn` largas** (y las de `include` y `_count`): con más de 1000 elementos van en
  un solo parámetro (`= ANY($1)` en Postgres, `json_each(?)` en SQLite), así que no chocan con el
  límite de parámetros de la base; también con `mode: "insensitive"`. Las listas de claves que arma el
  motor (la relectura de `createManyAndReturn`/`updateManyAndReturn`, `include` y `_count` sobre claves
  compuestas, `set`/`disconnect`/`deleteMany` anidados) van en un parámetro si la clave tiene un campo, y
  si es compuesta se reparten en varias sentencias dentro del límite (en SQLite, además, de 500 grupos
  como máximo: SQLite rechaza expresiones de más de 1000 niveles y un `OR` largo lo es).
- **Filtros de texto en `Json` (SQLite).** `string_contains`, `string_starts_with` y
  `string_ends_with` comparan el texto literal: `%`, `_`, `*`, `?` y `[` no son comodines.

Lo que **no** se limita, porque es parte de la API (y del cliente de referencia):

- Un cliente que controla `where` puede filtrar por cualquier campo no único con cualquier operador
  (`{ token: { not: "" } }` en `findFirst`), y uno que controla `select`, `include` u `omit` puede
  pedir campos y relaciones que la app no pensaba devolver (`omit: { password: false }`).
- `distinct` se aplica en memoria; un `include` paginado hace una consulta por registro padre.
- `search` usa `to_tsquery`: una sintaxis inválida da un error de la base.
- Los errores de validación nombran campos y repiten parte de la entrada; `log_queries` escribe los
  valores de los parámetros.

Por eso, con datos de una request, construye los argumentos con los campos y operadores que la ruta
permite (o valida el cuerpo con un esquema estricto) en lugar de reenviar el JSON tal cual.

### Escrituras y concurrencia

- `update`, `delete`, `upsert` (cuando no es nativo), `updateMany`, `updateManyAndReturn` y
  `deleteMany` repiten su `where` completo en la sentencia `UPDATE`/`DELETE` (`WHERE pk = ? AND
  (<where>)`). Si otra transacción cambió la fila entre la lectura y la escritura, la fila ya no
  cumple el filtro: `update`/`delete` fallan con `P2025` (`No record was found for an update.` /
  `... for a delete.`) y los `*Many` no la cuentan. La concurrencia optimista funciona:
  `updateMany({ where: { id, version }, data: { version: { increment: 1 } } })` tiene un solo
  ganador también en Postgres (`READ COMMITTED`).
- Las escrituras anidadas de relaciones (`posts: { update: ... }`) corren en una transacción, como en
  el cliente de referencia, pero sin bloquear las filas que leen.

### Transacciones

- Una transacción interactiva que expira mientras la closure termina nunca devuelve `Ok`: si el
  temporizador ya hizo rollback, el resultado es `P2028`.
- Si el futuro de una transacción se cancela (por ejemplo, el cliente HTTP cortó la conexión), sqlx
  hace rollback y el temporizador se detiene en el acto.

### Esquemas

- `corvis-orm generate` rechaza valores de atributos con más de 64 niveles de `[...]`/`f(...)`
  anidados en el `schema.prisma` (antes desbordaban la pila).

## Qué no está soportado (por ahora)

- MySQL, SQL Server, MongoDB y CockroachDB.
- `migrate dev`/`diff`/`reset`. `db_push` crea lo que falta como lo crearía la migración del cliente de referencia para los
  mismos modelos (tablas con su clave primaria completa, valores por defecto e índices, tablas de unión de las relaciones
  muchos a muchos implícitas, claves foráneas y, en Postgres, tipos enum, tipos nativos `@db.*`, listas como arreglos y los
  esquemas de `@@schema`), todo en una transacción; rechaza antes de ejecutar nada lo que no sabe crear (un tipo nativo o un
  tipo de índice desconocido); y no modifica las tablas que ya existen. Para recrear una base de desarrollo creada por una
  versión anterior, usa `db_push_force_reset`.
- Middlewares, `$extends`, `$on` (eventos de log) y métricas.
- SQL crudo dentro de `_transaction_batch` (úsalo dentro de `_transaction`).
- Los mensajes de error siguen a la versión 6 del cliente de referencia con su motor nativo. Con la versión 7 y driver
  adapters, algunos `meta` cambian (por ejemplo, `P2002` no trae `target`); el `code` es el mismo.

## Pruebas

```text
cargo test -p corvis-orm
```

- Las pruebas de SQLite usan una base en memoria. Cada pool con `sqlite::memory:` usa exactamente una conexión permanente,
  porque cada conexión a `:memory:` sería una base de datos distinta.
- Las pruebas de Postgres usan `CORVIS_ORM_PG` (por defecto `postgres://postgres:postgres@127.0.0.1:55432`) y crean bases `orm_test_<nombre>_<ejecución>` y roles `orm_<nombre>_<ejecución>`, únicos por ejecución.
  Si el servidor no responde, se omiten.
- `tests/pg_oracle.rs` es el oráculo diferencial: ejecuta los casos de `tests/fixtures/oracle/results.json`, grabados con
  el cliente JavaScript de referencia (6.19) sobre `tests/fixtures/oracle/schema.prisma`, y exige el mismo resultado (texto y orden de claves),
  el mismo código, `meta` y mensaje de error.
- `tests/decimal_js.rs` compara `Decimal` con decimal.js 10.6 (2500 casos por operación) y `tests/codegen_schemas.rs` comprueba
  que los modelos generados de varios esquemas compilan.
