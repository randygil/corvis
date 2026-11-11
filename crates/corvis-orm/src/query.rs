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

/// Serializes the value given to a typed helper.
///
/// A value whose `Serialize` fails, or a non-finite float (`NaN`, `inf`, `-inf`, alone or inside a list, an
/// `Option` or a struct), is a validation error: `serde_json` would turn a non-finite float into `null`, and a filter
/// `{ score: null }` means `IS NULL`, which no caller means by `NaN`. Write `is_null()` / `set_null()` for `null`.
pub fn to_json<T: Serialize + ?Sized>(v: &T) -> Result<Value> {
    v.serialize(finite::Check).map_err(|_| {
        OrmError::validation("NaN and infinite values cannot be compared or stored; use `is_null()` / `set_null()` for null.")
    })?;
    serde_json::to_value(v)
        .map_err(|e| OrmError::validation(format!("Failed to serialize `{}` as a query value: {e}", std::any::type_name::<T>())))
}

/// A serializer that only checks that a value holds no non-finite float.
mod finite {
    use serde::ser::{self, Serialize};

    #[derive(Debug)]
    pub(super) struct NonFinite;

    impl std::fmt::Display for NonFinite {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("non-finite float")
        }
    }

    impl std::error::Error for NonFinite {}

    impl ser::Error for NonFinite {
        fn custom<T: std::fmt::Display>(_: T) -> Self {
            NonFinite
        }
    }

    pub(super) struct Check;

    type R = Result<(), NonFinite>;

    fn float(v: f64) -> R {
        if v.is_finite() { Ok(()) } else { Err(NonFinite) }
    }

    macro_rules! accept {
        ($($name:ident($t:ty)),*) => {$(
            fn $name(self, _: $t) -> R {
                Ok(())
            }
        )*};
    }

    impl ser::Serializer for Check {
        type Ok = ();
        type Error = NonFinite;
        type SerializeSeq = Check;
        type SerializeTuple = Check;
        type SerializeTupleStruct = Check;
        type SerializeTupleVariant = Check;
        type SerializeMap = Check;
        type SerializeStruct = Check;
        type SerializeStructVariant = Check;

        accept!(
            serialize_bool(bool),
            serialize_i8(i8),
            serialize_i16(i16),
            serialize_i32(i32),
            serialize_i64(i64),
            serialize_i128(i128),
            serialize_u8(u8),
            serialize_u16(u16),
            serialize_u32(u32),
            serialize_u64(u64),
            serialize_u128(u128),
            serialize_char(char),
            serialize_str(&str),
            serialize_bytes(&[u8])
        );
        fn serialize_f32(self, v: f32) -> R {
            float(v.into())
        }
        fn serialize_f64(self, v: f64) -> R {
            float(v)
        }
        fn serialize_none(self) -> R {
            Ok(())
        }
        fn serialize_some<T: Serialize + ?Sized>(self, v: &T) -> R {
            v.serialize(self)
        }
        fn serialize_unit(self) -> R {
            Ok(())
        }
        fn serialize_unit_struct(self, _: &'static str) -> R {
            Ok(())
        }
        fn serialize_unit_variant(self, _: &'static str, _: u32, _: &'static str) -> R {
            Ok(())
        }
        fn serialize_newtype_struct<T: Serialize + ?Sized>(self, _: &'static str, v: &T) -> R {
            v.serialize(self)
        }
        fn serialize_newtype_variant<T: Serialize + ?Sized>(self, _: &'static str, _: u32, _: &'static str, v: &T) -> R {
            v.serialize(self)
        }
        fn serialize_seq(self, _: Option<usize>) -> Result<Check, NonFinite> {
            Ok(Check)
        }
        fn serialize_tuple(self, _: usize) -> Result<Check, NonFinite> {
            Ok(Check)
        }
        fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Check, NonFinite> {
            Ok(Check)
        }
        fn serialize_tuple_variant(self, _: &'static str, _: u32, _: &'static str, _: usize) -> Result<Check, NonFinite> {
            Ok(Check)
        }
        fn serialize_map(self, _: Option<usize>) -> Result<Check, NonFinite> {
            Ok(Check)
        }
        fn serialize_struct(self, _: &'static str, _: usize) -> Result<Check, NonFinite> {
            Ok(Check)
        }
        fn serialize_struct_variant(self, _: &'static str, _: u32, _: &'static str, _: usize) -> Result<Check, NonFinite> {
            Ok(Check)
        }
    }

    macro_rules! compound {
        ($($tr:ident :: $method:ident),*) => {$(
            impl ser::$tr for Check {
                type Ok = ();
                type Error = NonFinite;
                fn $method<T: Serialize + ?Sized>(&mut self, v: &T) -> R {
                    v.serialize(Check)
                }
                fn end(self) -> R {
                    Ok(())
                }
            }
        )*};
    }
    compound!(
        SerializeSeq::serialize_element,
        SerializeTuple::serialize_element,
        SerializeTupleStruct::serialize_field,
        SerializeTupleVariant::serialize_field
    );

    impl ser::SerializeMap for Check {
        type Ok = ();
        type Error = NonFinite;
        fn serialize_key<T: Serialize + ?Sized>(&mut self, k: &T) -> R {
            k.serialize(Check)
        }
        fn serialize_value<T: Serialize + ?Sized>(&mut self, v: &T) -> R {
            v.serialize(Check)
        }
        fn end(self) -> R {
            Ok(())
        }
    }

    macro_rules! fields {
        ($($tr:ident),*) => {$(
            impl ser::$tr for Check {
                type Ok = ();
                type Error = NonFinite;
                fn serialize_field<T: Serialize + ?Sized>(&mut self, _: &'static str, v: &T) -> R {
                    v.serialize(Check)
                }
                fn end(self) -> R {
                    Ok(())
                }
            }
        )*};
    }
    fields!(SerializeStruct, SerializeStructVariant);
}

