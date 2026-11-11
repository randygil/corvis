//! # corvis-orm
//!
//! The ORM of [corvis](https://github.com/randygil/corvis), built on sqlx
//! (SQLite and PostgreSQL).
//!
//! - Models are plain structs with `#[derive(Model)]` (the `schema.prisma` equivalent).
//! - The client offers `find_many`, `find_unique`, `create`,
//!   `update`, `upsert`, `delete`, `*_many`, `count`, `aggregate`, `group_by`, with
//!   JSON query arguments (`json!({ "where": ..., "include": ... })`) or typed helpers.
//! - `db.db_push::<(User, Post)>()` creates tables, unique indexes and foreign keys.
//! - Errors carry the standard `P2xxx` codes (`P2002`, `P2025`, ...) or are a
//!   [`QueryValidationError`] for invalid arguments.
//! - [`OrmModule`] / [`OrmService`] / [`OrmExceptionFilter`] integrate
//!   it with corvis: an injectable client and a filter that maps errors to HTTP responses.
//!
//! ```ignore
//! use corvis_orm::prelude::*;
//!
//! #[derive(Debug, Clone, Serialize, Deserialize, Model)]
//! pub struct User {
//!     #[id] #[default(autoincrement)] pub id: i64,
//!     #[unique] pub email: String,
//!     pub name: Option<String>,
//!     #[relation(has_many)] #[serde(default, skip_serializing_if = "Option::is_none")]
//!     pub posts: Option<Vec<Post>>,
//! }
//!
//! let db = OrmClient::new("sqlite::memory:")?;
//! db.db_push::<(User, Post)>().await?;
//! let users: Vec<User> = db.user().find_many(json!({ "include": { "posts": true } })).await?;
//! let same = db.user().find_many(()).include(user::posts::include()).exec().await?;
//! ```

#![forbid(unsafe_code)]

// Unit tests derive models, whose generated code names the crate by its path.
#[cfg(test)]
extern crate self as corvis_orm;

mod client;
pub mod codegen;
mod datetime;
mod db;
pub mod decimal;
mod engine;
mod error;
mod meta;
mod migrate;
#[cfg(feature = "corvis")]
mod module;
pub mod psl;
mod query;
mod raw;
mod schema;
pub mod serde_helpers;
mod transaction;
mod url_params;
pub mod util;

pub use client::{OrmClient, OrmClientOptions};
pub use datetime::DateTime;
pub use db::{Dialect, SqlValue};
pub use decimal::{Decimal, DecimalConfig, DecimalCtor, DecimalError, Rounding};
pub use error::{CLIENT_VERSION, InitializationError, KnownRequestError, OrmError, QueryValidationError, Result, UnknownRequestError};
pub use meta::{
    DefaultValue, EnumMeta, EnumValueMeta, FieldKind, FieldMeta, IndexMeta, JoinTable, Model, ModelMeta, ModelSet, OrmEnum,
    ReferentialAction, RelationKind, RelationMeta, ResolvedRelation, ScalarType, UniqueMeta, UnsupportedMeta, m2m_relation_name,
    resolve_relation,
};
pub use migrate::{MigrateStatus, MigrationInfo};
#[cfg(feature = "corvis")]
pub use module::{ORM_MODULE_OPTIONS, OrmExceptionFilter, OrmModule, OrmModuleOptions, OrmResultExt, OrmService};
pub use query::{
    Action, Args, BatchPayload, Data, DataArg, Delegate, IncludeParam, OrderByParam, Query, SelectParam, SetParam, SortOrder, WhereArg,
    WhereParam, WithCount, and, data_object, not, or, to_json, where_object,
};

pub use raw::{Bytes, RawArg, RawSql, ToSqlArg};
pub use transaction::{Batch, BatchQuery, IsolationLevel, TransactionOptions};

pub use corvis_orm_macros::{Model, OrmEnum, decimal, sql};
pub use serde;
pub use serde_json;

/// Query actions (the `A` of `Query<M, A>`).
pub mod actions {
    pub use crate::query::{
        Aggregate, Count, Create, CreateMany, Delete, DeleteMany, FindFirst, FindFirstOrThrow, FindMany, FindUnique, FindUniqueOrThrow,
        GroupBy, Update, UpdateMany, Upsert,
    };
}

/// `use corvis_orm::prelude::*;`
pub mod prelude {
    pub use crate::{
        BatchPayload, DateTime, Decimal, DecimalConfig, DecimalCtor, IsolationLevel, Model, OrmClient, OrmEnum, OrmError, RawSql,
        SortOrder, TransactionOptions, any_null, db_null, decimal, json_null, or_undefined, sql, undefined,
    };
    #[cfg(feature = "corvis")]
    pub use crate::{OrmExceptionFilter, OrmModule, OrmModuleOptions, OrmResultExt, OrmService};
    pub use serde::{Deserialize, Serialize};
    pub use serde_json::{Value, json};
}

/// JS `undefined` for query arguments built with `json!`: the key (or list item) is
/// dropped before the query runs, so it means "not given", unlike `null`.
///
/// ```ignore
/// db.transaction().find_many(json!({
///     "where": { "createdAt": { "gte": corvis_orm::or_undefined(from), "lte": corvis_orm::or_undefined(to) },
///                "currency": corvis_orm::undefined() }
/// })).await?;
/// ```
pub fn undefined() -> serde_json::Value {
    engine::undefined_marker()
}

/// The database `NULL` of a Json field (`DbNull`): in `data` it stores `NULL`, in `equals` / `not` it compares with
/// `NULL`. A plain `{ "$type": "DbNull" }` object is JSON data, like any other object a request may carry.
pub fn db_null() -> serde_json::Value {
    engine::null_marker(engine::NullKind::Db)
}

/// The JSON `null` value of a Json field (`JsonNull`), as opposed to the database `NULL` ([`db_null`]).
pub fn json_null() -> serde_json::Value {
    engine::null_marker(engine::NullKind::Json)
}

/// Either null of a Json field (`AnyNull`): in a filter it matches the database `NULL` and the JSON `null`.
pub fn any_null() -> serde_json::Value {
    engine::null_marker(engine::NullKind::Any)
}

/// `Some(v)` -> `v`, `None` -> [`undefined()`] (`value ?? undefined`).
///
/// A non-finite float becomes `null`, as the reference client sends `NaN` (recorded). A value whose `Serialize`
/// fails does not: the query it is part of fails with a validation error before any SQL runs, as JavaScript fails
/// on a value `JSON.stringify` cannot write.
pub fn or_undefined<T: serde::Serialize>(v: Option<T>) -> serde_json::Value {
    match v.map(serde_json::to_value) {
        Some(Ok(v)) => v,
        Some(Err(e)) => engine::invalid_argument(format!("Failed to serialize `{}` as a query value: {e}", std::any::type_name::<T>())),
        None => undefined(),
    }
}

/// Compiles a `where` object into the SQL condition used for the client's
/// database (placeholders included). Useful to inspect the generated SQL.
pub fn debug_sql(client: &OrmClient, model: &'static ModelMeta, r#where: &serde_json::Value) -> Result<String> {
    engine::where_to_sql(client, model, r#where).map(|(sql, _)| sql)
}
