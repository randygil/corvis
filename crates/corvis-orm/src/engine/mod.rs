//! The query engine: translates JSON query arguments into SQL.
//!
//! Every public API (JSON or typed) ends up calling [`run`] with the action name
//! (`findMany`, `create`, ...) and a JSON object of arguments, the same shape the
//! JavaScript clients send to their query engine.

mod aggregate;
mod filter;
#[cfg(test)]
mod key_list_tests;
#[cfg(test)]
mod pg_sql_snapshot;
mod read;
#[cfg(test)]
pub(crate) mod recorder;
#[cfg(test)]
mod sqlite_operand_tests;
mod write;

use std::fmt::Display;

use serde_json::{Map, Value};

use crate::client::OrmClient;
use crate::db::{Dialect, SqlValue, parse_datetime};
use crate::error::{OrmError, Result, suggest};
use crate::meta::{DefaultValue, FieldKind, FieldMeta, ModelMeta, ResolvedRelation, resolve_relation};

/// Most `OR`ed groups (key groups, `deleteMany` items) one engine-built statement takes. SQLite parses
/// `a OR b OR ...` as a left-deep tree and rejects an expression deeper than 1000 levels (`SQLITE_MAX_EXPR_DEPTH`);
/// Postgres accepts long lists but plans them slowly (a statement with 16383 two-field groups did not finish in half
/// an hour). Lists of single-field keys longer than this are one `in` list instead.
const OR_GROUPS_PER_STATEMENT: usize = 500;

/// Executes an action against a model.
pub(crate) async fn run(client: OrmClient, model: &'static ModelMeta, action: &'static str, args: Value) -> Result<Value> {
    let e = Engine { client, action, root: model };
    let mut args = args;
    // Before anything walks the arguments recursively (`strip_undefined`, `where_sql`, ...).
    check_depth(&e, args_depth_limit(&e.client), &mut args)?;
    if let Some(msg) = find_invalid_argument(&args) {
        return Err(e.err(msg));
    }
    strip_undefined(&mut args);
    let args = match args {
        Value::Null => Map::new(),
        Value::Object(m) => m,
        other => {
            return Err(e.err(format!(
                "Argument of `client.{}.{}()` must be an object, provided {}.",
                model.client_name,
                action,
                json_type(&other)
            )));
        }
    };
    match action {
        "findMany" => e.find_many_action(model, args).await,
        "findFirst" => e.find_first_action(model, args, false).await,
        "findFirstOrThrow" => e.find_first_action(model, args, true).await,
        "findUnique" => e.find_unique_action(model, args, false).await,
        "findUniqueOrThrow" => e.find_unique_action(model, args, true).await,
        "create" => e.create_action(model, args).await,
        "createMany" => e.create_many_action(model, args, false).await,
        "createManyAndReturn" => e.create_many_action(model, args, true).await,
        "update" => e.update_action(model, args).await,
        "updateMany" => e.update_many_action(model, args, false).await,
        "updateManyAndReturn" => e.update_many_action(model, args, true).await,
        "upsert" => e.upsert_action(model, args).await,
        "delete" => e.delete_action(model, args).await,
        "deleteMany" => e.delete_many_action(model, args).await,
        "count" => e.count_action(model, args).await,
        "aggregate" => e.aggregate_action(model, args).await,
        "groupBy" => e.group_by_action(model, args).await,
        other => Err(e.err(format!("Unknown action `{other}`"))),
    }
}

fn args_depth_limit(client: &OrmClient) -> usize {
    client.options().max_query_depth
}

/// `true` when `v` nests objects / lists deeper than `max` levels (`v` itself is level 1).
/// Iterative, so it is safe on any input.
pub(crate) fn exceeds_depth(v: &Value, max: usize) -> bool {
    let mut stack: Vec<(&Value, usize)> = vec![(v, 1)];
    while let Some((v, depth)) = stack.pop() {
        let children: Box<dyn Iterator<Item = &Value>> = match v {
            Value::Object(m) => Box::new(m.values()),
            Value::Array(a) => Box::new(a.iter()),
            _ => continue,
        };
        if depth > max {
            return true;
        }
        stack.extend(children.filter(|c| c.is_object() || c.is_array()).map(|c| (c, depth + 1)));
    }
    false
}

/// Drops a value without recursion (the default `Drop` of a deeply nested `Value` recurses).
pub(crate) fn drop_flat(v: Value) {
    let mut stack = vec![v];
    while let Some(v) = stack.pop() {
        match v {
            Value::Object(m) => stack.extend(m.into_iter().map(|(_, x)| x)),
            Value::Array(a) => stack.extend(a),
            _ => {}
        }
    }
}