/// The error of the parameter `key` whose value could not be serialized.
fn param_error(key: &str, e: OrmError) -> OrmError {
    OrmError::validation(format!("Argument `{key}`: {}", e.message()))
}

/// The first error of a list of parameters.
fn first_error<'a>(errors: impl IntoIterator<Item = Option<&'a OrmError>>) -> Option<OrmError> {
    errors.into_iter().flatten().next().cloned()
}

macro_rules! param {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        ///
        /// A parameter built from a value that could not be serialized (see [`to_json`]) carries the error, and
        /// any query it is given to fails with it before running.
        pub struct $name<M> {
            pub(crate) key: String,
            pub(crate) value: Value,
            pub(crate) error: Option<OrmError>,
            _m: PhantomData<fn() -> M>,
        }

        impl<M> $name<M> {
            pub fn new(key: impl Into<String>, value: Value) -> Self {
                $name { key: key.into(), value, error: None, _m: PhantomData }
            }
            /// The parameter for a value that may have failed to serialize: the error is kept, the value is `null`.
            pub fn try_new(key: impl Into<String>, value: Result<Value>) -> Self {
                let key = key.into();
                match value {
                    Ok(value) => $name { key, value, error: None, _m: PhantomData },
                    Err(e) => {
                        let error = Some(param_error(&key, e));
                        $name { key, value: Value::Null, error, _m: PhantomData }
                    }
                }
            }
            pub fn key(&self) -> &str {
                &self.key
            }
            /// The JSON value (`null` when the parameter carries an [error](Self::error)).
            pub fn value(&self) -> &Value {
                &self.value
            }
            /// The error of a value that could not be serialized.
            pub fn error(&self) -> Option<&OrmError> {
                self.error.as_ref()
            }
            /// `(key, value)`, or the error the parameter carries.
            pub fn into_pair(self) -> Result<(String, Value)> {
                match self.error {
                    Some(e) => Err(e),
                    None => Ok((self.key, self.value)),
                }
            }
        }

        impl<M> Clone for $name<M> {
            fn clone(&self) -> Self {
                $name { key: self.key.clone(), value: self.value.clone(), error: self.error.clone(), _m: PhantomData }
            }
        }

        impl<M> std::fmt::Debug for $name<M> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match &self.error {
                    Some(e) => write!(f, "{}({}: error: {})", stringify!($name), self.key, e.message()),
                    None => write!(f, "{}({}: {})", stringify!($name), self.key, self.value),
                }
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
        WhereParam::try_field(key, op, Ok(value))
    }
    /// `{ key: { op: value } }` for a value that may have failed to serialize (see [`WhereParam::try_new`]).
    pub fn try_field(key: &str, op: &str, value: Result<Value>) -> Self {
        WhereParam::try_new(key, value.map(|v| Value::Object(Map::from_iter([(op.to_string(), v)]))))
    }
}

