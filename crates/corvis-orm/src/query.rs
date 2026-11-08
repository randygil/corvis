//! Model delegates (`db.user()`), query builders and typed parameters.
//!
//! Both styles build the same JSON query arguments:
//!
//! ```ignore
//! // JSON arguments
//! db.user().find_many(json!({ "where": { "email": { "contains": "@x" } }, "take": 10 })).await?;
//! // typed helpers
//! db.user().find_many(vec![user::email::contains("@x")]).take(10).exec().await?;
//! ```

use std::future::{Future, IntoFuture};
use std::marker::PhantomData;
use std::pin::Pin;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::client::OrmClient;
use crate::error::{OrmError, Result};
use crate::meta::Model;

/// Serializes any value to JSON (`Null` on failure). Used by the generated helpers.
pub fn to_json<T: Serialize + ?Sized>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

macro_rules! param {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        pub struct $name<M> {
            pub(crate) key: String,
            pub(crate) value: Value,
            _m: PhantomData<fn() -> M>,
        }

        impl<M> $name<M> {
            pub fn new(key: impl Into<String>, value: Value) -> Self {
                $name { key: key.into(), value, _m: PhantomData }
            }
            pub fn key(&self) -> &str {
                &self.key
            }
            pub fn value(&self) -> &Value {
                &self.value
            }
            pub fn into_pair(self) -> (String, Value) {
                (self.key, self.value)
            }
        }

        impl<M> Clone for $name<M> {
            fn clone(&self) -> Self {
                $name::new(self.key.clone(), self.value.clone())
            }
        }

        impl<M> std::fmt::Debug for $name<M> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}({}: {})", stringify!($name), self.key, self.value)
            }
        }
    };
}

param!(
    /// A `where` entry: `user::email::contains("@x")`.
    WhereParam
);
param!(
    /// A `data` entry: `user::name::set("Ana")`.
    SetParam
);
param!(
    /// An `orderBy` entry: `user::id::order(SortOrder::Desc)`.
    OrderByParam
);
param!(
    /// An `include` entry: `user::posts::include()`.
    IncludeParam
);
param!(
    /// A `select` entry: `user::email::select()`.
    SelectParam
);

impl<M> WhereParam<M> {
    /// `{ key: { op: value } }`
    pub fn field(key: &str, op: &str, value: Value) -> Self {
        let mut m = Map::new();
        m.insert(op.to_string(), value);
        WhereParam::new(key, Value::Object(m))
    }
}

/// Builds a where object from typed params (duplicated keys are combined with `AND`).
pub fn where_object<M>(params: Vec<WhereParam<M>>) -> Value {
    let mut obj = Map::new();
    let mut extra = Vec::new();
    for p in params {
        if obj.contains_key(&p.key) {
            let mut m = Map::new();
            m.insert(p.key, p.value);
            extra.push(Value::Object(m));
        } else {
            obj.insert(p.key, p.value);
        }
    }
    if !extra.is_empty() {
        match obj.get_mut("AND") {
            Some(Value::Array(a)) => a.extend(extra),
            Some(other) => {
                let prev = other.take();
                *other = Value::Array(std::iter::once(prev).chain(extra).collect());
            }
            None => {
                obj.insert("AND".into(), Value::Array(extra));
            }
        }
    }
    Value::Object(obj)
}

/// Builds a data object from typed params (last value wins).
pub fn data_object<M>(params: Vec<SetParam<M>>) -> Value {
    Value::Object(params.into_iter().map(|p| (p.key, p.value)).collect())
}

/// `{ AND: [...] }`
pub fn and<M>(params: Vec<WhereParam<M>>) -> WhereParam<M> {
    WhereParam::new("AND", Value::Array(params.into_iter().map(|p| where_object(vec![p])).collect()))
}

/// `{ OR: [...] }`
pub fn or<M>(params: Vec<WhereParam<M>>) -> WhereParam<M> {
    WhereParam::new("OR", Value::Array(params.into_iter().map(|p| where_object(vec![p])).collect()))
}

/// `{ NOT: { ... } }`
pub fn not<M>(params: Vec<WhereParam<M>>) -> WhereParam<M> {
    WhereParam::new("NOT", where_object(params))
}

/// `"asc" | "desc"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    #[default]
    Asc,
    Desc,
}

impl SortOrder {
    pub fn as_str(&self) -> &'static str {
        match self {
            SortOrder::Asc => "asc",
            SortOrder::Desc => "desc",
        }
    }
}

/// `{ count }` returned by `createMany`, `updateMany` and `deleteMany`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchPayload {
    pub count: i64,
}

