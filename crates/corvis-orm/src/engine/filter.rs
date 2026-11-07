//! `where` and `orderBy` compilation.

use serde_json::{Map, Value};

use super::{Engine, json_type};
use crate::db::{Dialect, Sql, SqlValue};
use crate::error::Result;
use crate::meta::{FieldKind, FieldMeta, ModelMeta, RelationKind, ResolvedRelation};

const SCALAR_OPS: &[&str] =
    &["equals", "not", "in", "notIn", "lt", "lte", "gt", "gte", "contains", "startsWith", "endsWith", "mode", "search"];
const LIST_OPS: &[&str] = &["equals", "has", "hasEvery", "hasSome", "isEmpty"];
const JSON_OPS: &[&str] = &[
    "equals", "not", "path", "mode", "string_contains", "string_starts_with", "string_ends_with", "array_contains",
    "array_starts_with", "array_ends_with", "lt", "lte", "gt", "gte",
];

fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// `[v]` as JSON text (so `json_extract(.., '$[0]')` yields `v` in SQLite's representation).
fn json_wrap(v: &Value) -> String {
    Value::Array(vec![v.clone()]).to_string()
}

/// The `DbNull` / `JsonNull` / `AnyNull` sentinels (`{ "$type": ... }`).
fn null_sentinel(v: &Value) -> Option<&str> {
    v.as_object().filter(|o| o.len() == 1).and_then(|o| o.get("$type")).and_then(|t| t.as_str()).filter(|t| matches!(*t, "DbNull" | "JsonNull" | "AnyNull"))
}

/// What an `orderBy` entry sorts on.
#[derive(Debug, Clone)]
pub(crate) enum OrderExpr {
    Field(&'static FieldMeta),
    /// `{ author: { name: "asc" } }` (possibly through several to-one relations).
    RelationField(Vec<ResolvedRelation>, &'static FieldMeta),
    /// `{ posts: { _count: "desc" } }`
    RelationCount(ResolvedRelation),
    /// groupBy: `{ _count: { id: "desc" } }`
    Aggregate(&'static str, Option<&'static FieldMeta>),
}

#[derive(Debug, Clone)]
pub(crate) struct OrderItem {
    pub expr: OrderExpr,
    pub desc: bool,
    pub nulls: Option<&'static str>,
}

impl Engine {
    /// Writes a boolean SQL expression for a `where` argument.
    pub(crate) fn where_sql(&self, q: &mut Sql, model: &'static ModelMeta, alias: &str, w: &Value) -> Result<()> {
        match w {
            Value::Null => {
                q.push("1=1");
                Ok(())
            }
            Value::Object(map) => self.where_obj(q, model, alias, map),
            other => Err(self.err(format!(
                "Argument `where`: Invalid value provided. Expected {}WhereInput, provided {}.",
                model.name,
                json_type(other)
            ))),
        }
    }

    fn where_obj(&self, q: &mut Sql, model: &'static ModelMeta, alias: &str, map: &Map<String, Value>) -> Result<()> {
        if map.is_empty() {
            q.push("1=1");
            return Ok(());
        }
        q.push("(");
        for (i, (k, v)) in map.iter().enumerate() {
            if i > 0 {
                q.push(" AND ");
            }
            match k.as_str() {
                "AND" => self.where_list(q, model, alias, v, " AND ", "1=1")?,
                "OR" => self.where_list(q, model, alias, v, " OR ", "1=0")?,
                "NOT" => match v {
                    // `NOT: [a, b]` is `NOT a AND NOT b`.
                    Value::Array(items) if !items.is_empty() => {
                        q.push("(");
                        for (j, item) in items.iter().enumerate() {
                            if j > 0 {
                                q.push(" AND ");
                            }
                            q.push("NOT ");
                            self.where_sql(q, model, alias, item)?;
                        }
                        q.push(")");
                    }
                    Value::Array(_) => {
                        q.push("1=1");
                    }
                    other => {
                        q.push("NOT ");
                        self.where_sql(q, model, alias, other)?;
                    }
                },
                key => {
                    let f = match model.field(key) {
                        Some(f) => f,
                        None => {
                            let mut options: Vec<&str> = vec!["AND", "OR", "NOT"];
                            options.extend(model.fields.iter().map(|f| f.name));
                            return Err(self.unknown_argument(key, options));
                        }
                    };
                    if f.is_relation() {
                        self.relation_filter(q, model, alias, f, v)?;
                    } else if f.list {
                        self.list_filter(q, alias, f, v)?;
                    } else if f.kind == FieldKind::Json {
                        self.json_filter(q, alias, f, v)?;
                    } else {
                        self.scalar_filter(q, alias, f, v)?;
                    }
                }
            }
        }
        q.push(")");
        Ok(())
    }

    fn where_list(&self, q: &mut Sql, model: &'static ModelMeta, alias: &str, v: &Value, sep: &str, empty: &str) -> Result<()> {
        match v {
            Value::Array(items) => {
                if items.is_empty() {
                    q.push(empty);
                    return Ok(());
                }
                q.push("(");
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        q.push(sep);
                    }
                    self.where_sql(q, model, alias, item)?;
                }
                q.push(")");
                Ok(())
            }
            other => self.where_sql(q, model, alias, other),
        }
    }