/// Rejects query arguments nested deeper than `max` (the reference query engine fails the same
/// way, at its JSON recursion limit).
fn check_depth(e: &Engine, max: usize, args: &mut Value) -> Result<()> {
    if !exceeds_depth(args, max) {
        return Ok(());
    }
    drop_flat(std::mem::take(args));
    Err(depth_error(e, max))
}

fn depth_error(e: &Engine, max: usize) -> OrmError {
    e.err(format!("Query arguments are nested too deeply: the maximum depth is {max} (`max_query_depth`)."))
}

/// The value of [`crate::undefined`]: JS `undefined` inside query arguments.
pub(crate) const UNDEFINED: &str = "\u{0}corvis_orm::undefined\u{0}";

/// The prefix of a value [`invalid_argument`] stands for (NUL characters cannot come from a request: they are
/// rejected in strings).
const INVALID: &str = "\u{0}corvis_orm::invalid\u{0}";

/// A JSON value that makes the query it is put in fail with `message` (a validation error) before any SQL runs: how
/// a JSON builder such as [`crate::or_undefined`] reports a value it could not convert.
pub(crate) fn invalid_argument(message: String) -> Value {
    Value::String(format!("{INVALID}{message}"))
}

/// The message of the first [`invalid_argument`] in `v`, if any.
fn find_invalid_argument(v: &Value) -> Option<&str> {
    match v {
        Value::String(s) => s.strip_prefix(INVALID),
        Value::Array(a) => a.iter().find_map(find_invalid_argument),
        Value::Object(m) => m.values().find_map(find_invalid_argument),
        _ => None,
    }
}

fn is_undefined(v: &Value) -> bool {
    matches!(v, Value::String(s) if s == UNDEFINED)
}

/// Drops `undefined` object entries and list items, like `JSON.stringify` does.
pub(crate) fn strip_undefined(v: &mut Value) {
    match v {
        Value::Object(m) => {
            m.retain(|_, x| !is_undefined(x));
            m.values_mut().for_each(strip_undefined);
        }
        Value::Array(a) => {
            a.retain(|x| !is_undefined(x));
            a.iter_mut().for_each(strip_undefined);
        }
        _ => {}
    }
}

/// All the actions of a model delegate.
pub(crate) const ACTIONS: &[&str] = &[
    "findMany",
    "findFirst",
    "findFirstOrThrow",
    "findUnique",
    "findUniqueOrThrow",
    "create",
    "createMany",
    "createManyAndReturn",
    "update",
    "updateMany",
    "updateManyAndReturn",
    "upsert",
    "delete",
    "deleteMany",
    "count",
    "aggregate",
    "groupBy",
];

pub(crate) fn static_action(a: &str) -> Option<&'static str> {
    ACTIONS.iter().copied().find(|x| *x == a)
}

/// A record identity: the values of its primary key (or first unique) fields.
pub(crate) type Id = Map<String, Value>;

/// JSON type name in schema terms (for error messages).
pub(crate) fn json_type(v: &Value) -> &'static str {
    match v {
        Value::Null => "Null",
        Value::Bool(_) => "Boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "Int",
        Value::Number(_) => "Float",
        Value::String(_) => "String",
        Value::Array(_) => "List",
        Value::Object(_) => "Object",
    }
}

#[derive(Clone)]
pub(crate) struct Engine {
    pub(crate) client: OrmClient,
    pub(crate) action: &'static str,
    pub(crate) root: &'static ModelMeta,
}

impl Engine {
    pub(crate) fn dialect(&self) -> Dialect {
        self.client.dialect()
    }

    /// A `QueryValidationError` for the current invocation.
    pub(crate) fn err(&self, msg: impl Display) -> OrmError {
        OrmError::validation(format!("Invalid `client.{}.{}()` invocation:\n\n{msg}", self.root.client_name, self.action))
    }

