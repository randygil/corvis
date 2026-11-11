//! `count`, `aggregate` and `groupBy`.
//!
//! Result shape follows the reference client: the aggregate groups come in the order the
//! caller wrote them (`{ _sum, _avg, _count }`), each with its fields in the given
//! order, and `groupBy` rows put the aggregates first and the `by` fields last
//! (the selection is built as `{ ...aggregates, ...by }`).

use serde_json::{Map, Value};

use super::filter::{OrderExpr, OrderItem};
use super::{Engine, json_type};
use crate::db::{Dialect, Sql, SqlValue, coerce, coerce_field_d};
use crate::error::{OrmError, Result};
use crate::meta::{FieldKind, FieldMeta, ModelMeta};

const AGGS: &[&str] = &["_count", "_sum", "_avg", "_min", "_max"];

/// One requested aggregate column.
struct Agg {
    /// `_count`, `_sum`, ...
    group: &'static str,
    /// `None` for `_count: { _all: true }` / `_count: true`.
    field: Option<&'static FieldMeta>,
    /// Result key inside the group (`_all`, `views`).
    key: String,
}

/// The requested aggregates, in output order.
struct AggSelection {
    aggs: Vec<Agg>,
    /// Group names in the order they were given (`_count` included even when it is scalar).
    groups: Vec<&'static str>,
    /// `_count: true` → the result is `_count: <n>` instead of `_count: { _all: <n> }`.
    count_scalar: bool,
}

fn agg_name(k: &str) -> Option<&'static str> {
    AGGS.iter().copied().find(|a| *a == k)
}

fn sql_func(group: &str) -> &'static str {
    match group {
        "_count" => "COUNT",
        "_sum" => "SUM",
        "_avg" => "AVG",
        "_min" => "MIN",
        _ => "MAX",
    }
}

/// `_sum` -> `Sum` (type names in messages).
fn group_type(group: &str) -> &'static str {
    match group {
        "_count" => "Count",
        "_sum" => "Sum",
        "_avg" => "Avg",
        "_min" => "Min",
        _ => "Max",
    }
}

/// `P2019` ("Input error. ...") raised by the engine for invalid `groupBy` arguments.
pub(crate) fn input_error(model: &ModelMeta, details: String) -> OrmError {
    OrmError::known("P2019", format!("Input error. {details}"), serde_json::json!({ "modelName": model.name, "details": details }))
}

fn is_numeric(f: &FieldMeta) -> bool {
    matches!(f.kind, FieldKind::Int | FieldKind::BigInt | FieldKind::Float | FieldKind::Decimal)
}

impl Engine {
    /// Checks that `group` can be computed on `f` (`_sum` / `_avg` need numbers, `_min` / `_max` comparable scalars).
    fn check_agg_field(&self, group: &str, f: &'static FieldMeta) -> Result<()> {
        let ok = match group {
            "_count" => true,
            "_sum" | "_avg" => !f.list && is_numeric(f),
            _ => !f.list && !matches!(f.kind, FieldKind::Json | FieldKind::Bytes),
        };
        if ok {
            Ok(())
        } else {
            let options = self.root.scalars().filter(|x| match group {
                "_sum" | "_avg" => !x.list && is_numeric(x),
                _ => !x.list && !matches!(x.kind, FieldKind::Json | FieldKind::Bytes),
            });
            Err(self.unknown_argument(f.name, options.map(|x| x.name)))
        }
    }

