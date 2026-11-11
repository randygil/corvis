//! Reads: `findMany`, `findFirst`, `findUnique`, relations (`include` / `select` /
//! `omit`) and `_count`.
//!
//! Rows are fetched with every scalar column, relations are attached, and only then
//! each record is shaped into the output object, with the reference client's key order: the `select`
//! order, or (without `select`) every non-omitted scalar in schema order followed by
//! the `include` entries in their order.

use std::collections::{HashMap, HashSet};

use futures::FutureExt;
use futures::future::BoxFuture;
use serde_json::{Map, Value, json};

use super::filter::{OrderExpr, OrderItem};
use super::{Engine, Id, key_of};
use crate::db::{Sql, SqlValue};
use crate::error::{OrmError, Result};
use crate::meta::{FieldMeta, ModelMeta, RelationKind, ResolvedRelation};

/// Selected `_count` relations: (relation field, optional nested `where`).
type CountSelection = (&'static FieldMeta, Option<Value>);
/// Rows with their relation selection.
type FetchedRows = (Vec<Map<String, Value>>, Selection);

pub(crate) const FIND_MANY_ARGS: &[&str] =
    &["where", "orderBy", "take", "skip", "cursor", "select", "include", "omit", "distinct", "relationLoadStrategy"];
pub(crate) const FIND_UNIQUE_ARGS: &[&str] = &["where", "select", "include", "omit", "relationLoadStrategy"];

/// One entry of the output object.
#[derive(Debug, Clone)]
pub(crate) enum SelItem {
    Scalar(&'static FieldMeta),
    Relation(&'static FieldMeta, Map<String, Value>),
    /// `_count: { posts: <where>, ... }`
    Count(Vec<(&'static FieldMeta, Option<Value>)>),
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Selection {
    pub items: Vec<SelItem>,
}

impl Selection {
    fn relations(&self) -> impl Iterator<Item = (&'static FieldMeta, &Map<String, Value>)> {
        self.items.iter().filter_map(|i| match i {
            SelItem::Relation(f, a) => Some((*f, a)),
            _ => None,
        })
    }
    fn counts(&self) -> Vec<(&'static FieldMeta, Option<Value>)> {
        self.items
            .iter()
            .filter_map(|i| match i {
                SelItem::Count(c) => Some(c.clone()),
                _ => None,
            })
            .flatten()
            .collect()
    }
}

fn to_many(r: &ResolvedRelation) -> bool {
    matches!(r.kind, RelationKind::HasMany | RelationKind::ManyToMany)
}

impl Engine {
    pub(crate) fn int_arg(&self, args: &Map<String, Value>, key: &str) -> Result<Option<i64>> {
        match args.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v.as_i64().map(Some).ok_or_else(|| {
                self.err(format!("Argument `{key}`: Invalid value provided. Expected Int, provided {}.", super::json_type(v)))
            }),
        }
    }

    /// Parses `select` / `include` / `omit`.
    pub(crate) fn parse_selection(&self, model: &'static ModelMeta, args: &Map<String, Value>) -> Result<Selection> {
        let select = args.get("select").filter(|v| !v.is_null());
        let include = args.get("include").filter(|v| !v.is_null());
        let omit = args.get("omit").filter(|v| !v.is_null());
        if select.is_some() && include.is_some() {
            return Err(self.err("Please either use `include` or `select`, but not both at the same time."));
        }
        if select.is_some() && omit.is_some() {
            return Err(self.err("Please either use `omit` or `select`, but not both at the same time."));
        }
        let mut sel = Selection::default();
        if let Some(s) = select {
            let obj = self.obj(s, "select", &format!("{}Select", model.name))?;
            for (k, v) in obj {
                if k == "_count" {
                    if let Some(c) = self.parse_count_selection(model, v)? {
                        sel.items.push(SelItem::Count(c));
                    }
                    continue;
                }
                let f = self.selection_field(model, k, true)?;
                if !self.enabled(k, f, v)? {
                    continue;
                }
                if f.is_scalar() {
                    sel.items.push(SelItem::Scalar(f));
                } else {
                    sel.items.push(SelItem::Relation(f, v.as_object().cloned().unwrap_or_default()));
                }
            }
            return Ok(sel);
        }
        let mut omitted: HashSet<&str> = self.client.global_omit(model);
        if let Some(o) = omit {
            let obj = self.obj(o, "omit", &format!("{}Omit", model.name))?;
            for (k, v) in obj {
                let f = self.field(model, k)?;
                if !f.is_scalar() {
                    return Err(self.err(format!("Argument `omit`: `{k}` is a relation; only scalar fields can be omitted.")));
                }
                match v {
                    Value::Bool(true) => {
                        omitted.insert(f.name);
                    }
                    Value::Bool(false) => {
                        omitted.remove(f.name);
                    }
                    other => {
                        return Err(self.err(format!(
                            "Argument `{k}`: Invalid value provided. Expected Boolean, provided {}.",
                            super::json_type(other)
                        )));
                    }
                }
            }
        }
        for f in model.scalars() {
            if !omitted.contains(f.name) {
                sel.items.push(SelItem::Scalar(f));
            }
        }
        if let Some(i) = include {
            let obj = self.obj(i, "include", &format!("{}Include", model.name))?;
            for (k, v) in obj {
                if k == "_count" {
                    if let Some(c) = self.parse_count_selection(model, v)? {
                        sel.items.push(SelItem::Count(c));
                    }
                    continue;
                }
                let f = self.selection_field(model, k, false)?;
                if f.is_scalar() {
                    return Err(self.err(format!(
                        "Invalid scalar field `{k}` for include statement on model `{}`. Note that include statements only accept relation fields.",
                        model.name
                    )));
                }
                if !self.enabled(k, f, v)? {
                    continue;
                }
                sel.items.push(SelItem::Relation(f, v.as_object().cloned().unwrap_or_default()));
            }
        }
        Ok(sel)
    }

    fn selection_field(&self, model: &'static ModelMeta, k: &str, is_select: bool) -> Result<&'static FieldMeta> {
        match model.field(k) {
            Some(f) => Ok(f),
            None => {
                let options: Vec<&str> = if is_select {
                    model.fields.iter().map(|f| f.name).chain(["_count"]).collect()
                } else {
                    model.relations().map(|f| f.name).chain(["_count"]).collect()
                };
                let kind = if is_select { "select" } else { "include" };
                let hint = crate::error::suggest(k, options.iter().copied()).map(|s| format!(" Did you mean `{s}`?")).unwrap_or_default();
                Err(self.err(format!(
                    "Unknown field `{k}` for {kind} statement on model `{}`.{hint} Available options are marked with ?.",
                    model.name
                )))
            }
        }
    }

    fn enabled(&self, k: &str, f: &FieldMeta, v: &Value) -> Result<bool> {
        match v {
            Value::Bool(b) => Ok(*b),
            Value::Object(_) if f.is_relation() => Ok(true),
            other => Err(self.err(format!(
                "Argument `{k}`: Invalid value provided. Expected Boolean{}, provided {}.",
                if f.is_relation() { " or object" } else { "" },
                super::json_type(other)
            ))),
        }
    }

    fn parse_count_selection(&self, model: &'static ModelMeta, v: &Value) -> Result<Option<Vec<CountSelection>>> {
        let to_many_rel = |f: &FieldMeta| f.is_relation() && f.list;
        match v {
            Value::Bool(false) | Value::Null => Ok(None),
            Value::Bool(true) => Ok(Some(model.relations().filter(|f| to_many_rel(f)).map(|f| (f, None)).collect())),
            Value::Object(o) => {
                self.check_keys(o, &["select"])?;
                let inner = self.obj(o.get("select").unwrap_or(&Value::Null), "select", &format!("{}CountOutputTypeSelect", model.name))?;
                let mut out = Vec::new();
                for (k, cv) in inner {
                    let f = match model.field(k) {
                        Some(f) if to_many_rel(f) => f,
                        _ => {
                            return Err(self.unknown_argument(k, model.relations().filter(|f| to_many_rel(f)).map(|f| f.name)));
                        }
                    };
                    match cv {
                        Value::Bool(false) => {}
                        Value::Bool(true) => out.push((f, None)),
                        Value::Object(w) => {
                            self.check_keys(w, &["where"])?;
                            out.push((f, w.get("where").cloned().filter(|x| !x.is_null())));
                        }
                        other => {
                            return Err(self.err(format!(
                                "Argument `{k}`: Invalid value provided. Expected Boolean or object, provided {}.",
                                super::json_type(other)
                            )));
                        }
                    }
                }
                Ok(Some(out))
            }
            other => Err(self.err(format!(
                "Argument `_count`: Invalid value provided. Expected Boolean or object, provided {}.",
                super::json_type(other)
            ))),
        }
    }

    /// `SELECT <all scalars> FROM table AS alias`.
    pub(crate) fn select_scalars(&self, q: &mut Sql, model: &'static ModelMeta, alias: &str) {
        q.push("SELECT ");
        for (i, f) in model.scalars().enumerate() {
            if i > 0 {
                q.push(", ");
            }
            q.col(alias, f.column);
        }
        q.push(" FROM ").table(model.schema, model.table).push(" AS ").push(alias);
    }

    /// Decodes rows selected with [`Engine::select_scalars`].
    pub(crate) fn decode_scalars(&self, model: &'static ModelMeta, rows: Vec<Vec<Value>>) -> Result<Vec<Map<String, Value>>> {
        let d = self.dialect();
        rows.into_iter()
            .map(|r| {
                model
                    .scalars()
                    .zip(r)
                    .map(|(f, v)| match self.int_out_of_range(model, f, &v) {
                        Some(e) => Err(e),
                        None => Ok((f.name.to_string(), crate::db::coerce_field_d(f, v, d))),
                    })
                    .collect()
            })
            .collect()
    }

    /// `P2023` for an `Int` column holding a value outside 32 bits, which SQLite can store (its integers are 64-bit):
    /// the reference client fails like this when it reads such a row. A float (`1.5`, `1e30`, which SQLite also
    /// stores in an `INTEGER` column) fails the same way, never truncated or saturated.
    fn int_out_of_range(&self, model: &'static ModelMeta, f: &FieldMeta, v: &Value) -> Option<OrmError> {
        if f.kind != crate::meta::FieldKind::Int || f.list {
            return None;
        }
        let Value::Number(n) = v else { return None };
        if n.as_i64().is_some_and(|i| i32::try_from(i).is_ok()) {
            return None;
        }
        let detail =
            format!("Conversion failed: Value {n} does not fit in an INT column, try migrating the '{}' column type to BIGINT", f.column);
        Some(OrmError::known("P2023", format!("Inconsistent column data: {detail}"), json!({ "modelName": model.name, "message": detail })))
    }

    /// Plain scalar rows matching `where` (no relations).
    pub(crate) async fn scalar_rows(&self, model: &'static ModelMeta, w: &Value, limit: Option<i64>) -> Result<Vec<Map<String, Value>>> {
        let mut q = Sql::new(self.dialect());
        self.select_scalars(&mut q, model, "t0");
        q.push(" WHERE ");
        self.where_sql(&mut q, model, "t0", w)?;
        if let Some(l) = limit {
            q.push(" LIMIT ").bind(SqlValue::Int(l));
        }
        let rows = self.client.fetch(&q, Some(model)).await?;
        self.decode_scalars(model, rows.rows)
    }

    /// Shapes a fetched record into the output object.
    pub(crate) fn shape(&self, sel: &Selection, mut row: Map<String, Value>) -> Map<String, Value> {
        let mut out = Map::new();
        for item in &sel.items {
            match item {
                SelItem::Scalar(f) => {
                    out.insert(f.name.to_string(), row.remove(f.name).unwrap_or(Value::Null));
                }
                SelItem::Relation(f, _) => {
                    out.insert(f.name.to_string(), row.remove(f.name).unwrap_or(Value::Null));
                }
                SelItem::Count(_) => {
                    out.insert("_count".into(), row.remove("_count").unwrap_or_else(|| Value::Object(Map::new())));
                }
            }
        }
        out
    }

    /// Shaped records (honouring `select` / `include` / `omit`).
    pub(crate) fn find_records<'a>(
        &'a self,
        model: &'static ModelMeta,
        args: Map<String, Value>,
    ) -> BoxFuture<'a, Result<Vec<Map<String, Value>>>> {
        async move {
            let (rows, sel) = self.fetch_rows(model, args).await?;
            Ok(rows.into_iter().map(|r| self.shape(&sel, r)).collect())
        }
        .boxed()
    }

    /// Records with every scalar plus the selected relations and `_count` attached (not shaped).
    pub(crate) fn fetch_rows<'a>(&'a self, model: &'static ModelMeta, args: Map<String, Value>) -> BoxFuture<'a, Result<FetchedRows>> {
        async move {
            self.check_keys(&args, FIND_MANY_ARGS)?;
            if let Some(s) = args.get("relationLoadStrategy").filter(|v| !v.is_null())
                && !matches!(s.as_str(), Some("join" | "query"))
            {
                return Err(self.err(format!(
                    "Argument `relationLoadStrategy`: Invalid value provided. Expected RelationLoadStrategy (\"query\" | \"join\"), provided {}.",
                    super::json_type(s)
                )));
            }
            let sel = self.parse_selection(model, &args)?;
            let mut records = self.query_rows(model, &args).await?;
            for (f, nested) in sel.relations() {
                self.load_relation(model, &mut records, f, nested.clone()).await?;
            }
            let counts = sel.counts();
            if sel.items.iter().any(|i| matches!(i, SelItem::Count(_))) {
                self.load_counts(model, &mut records, &counts).await?;
            }
            Ok((records, sel))
        }
        .boxed()
    }

    /// Scalar rows for `where`, `orderBy`, `cursor`, `take`, `skip` and `distinct`.
    pub(crate) async fn query_rows(&self, model: &'static ModelMeta, args: &Map<String, Value>) -> Result<Vec<Map<String, Value>>> {
        let take = self.int_arg(args, "take")?;
        let skip = self.int_arg(args, "skip")?;
        if skip.is_some_and(|s| s < 0) {
            return Err(
                self.err("Invalid value for skip argument: Value can only be positive, found: ".to_string() + &skip.unwrap().to_string())
            );
        }
        let distinct: Option<Vec<&'static FieldMeta>> = match args.get("distinct") {
            None | Some(Value::Null) => None,
            Some(v) => {
                let names: Vec<&Value> = match v {
                    Value::Array(a) => a.iter().collect(),
                    other => vec![other],
                };
                let mut fs = Vec::new();
                for n in names {
                    let name = n.as_str().ok_or_else(|| self.err("Argument `distinct`: expected a list of field names."))?;
                    match model.field(name) {
                        Some(f) if f.is_scalar() => fs.push(f),
                        _ => return Err(self.unknown_argument(name, model.scalars().map(|f| f.name))),
                    }
                }
                Some(fs)
            }
        };
        let cursor = args.get("cursor").filter(|c| !c.is_null());
        let mut order = self.parse_order(model, args.get("orderBy"), false)?;
        let reverse = take.is_some_and(|t| t < 0);
        // The primary key is added as a tie-breaker for cursors and negative `take`.
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
                let c = &self.check_unique_where(model, c)?;
                match self.scalar_rows(model, c, Some(1)).await?.into_iter().next() {
                    Some(r) => Some(r),
                    None => return Ok(Vec::new()),
                }
            }
        };

        let mut q = Sql::new(self.dialect());
        self.select_scalars(&mut q, model, "t0");
        q.push(" WHERE ");
        self.where_sql(&mut q, model, "t0", args.get("where").unwrap_or(&Value::Null))?;
        if let Some(row) = &cursor_row {
            q.push(" AND (");
            self.cursor_sql(&mut q, &order, row, reverse)?;
            q.push(")");
        }
        self.order_sql(&mut q, "t0", &order, reverse);
        if distinct.is_none() {
            self.limit_sql(&mut q, take.map(|t| t.checked_abs().unwrap_or(i64::MAX)), skip);
        }
        let rows = self.client.fetch(&q, Some(model)).await?;
        let mut records = self.decode_scalars(model, rows.rows)?;
        if let Some(fields) = &distinct {
            let mut seen = HashSet::new();
            records.retain(|r| seen.insert(key_of(&fields.iter().map(|f| r.get(f.name).unwrap_or(&Value::Null)).collect::<Vec<_>>())));
            let skip = skip.unwrap_or(0) as usize;
            records = records.into_iter().skip(skip).collect();
            if let Some(t) = take {
                records.truncate(t.unsigned_abs() as usize);
            }
        }
        if reverse {
            records.reverse();
        }
        Ok(records)
    }

    pub(crate) fn limit_sql(&self, q: &mut Sql, take: Option<i64>, skip: Option<i64>) {
        match (take, skip) {
            (Some(t), s) => {
                q.push(" LIMIT ").bind(SqlValue::Int(t));
                if let Some(s) = s.filter(|s| *s > 0) {
                    q.push(" OFFSET ").bind(SqlValue::Int(s));
                }
            }
            (None, Some(s)) if s > 0 => {
                if self.dialect() == crate::db::Dialect::Sqlite {
                    q.push(" LIMIT -1");
                }
                q.push(" OFFSET ").bind(SqlValue::Int(s));
            }
            _ => {}
        }
    }

    /// Keyset condition "row >= cursor" in the effective ordering.
    pub(crate) fn cursor_sql(&self, q: &mut Sql, order: &[OrderItem], row: &Map<String, Value>, reverse: bool) -> Result<()> {
        let fields: Vec<(&'static FieldMeta, bool)> = order
            .iter()
            .map(|o| match &o.expr {
                OrderExpr::Field(f) => Ok((*f, o.desc != reverse)),
                _ => Err(self.err("`cursor` can only be combined with `orderBy` on scalar fields of the model.")),
            })
            .collect::<Result<_>>()?;
        for i in 0..fields.len() {
            if i > 0 {
                q.push(" OR ");
            }
            q.push("(");
            for (j, (f, desc)) in fields.iter().take(i + 1).enumerate() {
                if j > 0 {
                    q.push(" AND ");
                }
                let v = self.value(f, row.get(f.name).unwrap_or(&Value::Null), true)?;
                let last = j == i;
                let op = if !last {
                    " = "
                } else if i == fields.len() - 1 {
                    if *desc { " <= " } else { " >= " }
                } else if *desc {
                    " < "
                } else {
                    " > "
                };
                self.operand(q, "t0", f);
                q.push(op).bind(v);
            }
            q.push(")");
        }
        Ok(())
    }

    /// Ids of the targets linked to each parent through an implicit m2m join table.
    async fn m2m_links(&self, r: &ResolvedRelation, parent_keys: &[Value]) -> Result<Vec<(Value, Value)>> {
        let join = r.join.as_ref().unwrap();
        if parent_keys.is_empty() {
            return Ok(Vec::new());
        }
        let pk = r.parent_fields[0];
        let tk = r.target_fields[0];
        let mut q = Sql::new(self.dialect());
        q.push("SELECT ").col("j", join.parent_column).push(", ").col("j", join.target_column);
        q.push(" FROM ").table(join.schema, &join.table).push(" AS j WHERE ").col("j", join.parent_column);
        let vals = parent_keys.iter().map(|k| self.value(pk, k, false)).collect::<Result<Vec<_>>>()?;
        self.push_in_list(&mut q, vals, pk.kind, false);
        let rows = self.client.fetch(&q, None).await?;
        let d = self.dialect();
        Ok(rows
            .rows
            .into_iter()
            .map(|r| {
                let mut it = r.into_iter();
                let a = crate::db::coerce_field_d(pk, it.next().unwrap_or(Value::Null), d);
                let b = crate::db::coerce_field_d(tk, it.next().unwrap_or(Value::Null), d);
                (a, b)
            })
            .collect())
    }

    fn load_relation<'a>(
        &'a self,
        model: &'static ModelMeta,
        records: &'a mut [Map<String, Value>],
        f: &'static FieldMeta,
        mut nested: Map<String, Value>,
    ) -> BoxFuture<'a, Result<()>> {
        async move {
            let r = self.relation(model, f)?;
            let list = to_many(&r);
            if !list {
                self.check_keys(&nested, &["select", "include", "omit", "relationLoadStrategy"])?;
            } else {
                self.check_keys(&nested, FIND_MANY_ARGS)?;
            }
            let child_sel = self.parse_selection(r.target, &nested)?;
            let user_where = nested.remove("where").filter(|v| !v.is_null());
            let finish = |children: Vec<Map<String, Value>>| -> Value {
                if list {
                    Value::Array(children.into_iter().map(|c| Value::Object(self.shape(&child_sel, c))).collect())
                } else {
                    children.into_iter().next().map(|c| Value::Object(self.shape(&child_sel, c))).unwrap_or(Value::Null)
                }
            };
            let paginated = list && ["take", "skip", "cursor", "distinct"].iter().any(|k| nested.get(*k).is_some_and(|v| !v.is_null()));

            // ----- implicit many-to-many -----
            if r.kind == RelationKind::ManyToMany {
                let pf = r.parent_fields[0];
                let tf = r.target_fields[0];
                let mut keys = Vec::new();
                let mut seen = HashSet::new();
                for rec in records.iter() {
                    let v = rec.get(pf.name).cloned().unwrap_or(Value::Null);
                    if !v.is_null() && seen.insert(v.to_string()) {
                        keys.push(v);
                    }
                }
                let links = self.m2m_links(&r, &keys).await?;
                let mut by_parent: HashMap<String, Vec<Value>> = HashMap::new();
                for (a, b) in &links {
                    by_parent.entry(a.to_string()).or_default().push(b.clone());
                }
                let run = |ids: Vec<Value>| {
                    let mut w = Map::new();
                    w.insert(tf.name.to_string(), json!({ "in": ids }));
                    let w = match &user_where {
                        Some(uw) => json!({ "AND": [Value::Object(w), uw.clone()] }),
                        None => Value::Object(w),
                    };
                    let mut child_args = nested.clone();
                    child_args.insert("where".into(), w);
                    child_args
                };
                if paginated {
                    for rec in records.iter_mut() {
                        let k = rec.get(pf.name).cloned().unwrap_or(Value::Null).to_string();
                        let ids = by_parent.get(&k).cloned().unwrap_or_default();
                        let children = if ids.is_empty() { Vec::new() } else { self.fetch_rows(r.target, run(ids)).await?.0 };
                        rec.insert(f.name.to_string(), finish(children));
                    }
                    return Ok(());
                }
                let all_ids: Vec<Value> = {
                    let mut s = HashSet::new();
                    links.iter().map(|(_, b)| b.clone()).filter(|b| s.insert(b.to_string())).collect()
                };
                let children = if all_ids.is_empty() { Vec::new() } else { self.fetch_rows(r.target, run(all_ids)).await?.0 };
                for rec in records.iter_mut() {
                    let k = rec.get(pf.name).cloned().unwrap_or(Value::Null).to_string();
                    let linked: HashSet<String> = by_parent.get(&k).map(|v| v.iter().map(|x| x.to_string()).collect()).unwrap_or_default();
                    let mine: Vec<Map<String, Value>> = children
                        .iter()
                        .filter(|c| linked.contains(&c.get(tf.name).cloned().unwrap_or(Value::Null).to_string()))
                        .cloned()
                        .collect();
                    rec.insert(f.name.to_string(), finish(mine));
                }
                return Ok(());
            }

            // ----- one-to-many / one-to-one, one query for every parent -----
            if !paginated {
                let mut keys: Vec<Vec<Value>> = Vec::new();
                let mut seen = HashSet::new();
                for rec in records.iter() {
                    let k: Vec<Value> = r.parent_fields.iter().map(|pf| rec.get(pf.name).cloned().unwrap_or(Value::Null)).collect();
                    if k.iter().all(|v| !v.is_null()) && seen.insert(key_of(&k.iter().collect::<Vec<_>>())) {
                        keys.push(k);
                    }
                }
                let mut groups: HashMap<String, Vec<Map<String, Value>>> = HashMap::new();
                // Single-field keys are one `in` list (bound as one parameter when long); compound keys an `OR` of
                // groups, split into statements within the parameter limit. Without pagination the children of
                // each parent are the same whichever statement returns them.
                let links: Vec<Value> = if keys.is_empty() {
                    Vec::new()
                } else if r.target_fields.len() == 1 {
                    let ids: Vec<Value> = keys.into_iter().map(|mut k| k.swap_remove(0)).collect();
                    vec![Value::Object(Map::from_iter([(r.target_fields[0].name.to_string(), json!({ "in": ids }))]))]
                } else {
                    let as_where =
                        |k: Vec<Value>| -> Value { Value::Object(r.target_fields.iter().map(|tf| tf.name.to_string()).zip(k).collect()) };
                    let fields = r.target_fields.len();
                    self.statement_chunks(keys, |_| fields)
                        .into_iter()
                        .map(|chunk| json!({ "OR": chunk.into_iter().map(as_where).collect::<Vec<_>>() }))
                        .collect()
                };
                for link in links {
                    let w = match &user_where {
                        Some(uw) => json!({ "AND": [link, uw.clone()] }),
                        None => link,
                    };
                    let mut child_args = nested.clone();
                    child_args.insert("where".into(), w);
                    let (children, _) = self.fetch_rows(r.target, child_args).await?;
                    for c in children {
                        let k: Vec<&Value> = r.target_fields.iter().map(|tf| c.get(tf.name).unwrap_or(&Value::Null)).collect();
                        groups.entry(key_of(&k)).or_default().push(c);
                    }
                }
                for rec in records.iter_mut() {
                    let k: Vec<&Value> = r.parent_fields.iter().map(|pf| rec.get(pf.name).unwrap_or(&Value::Null)).collect();
                    let children = groups.get(&key_of(&k)).cloned().unwrap_or_default();
                    rec.insert(f.name.to_string(), finish(children));
                }
                return Ok(());
            }

            // ----- paginated to-many: one query per parent -----
            for rec in records.iter_mut() {
                let mut join = Map::new();
                let mut null_key = false;
                for (pf, tf) in r.parent_fields.iter().zip(r.target_fields.iter()) {
                    let v = rec.get(pf.name).cloned().unwrap_or(Value::Null);
                    null_key |= v.is_null();
                    join.insert(tf.name.to_string(), v);
                }
                if null_key {
                    rec.insert(f.name.to_string(), finish(Vec::new()));
                    continue;
                }
                let mut and = vec![Value::Object(join)];
                if let Some(uw) = &user_where {
                    and.push(uw.clone());
                }
                let mut child_args = nested.clone();
                child_args.insert("where".into(), json!({ "AND": and }));
                let (children, _) = self.fetch_rows(r.target, child_args).await?;
                rec.insert(f.name.to_string(), finish(children));
            }
            Ok(())
        }
        .boxed()
    }

    async fn load_counts(
        &self,
        model: &'static ModelMeta,
        records: &mut [Map<String, Value>],
        counts: &[(&'static FieldMeta, Option<Value>)],
    ) -> Result<()> {
        let mut per_record: Vec<Map<String, Value>> = vec![Map::new(); records.len()];
        for (f, w) in counts {
            let r = self.relation(model, f)?;
            let keys: Vec<Vec<Value>> = records
                .iter()
                .map(|rec| r.parent_fields.iter().map(|pf| rec.get(pf.name).cloned().unwrap_or(Value::Null)).collect())
                .collect();
            let counted = self.count_related(&r, &keys, w.as_ref()).await?;
            for (i, k) in keys.iter().enumerate() {
                let n = counted.get(&key_of(&k.iter().collect::<Vec<_>>())).copied().unwrap_or(0);
                per_record[i].insert(f.name.to_string(), Value::from(n));
            }
        }
        for (rec, c) in records.iter_mut().zip(per_record) {
            rec.insert("_count".into(), Value::Object(c));
        }
        Ok(())
    }

    /// Number of related records for each parent key (`key_of(parent fields)` → count).
    async fn count_related(&self, r: &ResolvedRelation, keys: &[Vec<Value>], w: Option<&Value>) -> Result<HashMap<String, i64>> {
        let mut out = HashMap::new();
        let keys: Vec<&Vec<Value>> = {
            let mut seen = HashSet::new();
            keys.iter().filter(|k| k.iter().all(|v| !v.is_null())).filter(|k| seen.insert(key_of(&k.iter().collect::<Vec<_>>()))).collect()
        };
        if keys.is_empty() {
            return Ok(out);
        }
        let d = self.dialect();
        if let Some(join) = &r.join {
            let mut q = Sql::new(d);
            let pf = r.parent_fields[0];
            let tf = r.target_fields[0];
            q.push("SELECT ").col("j", join.parent_column).push(", COUNT(*) FROM ").table(join.schema, &join.table).push(" AS j");
            q.push(" INNER JOIN ")
                .table(r.target.schema, r.target.table)
                .push(" AS t0 ON ")
                .col("t0", tf.column)
                .push(" = ")
                .col("j", join.target_column);
            q.push(" WHERE ").col("j", join.parent_column);
            let vals = keys.iter().map(|k| self.value(pf, &k[0], false)).collect::<Result<Vec<_>>>()?;
            self.push_in_list(&mut q, vals, pf.kind, false);
            if let Some(w) = w {
                q.push(" AND ");
                self.where_sql(&mut q, r.target, "t0", w)?;
            }
            q.push(" GROUP BY ").col("j", join.parent_column);
            let rows = self.client.fetch(&q, None).await?;
            for row in rows.rows {
                let mut it = row.into_iter();
                let k = crate::db::coerce_field_d(pf, it.next().unwrap_or(Value::Null), d);
                let n = it.next().and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))).unwrap_or(0);
                out.insert(key_of(&[&k]), n);
            }
            return Ok(out);
        }
        // Single-field keys are one `in` list; compound keys an `OR` of groups, one statement per chunk.
        let chunks = if r.target_fields.len() == 1 { vec![keys] } else { self.statement_chunks(keys, |_| r.target_fields.len()) };
        for keys in chunks {
            let mut q = Sql::new(d);
            q.push("SELECT ");
            for tf in &r.target_fields {
                q.col("t0", tf.column).push(", ");
            }
            q.push("COUNT(*) FROM ").table(r.target.schema, r.target.table).push(" AS t0 WHERE ");
            if r.target_fields.len() == 1 {
                q.col("t0", r.target_fields[0].column);
                let vals = keys.iter().map(|k| self.value(r.target_fields[0], &k[0], false)).collect::<Result<Vec<_>>>()?;
                self.push_in_list(&mut q, vals, r.target_fields[0].kind, false);
            } else {
                q.push("(");
                for (i, k) in keys.iter().enumerate() {
                    if i > 0 {
                        q.push(" OR ");
                    }
                    q.push("(");
                    for (j, (tf, v)) in r.target_fields.iter().zip(k.iter()).enumerate() {
                        if j > 0 {
                            q.push(" AND ");
                        }
                        q.col("t0", tf.column).push(" = ").bind(self.value(tf, v, false)?);
                    }
                    q.push(")");
                }
                q.push(")");
            }
            if let Some(w) = w {
                q.push(" AND ");
                self.where_sql(&mut q, r.target, "t0", w)?;
            }
            q.push(" GROUP BY ");
            for (i, tf) in r.target_fields.iter().enumerate() {
                if i > 0 {
                    q.push(", ");
                }
                q.col("t0", tf.column);
            }
            let rows = self.client.fetch(&q, None).await?;
            for row in rows.rows {
                let n = row.last().and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))).unwrap_or(0);
                let k: Vec<Value> = r.target_fields.iter().zip(row).map(|(tf, v)| crate::db::coerce_field_d(tf, v, d)).collect();
                out.insert(key_of(&k.iter().collect::<Vec<_>>()), n);
            }
        }
        Ok(out)
    }

    /// `SELECT COUNT(*) FROM table AS t0 WHERE ...`
    pub(crate) async fn count_where(&self, model: &'static ModelMeta, w: &Value) -> Result<i64> {
        let mut q = Sql::new(self.dialect());
        q.push("SELECT COUNT(*) FROM ").table(model.schema, model.table).push(" AS t0 WHERE ");
        self.where_sql(&mut q, model, "t0", w)?;
        let rows = self.client.fetch(&q, Some(model)).await?;
        Ok(rows.rows.first().and_then(|r| r.first()).and_then(|v| v.as_i64()).unwrap_or(0))
    }

    // ---------------------------------------------------------------------
    // Actions
    // ---------------------------------------------------------------------

    pub(crate) async fn find_many_action(&self, model: &'static ModelMeta, args: Map<String, Value>) -> Result<Value> {
        let records = self.find_records(model, args).await?;
        Ok(Value::Array(records.into_iter().map(Value::Object).collect()))
    }

    pub(crate) async fn find_first_action(&self, model: &'static ModelMeta, mut args: Map<String, Value>, throw: bool) -> Result<Value> {
        let take = self.int_arg(&args, "take")?;
        args.insert("take".into(), Value::from(if take.is_some_and(|t| t < 0) { -1 } else { 1 }));
        let first = self.find_records(model, args).await?.into_iter().next();
        match first {
            Some(r) => Ok(Value::Object(r)),
            None if throw => Err(OrmError::not_found(model, "No record was found for a query.")),
            None => Ok(Value::Null),
        }
    }

    pub(crate) async fn find_unique_action(&self, model: &'static ModelMeta, mut args: Map<String, Value>, throw: bool) -> Result<Value> {
        self.check_keys(&args, FIND_UNIQUE_ARGS)?;
        let w = args.get("where").cloned().unwrap_or(Value::Null);
        if w.is_null() {
            return Err(self.err(format!("Argument `where` is missing. Expected {}WhereUniqueInput.", model.name)));
        }
        let w = self.check_unique_where(model, &w)?;
        args.insert("where".into(), w);
        args.insert("take".into(), Value::from(1));
        let first = self.find_records(model, args).await?.into_iter().next();
        match first {
            Some(r) => Ok(Value::Object(r)),
            None if throw => Err(OrmError::not_found(model, "No record was found for a query.")),
            None => Ok(Value::Null),
        }
    }

    /// Re-reads a record by its id honouring `select` / `include` / `omit` of the original call.
    pub(crate) async fn read_back(&self, model: &'static ModelMeta, id: &Id, args: &Map<String, Value>) -> Result<Value> {
        let mut a = Map::new();
        a.insert("where".into(), Value::Object(id.clone()));
        for k in ["select", "include", "omit"] {
            if let Some(v) = args.get(k) {
                a.insert(k.into(), v.clone());
            }
        }
        a.insert("take".into(), Value::from(1));
        let rec = self.find_records(model, a).await?.into_iter().next();
        rec.map(Value::Object).ok_or_else(|| OrmError::not_found(model, "Record not found after write."))
    }

    /// Re-reads several records by id (createManyAndReturn / updateManyAndReturn), in the given order.
    pub(crate) async fn read_back_many(&self, model: &'static ModelMeta, ids: &[Id], args: &Map<String, Value>) -> Result<Value> {
        let pk = model.pk();
        let mut sel = Selection::default();
        let mut rows = Vec::new();
        let any_of = |keys: Vec<Id>| json!({ "OR": keys.into_iter().map(Value::Object).collect::<Vec<_>>() });
        for filter in self.key_filters(&pk, ids.to_vec(), any_of) {
            let mut a = Map::new();
            a.insert("where".into(), filter);
            for k in ["select", "include", "omit"] {
                if let Some(v) = args.get(k) {
                    a.insert(k.into(), v.clone());
                }
            }
            let (chunk, s) = self.fetch_rows(model, a).await?;
            rows.extend(chunk);
            sel = s;
        }
        let key = |r: &Map<String, Value>| key_of(&pk.iter().map(|f| r.get(f.name).unwrap_or(&Value::Null)).collect::<Vec<_>>());
        let mut by_id: HashMap<String, Map<String, Value>> = rows.into_iter().map(|r| (key(&r), r)).collect();
        let mut out = Vec::new();
        for id in ids {
            if let Some(r) = by_id.remove(&key(id)) {
                out.push(Value::Object(self.shape(&sel, r)));
            }
        }
        Ok(Value::Array(out))
    }
}