/// Builds a where object from typed params (duplicated keys are combined with `AND`), or the first error they carry.
pub fn where_object<M>(params: Vec<WhereParam<M>>) -> Result<Value> {
    if let Some(e) = first_error(params.iter().map(|p| p.error.as_ref())) {
        return Err(e);
    }
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
    Ok(Value::Object(obj))
}

/// Builds a data object from typed params (last value wins), or the first error they carry.
pub fn data_object<M>(params: Vec<SetParam<M>>) -> Result<Value> {
    if let Some(e) = first_error(params.iter().map(|p| p.error.as_ref())) {
        return Err(e);
    }
    Ok(Value::Object(params.into_iter().map(|p| (p.key, p.value)).collect()))
}

/// One where object per param (the lists of `AND` and `OR`).
fn where_list<M>(params: Vec<WhereParam<M>>) -> Result<Value> {
    params.into_iter().map(|p| where_object(vec![p])).collect::<Result<Vec<_>>>().map(Value::Array)
}

/// `{ AND: [...] }`
pub fn and<M>(params: Vec<WhereParam<M>>) -> WhereParam<M> {
    WhereParam::try_new("AND", where_list(params))
}

/// `{ OR: [...] }`
pub fn or<M>(params: Vec<WhereParam<M>>) -> WhereParam<M> {
    WhereParam::try_new("OR", where_list(params))
}

/// `{ NOT: { ... } }`
pub fn not<M>(params: Vec<WhereParam<M>>) -> WhereParam<M> {
    WhereParam::try_new("NOT", where_object(params))
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

fn merge_count(prev: &mut Map<String, Value>, next: Map<String, Value>) {
    for (k, v) in next {
        match (prev.get_mut(&k), v) {
            (Some(Value::Object(a)), Value::Object(b)) if k == "select" => a.extend(b),
            (_, v) => {
                prev.insert(k, v);
            }
        }
    }
}

/// A model plus the relation counts requested with `<model>::<relation>::count()`:
/// `db.user().find_many(()).include(user::posts::count()).exec_as::<Vec<user::WithCount>>()`.
///
/// It reads and writes the model fields followed by `_count`, like the engine's JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WithCount<M, C> {
    #[serde(flatten)]
    pub model: M,
    #[serde(rename = "_count")]
    pub count: C,
}

/// `{ count }` returned by `createMany`, `updateMany` and `deleteMany`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchPayload {
    pub count: i64,
}

/// Arguments of a query: a JSON object or typed params.
pub struct Args<M> {
    pub(crate) value: Value,
    pub(crate) error: Option<OrmError>,
    _m: PhantomData<fn() -> M>,
}

impl<M> Args<M> {
    fn new(value: Value) -> Self {
        Args { value, error: None, _m: PhantomData }
    }

    /// `{ key: value }`, or the error of the params `value` was built from.
    fn wrap(key: &str, value: Result<Value>) -> Self {
        match value {
            Ok(v) => Args::new(Value::Object(Map::from_iter([(key.to_string(), v)]))),
            Err(e) => Args { value: Value::Null, error: Some(e), _m: PhantomData },
        }
    }
}

/// A serializable value (usually a DTO) used as the data of a write:
/// `db.user().create(Data(&dto))` and `.data(Data(&dto))`.
///
/// Fields the value skips when serializing (`skip_serializing_if`) are not sent. When the value fails to
/// serialize, the query fails with a validation error before running any SQL.
pub struct Data<T>(pub T);

fn serialize_data<T: Serialize>(d: &Data<T>) -> (Value, Option<OrmError>) {
    match serde_json::to_value(&d.0) {
        Ok(v) => (v, None),
        Err(e) => {
            (Value::Null, Some(OrmError::validation(format!("Failed to serialize `{}` as query data: {e}", std::any::type_name::<T>()))))
        }
    }
}

impl<M, T: Serialize> From<Data<T>> for Args<M> {
    fn from(d: Data<T>) -> Self {
        let (value, error) = serialize_data(&d);
        Args { value: serde_json::json!({ "data": value }), error, _m: PhantomData }
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
        Args::wrap("where", where_object(vec![p]))
    }
}
impl<M> From<Vec<WhereParam<M>>> for Args<M> {
    fn from(p: Vec<WhereParam<M>>) -> Self {
        Args::wrap("where", where_object(p))
    }
}
impl<M> From<Vec<SetParam<M>>> for Args<M> {
    fn from(p: Vec<SetParam<M>>) -> Self {
        Args::wrap("data", data_object(p))
    }
}
impl<M> From<Vec<Vec<SetParam<M>>>> for Args<M> {
    fn from(p: Vec<Vec<SetParam<M>>>) -> Self {
        Args::wrap("data", p.into_iter().map(data_object).collect::<Result<Vec<_>>>().map(Value::Array))
    }
}