    fn scalar_filter(&self, q: &mut Sql, alias: &str, f: &'static FieldMeta, v: &Value) -> Result<()> {
        match v {
            Value::Null => {
                q.col(alias, f.column).push(" IS NULL");
                Ok(())
            }
            Value::Object(ops) => self.scalar_ops(q, alias, f, ops),
            other => {
                let val = self.value(f, other, true)?;
                q.col(alias, f.column).push(" = ").bind(val);
                Ok(())
            }
        }
    }

    fn scalar_ops(&self, q: &mut Sql, alias: &str, f: &'static FieldMeta, ops: &Map<String, Value>) -> Result<()> {
        let insensitive = match ops.get("mode") {
            None => false,
            Some(Value::String(s)) if s == "insensitive" => true,
            Some(Value::String(s)) if s == "default" => false,
            Some(other) => {
                return Err(self.err(format!(
                    "Argument `mode`: Invalid value provided. Expected QueryMode (\"default\" | \"insensitive\"), provided {}.",
                    json_type(other)
                )));
            }
        };
        let entries: Vec<(&String, &Value)> = ops.iter().filter(|(k, _)| k.as_str() != "mode").collect();
        if entries.is_empty() {
            q.push("1=1");
            return Ok(());
        }
        let ci = insensitive && f.kind == FieldKind::String;
        let col = |q: &mut Sql| {
            if ci {
                q.push("LOWER(").col(alias, f.column).push(")");
            } else {
                q.col(alias, f.column);
            }
        };
        let bind = |q: &mut Sql, v: SqlValue| {
            if ci {
                q.push("LOWER(").bind(v).push(")");
            } else {
                q.bind(v);
            }
        };
        q.push("(");
        for (i, (op, val)) in entries.into_iter().enumerate() {
            if i > 0 {
                q.push(" AND ");
            }
            match op.as_str() {
                "equals" => {
                    if val.is_null() {
                        q.col(alias, f.column).push(" IS NULL");
                    } else {
                        let v = self.value(f, val, true)?;
                        col(q);
                        q.push(" = ");
                        bind(q, v);
                    }
                }
                "not" => match val {
                    Value::Null => {
                        q.col(alias, f.column).push(" IS NOT NULL");
                    }
                    Value::Object(inner) => {
                        // `not: { contains: .., mode }` inherits the outer mode.
                        let mut inner = inner.clone();
                        if let Some(m) = ops.get("mode")
                            && !inner.contains_key("mode")
                        {
                            inner.insert("mode".into(), m.clone());
                        }
                        q.push("NOT ");
                        self.scalar_ops(q, alias, f, &inner)?;
                    }
                    other => {
                        let v = self.value(f, other, true)?;
                        col(q);
                        q.push(" <> ");
                        bind(q, v);
                    }
                },
                "in" | "notIn" => {
                    let items: Vec<Value> = match val {
                        Value::Array(a) => a.clone(),
                        Value::Null => {
                            return Err(self.err(format!("Argument `{op}` must not be null.")));
                        }
                        other => vec![other.clone()],
                    };
                    if items.is_empty() {
                        q.push(if op == "in" { "1=0" } else { "1=1" });
                        continue;
                    }
                    col(q);
                    q.push(if op == "in" { " IN (" } else { " NOT IN (" });
                    for (j, item) in items.iter().enumerate() {
                        if j > 0 {
                            q.push(", ");
                        }
                        let v = self.value(f, item, false)?;
                        bind(q, v);
                    }
                    q.push(")");
                }
                "lt" | "lte" | "gt" | "gte" => {
                    let v = self.value(f, val, false)?;
                    let sym = match op.as_str() {
                        "lt" => " < ",
                        "lte" => " <= ",
                        "gt" => " > ",
                        _ => " >= ",
                    };
                    col(q);
                    q.push(sym);
                    bind(q, v);
                }
                "contains" | "startsWith" | "endsWith" => {
                    if f.kind != FieldKind::String {
                        return Err(self.unknown_argument(op, ["equals", "in", "notIn", "lt", "lte", "gt", "gte", "not"]));
                    }
                    let s = val.as_str().ok_or_else(|| {
                        self.err(format!("Argument `{op}`: Invalid value provided. Expected String, provided {}.", json_type(val)))
                    })?;
                    self.string_match(q, alias, f, op, s, insensitive);
                }
                "search" => {
                    let s = val.as_str().ok_or_else(|| {
                        self.err(format!("Argument `search`: Invalid value provided. Expected String, provided {}.", json_type(val)))
                    })?;
                    match self.dialect() {
                        Dialect::Postgres => {
                            q.push("to_tsvector(concat_ws(' ', ").col(alias, f.column).push(")) @@ to_tsquery(");
                            q.bind(SqlValue::Text(s.to_string())).push(")");
                        }
                        Dialect::Sqlite => {
                            // No full-text index: every term must appear.
                            let terms: Vec<String> =
                                s.split(|c: char| c.is_whitespace() || "&|!()<>:*".contains(c)).filter(|t| !t.is_empty()).map(str::to_string).collect();
                            if terms.is_empty() {
                                q.push("1=1");
                            }
                            for (j, t) in terms.iter().enumerate() {
                                if j > 0 {
                                    q.push(" AND ");
                                }
                                q.push("INSTR(LOWER(").col(alias, f.column).push("), LOWER(").bind(SqlValue::Text(t.clone())).push(")) > 0");
                            }
                        }
                    }
                }
                other => {
                    return Err(self.unknown_argument(other, SCALAR_OPS.iter().copied()));
                }
            }
        }
        q.push(")");
        Ok(())
    }