    /// Parses `_count` / `_sum` / `_avg` / `_min` / `_max` from `args`, keeping the caller's order.
    fn parse_aggs(&self, model: &'static ModelMeta, args: &Map<String, Value>) -> Result<AggSelection> {
        let mut sel = AggSelection { aggs: Vec::new(), groups: Vec::new(), count_scalar: false };
        for (k, v) in args {
            let Some(group) = agg_name(k) else { continue };
            match (group, v) {
                (_, Value::Null | Value::Bool(false)) => {}
                ("_count", Value::Bool(true)) => {
                    sel.count_scalar = true;
                    sel.groups.push(group);
                    sel.aggs.push(Agg { group, field: None, key: "_all".into() });
                }
                (_, Value::Object(o)) => {
                    if !o.values().any(|v| v == &Value::Bool(true)) {
                        return Err(self.err(format!(
                            "The `select` statement for type {}{}AggregateOutputType must not be empty.",
                            model.name,
                            group_type(group)
                        )));
                    }
                    sel.groups.push(group);
                    for (k, enabled) in o {
                        match enabled {
                            Value::Bool(true) => {}
                            Value::Bool(false) | Value::Null => continue,
                            other => {
                                return Err(self.err(format!(
                                    "Argument `{k}`: Invalid value provided. Expected Boolean, provided {}.",
                                    json_type(other)
                                )));
                            }
                        }
                        if group == "_count" && k == "_all" {
                            sel.aggs.push(Agg { group, field: None, key: "_all".into() });
                            continue;
                        }
                        let f = match model.field(k) {
                            Some(f) if f.is_scalar() => f,
                            _ => {
                                let mut options: Vec<&str> = if group == "_count" { vec!["_all"] } else { vec![] };
                                options.extend(model.scalars().map(|f| f.name));
                                return Err(self.unknown_argument(k, options));
                            }
                        };
                        self.check_agg_field(group, f)?;
                        sel.aggs.push(Agg { group, field: Some(f), key: k.clone() });
                    }
                }
                (_, other) => {
                    return Err(self.err(format!(
                        "Argument `{group}`: Invalid value provided. Expected {}{}AggregateInputType, provided {}.",
                        model.name,
                        group_type(group),
                        json_type(other)
                    )));
                }
            }
        }
        Ok(sel)
    }

    /// The SQL of an aggregate over `f` (or `COUNT(*)`), cast so that it decodes like the reference client's result type.
    fn agg_expr_sql(&self, q: &mut Sql, alias: &str, group: &str, f: Option<&'static FieldMeta>) {
        let pg = self.dialect() == Dialect::Postgres;
        let kind = f.map(|f| f.kind);
        let cast = match (group, kind) {
            ("_count", _) if pg => Some("BIGINT"),
            ("_sum", Some(FieldKind::Int | FieldKind::BigInt)) if pg => Some("BIGINT"),
            ("_sum" | "_avg", Some(FieldKind::Decimal)) => None,
            ("_avg", _) if pg => Some("DOUBLE PRECISION"),
            ("_sum", Some(FieldKind::Float)) if pg => Some("DOUBLE PRECISION"),
            _ => None,
        };
        if cast.is_some() {
            q.push("CAST(");
        }
        q.push(sql_func(group)).push("(");
        match f {
            Some(f) if group != "_count" => self.operand(q, alias, f),
            Some(f) => {
                q.col(alias, f.column);
            }
            None => {
                q.push("*");
            }
        }
        q.push(")");
        if let Some(c) = cast {
            q.push(" AS ").push(c).push(")");
        }
    }