/// Splits a built argument into the value the query carries and its error (`null` and the error on failure).
fn split(v: Result<Value>) -> (Value, Option<OrmError>) {
    match v {
        Ok(v) => (v, None),
        Err(e) => (Value::Null, Some(e)),
    }
}

/// `data` of `update`/`create`: JSON or typed params.
pub struct DataArg<M>(Value, Option<OrmError>, PhantomData<fn() -> M>);
impl<M> From<Value> for DataArg<M> {
    fn from(v: Value) -> Self {
        DataArg(v, None, PhantomData)
    }
}
impl<M> From<Vec<SetParam<M>>> for DataArg<M> {
    fn from(p: Vec<SetParam<M>>) -> Self {
        let (value, error) = split(data_object(p));
        DataArg(value, error, PhantomData)
    }
}
impl<M, T: Serialize> From<Data<T>> for DataArg<M> {
    fn from(d: Data<T>) -> Self {
        let (value, error) = serialize_data(&d);
        DataArg(value, error, PhantomData)
    }
}

/// `where` given to a builder: JSON, one param or a list of params.
pub struct WhereArg<M>(Value, Option<OrmError>, PhantomData<fn() -> M>);
impl<M> From<Value> for WhereArg<M> {
    fn from(v: Value) -> Self {
        WhereArg(v, None, PhantomData)
    }
}
impl<M> From<WhereParam<M>> for WhereArg<M> {
    fn from(p: WhereParam<M>) -> Self {
        let (value, error) = split(where_object(vec![p]));
        WhereArg(value, error, PhantomData)
    }
}
impl<M> From<Vec<WhereParam<M>>> for WhereArg<M> {
    fn from(p: Vec<WhereParam<M>>) -> Self {
        let (value, error) = split(where_object(p));
        WhereArg(value, error, PhantomData)
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
    /// `createManyAndReturn` → `Vec<M>`
    CreateManyAndReturn = "createManyAndReturn" => |v| -> Vec<M> { deser(v) };
    /// `createMany` → `{ count }`
    CreateMany = "createMany" => |v| -> BatchPayload { deser(v) };
    /// `update` → `M` (P2025 when missing)
    Update = "update" => |v| -> M { deser(v) };
    /// `updateMany` → `{ count }`
    UpdateMany = "updateMany" => |v| -> BatchPayload { deser(v) };
    /// `updateManyAndReturn` → `Vec<M>`
    UpdateManyAndReturn = "updateManyAndReturn" => |v| -> Vec<M> { deser(v) };
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
    error: Option<OrmError>,
    _p: PhantomData<fn() -> (M, A)>,
}

impl<M: Model, A: Action<M>> Query<M, A> {
    pub(crate) fn new(client: OrmClient, args: Value) -> Self {
        Query { client, args, error: None, _p: PhantomData }
    }

    fn with_error(mut self, error: Option<OrmError>) -> Self {
        if self.error.is_none() {
            self.error = error;
        }
        self
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
        let WhereArg(w, error, _) = w.into();
        if let Some(m) = self.args_mut() {
            let merged = match m.remove("where") {
                Some(prev) if !prev.is_null() => serde_json::json!({ "AND": [prev, w] }),
                _ => w,
            };
            m.insert("where".into(), merged);
        }
        self.with_error(error)
    }

    /// Appends an `orderBy` entry.
    pub fn order_by(mut self, o: OrderByParam<M>) -> Self {
        self = self.with_error(o.error);
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
        let WhereArg(w, error, _) = w.into();
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
        self.arg("cursor", w).with_error(error)
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
        self = self.with_error(i.error);
        if let Some(m) = self.args_mut() {
            let inc = m.entry("include").or_insert_with(|| Value::Object(Map::new()));
            if let Value::Object(o) = inc {
                match (o.get_mut(&i.key), i.value) {
                    // Several `_count` includes become one `_count: { select: { a: true, b: true } }`.
                    (Some(Value::Object(prev)), Value::Object(next)) if i.key == "_count" => merge_count(prev, next),
                    (_, value) => {
                        o.insert(i.key, value);
                    }
                }
            }
        }
        self
    }