    pub(crate) fn obj<'v>(&self, v: &'v Value, arg: &str, expected: &str) -> Result<&'v Map<String, Value>> {
        v.as_object()
            .ok_or_else(|| self.err(format!("Argument `{arg}`: Invalid value provided. Expected {expected}, provided {}.", json_type(v))))
    }

    /// Rejects unknown keys with a "Did you mean ...?" suggestion.
    pub(crate) fn check_keys(&self, obj: &Map<String, Value>, allowed: &[&str]) -> Result<()> {
        for k in obj.keys() {
            if !allowed.contains(&k.as_str()) {
                return Err(self.unknown_argument(k, allowed.iter().copied()));
            }
        }
        Ok(())
    }

    pub(crate) fn unknown_argument<'a>(&self, name: &str, options: impl IntoIterator<Item = &'a str>) -> OrmError {
        let options: Vec<&str> = options.into_iter().collect();
        let available = options.iter().map(|o| format!("`{o}`")).collect::<Vec<_>>().join(", ");
        match suggest(name, options.iter().copied()) {
            Some(s) => self.err(format!("Unknown argument `{name}`. Did you mean `{s}`? Available options: {available}.")),
            None => self.err(format!("Unknown argument `{name}`. Available options: {available}.")),
        }
    }

    /// Looks up a field of `model` used as an argument.
    pub(crate) fn field(&self, model: &'static ModelMeta, name: &str) -> Result<&'static FieldMeta> {
        model.field(name).ok_or_else(|| self.unknown_argument(name, model.fields.iter().map(|f| f.name)))
    }

    pub(crate) fn relation(&self, model: &'static ModelMeta, f: &'static FieldMeta) -> Result<ResolvedRelation> {
        resolve_relation(model, f).map_err(|m| self.err(m))
    }

    /// Postgres type name of an enum (`"Role"` / `"schema"."Role"`).
    pub(crate) fn enum_type_sql(e: &crate::meta::EnumMeta) -> String {
        match e.schema {
            Some(s) => format!("{}.{}", crate::db::quote_ident(s), crate::db::quote_ident(e.db_name)),
            None => crate::db::quote_ident(e.db_name),
        }
    }

    /// Parameters an engine-built list may bind in one statement: half the database's limit (32766 on SQLite,
    /// 65535 on Postgres), leaving the other half to the rest of the statement.
    fn list_param_budget(&self) -> usize {
        match self.dialect() {
            Dialect::Sqlite => 32766 / 2,
            Dialect::Postgres => 65535 / 2,
        }
    }

    /// Splits `items` into the groups of one statement each: at most [`Engine::list_param_budget`] parameters
    /// (`params` gives those of an item) and at most [`OR_GROUPS_PER_STATEMENT`] items. A list that fits in one
    /// statement stays one group, so the statement is the one written before chunking existed.
    pub(crate) fn statement_chunks<T>(&self, items: Vec<T>, params: impl Fn(&T) -> usize) -> Vec<Vec<T>> {
        let budget = self.list_param_budget();
        let max_items = OR_GROUPS_PER_STATEMENT;
        let mut chunks: Vec<Vec<T>> = Vec::new();
        let mut used = 0;
        for item in items {
            let n = params(&item);
            match chunks.last_mut() {
                Some(chunk) if chunk.len() < max_items && used + n <= budget => chunk.push(item),
                _ => {
                    chunks.push(vec![item]);
                    used = 0;
                }
            }
            used += n;
        }
        chunks
    }

    /// The `where` filters, one per statement, that together match the rows whose key (`fields`) is one of `keys`.
    ///
    /// A list that fits in one statement is written by `shape` (the caller's `OR` of the keys, as before chunking
    /// existed). A longer list of single-field keys is one `in` filter, bound as one parameter; a longer list of
    /// compound keys is split into several statements of one `shape` each (see [`Engine::statement_chunks`]).
    pub(crate) fn key_filters(&self, fields: &[&'static FieldMeta], keys: Vec<Id>, shape: impl Fn(Vec<Id>) -> Value) -> Vec<Value> {
        let chunks = self.statement_chunks(keys, |_| fields.len());
        if let ([f], [_, _, ..]) = (fields, chunks.as_slice()) {
            let values: Vec<Value> = chunks.into_iter().flatten().map(|mut k| k.remove(f.name).unwrap_or(Value::Null)).collect();
            return vec![Value::Object(Map::from_iter([(f.name.to_string(), serde_json::json!({ "in": values }))]))];
        }
        chunks.into_iter().map(shape).collect()
    }

    /// `true` when `f` is read through `CAST(.. AS NUMERIC)` wherever it is compared, sorted, aggregated or used in
    /// arithmetic: a `Decimal` on SQLite. Its column may be `DECIMAL` (numeric affinity, what `db push` and the
    /// reference migration create) or `TEXT` (created by `db push` of earlier versions), and only the cast makes a
    /// `TEXT` column compare as numbers. Postgres columns are `numeric` and are never cast.
    pub(crate) fn numeric_operand(&self, f: &FieldMeta) -> bool {
        self.dialect() == Dialect::Sqlite && f.kind == FieldKind::Decimal && !f.list
    }

    /// `alias."column"` as an operand of a comparison, an ordering, an aggregate or arithmetic
    /// (see [`Engine::numeric_operand`]). Only the quoted column is wrapped, never a value.
    pub(crate) fn operand(&self, q: &mut crate::db::Sql, alias: &str, f: &FieldMeta) {
        if self.numeric_operand(f) {
            q.push("CAST(").col(alias, f.column).push(" AS NUMERIC)");
        } else {
            q.col(alias, f.column);
        }
    }

    /// The `::cast` a Postgres parameter for `f` needs (enums, json, ...), if any.
    fn pg_cast(&self, f: &FieldMeta, list: bool) -> Option<String> {
        if self.dialect() != Dialect::Postgres {
            return None;
        }
        let base = match (f.kind, f.native_name()) {
            (FieldKind::Enum, _) => f.enum_meta().map(Self::enum_type_sql),
            (_, Some("Uuid")) => Some("uuid".to_string()),
            (FieldKind::Json, Some("Json")) => Some("json".to_string()),
            (_, Some("Inet")) => Some("inet".to_string()),
            (_, Some("Citext")) => Some("citext".to_string()),
            (_, Some("Xml")) => Some("xml".to_string()),
            (_, Some("Money")) => Some("money".to_string()),
            (_, Some("Bit")) | (_, Some("VarBit")) => f.native_type.map(|t| t.to_lowercase()),
            (FieldKind::String, Some("Char")) => f.native_type.map(|t| t.to_lowercase()),
            (FieldKind::DateTime, Some("Date")) => Some("date".to_string()),
            (FieldKind::DateTime, Some("Time")) => Some("time".to_string()),
            (FieldKind::DateTime, Some("Timetz")) => Some("timetz".to_string()),
            (FieldKind::DateTime, Some("Timestamptz")) => Some("timestamptz".to_string()),
            (FieldKind::DateTime, _) => Some("timestamp".to_string()),
            (FieldKind::Int, Some("SmallInt")) => Some("int2".to_string()),
            (FieldKind::Int, Some("Oid")) => Some("oid".to_string()),
            (FieldKind::Float, Some("Real")) => Some("float4".to_string()),
            _ => None,
        };
        base.map(|b| if list { format!("{b}[]") } else { b })
    }

    /// Converts a JSON value into a bound value for `field`, validating its type.
    pub(crate) fn value(&self, f: &FieldMeta, v: &Value, allow_null: bool) -> Result<SqlValue> {
        let raw = self.raw_value(f, v, allow_null)?;
        Ok(match self.pg_cast(f, f.list) {
            Some(c) => SqlValue::Cast(Box::new(raw), c),
            None => raw,
        })
    }

    /// A single element of `f` (scalar lists: one item), without cast.
    pub(crate) fn element_value(&self, f: &FieldMeta, v: &Value) -> Result<SqlValue> {
        let raw = self.scalar_value(f, v, false)?;
        Ok(match self.pg_cast(f, false) {
            Some(c) => SqlValue::Cast(Box::new(raw), c),
            None => raw,
        })
    }

    fn raw_value(&self, f: &FieldMeta, v: &Value, allow_null: bool) -> Result<SqlValue> {
        if f.list {
            if v.is_null() {
                if f.optional || allow_null {
                    return Ok(SqlValue::Null(f.kind));
                }
                return Err(self.err(format!("Argument `{}` must not be null.", f.name)));
            }
            let items = match v {
                Value::Array(a) => a.clone(),
                // A single value is accepted for a list (the reference client does the same for scalar lists).
                other => vec![other.clone()],
            };
            let vals = items.iter().map(|x| self.scalar_value(f, x, false)).collect::<Result<Vec<_>>>()?;
            return Ok(SqlValue::Array(vals, f.kind));
        }
        self.scalar_value(f, v, allow_null)
    }

    fn scalar_value(&self, f: &FieldMeta, v: &Value, allow_null: bool) -> Result<SqlValue> {
        // `DbNull` / `JsonNull` / `AnyNull` sentinels (`{ "$type": ... }`).
        if f.kind == FieldKind::Json
            && let Some(t) = v.as_object().and_then(|o| o.get("$type")).and_then(|t| t.as_str())
        {
            match t {
                "DbNull" | "AnyNull" => {
                    if !f.optional && !allow_null {
                        return Err(self.err(format!("Argument `{}` must not be null.", f.name)));
                    }
                    return Ok(SqlValue::Null(f.kind));
                }
                "JsonNull" => return Ok(SqlValue::Json(Value::Null)),
                _ => {}
            }
        }
        if v.is_null() {
            if f.optional || allow_null {
                return Ok(SqlValue::Null(f.kind));
            }
            return Err(self.err(format!("Argument `{}` must not be null.", f.name)));
        }
        let expected = match (f.kind, f.enum_meta()) {
            (FieldKind::Enum, Some(e)) => e.name,
            (k, _) => k.schema_name(),
        };
        let bad = || self.err(format!("Argument `{}`: Invalid value provided. Expected {expected}, provided {}.", f.name, json_type(v)));
        Ok(match f.kind {
            FieldKind::Int => {
                let i = match v {
                    Value::Number(n) => match n.as_i64() {
                        Some(i) => i,
                        None => match n.as_f64() {
                            Some(x) if x.fract() == 0.0 && fits_i64(x) => x as i64,
                            _ => return Err(bad()),
                        },
                    },
                    _ => return Err(bad()),
                };
                match (i32::try_from(i), self.dialect()) {
                    (Ok(i), Dialect::Postgres) => SqlValue::Int32(i),
                    // Like the reference client, the query fails when the parameter is converted (`ToSql`).
                    (Err(_), Dialect::Postgres) => SqlValue::OutOfRange(i),
                    // SQLite integers are 64-bit and the reference client binds them as such (recorded: a filter
                    // with 3000000000 runs, a write stores it, and reading it into an `Int` field fails, see
                    // `Engine::int_out_of_range`).
                    (_, Dialect::Sqlite) => SqlValue::Int(i),
                }
            }
            // Out of the 64-bit range is an error, never a saturated value.
            FieldKind::BigInt => match v {
                Value::Number(n) => match n.as_i64() {
                    Some(i) => SqlValue::Int(i),
                    None => match n.as_f64() {
                        Some(x) if x.fract() == 0.0 && fits_i64(x) && !n.is_u64() => SqlValue::Int(x as i64),
                        _ => {
                            return Err(
                                self.err(format!("Argument `{}`: Invalid value provided. Expected BigInt, provided Float.", f.name))
                            );
                        }
                    },
                },
                // BigInt may come as a string (`"123"`, JSON-safe).
                Value::String(s) => SqlValue::Int(s.parse::<i64>().map_err(|_| bad())?),
                _ => return Err(bad()),
            },
            FieldKind::Float => match v {
                Value::Number(n) => SqlValue::Float(n.as_f64().ok_or_else(bad)?),
                Value::String(s) if s == "NaN" || s == "Infinity" || s == "-Infinity" => {
                    SqlValue::Float(s.replace("Infinity", "inf").parse().unwrap_or(f64::NAN))
                }
                _ => return Err(bad()),
            },
            FieldKind::Decimal => {
                let d = self.decimal_input(f, v)?;
                // A value that does not fit a NUMERIC stays a `Decimal`: running the query fails
                // with a conversion error (`check_params`), as in the reference engine, before
                // anything tries to print it.
                match (self.dialect(), d.fits_numeric()) {
                    (Dialect::Postgres, _) => SqlValue::Decimal(d),
                    (Dialect::Sqlite, true) => SqlValue::Text(d.to_fixed_plain()),
                    // SQLite reads it as a number (recorded: `lt: "1e140000"` matches every row, `lte: "9999e-262144"`
                    // none); the exponent form keeps the text short, where the fixed form would have 140,000 digits.
                    // Writes are bounded before they get here (`Engine::data_value`).
                    (Dialect::Sqlite, false) => SqlValue::Text(d.to_string()),
                }
            }
            FieldKind::Boolean => SqlValue::Bool(v.as_bool().ok_or_else(bad)?),
            FieldKind::String => {
                let s = v.as_str().ok_or_else(bad)?;
                if self.dialect() == Dialect::Postgres && f.native_name() == Some("Uuid") {
                    match uuid::Uuid::parse_str(s) {
                        Ok(u) => SqlValue::Uuid(u),
                        Err(_) => {
                            let e = uuid_error(s);
                            return Err(OrmError::known(
                                "P2023",
                                format!("Inconsistent column data: Error creating UUID, {e}"),
                                serde_json::json!({ "modelName": self.root.name, "message": format!("Error creating UUID, {e}") }),
                            ));
                        }
                    }
                } else {
                    SqlValue::Text(s.to_string())
                }
            }
            FieldKind::Enum => {
                let s = v.as_str().ok_or_else(bad)?;
                match f.enum_meta() {
                    Some(e) => match e.to_db(s) {
                        Some(db) => SqlValue::Text(db.to_string()),
                        None => {
                            return Err(self.err(format!("Invalid value for argument `{}`. Expected {}.", f.name, e.name)));
                        }
                    },
                    None => SqlValue::Text(s.to_string()),
                }
            }
            FieldKind::DateTime => {
                let d = match v {
                    Value::String(s) => parse_datetime(s).ok_or_else(|| {
                        self.err(format!("Argument `{}`: Invalid value provided. Expected ISO-8601 DateTime, provided \"{s}\".", f.name))
                    })?,
                    Value::Number(n) => n.as_i64().and_then(chrono::DateTime::from_timestamp_millis).ok_or_else(bad)?,
                    _ => return Err(bad()),
                };
                let d = crate::DateTime::from(d).inner();
                match (self.dialect(), f.native_name()) {
                    (Dialect::Postgres, Some("Date")) => SqlValue::Date(d.date_naive()),
                    (Dialect::Postgres, Some("Time" | "Timetz")) => SqlValue::Time(d.time()),
                    (Dialect::Postgres, Some("Timestamptz")) => SqlValue::DateTime(d),
                    (Dialect::Postgres, _) => SqlValue::NaiveDateTime(d.naive_utc()),
                    _ => SqlValue::DateTime(d),
                }
            }
            FieldKind::Json => SqlValue::Json(v.clone()),
            FieldKind::Bytes => match v {
                Value::Array(a) => {
                    SqlValue::Bytes(a.iter().map(|x| x.as_u64().and_then(|b| u8::try_from(b).ok()).ok_or_else(bad)).collect::<Result<_>>()?)
                }
                Value::Object(o) if o.get("$type").and_then(|t| t.as_str()) == Some("Bytes") => {
                    let s = o.get("value").and_then(|x| x.as_str()).ok_or_else(bad)?;
                    SqlValue::Bytes(crate::util::base64_decode(s).ok_or_else(bad)?)
                }
                Value::String(s) => SqlValue::Bytes(crate::util::base64_decode(s).ok_or_else(bad)?),
                _ => return Err(bad()),
            },
            FieldKind::Relation => return Err(self.err(format!("`{}` is a relation, not a scalar", f.name))),
        })
    }

    /// A `Decimal` argument (number or decimal string).
    fn decimal_input(&self, f: &FieldMeta, v: &Value) -> Result<crate::Decimal> {
        Ok(match v {
            Value::Number(n) => match (n.as_i64(), n.as_u64()) {
                (Some(i), _) => crate::Decimal::from(i),
                (_, Some(u)) => crate::Decimal::from(u),
                _ => crate::Decimal::from(n.as_f64().ok_or_else(|| {
                    self.err(format!("Argument `{}`: Invalid value provided. Expected Decimal, provided Float.", f.name))
                })?),
            },
            Value::String(s) => crate::Decimal::parse(s)
                .map_err(|_| self.err(format!("Argument `{}`: Invalid value provided. Expected Decimal, provided String.", f.name)))?,
            other => {
                return Err(self.err(format!(
                    "Argument `{}`: Invalid value provided. Expected Decimal, provided {}.",
                    f.name,
                    json_type(other)
                )));
            }
        })
    }

    /// A value written to a column (`data` of creates, updates and upserts).
    ///
    /// Postgres enforces the precision and scale of `NUMERIC(p, s)` columns itself. SQLite does not: a
    /// `DECIMAL` column has numeric affinity and keeps what it is given as a 64-bit integer or float (15
    /// significant digits), like with the reference client's own SQLite migration. So a `Decimal` is
    /// checked against the column it stands for, `@db.Decimal(p, s)` or the default `Decimal(65, 30)`: it
    /// is rounded to `s` decimal places like Postgres does, and a value whose integer part does not fit is
    /// rejected. This also bounds what a short input such as `"1e100000"` stores.
    pub(crate) fn data_value(&self, f: &FieldMeta, v: &Value) -> Result<SqlValue> {
        if self.dialect() != Dialect::Sqlite || f.kind != FieldKind::Decimal || f.list || !(v.is_string() || v.is_number()) {
            return self.value(f, v, false);
        }
        let d = self.decimal_input(f, v)?;
        if !d.is_finite() {
            return self.value(f, v, false);
        }
        let (p, s) = decimal_precision(f);
        let r = d.to_decimal_places_with(s, crate::Decimal::ROUND_HALF_UP);
        if !r.is_zero() && r.abs().gte(crate::Decimal::from(10).pow(p as i64 - s as i64)) {
            return Err(self.err(format!(
                "Argument `{}`: Invalid value provided. A Decimal({p}, {s}) value must round to an absolute value less than 10^{}.",
                f.name,
                p - s
            )));
        }
        Ok(SqlValue::Text(r.to_fixed_plain()))
    }

    /// Value generated for a field missing from `data` on create.
    pub(crate) fn default_value(&self, f: &FieldMeta) -> Result<Option<SqlValue>> {
        let now = || Value::String(crate::DateTime::now().to_iso_string());
        if f.updated_at {
            return Ok(Some(self.value(f, &now(), false)?));
        }
        let text = |s: String| -> Result<Option<SqlValue>> { Ok(Some(self.value(f, &Value::String(s), false)?)) };
        match f.default {
            None | Some(DefaultValue::Autoincrement) | Some(DefaultValue::DbGenerated(_)) => Ok(None),
            Some(DefaultValue::Now) => Ok(Some(self.value(f, &now(), false)?)),
            Some(DefaultValue::Uuid) => text(uuid::Uuid::new_v4().to_string()),
            Some(DefaultValue::UuidV7) => text(uuid::Uuid::now_v7().to_string()),
            Some(DefaultValue::Cuid) => text(crate::util::cuid()),
            Some(DefaultValue::Cuid2) => text(crate::util::cuid2()),
            Some(DefaultValue::Nanoid(n)) => text(crate::util::nanoid(n as usize)),
            Some(DefaultValue::Ulid) => text(crate::util::ulid()),
            Some(DefaultValue::Literal(l)) => {
                let mut v: Value = serde_json::from_str(l).map_err(|e| self.err(format!("Invalid default for `{}`: {e}", f.name)))?;
                // `Json @default("{}")` holds the JSON text.
                if f.kind == FieldKind::Json
                    && !f.list
                    && let Value::String(text) = &v
                    && let Ok(parsed) = serde_json::from_str::<Value>(text)
                {
                    v = parsed;
                }
                Ok(Some(self.value(f, &v, false)?))
            }
        }
    }

    /// Expands compound unique keys (`{ a_b: { a, b } }`) into plain fields, keeping the rest.
    pub(crate) fn expand_compound(&self, model: &'static ModelMeta, w: &Map<String, Value>) -> Result<Map<String, Value>> {
        let mut out = Map::new();
        for (k, v) in w {
            if model.field(k).is_none()
                && let Some(fields) = model.compound_unique(k)
            {
                let o = self.obj(v, k, &format!("{}{}CompoundUniqueInput", model.name, pascal(k)))?;
                for f in &fields {
                    match o.get(f.name) {
                        Some(x) => {
                            out.insert(f.name.to_string(), x.clone());
                        }
                        None => {
                            return Err(self.err(format!("Argument `{}` is missing.", f.name)));
                        }
                    }
                }
                let allowed: Vec<&str> = fields.iter().map(|f| f.name).collect();
                self.check_keys(o, &allowed)?;
                continue;
            }
            out.insert(k.clone(), v.clone());
        }
        Ok(out)
    }

    /// `{ id: 1 }` / `{ email: { equals: "x" } }` / `{ a_b: { a, b } }` – a `WhereUniqueInput`.
    /// Returns the where with compound keys expanded into plain fields.
    pub(crate) fn check_unique_where(&self, model: &'static ModelMeta, w: &Value) -> Result<Value> {
        let obj = self.obj(w, "where", &format!("{}WhereUniqueInput", model.name))?;
        // Unique fields (`@id`, `@unique` and the members of a compound key) only take the value
        // itself, never a filter object or `null`, like the reference client: otherwise
        // `{ email, resetToken: { not: "" } }` turns a lookup by two secrets into a lookup by one.
        let is_unique = |f: &FieldMeta| f.is_scalar() && !f.list && (f.is_unique || (f.is_id && !model.has_compound_pk()));
        for (k, v) in obj {
            if let Some(f) = model.field(k) {
                if is_unique(f) {
                    self.unique_value(f, v)?;
                }
            } else if let Some(fields) = model.compound_unique(k)
                && let Value::Object(o) = v
            {
                for f in &fields {
                    if let Some(x) = o.get(f.name) {
                        self.unique_value(f, x)?;
                    }
                }
            }
        }
        let has_unique = obj.iter().any(|(k, v)| {
            model.field(k).is_some_and(|f| is_unique(f) && !v.is_null())
                || (model.field(k).is_none() && model.compound_unique(k).is_some() && v.is_object())
        });
        if !has_unique {
            let mut names: Vec<String> =
                model.scalars().filter(|f| (f.is_id && !model.has_compound_pk()) || f.is_unique).map(|f| format!("`{}`", f.name)).collect();
            names.extend(model.compound_unique_names().into_iter().map(|n| format!("`{n}`")));
            return Err(self.err(format!(
                "Argument `where` of type {}WhereUniqueInput needs at least one of {} arguments.",
                model.name,
                names.join(" or ")
            )));
        }
        Ok(Value::Object(self.expand_compound(model, obj)?))
    }
}

impl Engine {
    /// The value of a unique field in a `WhereUniqueInput`: a scalar of its type, like the
    /// reference client (``Argument `email`: Invalid value provided. Expected String, provided Object.``).
    fn unique_value(&self, f: &FieldMeta, v: &Value) -> Result<()> {
        match v {
            Value::Null => Err(self.err(format!("Argument `{}` must not be null.", f.name))),
            Value::Object(_) | Value::Array(_) => {
                let expected = match (f.kind, f.enum_meta()) {
                    (FieldKind::Enum, Some(e)) => e.name,
                    (k, _) => k.schema_name(),
                };
                Err(self.err(format!("Argument `{}`: Invalid value provided. Expected {expected}, provided {}.", f.name, json_type(v))))
            }
            _ => Ok(()),
        }
    }
}

/// `true` when an integral `f64` is inside the `i64` range (`as i64` would saturate otherwise).
fn fits_i64(x: f64) -> bool {
    (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&x)
}

/// `(precision, scale)` of a `Decimal` column: `@db.Decimal(p, s)` or the default `(65, 30)`.
fn decimal_precision(f: &FieldMeta) -> (u32, u32) {
    let parsed = f.native_type.filter(|_| f.native_name() == Some("Decimal")).and_then(|t| {
        let args = t.split_once('(')?.1.trim_end_matches(')');
        let mut it = args.split(',').map(|x| x.trim().parse::<u32>());
        let p = it.next()?.ok()?;
        let s = it.next().and_then(|x| x.ok()).unwrap_or(0);
        (s <= p).then_some((p, s))
    });
    parsed.unwrap_or((65, 30))
}

/// The message of a UUID parse error, worded like the `uuid` crate used by the reference query engine.
fn uuid_error(s: &str) -> String {
    let b = s.as_bytes();
    let (body, offset, simple) = if b.len() >= 2 && b[0] == b'{' && b[b.len() - 1] == b'}' {
        (&s[1..s.len() - 1], 1, false)
    } else if let Some(rest) = s.strip_prefix("urn:uuid:") {
        (rest, "urn:uuid:".len(), false)
    } else {
        (s, 0, true)
    };
    let mut hyphens = 0usize;
    let mut bounds = [0usize; 4];
    for (i, c) in body.char_indices() {
        if c == '-' {
            if hyphens < 4 {
                bounds[hyphens] = i;
            }
            hyphens += 1;
        } else if !c.is_ascii_hexdigit() {
            return format!(
                "invalid character: expected an optional prefix of `urn:uuid:` followed by [0-9a-fA-F-], found `{c}` at {}",
                i + offset + 1
            );
        }
    }
    if hyphens == 0 && simple {
        return format!("invalid length: expected length 32 for simple format, found {}", body.len());
    }
    if hyphens != 4 {
        return format!("invalid group count: expected 5, found {}", hyphens + 1);
    }
    const STARTS: [usize; 5] = [0, 9, 14, 19, 24];
    const LENS: [usize; 5] = [8, 4, 4, 4, 12];
    let mut start = 0;
    for (g, end) in bounds.iter().enumerate() {
        if *end != STARTS[g + 1] - 1 {
            return format!("invalid group length in group {g}: expected {}, found {}", LENS[g], end - start);
        }
        start = end + 1;
    }
    format!("invalid group length in group 4: expected 12, found {}", body.len() - start)
}

/// `itemId_month` → `ItemIdMonth`
fn pascal(s: &str) -> String {
    s.split('_')
        .map(|p| {
            let mut c = p.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

/// Stable string key of a JSON value (for grouping rows).
pub(crate) fn key_of(values: &[&Value]) -> String {
    values.iter().map(|v| v.to_string()).collect::<Vec<_>>().join("\u{1}")
}

/// Compiles a `where` argument to SQL (debugging / tests).
pub(crate) fn where_to_sql(client: &OrmClient, model: &'static ModelMeta, w: &Value) -> Result<(String, Vec<SqlValue>)> {
    let e = Engine { client: client.clone(), action: "findMany", root: model };
    if exceeds_depth(w, args_depth_limit(client)) {
        return Err(depth_error(&e, args_depth_limit(client)));
    }
    let mut q = crate::db::Sql::new(client.dialect());
    e.where_sql(&mut q, model, "t0", w)?;
    Ok((q.text, q.params))
}