    fn string_match(&self, q: &mut Sql, alias: &str, f: &FieldMeta, op: &str, s: &str, insensitive: bool) {
        let pattern = match op {
            "contains" => format!("%{}%", escape_like(s)),
            "startsWith" => format!("{}%", escape_like(s)),
            _ => format!("%{}", escape_like(s)),
        };
        match (self.dialect(), insensitive) {
            (Dialect::Postgres, ci) => {
                q.col(alias, f.column).push(if ci { "::text ILIKE " } else { "::text LIKE " });
                q.bind(SqlValue::Text(pattern));
            }
            (Dialect::Sqlite, true) => {
                q.push("LOWER(").col(alias, f.column).push(") LIKE LOWER(");
                q.bind(SqlValue::Text(pattern)).push(") ESCAPE '\\'");
            }
            // SQLite's LIKE is case-insensitive: use exact string functions instead.
            (Dialect::Sqlite, false) => match op {
                "contains" => {
                    q.push("INSTR(").col(alias, f.column).push(", ");
                    q.bind(SqlValue::Text(s.to_string())).push(") > 0");
                }
                "startsWith" => {
                    q.push("SUBSTR(").col(alias, f.column).push(", 1, LENGTH(");
                    q.bind(SqlValue::Text(s.to_string())).push(")) = ");
                    q.bind(SqlValue::Text(s.to_string()));
                }
                _ => {
                    q.push("SUBSTR(").col(alias, f.column).push(", -LENGTH(");
                    q.bind(SqlValue::Text(s.to_string())).push(")) = ");
                    q.bind(SqlValue::Text(s.to_string()));
                }
            },
        }
    }