    /// Converts a decoded aggregate into the output type of the reference client.
    fn agg_value(&self, a: &Agg, v: Value) -> Value {
        // SQLite computes `AVG` as a double; the reference client keeps 16 significant digits of it (recorded:
        // `3002399751580330` for 9007199254740991 / 3, `"1.333333333333333"` for a `Decimal` 4 / 3).
        let v = match (a.group, self.dialect(), v.as_f64()) {
            ("_avg", Dialect::Sqlite, Some(x)) if x.is_finite() => {
                format!("{x:.15e}").parse::<f64>().ok().and_then(serde_json::Number::from_f64).map_or(v, Value::Number)
            }
            _ => v,
        };
        match (a.group, a.field) {
            ("_count", _) => Value::from(v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())).unwrap_or(0)),
            (_, Some(f)) if f.kind == FieldKind::Decimal => coerce(FieldKind::Decimal, v),
            ("_avg", _) => coerce(FieldKind::Float, v),
            ("_sum", Some(f)) if matches!(f.kind, FieldKind::Int | FieldKind::BigInt) => coerce(FieldKind::BigInt, v),
            ("_sum", _) => coerce(FieldKind::Float, v),
            (_, Some(f)) => coerce_field_d(f, v, self.dialect()),
            _ => v,
        }
    }

    /// Builds the aggregate part of a result row (`{ _sum: {..}, _count: {..} }`).
    fn agg_object(&self, sel: &AggSelection, values: impl Iterator<Item = Value>) -> Map<String, Value> {
        let mut groups: Vec<(&'static str, Map<String, Value>)> = sel.groups.iter().map(|g| (*g, Map::new())).collect();
        for (a, v) in sel.aggs.iter().zip(values) {
            let v = self.agg_value(a, v);
            if let Some((_, m)) = groups.iter_mut().find(|(g, _)| *g == a.group) {
                m.insert(a.key.clone(), v);
            }
        }
        let mut out = Map::new();
        for (g, mut m) in groups {
            if g == "_count" && sel.count_scalar {
                out.insert(g.into(), m.remove("_all").unwrap_or(Value::from(0)));
            } else {
                out.insert(g.into(), Value::Object(m));
            }
        }
        out
    }

    /// `(SELECT t0.* FROM table AS t0 WHERE ... ORDER BY ... LIMIT ...) AS sub`, honouring `cursor`.
    async fn filtered_subquery(&self, q: &mut Sql, model: &'static ModelMeta, args: &Map<String, Value>) -> Result<()> {
        let take = self.int_arg(args, "take")?;
        let skip = self.int_arg(args, "skip")?;
        if let Some(s) = skip.filter(|s| *s < 0) {
            return Err(self.err(format!("Invalid value for skip argument: Value can only be positive, found: {s}")));
        }
        let cursor = args.get("cursor").filter(|c| !c.is_null());
        let reverse = take.is_some_and(|t| t < 0);
        let mut order = self.parse_order(model, args.get("orderBy"), false)?;
        if cursor.is_some() || reverse {
            for pk in model.pk() {
                if !order.iter().any(|o| matches!(o.expr, OrderExpr::Field(f) if f.name == pk.name)) {
                    order.push(OrderItem { expr: OrderExpr::Field(pk), desc: false, nulls: None });
                }
            }
        }
        let cursor_row = match cursor {
            None => None,
            Some(c) => {
                let c = self.check_unique_where(model, c)?;
                Some(self.scalar_rows(model, &c, Some(1)).await?.into_iter().next())
            }
        };
        q.push("(SELECT t0.* FROM ").table(model.schema, model.table).push(" AS t0 WHERE ");
        self.where_sql(q, model, "t0", args.get("where").unwrap_or(&Value::Null))?;
        match cursor_row {
            Some(Some(row)) => {
                q.push(" AND (");
                self.cursor_sql(q, &order, &row, reverse)?;
                q.push(")");
            }
            // A cursor that matches no record selects nothing.
            Some(None) => {
                q.push(" AND 1=0");
            }
            None => {}
        }
        self.order_sql(q, "t0", &order, reverse);
        self.limit_sql(q, take.map(|t| t.checked_abs().unwrap_or(i64::MAX)), skip);
        q.push(") AS sub");
        Ok(())
    }

    async fn run_aggregates(&self, model: &'static ModelMeta, sel: &AggSelection, args: &Map<String, Value>) -> Result<Map<String, Value>> {
        let mut q = Sql::new(self.dialect());
        q.push("SELECT ");
        for (i, a) in sel.aggs.iter().enumerate() {
            if i > 0 {
                q.push(", ");
            }
            self.agg_expr_sql(&mut q, "sub", a.group, a.field);
        }
        q.push(" FROM ");
        self.filtered_subquery(&mut q, model, args).await?;
        let rows = self.client.fetch(&q, Some(model)).await?;
        let row = rows.rows.into_iter().next().unwrap_or_default();
        Ok(self.agg_object(sel, row.into_iter()))
    }

    pub(crate) async fn count_action(&self, model: &'static ModelMeta, args: Map<String, Value>) -> Result<Value> {
        self.check_keys(&args, &["where", "orderBy", "take", "skip", "cursor", "select"])?;
        let select = args.get("select").filter(|v| !matches!(v, Value::Null | Value::Bool(true)));
        let Some(select) = select else {
            let paged = ["take", "skip", "cursor"].iter().any(|k| args.get(*k).is_some_and(|v| !v.is_null()));
            if !paged {
                let w = args.get("where").cloned().unwrap_or(Value::Null);
                return Ok(Value::from(self.count_where(model, &w).await?));
            }
            let sel = AggSelection {
                aggs: vec![Agg { group: "_count", field: None, key: "_all".into() }],
                groups: vec!["_count"],
                count_scalar: true,
            };
            let out = self.run_aggregates(model, &sel, &args).await?;
            return Ok(out.get("_count").cloned().unwrap_or(Value::from(0)));
        };
        let mut m = Map::new();
        m.insert("_count".into(), select.clone());
        let sel = self.parse_aggs(model, &m)?;
        if sel.aggs.is_empty() {
            return Ok(Value::Object(Map::new()));
        }
        let out = self.run_aggregates(model, &sel, &args).await?;
        Ok(out.get("_count").cloned().unwrap_or(Value::Object(Map::new())))
    }

    pub(crate) async fn aggregate_action(&self, model: &'static ModelMeta, args: Map<String, Value>) -> Result<Value> {
        self.check_keys(&args, &["where", "orderBy", "take", "skip", "cursor", "_count", "_sum", "_avg", "_min", "_max"])?;
        let sel = self.parse_aggs(model, &args)?;
        if sel.aggs.is_empty() {
            let mut out = Map::new();
            for g in &sel.groups {
                out.insert(g.to_string(), Value::Object(Map::new()));
            }
            return Ok(Value::Object(out));
        }
        Ok(Value::Object(self.run_aggregates(model, &sel, &args).await?))
    }

    fn by_fields(&self, model: &'static ModelMeta, args: &Map<String, Value>) -> Result<Vec<&'static FieldMeta>> {
        let scalar =
            |name: &str| -> Result<&'static FieldMeta> {
                match model.field(name) {
                    Some(f) if f.is_scalar() => Ok(f),
                    _ => Err(self
                        .err(format!("Argument `by`: Invalid value provided. Expected {}ScalarFieldEnum, provided `{name}`.", model.name))),
                }
            };
        match args.get("by") {
            Some(Value::Array(a)) if !a.is_empty() => a
                .iter()
                .map(|v| {
                    let name = v.as_str().ok_or_else(|| {
                        self.err(format!(
                            "Argument `by`: Invalid value provided. Expected {}ScalarFieldEnum, provided {}.",
                            model.name,
                            json_type(v)
                        ))
                    })?;
                    scalar(name)
                })
                .collect(),
            Some(Value::String(s)) => Ok(vec![scalar(s)?]),
            Some(Value::Array(_)) => Err(self.err("Argument `by`: expected at least one field.")),
            _ => Err(self.err("Argument `by` is missing.")),
        }
    }

    pub(crate) async fn group_by_action(&self, model: &'static ModelMeta, args: Map<String, Value>) -> Result<Value> {
        self.check_keys(&args, &["by", "where", "orderBy", "having", "take", "skip", "_count", "_sum", "_avg", "_min", "_max"])?;
        let by = self.by_fields(model, &args)?;
        let sel = self.parse_aggs(model, &args)?;
        let order = self.parse_order(model, args.get("orderBy"), true)?;
        let missing: Vec<&str> = order
            .iter()
            .filter_map(|o| match &o.expr {
                OrderExpr::Field(f) if !by.iter().any(|b| b.name == f.name) => Some(f.name),
                _ => None,
            })
            .collect();
        if !missing.is_empty() {
            return Err(input_error(
                model,
                format!(
                    "Every field used for orderBy must be included in the by-arguments of the query. Missing fields: {}",
                    missing.join(", ")
                ),
            ));
        }
        let take = self.int_arg(&args, "take")?;
        let skip = self.int_arg(&args, "skip")?;
        if let Some(s) = skip.filter(|s| *s < 0) {
            return Err(self.err(format!("Invalid value for skip argument: Value can only be positive, found: {s}")));
        }

        let mut q = Sql::new(self.dialect());
        q.push("SELECT ");
        for (i, f) in by.iter().enumerate() {
            if i > 0 {
                q.push(", ");
            }
            q.col("t0", f.column);
        }
        for a in &sel.aggs {
            q.push(", ");
            self.agg_expr_sql(&mut q, "t0", a.group, a.field);
        }
        q.push(" FROM ").table(model.schema, model.table).push(" AS t0 WHERE ");
        self.where_sql(&mut q, model, "t0", args.get("where").unwrap_or(&Value::Null))?;
        q.push(" GROUP BY ");
        for (i, f) in by.iter().enumerate() {
            if i > 0 {
                q.push(", ");
            }
            q.col("t0", f.column);
        }
        if let Some(h) = args.get("having").filter(|h| !h.is_null()) {
            q.push(" HAVING ");
            self.having_sql(&mut q, model, &by, h)?;
        }
        let reverse = take.is_some_and(|t| t < 0);
        self.order_sql(&mut q, "t0", &order, reverse);
        self.limit_sql(&mut q, take.map(|t| t.checked_abs().unwrap_or(i64::MAX)), skip);
        let rows = self.client.fetch(&q, Some(model)).await?;
        let d = self.dialect();
        let mut out = Vec::new();
        for row in rows.rows {
            let mut it = row.into_iter();
            let keys: Vec<(&'static FieldMeta, Value)> =
                by.iter().map(|f| (*f, coerce_field_d(f, it.next().unwrap_or(Value::Null), d))).collect();
            let mut obj = self.agg_object(&sel, it);
            for (f, v) in keys {
                obj.insert(f.name.to_string(), v);
            }
            out.push(Value::Object(obj));
        }
        if reverse {
            out.reverse();
        }
        Ok(Value::Array(out))
    }

    // ---------------------------------------------------------------------
    // having
    // ---------------------------------------------------------------------

    /// `having`: a `ScalarWhereWithAggregatesInput` (`{ views: { _sum: { gt: 5 } }, OR: [...] }`).
    fn having_sql(&self, q: &mut Sql, model: &'static ModelMeta, by: &[&'static FieldMeta], h: &Value) -> Result<()> {
        let obj = match h {
            Value::Object(o) => o,
            other => {
                return Err(self.err(format!(
                    "Argument `having`: Invalid value provided. Expected {}ScalarWhereWithAggregatesInput, provided {}.",
                    model.name,
                    json_type(other)
                )));
            }
        };
        if obj.is_empty() {
            q.push("1=1");
            return Ok(());
        }
        q.push("(");
        for (i, (k, v)) in obj.iter().enumerate() {
            if i > 0 {
                q.push(" AND ");
            }
            match k.as_str() {
                "AND" | "OR" => {
                    let (sep, empty) = if k == "AND" { (" AND ", "1=1") } else { (" OR ", "1=0") };
                    let items: Vec<&Value> = match v {
                        Value::Array(a) => a.iter().collect(),
                        other => vec![other],
                    };
                    if items.is_empty() {
                        q.push(empty);
                        continue;
                    }
                    q.push("(");
                    for (j, item) in items.iter().enumerate() {
                        if j > 0 {
                            q.push(sep);
                        }
                        self.having_sql(q, model, by, item)?;
                    }
                    q.push(")");
                }
                "NOT" => {
                    let items: Vec<&Value> = match v {
                        Value::Array(a) => a.iter().collect(),
                        other => vec![other],
                    };
                    if items.is_empty() {
                        q.push("1=1");
                        continue;
                    }
                    q.push("(");
                    for (j, item) in items.iter().enumerate() {
                        if j > 0 {
                            q.push(" AND ");
                        }
                        q.push("NOT ");
                        self.having_sql(q, model, by, item)?;
                    }
                    q.push(")");
                }
                name => {
                    let f = match model.field(name) {
                        Some(f) if f.is_scalar() => f,
                        _ => {
                            let mut options: Vec<&str> = vec!["AND", "OR", "NOT"];
                            options.extend(model.scalars().map(|f| f.name));
                            return Err(self.unknown_argument(name, options));
                        }
                    };
                    self.having_field(q, f, by, v)?;
                }
            }
        }
        q.push(")");
        Ok(())
    }

    /// One field of `having`: plain filters (only on `by` fields) and `_count` / `_sum` / ... filters.
    fn having_field(&self, q: &mut Sql, f: &'static FieldMeta, by: &[&'static FieldMeta], v: &Value) -> Result<()> {
        let (aggs, plain): (Vec<(&String, &Value)>, Map<String, Value>) = match v {
            Value::Object(o) => {
                let aggs = o.iter().filter(|(k, _)| agg_name(k).is_some()).collect();
                let plain = o.iter().filter(|(k, _)| agg_name(k).is_none()).map(|(k, v)| (k.clone(), v.clone())).collect();
                (aggs, plain)
            }
            _ => (vec![], Map::new()),
        };
        let has_plain = !v.is_object() || !plain.is_empty();
        if has_plain && !by.iter().any(|b| b.name == f.name) {
            return Err(input_error(
                self.root,
                format!(
                    "Every field used in `having` filters must either be an aggregation filter or be included in the selection of the query. Missing fields: {}",
                    f.name
                ),
            ));
        }
        q.push("(");
        let mut first = true;
        if has_plain {
            let plain_v = if v.is_object() { Value::Object(plain) } else { v.clone() };
            self.where_sql(q, self.root, "t0", &Value::Object(Map::from_iter([(f.name.to_string(), plain_v)])))?;
            first = false;
        }
        for (k, filter) in aggs {
            if !first {
                q.push(" AND ");
            }
            first = false;
            let group = agg_name(k).unwrap_or("_count");
            self.check_agg_field(group, f)?;
            self.agg_filter_sql(q, group, f, filter)?;
        }
        if first {
            q.push("1=1");
        }
        q.push(")");
        Ok(())
    }

    /// Binds the value an aggregate of `f` is compared with. An aggregate has no type affinity in SQLite, so
    /// a `Decimal` operand (bound as text) is cast to a number there; otherwise the number would be compared
    /// with a string.
    fn bind_agg_operand(&self, q: &mut Sql, group: &str, f: &'static FieldMeta, v: &Value) -> Result<()> {
        let value = self.agg_operand(group, f, v)?;
        if group != "_count" && self.numeric_operand(f) {
            q.push("CAST(").bind(value).push(" AS NUMERIC)");
        } else {
            q.bind(value);
        }
        Ok(())
    }

    /// The value bound when comparing an aggregate of `f`.
    fn agg_operand(&self, group: &str, f: &'static FieldMeta, v: &Value) -> Result<SqlValue> {
        let bad = |expected: &str| {
            self.err(format!("Argument `{group}`: Invalid value provided. Expected {expected}, provided {}.", json_type(v)))
        };
        match group {
            "_count" => v.as_i64().map(SqlValue::Int).ok_or_else(|| bad("Int")),
            "_avg" if f.kind != FieldKind::Decimal => v.as_f64().map(SqlValue::Float).ok_or_else(|| bad("Float")),
            "_sum" if matches!(f.kind, FieldKind::Int | FieldKind::BigInt) => {
                v.as_i64().map(SqlValue::Int).ok_or_else(|| bad(f.kind.schema_name()))
            }
            _ => self.value(f, v, true),
        }
    }

    /// `SUM(t0.col) > $1 AND ...` for `{ _sum: { gt: 5 } }` (or a bare value, meaning `equals`).
    fn agg_filter_sql(&self, q: &mut Sql, group: &'static str, f: &'static FieldMeta, filter: &Value) -> Result<()> {
        let ops: Vec<(String, Value)> = match filter {
            Value::Object(o) => o.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            other => vec![("equals".to_string(), other.clone())],
        };
        if ops.is_empty() {
            q.push("1=1");
            return Ok(());
        }
        let field = Some(f);
        q.push("(");
        for (i, (op, val)) in ops.iter().enumerate() {
            if i > 0 {
                q.push(" AND ");
            }
            // The reference client refuses `null` in an aggregate filter (recorded: `having.*_null`), where a field
            // filter takes `equals: null` / `not: null` as `IS NULL` / `IS NOT NULL`.
            if val.is_null() && matches!(op.as_str(), "equals" | "lt" | "lte" | "gt" | "gte" | "not") {
                return Err(self.err(format!("Argument `{op}` must not be null.")));
            }
            match op.as_str() {
                "equals" | "lt" | "lte" | "gt" | "gte" => {
                    self.agg_expr_sql(q, "t0", group, field);
                    let sym = match op.as_str() {
                        "equals" => " = ",
                        "lt" => " < ",
                        "lte" => " <= ",
                        "gt" => " > ",
                        _ => " >= ",
                    };
                    q.push(sym);
                    self.bind_agg_operand(q, group, f, val)?;
                }
                "in" | "notIn" => {
                    let items = match val {
                        Value::Array(a) => a.clone(),
                        other => vec![other.clone()],
                    };
                    if items.iter().any(Value::is_null) {
                        return Err(self.err(format!(
                            "Argument `{op}`: Invalid value provided. Expected List{}FieldRefInput, provided a list with Null.",
                            f.kind.schema_name()
                        )));
                    }
                    if items.is_empty() {
                        q.push(if op == "in" { "1=0" } else { "1=1" });
                        continue;
                    }
                    self.agg_expr_sql(q, "t0", group, field);
                    q.push(if op == "in" { " IN (" } else { " NOT IN (" });
                    for (j, it) in items.iter().enumerate() {
                        if j > 0 {
                            q.push(", ");
                        }
                        self.bind_agg_operand(q, group, f, it)?;
                    }
                    q.push(")");
                }
                "not" => match val {
                    Value::Object(_) => {
                        q.push("NOT ");
                        self.agg_filter_sql(q, group, f, val)?;
                    }
                    other => {
                        self.agg_expr_sql(q, "t0", group, field);
                        q.push(" <> ");
                        self.bind_agg_operand(q, group, f, other)?;
                    }
                },
                other => {
                    return Err(self.unknown_argument(other, ["equals", "in", "notIn", "lt", "lte", "gt", "gte", "not"]));
                }
            }
        }
        q.push(")");
        Ok(())
    }
}