    /// Adds a `select` entry. The result usually no longer fits `M`: read it with
    /// `.exec_json()` or `.exec_as::<Partial>()`.
    pub fn select(mut self, s: SelectParam<M>) -> Self {
        self = self.with_error(s.error);
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
        let d = d.into();
        self.arg("data", d.0).with_error(d.1)
    }

    /// `create` branch of an `upsert`.
    pub fn create(self, d: impl Into<DataArg<M>>) -> Self {
        let d = d.into();
        self.arg("create", d.0).with_error(d.1)
    }

    /// `update` branch of an `upsert`.
    pub fn update(self, d: impl Into<DataArg<M>>) -> Self {
        let d = d.into();
        self.arg("update", d.0).with_error(d.1)
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
        if let Some(e) = self.error {
            return Err(e);
        }
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
        Box::pin(Query::<M, A>::new(client, self.args).with_error(self.error).exec())
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
                let args = args.into();
                Query::new(self.client.clone(), args.value).with_error(args.error)
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
        /// The `createManyAndReturn` action: like `createMany`, returning the created rows.
        create_many_and_return => CreateManyAndReturn;
        /// The `update` action: `{ where, data }`.
        update => Update;
        /// The `updateMany` action: `{ where, data }`.
        update_many => UpdateMany;
        /// The `updateManyAndReturn` action: like `updateMany`, returning the updated rows.
        ///
        /// Warning: without a `where` it updates every row of the table and returns all of them, held in
        /// memory at once. Always pass a filter, and bound it when the matching set can be large.
        update_many_and_return => UpdateManyAndReturn;
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Serialize)]
    struct Reading {
        label: &'static str,
        score: Option<f64>,
    }

    #[test]
    fn to_json_refuses_non_finite_floats_anywhere_in_the_value() {
        for (what, res) in [
            ("NaN", to_json(&f64::NAN)),
            ("f32 inf", to_json(&f32::INFINITY)),
            ("in a list", to_json(&vec![1.0, f64::NEG_INFINITY])),
            ("in an option", to_json(&Some(f64::NAN))),
            ("in a struct", to_json(&Reading { label: "x", score: Some(f64::NAN) })),
            ("in a map", to_json(&std::collections::BTreeMap::from([("k", f64::NAN)]))),
        ] {
            let e = res.expect_err(what);
            assert!(e.is_validation() && e.message().contains("NaN and infinite values"), "{what}: {e}");
        }
        assert_eq!(to_json(&Reading { label: "x", score: Some(1.5) }).unwrap(), json!({ "label": "x", "score": 1.5 }));
        assert_eq!(to_json(&None::<f64>).unwrap(), Value::Null);
        assert_eq!(to_json(&f64::MAX).unwrap(), json!(f64::MAX));
    }

    #[test]
    fn to_json_reports_serialization_errors_with_the_type() {
        let map = std::collections::HashMap::from([((1, 2), 3)]);
        let e = to_json(&map).unwrap_err();
        assert!(e.is_validation() && e.message().contains("HashMap"), "{e}");
    }

    #[test]
    fn params_carry_the_error_through_combinators() {
        let bad: WhereParam<()> = WhereParam::try_field("score", "equals", to_json(&f64::NAN));
        assert!(bad.value().is_null());
        assert!(bad.error().unwrap().message().starts_with("Argument `score`: NaN"), "{bad:?}");
        assert!(format!("{bad:?}").contains("error: Argument `score`"));
        let good: WhereParam<()> = WhereParam::field("id", "gt", json!(1));
        for p in [and(vec![good.clone(), bad.clone()]), or(vec![bad.clone()]), not(vec![good.clone(), bad.clone()])] {
            let e = p.error().unwrap_or_else(|| panic!("{p:?} lost the error"));
            assert!(e.message().contains("Argument `score`"), "{e}");
        }
        assert!(where_object(vec![good.clone(), bad.clone()]).is_err());
        assert_eq!(where_object(vec![good.clone()]).unwrap(), json!({ "id": { "gt": 1 } }));
        assert!(bad.clone().into_pair().is_err());
        assert_eq!(good.into_pair().unwrap(), ("id".to_string(), json!({ "gt": 1 })));
        let set: SetParam<()> = SetParam::try_new("score", to_json(&f64::NAN));
        assert!(data_object(vec![SetParam::new("a", json!(1)), set]).is_err());
    }
}