    /// Scalar list filters: `equals`, `has`, `hasEvery`, `hasSome`, `isEmpty`.
    fn list_filter(&self, q: &mut Sql, alias: &str, f: &'static FieldMeta, v: &Value) -> Result<()> {
        let ops = match v {
            Value::Null => {
                q.col(alias, f.column).push(" IS NULL");
                return Ok(());
            }
            Value::Object(o) => o,
            other => {
                return Err(self.err(format!(
                    "Argument `{}`: Invalid value provided. Expected {}NullableListFilter, provided {}.",
                    f.name,
                    f.kind.schema_name(),
                    json_type(other)
                )));
            }
        };
        if ops.is_empty() {
            q.push("1=1");
            return Ok(());
        }
        let pg = self.dialect() == Dialect::Postgres;
        q.push("(");
        for (i, (op, val)) in ops.iter().enumerate() {
            if i > 0 {
                q.push(" AND ");
            }
            match op.as_str() {
                "equals" => {
                    if val.is_null() {
                        q.col(alias, f.column).push(" IS NULL");
                    } else {
                        let v = self.value(f, val, false)?;
                        if pg {
                            q.col(alias, f.column).push(" = ").bind(v);
                        } else {
                            q.push("json(").col(alias, f.column).push(") = json(").bind(v).push(")");
                        }
                    }
                }
                "has" => {
                    if val.is_null() {
                        q.push("1=0");
                        continue;
                    }
                    let v = self.element_value(f, val)?;
                    if pg {
                        q.bind(v).push(" = ANY(").col(alias, f.column).push(")");
                    } else {
                        q.push("EXISTS (SELECT 1 FROM json_each(").col(alias, f.column).push(") WHERE value = ").bind(v).push(")");
                    }
                }
                "hasEvery" | "hasSome" => {
                    let items = val.as_array().cloned().unwrap_or_else(|| vec![val.clone()]);
                    if items.is_empty() {
                        q.push(if op == "hasEvery" { "1=1" } else { "1=0" });
                        continue;
                    }
                    if pg {
                        let v = self.value(f, &Value::Array(items), false)?;
                        q.col(alias, f.column).push(if op == "hasEvery" { " @> " } else { " && " }).bind(v);
                    } else {
                        q.push("(");
                        for (j, item) in items.iter().enumerate() {
                            if j > 0 {
                                q.push(if op == "hasEvery" { " AND " } else { " OR " });
                            }
                            let v = self.element_value(f, item)?;
                            q.push("EXISTS (SELECT 1 FROM json_each(").col(alias, f.column).push(") WHERE value = ").bind(v).push(")");
                        }
                        q.push(")");
                    }
                }
                "isEmpty" => {
                    let empty = val.as_bool().ok_or_else(|| {
                        self.err(format!("Argument `isEmpty`: Invalid value provided. Expected Boolean, provided {}.", json_type(val)))
                    })?;
                    match (pg, empty) {
                        (true, true) => {
                            q.push("COALESCE(array_length(").col(alias, f.column).push(", 1), 0) = 0");
                        }
                        (true, false) => {
                            q.push("COALESCE(array_length(").col(alias, f.column).push(", 1), 0) > 0");
                        }
                        (false, true) => {
                            q.push("COALESCE(json_array_length(").col(alias, f.column).push("), 0) = 0");
                        }
                        (false, false) => {
                            q.push("COALESCE(json_array_length(").col(alias, f.column).push("), 0) > 0");
                        }
                    }
                }
                other => return Err(self.unknown_argument(other, LIST_OPS.iter().copied())),
            }
        }
        q.push(")");
        Ok(())
    }