/// Arguments of a query: a JSON object or typed params.
pub struct Args<M> {
    pub(crate) value: Value,
    _m: PhantomData<fn() -> M>,
}

impl<M> Args<M> {
    fn new(value: Value) -> Self {
        Args { value, _m: PhantomData }
    }
}

impl<M> From<Value> for Args<M> {
    fn from(v: Value) -> Self {
        Args::new(v)
    }
}
impl<M> From<()> for Args<M> {
    fn from(_: ()) -> Self {
        Args::new(Value::Object(Map::new()))
    }
}
impl<M> From<WhereParam<M>> for Args<M> {
    fn from(p: WhereParam<M>) -> Self {
        Args::new(serde_json::json!({ "where": where_object(vec![p]) }))
    }
}
impl<M> From<Vec<WhereParam<M>>> for Args<M> {
    fn from(p: Vec<WhereParam<M>>) -> Self {
        Args::new(serde_json::json!({ "where": where_object(p) }))
    }
}
impl<M> From<Vec<SetParam<M>>> for Args<M> {
    fn from(p: Vec<SetParam<M>>) -> Self {
        Args::new(serde_json::json!({ "data": data_object(p) }))
    }
}
impl<M> From<Vec<Vec<SetParam<M>>>> for Args<M> {
    fn from(p: Vec<Vec<SetParam<M>>>) -> Self {
        Args::new(serde_json::json!({ "data": p.into_iter().map(data_object).collect::<Vec<_>>() }))
    }
}

/// `data` of `update`/`create`: JSON or typed params.
pub struct DataArg<M>(Value, PhantomData<fn() -> M>);
impl<M> From<Value> for DataArg<M> {
    fn from(v: Value) -> Self {
        DataArg(v, PhantomData)
    }
}
impl<M> From<Vec<SetParam<M>>> for DataArg<M> {
    fn from(p: Vec<SetParam<M>>) -> Self {
        DataArg(data_object(p), PhantomData)
    }
}

/// `where` given to a builder: JSON, one param or a list of params.
pub struct WhereArg<M>(Value, PhantomData<fn() -> M>);
impl<M> From<Value> for WhereArg<M> {
    fn from(v: Value) -> Self {
        WhereArg(v, PhantomData)
    }
}
impl<M> From<WhereParam<M>> for WhereArg<M> {
    fn from(p: WhereParam<M>) -> Self {
        WhereArg(where_object(vec![p]), PhantomData)
    }
}
impl<M> From<Vec<WhereParam<M>>> for WhereArg<M> {
    fn from(p: Vec<WhereParam<M>>) -> Self {
        WhereArg(where_object(p), PhantomData)
    }
}

/// An action of the query engine and how its result is converted.
pub trait Action<M>: Send + 'static {
    const NAME: &'static str;
    type Output: Send + 'static;
    fn convert(v: Value) -> Result<Self::Output>;
}

fn deser<T: DeserializeOwned>(v: Value) -> Result<T> {
    serde_json::from_value(v).map_err(|e| {
        OrmError::unknown(format!(
            "Failed to deserialize the result into `{}`: {e}. If you used `select`, use `.exec_as::<T>()` / `.exec_json()` with a partial type.",
            std::any::type_name::<T>()
        ))
    })
}