    /// The JSON value at `path` (`col #> '{a,b}'` / `json_extract(col, '$.a.b')`).
    fn json_at(&self, q: &mut Sql, alias: &str, f: &FieldMeta, path: Option<&[String]>, as_text: bool) {
        let jsonb = |q: &mut Sql| {
            if f.native_name() == Some("Json") {
                q.col(alias, f.column).push("::jsonb");
            } else {
                q.col(alias, f.column);
            }
        };
        match (self.dialect(), path) {
            (Dialect::Postgres, None) => {
                if as_text {
                    q.push("(");
                    jsonb(q);
                    q.push(" #>> '{}')");
                } else {
                    jsonb(q);
                }
            }
            (Dialect::Postgres, Some(p)) => {
                q.push("(");
                jsonb(q);
                q.push(if as_text { " #>> " } else { " #> " });
                let lit = format!("{{{}}}", p.iter().map(|s| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))).collect::<Vec<_>>().join(","));
                q.push(&format!("'{}'", lit.replace('\'', "''"))).push(")");
            }
            (Dialect::Sqlite, p) => {
                let mut path = String::from("$");
                for s in p.unwrap_or(&[]) {
                    if s.parse::<usize>().is_ok() {
                        path.push_str(&format!("[{s}]"));
                    } else {
                        path.push_str(&format!(".\"{}\"", s.replace('"', "\\\"")));
                    }
                }
                if as_text || path != "$" {
                    q.push("json_extract(").col(alias, f.column).push(", ").bind(SqlValue::Text(path)).push(")");
                } else {
                    q.push("json(").col(alias, f.column).push(")");
                }
            }
        }
    }

    fn json_literal(&self, q: &mut Sql, v: &Value) {
        match self.dialect() {
            Dialect::Postgres => {
                q.bind(SqlValue::Cast(Box::new(SqlValue::Json(v.clone())), "jsonb".into()));
            }
            Dialect::Sqlite => {
                q.push("json(").bind(SqlValue::Text(v.to_string())).push(")");
            }
        }
    }

    /// Json filters (`equals`, `not`, `path`, `string_*`, `array_*`, comparisons).
    fn json_filter(&self, q: &mut Sql, alias: &str, f: &'static FieldMeta, v: &Value) -> Result<()> {
        let ops = match v {
            Value::Object(o) if null_sentinel(v).is_none() => o,
            other => {
                return Err(self.err(format!(
                    "Argument `{}`: Invalid value provided. Expected Json{}Filter, provided {}.",
                    f.name,
                    if f.optional { "Nullable" } else { "" },
                    json_type(other)
                )));
            }
        };
        self.check_keys(ops, JSON_OPS)?;
        let path: Option<Vec<String>> = match ops.get("path") {
            None | Some(Value::Null) => None,
            Some(Value::Array(a)) => Some(a.iter().map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())).collect()),
            Some(Value::String(s)) => {
                // SQLite / MySQL style `$.a.b[0]`.
                let s = s.trim_start_matches('$');
                Some(
                    s.split(['.', '['])
                        .map(|p| p.trim_end_matches(']').trim_matches('"').to_string())
                        .filter(|p| !p.is_empty())
                        .collect(),
                )
            }
            Some(other) => {
                return Err(self.err(format!("Argument `path`: Invalid value provided. Expected List<String>, provided {}.", json_type(other))));
            }
        };
        let insensitive = ops.get("mode").and_then(|m| m.as_str()) == Some("insensitive");
        let entries: Vec<(&String, &Value)> = ops.iter().filter(|(k, _)| !matches!(k.as_str(), "path" | "mode")).collect();
        if entries.is_empty() {
            q.push("1=1");
            return Ok(());
        }
        let p = path.as_deref();
        let pg = self.dialect() == Dialect::Postgres;
        q.push("(");
        for (i, (op, val)) in entries.into_iter().enumerate() {
            if i > 0 {
                q.push(" AND ");
            }
            match op.as_str() {
                "equals" | "not" => {
                    let neg = op == "not";
                    match null_sentinel(val) {
                        Some("DbNull") => {
                            q.col(alias, f.column).push(if neg { " IS NOT NULL" } else { " IS NULL" });
                        }
                        Some("JsonNull") => {
                            if neg {
                                q.push("(");
                                self.json_at(q, alias, f, p, false);
                                q.push(" IS NOT NULL AND ");
                                self.json_at(q, alias, f, p, false);
                                q.push(" <> ");
                                self.json_literal(q, &Value::Null);
                                q.push(")");
                            } else {
                                self.json_at(q, alias, f, p, false);
                                q.push(" = ");
                                self.json_literal(q, &Value::Null);
                            }
                        }
                        Some(_) => {
                            // AnyNull
                            q.push(if neg { "NOT " } else { "" }).push("(");
                            q.col(alias, f.column).push(" IS NULL OR ");
                            self.json_at(q, alias, f, p, false);
                            q.push(" = ");
                            self.json_literal(q, &Value::Null);
                            q.push(")");
                        }
                        None => {
                            self.json_at(q, alias, f, p, false);
                            q.push(if neg { " <> " } else { " = " });
                            self.json_literal(q, val);
                        }
                    }
                }
                "string_contains" | "string_starts_with" | "string_ends_with" => {
                    let s = val.as_str().ok_or_else(|| self.err(format!("Argument `{op}`: Invalid value provided. Expected String, provided {}.", json_type(val))))?;
                    let pattern = match op.as_str() {
                        "string_contains" => format!("%{}%", escape_like(s)),
                        "string_starts_with" => format!("{}%", escape_like(s)),
                        _ => format!("%{}", escape_like(s)),
                    };
                    if pg {
                        q.push("(jsonb_typeof(");
                        self.json_at(q, alias, f, p, false);
                        q.push(") = 'string' AND ");
                        self.json_at(q, alias, f, p, true);
                        q.push(if insensitive { " ILIKE " } else { " LIKE " }).bind(SqlValue::Text(pattern)).push(")");
                    } else {
                        if insensitive {
                            q.push("LOWER(");
                        }
                        self.json_at(q, alias, f, p, true);
                        if insensitive {
                            q.push(") LIKE LOWER(").bind(SqlValue::Text(pattern)).push(") ESCAPE '\\'");
                        } else {
                            q.push(" GLOB ").bind(SqlValue::Text(pattern.replace("\\%", "%").replace("\\_", "_").replace('%', "*")));
                        }
                    }
                }
                "array_contains" | "array_starts_with" | "array_ends_with" => {
                    if pg {
                        match op.as_str() {
                            "array_contains" => {
                                let arr = if val.is_array() { val.clone() } else { Value::Array(vec![val.clone()]) };
                                q.push("(jsonb_typeof(");
                                self.json_at(q, alias, f, p, false);
                                q.push(") = 'array' AND ");
                                self.json_at(q, alias, f, p, false);
                                q.push(" @> ");
                                self.json_literal(q, &arr);
                                q.push(")");
                            }
                            "array_starts_with" => {
                                q.push("(");
                                self.json_at(q, alias, f, p, false);
                                q.push(" -> 0) = ");
                                self.json_literal(q, val);
                            }
                            _ => {
                                q.push("(");
                                self.json_at(q, alias, f, p, false);
                                q.push(" -> -1) = ");
                                self.json_literal(q, val);
                            }
                        }
                    } else {
                        let items = if val.is_array() && op == "array_contains" { val.as_array().unwrap().clone() } else { vec![val.clone()] };
                        let mut path = String::from("$");
                        for s in p.unwrap_or(&[]) {
                            path.push_str(&format!(".\"{s}\""));
                        }
                        match op.as_str() {
                            "array_contains" => {
                                q.push("(");
                                for (j, it) in items.iter().enumerate() {
                                    if j > 0 {
                                        q.push(" AND ");
                                    }
                                    q.push("EXISTS (SELECT 1 FROM json_each(").col(alias, f.column).push(", ").bind(SqlValue::Text(path.clone()));
                                    q.push(") WHERE json_quote(value) = json(").bind(SqlValue::Text(it.to_string())).push(") OR value = json(").bind(SqlValue::Text(it.to_string())).push("))");
                                }
                                q.push(")");
                            }
                            other => {
                                let idx = if other == "array_starts_with" { "[0]" } else { "[#-1]" };
                                q.push("json_extract(").col(alias, f.column).push(", ").bind(SqlValue::Text(format!("{path}{idx}"))).push(") = json_extract(json(");
                                q.bind(SqlValue::Text(json_wrap(val))).push("), '$[0]')");
                            }
                        }
                    }
                }
                "lt" | "lte" | "gt" | "gte" => {
                    let sym = match op.as_str() {
                        "lt" => " < ",
                        "lte" => " <= ",
                        "gt" => " > ",
                        _ => " >= ",
                    };
                    self.json_at(q, alias, f, p, false);
                    q.push(sym);
                    self.json_literal(q, val);
                }
                other => return Err(self.unknown_argument(other, JSON_OPS.iter().copied())),
            }
        }
        q.push(")");
        Ok(())
    }

    /// `EXISTS (SELECT 1 FROM target AS tN [JOIN ...] WHERE <join> [AND [NOT] <inner>])`
    fn exists(&self, q: &mut Sql, r: &ResolvedRelation, parent_alias: &str, inner: Option<&Value>, negate_inner: bool) -> Result<()> {
        let a = q.alias();
        q.push("EXISTS (SELECT 1 FROM ").table(r.target.schema, r.target.table).push(" AS ").push(&a);
        if let Some(join) = &r.join {
            let j = q.alias();
            q.push(" INNER JOIN ").table(join.schema, &join.table).push(" AS ").push(&j).push(" ON ");
            q.col(&j, join.target_column).push(" = ").col(&a, r.target_fields[0].column);
            q.push(" WHERE ").col(&j, join.parent_column).push(" = ").col(parent_alias, r.parent_fields[0].column);
        } else {
            q.push(" WHERE ");
            self.join_sql(q, r, parent_alias, &a);
        }
        if let Some(w) = inner {
            q.push(" AND ");
            if negate_inner {
                q.push("NOT ");
            }
            self.where_sql(q, r.target, &a, w)?;
        }
        q.push(")");
        Ok(())
    }

    pub(crate) fn join_sql(&self, q: &mut Sql, r: &ResolvedRelation, parent_alias: &str, target_alias: &str) {
        for (i, (pf, tf)) in r.parent_fields.iter().zip(r.target_fields.iter()).enumerate() {
            if i > 0 {
                q.push(" AND ");
            }
            q.col(target_alias, tf.column).push(" = ").col(parent_alias, pf.column);
        }
    }

    fn relation_filter(&self, q: &mut Sql, model: &'static ModelMeta, alias: &str, f: &'static FieldMeta, v: &Value) -> Result<()> {
        let r = self.relation(model, f)?;
        if matches!(r.kind, RelationKind::HasMany | RelationKind::ManyToMany) {
            let ops = self.obj(v, f.name, &format!("{}ListRelationFilter", r.target.name))?;
            self.check_keys(ops, &["every", "some", "none"])?;
            if ops.is_empty() {
                q.push("1=1");
                return Ok(());
            }
            q.push("(");
            for (i, (op, inner)) in ops.iter().enumerate() {
                if i > 0 {
                    q.push(" AND ");
                }
                match op.as_str() {
                    "some" => self.exists(q, &r, alias, Some(inner), false)?,
                    "none" => {
                        q.push("NOT ");
                        self.exists(q, &r, alias, Some(inner), false)?;
                    }
                    _ => {
                        q.push("NOT ");
                        self.exists(q, &r, alias, Some(inner), true)?;
                    }
                }
            }
            q.push(")");
            return Ok(());
        }
        // To-one relation.
        match v {
            Value::Null => {
                q.push("NOT ");
                self.exists(q, &r, alias, None, false)
            }
            Value::Object(ops) if ops.contains_key("is") || ops.contains_key("isNot") => {
                self.check_keys(ops, &["is", "isNot"])?;
                q.push("(");
                for (i, (op, inner)) in ops.iter().enumerate() {
                    if i > 0 {
                        q.push(" AND ");
                    }
                    match (op.as_str(), inner) {
                        ("is", Value::Null) => {
                            q.push("NOT ");
                            self.exists(q, &r, alias, None, false)?;
                        }
                        ("isNot", Value::Null) => self.exists(q, &r, alias, None, false)?,
                        ("is", w) => self.exists(q, &r, alias, Some(w), false)?,
                        (_, w) => {
                            q.push("NOT ");
                            self.exists(q, &r, alias, Some(w), false)?;
                        }
                    }
                }
                q.push(")");
                Ok(())
            }
            w => self.exists(q, &r, alias, Some(w), false),
        }
    }

    // ---------------------------------------------------------------------
    // orderBy
    // ---------------------------------------------------------------------

    fn sort_dir(&self, v: &Value, field: &str) -> Result<(bool, Option<&'static str>)> {
        match v {
            Value::String(s) if s == "asc" => Ok((false, None)),
            Value::String(s) if s == "desc" => Ok((true, None)),
            Value::Object(o) if o.contains_key("sort") => {
                self.check_keys(o, &["sort", "nulls"])?;
                let (desc, _) = self.sort_dir(&o["sort"], field)?;
                let nulls = match o.get("nulls").and_then(|n| n.as_str()) {
                    None => None,
                    Some("first") => Some("FIRST"),
                    Some("last") => Some("LAST"),
                    Some(other) => return Err(self.err(format!("Argument `nulls`: expected \"first\" or \"last\", provided \"{other}\"."))),
                };
                Ok((desc, nulls))
            }
            other => Err(self.err(format!(
                "Argument `{field}`: Invalid value provided. Expected SortOrder (\"asc\" | \"desc\"), provided {other}."
            ))),
        }
    }

    /// Parses `orderBy` (`{ id: "desc" }` or `[{ name: "asc" }, { id: "desc" }]`).
    pub(crate) fn parse_order(&self, model: &'static ModelMeta, v: Option<&Value>, group_by: bool) -> Result<Vec<OrderItem>> {
        let mut out = Vec::new();
        let Some(v) = v else { return Ok(out) };
        let entries: Vec<&Value> = match v {
            Value::Array(a) => a.iter().collect(),
            Value::Null => vec![],
            other => vec![other],
        };
        for e in entries {
            let obj = self.obj(e, "orderBy", &format!("{}OrderByWithRelationInput", model.name))?;
            for (k, dir) in obj {
                if group_by && matches!(k.as_str(), "_count" | "_sum" | "_avg" | "_min" | "_max") {
                    let agg: &'static str = match k.as_str() {
                        "_count" => "COUNT",
                        "_sum" => "SUM",
                        "_avg" => "AVG",
                        "_min" => "MIN",
                        _ => "MAX",
                    };
                    let inner = self.obj(dir, k, "object")?;
                    for (fname, d) in inner {
                        let f = if fname == "_all" { None } else { Some(self.field(model, fname)?) };
                        let (desc, nulls) = self.sort_dir(d, fname)?;
                        out.push(OrderItem { expr: OrderExpr::Aggregate(agg, f), desc, nulls });
                    }
                    continue;
                }
                let f = self.field(model, k)?;
                if f.is_scalar() {
                    let (desc, nulls) = self.sort_dir(dir, k)?;
                    out.push(OrderItem { expr: OrderExpr::Field(f), desc, nulls });
                    continue;
                }
                let r = self.relation(model, f)?;
                self.parse_relation_order(vec![r], dir, k, &mut out)?;
            }
        }
        Ok(out)
    }

    fn parse_relation_order(&self, path: Vec<ResolvedRelation>, dir: &Value, k: &str, out: &mut Vec<OrderItem>) -> Result<()> {
        let r = path.last().unwrap().clone();
        let inner = self.obj(dir, k, "object")?;
        let to_many = matches!(r.kind, RelationKind::HasMany | RelationKind::ManyToMany);
        for (ik, idir) in inner {
            if to_many {
                if ik != "_count" || path.len() > 1 {
                    return Err(self.unknown_argument(ik, ["_count"]));
                }
                let (desc, nulls) = self.sort_dir(idir, ik)?;
                out.push(OrderItem { expr: OrderExpr::RelationCount(r.clone()), desc, nulls });
                continue;
            }
            let tf = self.field(r.target, ik)?;
            if tf.is_scalar() {
                let (desc, nulls) = self.sort_dir(idir, ik)?;
                out.push(OrderItem { expr: OrderExpr::RelationField(path.clone(), tf), desc, nulls });
            } else {
                let nr = self.relation(r.target, tf)?;
                if matches!(nr.kind, RelationKind::HasMany | RelationKind::ManyToMany) {
                    return Err(self.err(format!("Ordering by the count of a nested to-many relation (`{ik}`) is not supported")));
                }
                let mut p = path.clone();
                p.push(nr);
                self.parse_relation_order(p, idir, ik, out)?;
            }
        }
        Ok(())
    }

    /// Writes the SQL expression an order item sorts on.
    pub(crate) fn order_expr_sql(&self, q: &mut Sql, alias: &str, item: &OrderItem) {
        match &item.expr {
            OrderExpr::Field(f) => {
                q.col(alias, f.column);
            }
            OrderExpr::RelationField(path, tf) => {
                // (SELECT tN.col FROM a AS t1 JOIN b AS t2 ON .. WHERE t1 joins parent LIMIT 1)
                let aliases: Vec<String> = path.iter().map(|_| q.alias()).collect();
                q.push("(SELECT ").col(aliases.last().unwrap(), tf.column).push(" FROM ");
                for (i, r) in path.iter().enumerate() {
                    if i > 0 {
                        q.push(" INNER JOIN ");
                    }
                    q.table(r.target.schema, r.target.table).push(" AS ").push(&aliases[i]);
                    if i > 0 {
                        q.push(" ON ");
                        self.join_sql(q, r, &aliases[i - 1], &aliases[i]);
                    }
                }
                q.push(" WHERE ");
                self.join_sql(q, &path[0], alias, &aliases[0]);
                q.push(" LIMIT 1)");
            }
            OrderExpr::RelationCount(r) => {
                let a = q.alias();
                q.push("(SELECT COUNT(*) FROM ").table(r.target.schema, r.target.table).push(" AS ").push(&a);
                if let Some(join) = &r.join {
                    let j = q.alias();
                    q.push(" INNER JOIN ").table(join.schema, &join.table).push(" AS ").push(&j).push(" ON ");
                    q.col(&j, join.target_column).push(" = ").col(&a, r.target_fields[0].column);
                    q.push(" WHERE ").col(&j, join.parent_column).push(" = ").col(alias, r.parent_fields[0].column);
                } else {
                    q.push(" WHERE ");
                    self.join_sql(q, r, alias, &a);
                }
                q.push(")");
            }
            OrderExpr::Aggregate(agg, f) => {
                q.push(agg).push("(");
                match f {
                    Some(f) => {
                        q.col(alias, f.column);
                    }
                    None => {
                        q.push("*");
                    }
                }
                q.push(")");
            }
        }
    }

    /// ` ORDER BY ...` (nothing when `items` is empty).
    pub(crate) fn order_sql(&self, q: &mut Sql, alias: &str, items: &[OrderItem], reverse: bool) {
        if items.is_empty() {
            return;
        }
        q.push(" ORDER BY ");
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                q.push(", ");
            }
            self.order_expr_sql(q, alias, item);
            let desc = item.desc != reverse;
            q.push(if desc { " DESC" } else { " ASC" });
            if let Some(n) = item.nulls {
                let n = if reverse { if n == "FIRST" { "LAST" } else { "FIRST" } } else { n };
                q.push(" NULLS ").push(n);
            }
        }
    }
}