macro_rules! actions {
    ($($(#[$doc:meta])* $name:ident = $op:literal => |$v:ident| -> $out:ty $body:block;)*) => {
        $(
            $(#[$doc])*
            pub struct $name;
            impl<M: Model> Action<M> for $name {
                const NAME: &'static str = $op;
                type Output = $out;
                fn convert($v: Value) -> Result<$out> $body
            }
        )*
    };
}

actions! {
    /// `findMany` → `Vec<M>`
    FindMany = "findMany" => |v| -> Vec<M> { deser(v) };
    /// `findFirst` → `Option<M>`
    FindFirst = "findFirst" => |v| -> Option<M> { deser(v) };
    /// `findFirstOrThrow` → `M` (P2025 when missing)
    FindFirstOrThrow = "findFirstOrThrow" => |v| -> M { deser(v) };
    /// `findUnique` → `Option<M>`
    FindUnique = "findUnique" => |v| -> Option<M> { deser(v) };
    /// `findUniqueOrThrow` → `M` (P2025 when missing)
    FindUniqueOrThrow = "findUniqueOrThrow" => |v| -> M { deser(v) };
    /// `create` → `M`
    Create = "create" => |v| -> M { deser(v) };
    /// `createMany` → `{ count }`
    CreateMany = "createMany" => |v| -> BatchPayload { deser(v) };
    /// `update` → `M` (P2025 when missing)
    Update = "update" => |v| -> M { deser(v) };
    /// `updateMany` → `{ count }`
    UpdateMany = "updateMany" => |v| -> BatchPayload { deser(v) };
    /// `upsert` → `M`
    Upsert = "upsert" => |v| -> M { deser(v) };
    /// `delete` → `M` (P2025 when missing)
    Delete = "delete" => |v| -> M { deser(v) };
    /// `deleteMany` → `{ count }`
    DeleteMany = "deleteMany" => |v| -> BatchPayload { deser(v) };
    /// `count` → `i64` (use `.exec_json()` with `select`)
    Count = "count" => |v| -> i64 { deser(v) };
    /// `aggregate` → JSON (`{ _count, _sum, _avg, _min, _max }`)
    Aggregate = "aggregate" => |v| -> Value { Ok(v) };
    /// `groupBy` → JSON array
    GroupBy = "groupBy" => |v| -> Value { Ok(v) };
}

/// A pending query. `.await` it (or call `.exec()`), or keep chaining builder methods.
#[must_use = "queries do nothing unless awaited or executed"]
pub struct Query<M, A> {
    client: OrmClient,
    args: Value,
    _p: PhantomData<fn() -> (M, A)>,
}

impl<M: Model, A: Action<M>> Query<M, A> {
    pub(crate) fn new(client: OrmClient, args: Value) -> Self {
        Query { client, args, _p: PhantomData }
    }

    fn args_mut(&mut self) -> Option<&mut Map<String, Value>> {
        if self.args.is_null() {
            self.args = Value::Object(Map::new());
        }
        self.args.as_object_mut()
    }

    /// Sets any raw argument (`.arg("take", json!(5))`).
    pub fn arg(mut self, key: &str, value: Value) -> Self {
        if let Some(m) = self.args_mut() {
            m.insert(key.to_string(), value);
        }
        self
    }

    /// `where` (combined with `AND` if one was already given).
    pub fn filter(mut self, w: impl Into<WhereArg<M>>) -> Self {
        let w = w.into().0;
        if let Some(m) = self.args_mut() {
            let merged = match m.remove("where") {
                Some(prev) if !prev.is_null() => serde_json::json!({ "AND": [prev, w] }),
                _ => w,
            };
            m.insert("where".into(), merged);
        }
        self
    }

    /// Appends an `orderBy` entry.
    pub fn order_by(mut self, o: OrderByParam<M>) -> Self {
        if let Some(m) = self.args_mut() {
            let mut entry = Map::new();
            entry.insert(o.key, o.value);
            match m.get_mut("orderBy") {
                Some(Value::Array(a)) => a.push(Value::Object(entry)),
                Some(prev) => {
                    let p = prev.take();
                    *prev = Value::Array(vec![p, Value::Object(entry)]);
                }
                None => {
                    m.insert("orderBy".into(), Value::Array(vec![Value::Object(entry)]));
                }
            }
        }
        self
    }

    pub fn take(self, n: i64) -> Self {
        self.arg("take", Value::from(n))
    }

    pub fn skip(self, n: i64) -> Self {
        self.arg("skip", Value::from(n))
    }

    /// Cursor-based pagination (`cursor: { id: 10 }`).
    pub fn cursor(self, w: impl Into<WhereArg<M>>) -> Self {
        let w = w.into().0;
        // Typed params produce `{ id: { equals: v } }`; cursors take plain values.
        let w = match w {
            Value::Object(o) => Value::Object(
                o.into_iter()
                    .map(|(k, v)| match v {
                        Value::Object(mut inner) if inner.len() == 1 && inner.contains_key("equals") => {
                            (k, inner.remove("equals").unwrap())
                        }
                        other => (k, other),
                    })
                    .collect(),
            ),
            other => other,
        };
        self.arg("cursor", w)
    }

    /// `distinct: ["field", ...]`
    pub fn distinct<I, S>(self, fields: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let v: Vec<Value> = fields.into_iter().map(|s| Value::String(s.into())).collect();
        self.arg("distinct", Value::Array(v))
    }

    /// Adds an `include` entry (`user::posts::include()`).
    pub fn include(mut self, i: IncludeParam<M>) -> Self {
        if let Some(m) = self.args_mut() {
            let inc = m.entry("include").or_insert_with(|| Value::Object(Map::new()));
            if let Value::Object(o) = inc {
                o.insert(i.key, i.value);
            }
        }
        self
    }

    /// Adds a `select` entry. The result usually no longer fits `M`: read it with
    /// `.exec_json()` or `.exec_as::<Partial>()`.
    pub fn select(mut self, s: SelectParam<M>) -> Self {
        if let Some(m) = self.args_mut() {
            let sel = m.entry("select").or_insert_with(|| Value::Object(Map::new()));
            if let Value::Object(o) = sel {
                o.insert(s.key, s.value);
            }
        }
        self
    }

    /// `data` (update, updateMany, create).
    pub fn data(self, d: impl Into<DataArg<M>>) -> Self {
        self.arg("data", d.into().0)
    }

    /// `create` branch of an `upsert`.
    pub fn create(self, d: impl Into<DataArg<M>>) -> Self {
        self.arg("create", d.into().0)
    }

    /// `update` branch of an `upsert`.
    pub fn update(self, d: impl Into<DataArg<M>>) -> Self {
        self.arg("update", d.into().0)
    }

    /// `skipDuplicates: true` (createMany).
    pub fn skip_duplicates(self) -> Self {
        self.arg("skipDuplicates", Value::Bool(true))
    }

    /// The JSON arguments that will be sent to the engine.
    pub fn args(&self) -> &Value {
        &self.args
    }

    /// Runs the query returning the raw JSON result.
    pub async fn exec_json(self) -> Result<Value> {
        crate::engine::run(self.client, M::meta(), A::NAME, self.args).await
    }

    /// Runs the query deserializing the result into any type.
    pub async fn exec_as<T: DeserializeOwned>(self) -> Result<T> {
        deser(self.exec_json().await?)
    }

    /// Runs the query.
    pub async fn exec(self) -> Result<A::Output> {
        A::convert(self.exec_json().await?)
    }
}

impl<M: Model, A: Action<M>> crate::transaction::BatchQuery for Query<M, A> {
    type Output = A::Output;
    fn exec_on(self, client: OrmClient) -> Pin<Box<dyn Future<Output = Result<A::Output>> + Send + 'static>> {
        Box::pin(Query::<M, A>::new(client, self.args).exec())
    }
}

impl<M: Model, A: Action<M>> IntoFuture for Query<M, A> {
    type Output = Result<A::Output>;
    type IntoFuture = Pin<Box<dyn Future<Output = Result<A::Output>> + Send + 'static>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.exec())
    }
}

/// `db.user()` – the operations of one model.
pub struct Delegate<M> {
    client: OrmClient,
    _m: PhantomData<fn() -> M>,
}

impl<M> Clone for Delegate<M> {
    fn clone(&self) -> Self {
        Delegate { client: self.client.clone(), _m: PhantomData }
    }
}

macro_rules! delegate_ops {
    ($($(#[$doc:meta])* $fn:ident => $action:ident;)*) => {
        $(
            $(#[$doc])*
            pub fn $fn(&self, args: impl Into<Args<M>>) -> Query<M, $action> {
                Query::new(self.client.clone(), args.into().value)
            }
        )*
    };
}

impl<M: Model> Delegate<M> {
    pub fn new(client: OrmClient) -> Self {
        Delegate { client, _m: PhantomData }
    }

    pub fn client(&self) -> &OrmClient {
        &self.client
    }

    delegate_ops! {
        /// The `findMany` action: `{ where, orderBy, take, skip, cursor, select, include, distinct }`.
        find_many => FindMany;
        /// The `findFirst` action (same arguments as `findMany`).
        find_first => FindFirst;
        /// The `findFirstOrThrow` action (same arguments as `findMany`).
        find_first_or_throw => FindFirstOrThrow;
        /// The `findUnique` action: `{ where: { id } }`.
        find_unique => FindUnique;
        /// The `findUniqueOrThrow` action: `{ where: { id } }`.
        find_unique_or_throw => FindUniqueOrThrow;
        /// The `create` action: `{ data, select, include }`.
        create => Create;
        /// The `createMany` action: `{ data: [...], skipDuplicates }`.
        create_many => CreateMany;
        /// The `update` action: `{ where, data }`.
        update => Update;
        /// The `updateMany` action: `{ where, data }`.
        update_many => UpdateMany;
        /// The `upsert` action: `{ where, create, update }`.
        upsert => Upsert;
        /// The `delete` action: `{ where }`.
        delete => Delete;
        /// The `deleteMany` action: `{ where }`.
        delete_many => DeleteMany;
        /// The `count` action: `{ where }`.
        count => Count;
        /// The `aggregate` action: `{ _count, _sum, _avg, _min, _max, where }`.
        aggregate => Aggregate;
        /// The `groupBy` action: `{ by, _count, ..., where, orderBy }`.
        group_by => GroupBy;
    }
}
