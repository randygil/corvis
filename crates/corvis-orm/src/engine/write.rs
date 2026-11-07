//! Writes: `create` (with nested writes), `createMany(AndReturn)`, `update`,
//! `updateMany(AndReturn)`, `upsert`, `delete` and `deleteMany`.
//!
//! Records are identified by their primary key values ([`Id`]), so compound ids
//! (`@@id([a, b])`) work everywhere.

use std::collections::HashSet;

use futures::FutureExt;
use futures::future::BoxFuture;
use serde_json::{Map, Value, json};

use super::{Engine, Id};
use crate::db::{Dialect, Sql, SqlValue};
use crate::error::{OrmError, Result};
use crate::meta::{FieldKind, FieldMeta, ModelMeta, RelationKind, ResolvedRelation, m2m_relation_name};

/// Right-hand side of `SET col = ...`.
#[derive(Debug, Clone)]
pub(crate) enum SetExpr {
    Value(SqlValue),
    /// `col = col <op> value`
    Arith(&'static str, SqlValue),
    /// Scalar list `push`.
    Push(SqlValue),
}

fn list_or_one(v: &Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a.clone(),
        Value::Null => vec![],
        other => vec![other.clone()],
    }
}

fn and(a: Value, b: Option<Value>) -> Value {
    match b {
        Some(b) if !b.is_null() && b != json!({}) => json!({ "AND": [a, b] }),
        _ => a,
    }
}

/// `where` matching any of `ids`.
pub(crate) fn ids_filter(ids: &[Id]) -> Value {
    match ids {
        [one] => Value::Object(one.clone()),
        many => json!({ "OR": many.iter().cloned().map(Value::Object).collect::<Vec<_>>() }),
    }
}

impl Engine {
    /// The schema name of the relation of `f` (`PostToUser` or the `@relation` name).
    pub(crate) fn relation_name(&self, model: &'static ModelMeta, f: &FieldMeta) -> String {
        let rel = f.relation.as_ref().unwrap();
        m2m_relation_name(model.name, (rel.target)().name, rel.name)
    }

    /// `one-to-one` / `one-to-many` / `many-to-many`.
    pub(crate) fn relation_type(&self, model: &'static ModelMeta, f: &FieldMeta, r: &ResolvedRelation) -> &'static str {
        match r.kind {
            RelationKind::ManyToMany => "many-to-many",
            RelationKind::HasMany => "one-to-many",
            RelationKind::HasOne => "one-to-one",
            RelationKind::BelongsTo => {
                let name = self.relation_name(model, f);
                let back = r.target.relations().find(|b| {
                    b.relation.is_some_and(|br| (br.target)().name == model.name && br.kind != RelationKind::BelongsTo)
                        && self.relation_name(r.target, b) == name
                });
                if back.is_some_and(|b| b.list) { "one-to-many" } else { "one-to-one" }
            }
        }
    }

    fn not_found(&self, cause: impl Into<String>) -> OrmError {
        OrmError::not_found(self.root, &cause.into())
    }

    /// `true` when `data` contains nested relation writes (they run in a transaction).
    pub(crate) fn has_nested(model: &'static ModelMeta, data: &Value) -> bool {
        data.as_object().is_some_and(|o| o.keys().any(|k| model.field(k).is_some_and(|f| f.is_relation())))
    }

    /// Runs `f` inside a transaction (reusing the current one if any).
    pub(crate) async fn in_tx<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(Engine) -> BoxFuture<'static, Result<T>> + Send,
    {
        if self.client.in_transaction() {
            return f(self.clone()).await;
        }
        let e = self.clone();
        // Like the engine's own transactions for nested writes: no `maxWait` / `timeout`
        // (the pool's connection timeout still applies).
        let forever = std::time::Duration::from_secs(60 * 60 * 24 * 365);
        let opts = crate::TransactionOptions { max_wait: Some(forever), timeout: Some(forever), isolation_level: None };
        self.client
            ._transaction_with(opts, move |tx| {
                let mut e = e;
                e.client = tx;
                f(e)
            })
            .await
    }

    fn insert_sql(&self, model: &'static ModelMeta, cols: &[(&'static FieldMeta, SqlValue)], returning: bool, skip_duplicates: bool) -> Sql {
        let mut q = Sql::new(self.dialect());
        q.push("INSERT ");
        if skip_duplicates && self.dialect() == Dialect::Sqlite {
            q.push("OR IGNORE ");
        }
        q.push("INTO ").table(model.schema, model.table);
        if cols.is_empty() {
            q.push(" DEFAULT VALUES");
        } else {
            q.push(" (");
            for (i, (f, _)) in cols.iter().enumerate() {
                if i > 0 {
                    q.push(", ");
                }
                q.ident(f.column);
            }
            q.push(") VALUES (");
            for (i, (_, v)) in cols.iter().enumerate() {
                if i > 0 {
                    q.push(", ");
                }
                q.bind(v.clone());
            }
            q.push(")");
        }
        if skip_duplicates && self.dialect() == Dialect::Postgres {
            q.push(" ON CONFLICT DO NOTHING");
        }
        if returning {
            q.push(" RETURNING ");
            for (i, f) in model.pk().iter().enumerate() {
                if i > 0 {
                    q.push(", ");
                }
                q.ident(f.column);
            }
        }
        q
    }

    /// Scalar columns of a create, applying defaults and checking required fields.
    fn complete_insert(&self, model: &'static ModelMeta, cols: &mut Vec<(&'static FieldMeta, SqlValue)>, provided: &HashSet<&'static str>) -> Result<()> {
        for f in model.scalars() {
            if provided.contains(f.name) {
                continue;
            }
            if let Some(d) = self.default_value(f)? {
                cols.push((f, d));
            } else if f.is_required_on_create() {
                // Mention the relation when the missing scalar is a foreign key.
                let rel = model.relations().find(|r| {
                    let m = r.relation.unwrap();
                    m.kind == RelationKind::BelongsTo && m.fields.iter().any(|x| model.field_any(x).is_some_and(|x| x.name == f.name))
                });
                return Err(match rel {
                    Some(r) => self.err(format!("Argument `{}` is missing.", r.name)),
                    None => self.err(format!("Argument `{}` is missing.", f.name)),
                });
            }
        }
        // Columns in schema order, like the reference query engine (parameter positions match).
        cols.sort_by_key(|(f, _)| model.fields.iter().position(|x| x.name == f.name).unwrap_or(usize::MAX));
        Ok(())
    }

    /// Runs an insert with `RETURNING <pk>` and returns the new id.
    async fn insert_returning(&self, model: &'static ModelMeta, cols: &[(&'static FieldMeta, SqlValue)], skip_duplicates: bool) -> Result<Option<Id>> {
        let q = self.insert_sql(model, cols, true, skip_duplicates);
        let rows = self.client.fetch(&q, Some(model)).await?;
        let d = self.dialect();
        Ok(rows
            .rows
            .into_iter()
            .next()
            .map(|r| model.pk().iter().zip(r).map(|(f, v)| (f.name.to_string(), crate::db::coerce_field_d(f, v, d))).collect()))
    }

    /// Scalar row by id.
    pub(crate) async fn row_by_id(&self, model: &'static ModelMeta, id: &Id) -> Result<Map<String, Value>> {
        let rows = self.scalar_rows(model, &Value::Object(id.clone()), Some(1)).await?;
        rows.into_iter().next().ok_or_else(|| self.not_found("Record not found."))
    }

    /// Ids of the rows matching `w`.
    pub(crate) async fn ids_where(&self, model: &'static ModelMeta, w: &Value, limit: Option<i64>) -> Result<Vec<Id>> {
        let pk = model.pk();
        let mut q = Sql::new(self.dialect());
        q.push("SELECT ");
        for (i, f) in pk.iter().enumerate() {
            if i > 0 {
                q.push(", ");
            }
            q.col("t0", f.column);
        }
        q.push(" FROM ").table(model.schema, model.table).push(" AS t0 WHERE ");
        self.where_sql(&mut q, model, "t0", w)?;
        if let Some(l) = limit {
            q.push(" LIMIT ").bind(SqlValue::Int(l));
        }
        let rows = self.client.fetch(&q, Some(model)).await?;
        let d = self.dialect();
        Ok(rows
            .rows
            .into_iter()
            .map(|r| pk.iter().zip(r).map(|(f, v)| (f.name.to_string(), crate::db::coerce_field_d(f, v, d))).collect())
            .collect())
    }

    /// ` WHERE (pk...) IN (SELECT pk... FROM t AS t0 WHERE <w> [LIMIT n])`
    fn pk_in_subquery(&self, q: &mut Sql, model: &'static ModelMeta, w: &Value, limit: Option<i64>) -> Result<()> {
        let pk = model.pk();
        q.push(" WHERE ");
        if pk.len() > 1 {
            q.push("(");
        }
        for (i, f) in pk.iter().enumerate() {
            if i > 0 {
                q.push(", ");
            }
            q.ident(f.column);
        }
        if pk.len() > 1 {
            q.push(")");
        }
        q.push(" IN (SELECT ");
        for (i, f) in pk.iter().enumerate() {
            if i > 0 {
                q.push(", ");
            }
            q.col("t0", f.column);
        }
        q.push(" FROM ").table(model.schema, model.table).push(" AS t0 WHERE ");
        self.where_sql(q, model, "t0", w)?;
        if let Some(l) = limit {
            q.push(" LIMIT ").bind(SqlValue::Int(l));
        }
        q.push(")");
        Ok(())
    }

    pub(crate) async fn update_where(&self, model: &'static ModelMeta, sets: &[(&'static FieldMeta, SetExpr)], w: &Value, limit: Option<i64>) -> Result<u64> {
        if sets.is_empty() {
            return Ok(self.ids_where(model, w, limit).await?.len() as u64);
        }
        let mut q = Sql::new(self.dialect());
        q.push("UPDATE ").table(model.schema, model.table).push(" SET ");
        self.set_list_sql(&mut q, sets, "");
        self.pk_in_subquery(&mut q, model, w, limit)?;
        self.client.execute(&q, Some(model)).await
    }

    /// `col = value, col = alias.col + value, ...` (`alias` qualifies the current values; empty for none).
    fn set_list_sql(&self, q: &mut Sql, sets: &[(&'static FieldMeta, SetExpr)], alias: &str) {
        for (i, (f, e)) in sets.iter().enumerate() {
            if i > 0 {
                q.push(", ");
            }
            q.ident(f.column).push(" = ");
            match e {
                SetExpr::Value(v) => {
                    q.bind(v.clone());
                }
                SetExpr::Arith(op, v) => {
                    q.col(alias, f.column).push(" ").push(op).push(" ").bind(v.clone());
                }
                SetExpr::Push(v) => match self.dialect() {
                    Dialect::Postgres => {
                        q.push("COALESCE(").col(alias, f.column).push(", '{}') || ").bind(v.clone());
                    }
                    Dialect::Sqlite => {
                        q.push("(SELECT json_group_array(value) FROM (SELECT value FROM json_each(COALESCE(").col(alias, f.column);
                        q.push(", '[]')) UNION ALL SELECT value FROM json_each(").bind(v.clone()).push(")))");
                    }
                },
            }
        }
    }

    pub(crate) async fn delete_where(&self, model: &'static ModelMeta, w: &Value, limit: Option<i64>) -> Result<u64> {
        let mut q = Sql::new(self.dialect());
        q.push("DELETE FROM ").table(model.schema, model.table);
        self.pk_in_subquery(&mut q, model, w, limit)?;
        self.client.execute(&q, Some(model)).await
    }

    /// `data: { field: value | { set | increment | decrement | multiply | divide | push } }`
    fn set_expr(&self, f: &'static FieldMeta, v: &Value) -> Result<SetExpr> {
        if let Value::Object(o) = v
            && (f.kind != FieldKind::Json || f.list)
            && !o.contains_key("$type")
        {
            let ops: &[&str] = if f.list { &["set", "push"] } else { &["set", "increment", "decrement", "multiply", "divide"] };
            self.check_keys(o, ops)?;
            if o.len() != 1 {
                return Err(self.err(format!(
                    "Argument `{}`: Invalid value provided. Expected exactly one field of {}, provided {}.",
                    f.name,
                    ops.join(", "),
                    o.len()
                )));
            }
            let (op, val) = o.iter().next().unwrap();
            if op == "set" {
                return Ok(SetExpr::Value(self.value(f, val, false)?));
            }
            if op == "push" {
                let tmp = Value::Array(list_or_one(val));
                return Ok(SetExpr::Push(self.value(f, &tmp, false)?));
            }
            if !f.kind.is_numeric() {
                return Err(self.err(format!("Argument `{op}` is only available for numeric fields; `{}` is {}.", f.name, f.kind.schema_name())));
            }
            let sym = match op.as_str() {
                "increment" => "+",
                "decrement" => "-",
                "multiply" => "*",
                _ => "/",
            };
            return Ok(SetExpr::Arith(sym, self.value(f, val, false)?));
        }
        Ok(SetExpr::Value(self.value(f, v, false)?))
    }

    /// Scalar-only `data` (createMany / updateMany).
    fn scalar_data<'d>(&self, model: &'static ModelMeta, data: &'d Map<String, Value>) -> Result<Vec<(&'static FieldMeta, &'d Value)>> {
        let mut out = Vec::new();
        for (k, v) in data {
            let f = match model.field(k) {
                Some(f) if f.is_scalar() => f,
                _ => return Err(self.unknown_argument(k, model.scalars().map(|f| f.name))),
            };
            out.push((f, v));
        }
        Ok(out)
    }

    fn updated_at_sets(&self, model: &'static ModelMeta, data: &Map<String, Value>) -> Result<Vec<(&'static FieldMeta, SetExpr)>> {
        let mut out = Vec::new();
        for f in model.scalars().filter(|f| f.updated_at) {
            if !data.contains_key(f.name) {
                out.push((f, SetExpr::Value(self.value(f, &Value::String(crate::DateTime::now().to_iso_string()), false)?)));
            }
        }
        Ok(out)
    }

    /// Expands compound unique keys of a nested `where` when possible.
    fn lenient_compound(&self, model: &'static ModelMeta, w: Value) -> Value {
        match &w {
            Value::Object(o) => self.expand_compound(model, o).map(Value::Object).unwrap_or(w),
            _ => w,
        }
    }

    /// Finds the record a nested `connect` (on the side holding the foreign key) points to.
    async fn connect_target(&self, parent: &'static ModelMeta, f: &FieldMeta, r: &ResolvedRelation, w: &Value) -> Result<Map<String, Value>> {
        let w = &self.check_unique_where(r.target, w)?;
        let rows = self.scalar_rows(r.target, w, Some(1)).await?;
        rows.into_iter().next().ok_or_else(|| {
            self.not_found(format!(
                "No '{}' record (needed to inline the relation on '{}' record(s)) was found for a nested connect on {} relation '{}'.",
                r.target.name,
                parent.name,
                self.relation_type(parent, f, r),
                self.relation_name(parent, f)
            ))
        })
    }

    /// Resolves a `belongs_to` nested write into the target row.
    async fn belongs_to_target(&self, model: &'static ModelMeta, f: &'static FieldMeta, r: &ResolvedRelation, op: &str, val: &Value) -> Result<Map<String, Value>> {
        match op {
            "connect" => self.connect_target(model, f, r, val).await,
            "create" => {
                let data = self.obj(val, "create", &format!("{}CreateWithout{}Input", r.target.name, model.name))?.clone();
                let id = self.create_record(r.target, data).await?;
                self.row_by_id(r.target, &id).await
            }
            "connectOrCreate" => {
                let o = self.obj(val, "connectOrCreate", "object")?;
                self.check_keys(o, &["where", "create"])?;
                let w = o.get("where").cloned().unwrap_or(Value::Null);
                let w = self.check_unique_where(r.target, &w)?;
                if let Some(row) = self.scalar_rows(r.target, &w, Some(1)).await?.into_iter().next() {
                    return Ok(row);
                }
                let data = self.obj(o.get("create").unwrap_or(&Value::Null), "create", "object")?.clone();
                let id = self.create_record(r.target, data).await?;
                self.row_by_id(r.target, &id).await
            }
            other => Err(self.unknown_argument(other, ["create", "connect", "connectOrCreate"])),
        }
    }

    /// `create` of one record with nested writes. Returns the id of the new record.
    pub(crate) fn create_record<'a>(&'a self, model: &'static ModelMeta, data: Map<String, Value>) -> BoxFuture<'a, Result<Id>> {
        async move {
            let mut cols: Vec<(&'static FieldMeta, SqlValue)> = Vec::new();
            let mut provided: HashSet<&'static str> = HashSet::new();
            let mut after: Vec<(&'static FieldMeta, Value)> = Vec::new();
            for (k, v) in &data {
                let f = self.field(model, k)?;
                if f.is_scalar() {
                    if provided.contains(f.name) {
                        return Err(self.err(format!("Argument `{k}` was provided twice (directly and through a relation).")));
                    }
                    let v = match v.as_object().and_then(|o| o.get("set")) {
                        Some(inner) if f.list => inner,
                        _ => v,
                    };
                    cols.push((f, self.value(f, v, false)?));
                    provided.insert(f.name);
                    continue;
                }
                let r = self.relation(model, f)?;
                if r.kind != RelationKind::BelongsTo {
                    after.push((f, v.clone()));
                    continue;
                }
                let ops = self.obj(v, k, &format!("{}CreateNestedOneWithout{}Input", r.target.name, model.name))?;
                self.check_keys(ops, &["create", "connect", "connectOrCreate"])?;
                if ops.len() != 1 {
                    return Err(self.err(format!("Argument `{k}` needs exactly one of `create`, `connect` or `connectOrCreate`.")));
                }
                let (op, val) = ops.iter().next().unwrap();
                let target_row = self.belongs_to_target(model, f, &r, op, val).await?;
                for (pf, tf) in r.parent_fields.iter().zip(r.target_fields.iter()) {
                    if provided.contains(pf.name) {
                        return Err(self.err(format!("Argument `{}` was provided twice (directly and through `{k}`).", pf.name)));
                    }
                    cols.push((pf, self.value(pf, target_row.get(tf.name).unwrap_or(&Value::Null), false)?));
                    provided.insert(pf.name);
                }
            }
            self.complete_insert(model, &mut cols, &provided)?;
            let id = self.insert_returning(model, &cols, false).await?.unwrap_or_default();
            for (f, v) in after {
                self.nested_relation_writes(model, &id, f, v, false).await?;
            }
            Ok(id)
        }
        .boxed()
    }

    // ---------------------------------------------------------------------
    // Implicit many-to-many
    // ---------------------------------------------------------------------

    async fn m2m_link(&self, r: &ResolvedRelation, parent_key: &Value, target_key: &Value) -> Result<()> {
        let join = r.join.as_ref().unwrap();
        let mut q = Sql::new(self.dialect());
        q.push("INSERT ");
        if self.dialect() == Dialect::Sqlite {
            q.push("OR IGNORE ");
        }
        q.push("INTO ").table(join.schema, &join.table).push(" (").ident(join.parent_column).push(", ").ident(join.target_column);
        q.push(") VALUES (").bind(self.value(r.parent_fields[0], parent_key, false)?).push(", ");
        q.bind(self.value(r.target_fields[0], target_key, false)?).push(")");
        if self.dialect() == Dialect::Postgres {
            q.push(" ON CONFLICT DO NOTHING");
        }
        self.client.execute(&q, None).await?;
        Ok(())
    }

    async fn m2m_unlink(&self, r: &ResolvedRelation, parent_key: &Value, target_keys: Option<&[Value]>) -> Result<u64> {
        let join = r.join.as_ref().unwrap();
        let mut q = Sql::new(self.dialect());
        q.push("DELETE FROM ").table(join.schema, &join.table).push(" WHERE ").ident(join.parent_column).push(" = ");
        q.bind(self.value(r.parent_fields[0], parent_key, false)?);
        if let Some(keys) = target_keys {
            if keys.is_empty() {
                return Ok(0);
            }
            q.push(" AND ").ident(join.target_column).push(" IN (");
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    q.push(", ");
                }
                q.bind(self.value(r.target_fields[0], k, false)?);
            }
            q.push(")");
        }
        self.client.execute(&q, None).await
    }

    /// The field of the target pointing back to `model` in the same relation.
    fn back_field(&self, model: &'static ModelMeta, f: &'static FieldMeta, r: &ResolvedRelation) -> Option<&'static FieldMeta> {
        let name = self.relation_name(model, f);
        r.target.relations().find(|b| {
            b.relation.is_some_and(|br| (br.target)().name == model.name)
                && !(model.name == r.target.name && b.name == f.name)
                && self.relation_name(r.target, b) == name
        })
    }

    /// Nested writes on a `has_many` / `has_one` / many-to-many relation of the record `id`.
    fn nested_relation_writes<'a>(&'a self, model: &'static ModelMeta, id: &'a Id, f: &'static FieldMeta, v: Value, in_update: bool) -> BoxFuture<'a, Result<()>> {
        async move {
            let r = self.relation(model, f)?;
            let list = matches!(r.kind, RelationKind::HasMany | RelationKind::ManyToMany);
            let m2m = r.kind == RelationKind::ManyToMany;
            let ops = self.obj(&v, f.name, "object")?.clone();
            let mut allowed = vec!["create", "createMany", "connect", "connectOrCreate"];
            if in_update {
                allowed.extend(["disconnect", "set", "delete", "deleteMany", "update", "updateMany", "upsert"]);
            }
            if !list {
                allowed.retain(|a| !matches!(*a, "createMany" | "set" | "deleteMany" | "updateMany"));
            }
            if m2m {
                allowed.retain(|a| *a != "createMany");
            }
            self.check_keys(&ops, &allowed)?;
            let rel_name = self.relation_name(model, f);
            let rel_type = self.relation_type(model, f, &r);

            // Values of the parent fields referenced by the relation.
            let parent: Map<String, Value> =
                if r.parent_fields.iter().all(|pf| id.contains_key(pf.name)) { id.clone() } else { self.row_by_id(model, id).await? };
            let parent_key = parent.get(r.parent_fields[0].name).cloned().unwrap_or(Value::Null);
            let mut fk = Map::new();
            if !m2m {
                for (pf, tf) in r.parent_fields.iter().zip(r.target_fields.iter()) {
                    fk.insert(tf.name.to_string(), parent.get(pf.name).cloned().unwrap_or(Value::Null));
                }
            }
            let back = self.back_field(model, f, &r);
            // `where` on the target restricted to the records related to this parent.
            let owned = |extra: Option<Value>| -> Value {
                if m2m {
                    let mut inner = Map::new();
                    inner.insert(r.parent_fields[0].name.to_string(), parent_key.clone());
                    let mut w = Map::new();
                    w.insert(back.map(|b| b.name).unwrap_or("").to_string(), json!({ "some": Value::Object(inner) }));
                    and(Value::Object(w), extra.map(|e| self.lenient_compound(r.target, e)))
                } else {
                    and(Value::Object(fk.clone()), extra.map(|e| self.lenient_compound(r.target, e)))
                }
            };
            let fk_sets = |null: bool| -> Result<Vec<(&'static FieldMeta, SetExpr)>> {
                r.target_fields
                    .iter()
                    .map(|tf| {
                        let v = if null { Value::Null } else { fk.get(tf.name).cloned().unwrap_or(Value::Null) };
                        if null && !tf.optional {
                            return Err(OrmError::known(
                                "P2014",
                                format!(
                                    "The change you are trying to make would violate the required relation '{rel_name}' between the `{}` and `{}` models.",
                                    r.target.name, model.name
                                ),
                                json!({ "relation_name": rel_name, "model_a_name": r.target.name, "model_b_name": model.name }),
                            ));
                        }
                        Ok((*tf, SetExpr::Value(self.value(tf, &v, true)?)))
                    })
                    .collect()
            };
            let target_key = |row: &Map<String, Value>| row.get(r.target_fields[0].name).cloned().unwrap_or(Value::Null);

            for (op, val) in &ops {
                match op.as_str() {
                    "create" => {
                        for item in list_or_one(val) {
                            let mut data = self.obj(&item, "create", &format!("{}CreateWithout{}Input", r.target.name, model.name))?.clone();
                            if m2m {
                                let tid = self.create_record(r.target, data).await?;
                                let row = self.row_by_id(r.target, &tid).await?;
                                self.m2m_link(&r, &parent_key, &target_key(&row)).await?;
                            } else {
                                if !list && in_update {
                                    self.release_one_to_one(&r, &fk, &rel_name, model).await?;
                                }
                                for (k, v) in &fk {
                                    data.insert(k.clone(), v.clone());
                                }
                                self.create_record(r.target, data).await?;
                            }
                        }
                    }
                    "createMany" => {
                        let o = self.obj(val, "createMany", "object")?;
                        self.check_keys(o, &["data", "skipDuplicates"])?;
                        let skip = o.get("skipDuplicates").and_then(|v| v.as_bool()).unwrap_or(false);
                        for item in list_or_one(o.get("data").unwrap_or(&Value::Null)) {
                            let mut data = self.obj(&item, "data", "object")?.clone();
                            for (k, v) in &fk {
                                data.insert(k.clone(), v.clone());
                            }
                            self.insert_scalars(r.target, &data, skip, false).await?;
                        }
                    }
                    "connect" => {
                        let items = list_or_one(val);
                        let mut found = 0usize;
                        for w in &items {
                            let w = &self.check_unique_where(r.target, w)?;
                            if m2m {
                                if let Some(row) = self.scalar_rows(r.target, w, Some(1)).await?.into_iter().next() {
                                    self.m2m_link(&r, &parent_key, &target_key(&row)).await?;
                                    found += 1;
                                }
                            } else {
                                if !list && in_update {
                                    let new_ids = self.ids_where(r.target, w, Some(1)).await?;
                                    let cur_ids = self.ids_where(r.target, &Value::Object(fk.clone()), None).await?;
                                    let others: Vec<_> = cur_ids.into_iter().filter(|c| !new_ids.contains(c)).collect();
                                    if !others.is_empty() {
                                        self.update_where(r.target, &fk_sets(true)?, &ids_filter(&others), None).await?;
                                    }
                                }
                                found += self.update_where(r.target, &fk_sets(false)?, w, Some(1)).await? as usize;
                            }
                        }
                        if found < items.len() {
                            return Err(if list {
                                self.not_found(format!("Expected {} records to be connected, found only {found}.", items.len()))
                            } else {
                                self.not_found(format!(
                                    "No '{}' record(s) (needed to inline the relation on '{}' record(s)) was found for a nested connect on {rel_type} relation '{rel_name}'.",
                                    r.target.name, r.target.name
                                ))
                            });
                        }
                    }
                    "connectOrCreate" => {
                        for item in list_or_one(val) {
                            let o = self.obj(&item, "connectOrCreate", "object")?;
                            self.check_keys(o, &["where", "create"])?;
                            let w = o.get("where").cloned().unwrap_or(Value::Null);
                            let w = self.check_unique_where(r.target, &w)?;
                            let existing = self.scalar_rows(r.target, &w, Some(1)).await?.into_iter().next();
                            match (existing, m2m) {
                                (Some(row), true) => self.m2m_link(&r, &parent_key, &target_key(&row)).await?,
                                (Some(_), false) => {
                                    self.update_where(r.target, &fk_sets(false)?, &w, Some(1)).await?;
                                }
                                (None, true) => {
                                    let data = self.obj(o.get("create").unwrap_or(&Value::Null), "create", "object")?.clone();
                                    let tid = self.create_record(r.target, data).await?;
                                    let row = self.row_by_id(r.target, &tid).await?;
                                    self.m2m_link(&r, &parent_key, &target_key(&row)).await?;
                                }
                                (None, false) => {
                                    let mut data = self.obj(o.get("create").unwrap_or(&Value::Null), "create", "object")?.clone();
                                    for (k, v) in &fk {
                                        data.insert(k.clone(), v.clone());
                                    }
                                    self.create_record(r.target, data).await?;
                                }
                            }
                        }
                    }
                    "disconnect" => {
                        if m2m {
                            let mut keys = Vec::new();
                            for w in list_or_one(val) {
                                for row in self.scalar_rows(r.target, &w, None).await? {
                                    keys.push(target_key(&row));
                                }
                            }
                            self.m2m_unlink(&r, &parent_key, Some(&keys)).await?;
                        } else if list {
                            for w in list_or_one(val) {
                                self.update_where(r.target, &fk_sets(true)?, &owned(Some(w)), None).await?;
                            }
                        } else if val.as_bool() == Some(true) || val.is_object() {
                            let extra = val.is_object().then(|| val.clone());
                            self.update_where(r.target, &fk_sets(true)?, &owned(extra), None).await?;
                        }
                    }
                    "set" => {
                        let items = list_or_one(val);
                        if m2m {
                            self.m2m_unlink(&r, &parent_key, None).await?;
                            let mut found = 0;
                            for w in &items {
                                let w = &self.check_unique_where(r.target, w)?;
                                if let Some(row) = self.scalar_rows(r.target, w, Some(1)).await?.into_iter().next() {
                                    self.m2m_link(&r, &parent_key, &target_key(&row)).await?;
                                    found += 1;
                                }
                            }
                            if found < items.len() {
                                return Err(self.not_found(format!("Expected {} records to be connected, found only {found}.", items.len())));
                            }
                        } else {
                            let mut keep: Vec<Id> = Vec::new();
                            for w in &items {
                                let w = &self.check_unique_where(r.target, w)?;
                                keep.extend(self.ids_where(r.target, w, Some(1)).await?);
                            }
                            let current = self.ids_where(r.target, &owned(None), None).await?;
                            let drop: Vec<Id> = current.into_iter().filter(|c| !keep.contains(c)).collect();
                            if !drop.is_empty() {
                                self.update_where(r.target, &fk_sets(true)?, &ids_filter(&drop), None).await?;
                            }
                            if keep.len() < items.len() {
                                return Err(self.not_found(format!("Expected {} records to be connected, found only {}.", items.len(), keep.len())));
                            }
                            if !keep.is_empty() {
                                self.update_where(r.target, &fk_sets(false)?, &ids_filter(&keep), None).await?;
                            }
                        }
                    }
                    "delete" => {
                        let items: Vec<Option<Value>> = if list {
                            list_or_one(val).into_iter().map(Some).collect()
                        } else if val.as_bool() == Some(true) {
                            vec![None]
                        } else if val.is_object() {
                            vec![Some(val.clone())]
                        } else {
                            vec![]
                        };
                        for w in items {
                            let n = self.delete_where(r.target, &owned(w), Some(1)).await?;
                            if n == 0 {
                                return Err(self.not_found(format!(
                                    "No '{}' record was found for a nested delete on {rel_type} relation '{rel_name}'.",
                                    r.target.name
                                )));
                            }
                        }
                    }
                    "deleteMany" => {
                        let items = list_or_one(val);
                        let w = if items.is_empty() { owned(None) } else { owned(Some(json!({ "OR": items }))) };
                        self.delete_where(r.target, &w, None).await?;
                    }
                    "update" => {
                        for item in list_or_one(val) {
                            let o = self.obj(&item, "update", "object")?;
                            let (w, data) = if list || (o.contains_key("data") && o.keys().all(|k| k == "where" || k == "data")) {
                                self.check_keys(o, &["where", "data"])?;
                                (o.get("where").cloned(), o.get("data").cloned().unwrap_or(Value::Null))
                            } else {
                                (None, item.clone())
                            };
                            let ids = self.ids_where(r.target, &owned(w), Some(1)).await?;
                            let Some(tid) = ids.into_iter().next() else {
                                return Err(self.not_found(format!(
                                    "No '{}' record was found for a nested update on {rel_type} relation '{rel_name}'.",
                                    r.target.name
                                )));
                            };
                            let data = self.obj(&data, "data", "object")?.clone();
                            self.update_record(r.target, tid, data).await?;
                        }
                    }
                    "updateMany" => {
                        for item in list_or_one(val) {
                            let o = self.obj(&item, "updateMany", "object")?;
                            self.check_keys(o, &["where", "data"])?;
                            let data = self.obj(o.get("data").unwrap_or(&Value::Null), "data", "object")?;
                            let sets = self.scalar_sets(r.target, data)?;
                            self.update_where(r.target, &sets, &owned(o.get("where").cloned()), None).await?;
                        }
                    }
                    "upsert" => {
                        for item in list_or_one(val) {
                            let o = self.obj(&item, "upsert", "object")?;
                            self.check_keys(o, &["where", "create", "update"])?;
                            let w = o.get("where").cloned();
                            let existing = self.ids_where(r.target, &owned(w), Some(1)).await?.into_iter().next();
                            match existing {
                                Some(tid) => {
                                    let data = self.obj(o.get("update").unwrap_or(&Value::Null), "update", "object")?.clone();
                                    self.update_record(r.target, tid, data).await?;
                                }
                                None => {
                                    let mut data = self.obj(o.get("create").unwrap_or(&Value::Null), "create", "object")?.clone();
                                    if m2m {
                                        let tid = self.create_record(r.target, data).await?;
                                        let row = self.row_by_id(r.target, &tid).await?;
                                        self.m2m_link(&r, &parent_key, &target_key(&row)).await?;
                                    } else {
                                        for (k, v) in &fk {
                                            data.insert(k.clone(), v.clone());
                                        }
                                        self.create_record(r.target, data).await?;
                                    }
                                }
                            }
                        }
                    }
                    _ => unreachable!(),
                }
            }
            Ok(())
        }
        .boxed()
    }

    /// One-to-one `create` in an update while a partner exists: the current one is disconnected
    /// (or the write fails with P2014 when its foreign key is required).
    async fn release_one_to_one(&self, r: &ResolvedRelation, fk: &Map<String, Value>, rel_name: &str, model: &'static ModelMeta) -> Result<()> {
        let existing = self.ids_where(r.target, &Value::Object(fk.clone()), None).await?;
        if existing.is_empty() {
            return Ok(());
        }
        if !r.target_fields.iter().all(|f| f.optional) {
            return Err(OrmError::known(
                "P2014",
                format!(
                    "The change you are trying to make would violate the required relation '{rel_name}' between the `{}` and `{}` models.",
                    r.target.name, model.name
                ),
                json!({ "relation_name": rel_name, "model_a_name": r.target.name, "model_b_name": model.name }),
            ));
        }
        let sets: Vec<(&'static FieldMeta, SetExpr)> = r.target_fields.iter().map(|tf| (*tf, SetExpr::Value(SqlValue::Null(tf.kind)))).collect();
        self.update_where(r.target, &sets, &ids_filter(&existing), None).await?;
        Ok(())
    }

    fn scalar_sets(&self, model: &'static ModelMeta, data: &Map<String, Value>) -> Result<Vec<(&'static FieldMeta, SetExpr)>> {
        let mut sets = Vec::new();
        for (f, v) in self.scalar_data(model, data)? {
            sets.push((f, self.set_expr(f, v)?));
        }
        sets.extend(self.updated_at_sets(model, data)?);
        Ok(sets)
    }

    /// Inserts a scalar-only record (createMany). Returns the affected rows and, when
    /// `returning`, the new id.
    async fn insert_scalars(&self, model: &'static ModelMeta, data: &Map<String, Value>, skip_duplicates: bool, returning: bool) -> Result<(u64, Option<Id>)> {
        let mut cols = Vec::new();
        let mut provided = HashSet::new();
        for (f, v) in self.scalar_data(model, data)? {
            let v = match v.as_object().and_then(|o| o.get("set")) {
                Some(inner) if f.list => inner,
                _ => v,
            };
            cols.push((f, self.value(f, v, false)?));
            provided.insert(f.name);
        }
        self.complete_insert(model, &mut cols, &provided)?;
        if returning {
            let id = self.insert_returning(model, &cols, skip_duplicates).await?;
            return Ok((id.is_some() as u64, id));
        }
        let q = self.insert_sql(model, &cols, false, skip_duplicates);
        Ok((self.client.execute(&q, Some(model)).await?, None))
    }

    /// `update` of one record (by id) with nested writes. Returns the (possibly new) id.
    pub(crate) fn update_record<'a>(&'a self, model: &'static ModelMeta, id: Id, data: Map<String, Value>) -> BoxFuture<'a, Result<Id>> {
        async move {
            let mut sets: Vec<(&'static FieldMeta, SetExpr)> = Vec::new();
            let mut after: Vec<(&'static FieldMeta, Value)> = Vec::new();
            let mut new_id = id.clone();
            for (k, v) in &data {
                let f = self.field(model, k)?;
                if f.is_scalar() {
                    let e = self.set_expr(f, v)?;
                    if id.contains_key(f.name)
                        && let SetExpr::Value(_) = e
                    {
                        let nv = v.as_object().and_then(|o| o.get("set")).cloned().unwrap_or_else(|| v.clone());
                        new_id.insert(f.name.to_string(), nv);
                    }
                    sets.push((f, e));
                    continue;
                }
                let r = self.relation(model, f)?;
                if r.kind != RelationKind::BelongsTo {
                    after.push((f, v.clone()));
                    continue;
                }
                let ops = self.obj(v, k, &format!("{}UpdateOneWithout{}NestedInput", r.target.name, model.name))?;
                let mut allowed = vec!["create", "connect", "connectOrCreate", "update", "upsert"];
                if r.parent_fields.iter().all(|p| p.optional) {
                    allowed.extend(["disconnect", "delete"]);
                }
                self.check_keys(ops, &allowed)?;
                let rel_name = self.relation_name(model, f);
                let rel_type = self.relation_type(model, f, &r);
                for (op, val) in ops {
                    match op.as_str() {
                        "disconnect" => {
                            if val.as_bool() == Some(false) {
                                continue;
                            }
                            for pf in &r.parent_fields {
                                sets.push((pf, SetExpr::Value(SqlValue::Null(pf.kind))));
                            }
                        }
                        "delete" => {
                            if val.as_bool() == Some(false) {
                                continue;
                            }
                            let row = self.row_by_id(model, &id).await?;
                            let mut w = Map::new();
                            for (pf, tf) in r.parent_fields.iter().zip(r.target_fields.iter()) {
                                w.insert(tf.name.to_string(), row.get(pf.name).cloned().unwrap_or(Value::Null));
                            }
                            let w = and(Value::Object(w), val.is_object().then(|| val.clone()));
                            for pf in &r.parent_fields {
                                sets.push((pf, SetExpr::Value(SqlValue::Null(pf.kind))));
                            }
                            // Clear the foreign key first, then delete the target.
                            self.update_where(model, &std::mem::take(&mut sets), &Value::Object(id.clone()), None).await?;
                            let n = self.delete_where(r.target, &w, Some(1)).await?;
                            if n == 0 {
                                return Err(self.not_found(format!(
                                    "No '{}' record was found for a nested delete on {rel_type} relation '{rel_name}'.",
                                    r.target.name
                                )));
                            }
                        }
                        "update" | "upsert" => {
                            let o = self.obj(val, op, "object")?;
                            let row = self.row_by_id(model, &id).await?;
                            let mut w = Map::new();
                            for (pf, tf) in r.parent_fields.iter().zip(r.target_fields.iter()) {
                                w.insert(tf.name.to_string(), row.get(pf.name).cloned().unwrap_or(Value::Null));
                            }
                            let linked = r.parent_fields.iter().all(|pf| !row.get(pf.name).unwrap_or(&Value::Null).is_null());
                            let tid = if linked { self.ids_where(r.target, &Value::Object(w), Some(1)).await?.into_iter().next() } else { None };
                            if op == "update" {
                                let data = if o.contains_key("data") && o.keys().all(|k| k == "where" || k == "data") {
                                    self.obj(&o["data"], "data", "object")?.clone()
                                } else {
                                    o.clone()
                                };
                                let Some(tid) = tid else {
                                    return Err(self.not_found(format!(
                                        "No '{}' record was found for a nested update on {rel_type} relation '{rel_name}'.",
                                        r.target.name
                                    )));
                                };
                                self.update_record(r.target, tid, data).await?;
                            } else {
                                self.check_keys(o, &["create", "update", "where"])?;
                                match tid {
                                    Some(tid) => {
                                        let data = self.obj(o.get("update").unwrap_or(&Value::Null), "update", "object")?.clone();
                                        self.update_record(r.target, tid, data).await?;
                                    }
                                    None => {
                                        let data = self.obj(o.get("create").unwrap_or(&Value::Null), "create", "object")?.clone();
                                        let tid = self.create_record(r.target, data).await?;
                                        let trow = self.row_by_id(r.target, &tid).await?;
                                        for (pf, tf) in r.parent_fields.iter().zip(r.target_fields.iter()) {
                                            sets.push((pf, SetExpr::Value(self.value(pf, trow.get(tf.name).unwrap_or(&Value::Null), false)?)));
                                        }
                                    }
                                }
                            }
                        }
                        _ => {
                            let target_row = self.belongs_to_target(model, f, &r, op, val).await?;
                            for (pf, tf) in r.parent_fields.iter().zip(r.target_fields.iter()) {
                                sets.push((pf, SetExpr::Value(self.value(pf, target_row.get(tf.name).unwrap_or(&Value::Null), false)?)));
                            }
                        }
                    }
                }
            }
            sets.extend(self.updated_at_sets(model, &data)?);
            if !sets.is_empty() {
                self.update_where(model, &sets, &Value::Object(id.clone()), None).await?;
            }
            for (f, v) in after {
                self.nested_relation_writes(model, &new_id, f, v, true).await?;
            }
            Ok(new_id)
        }
        .boxed()
    }

    // ---------------------------------------------------------------------
    // Actions
    // ---------------------------------------------------------------------

    fn data_arg(&self, model: &'static ModelMeta, args: &Map<String, Value>, key: &str, input: &str) -> Result<Map<String, Value>> {
        match args.get(key) {
            None | Some(Value::Null) => Err(self.err(format!("Argument `{key}` is missing."))),
            Some(v) => Ok(self.obj(v, key, &format!("{}{input}", model.name))?.clone()),
        }
    }

    pub(crate) async fn create_action(&self, model: &'static ModelMeta, args: Map<String, Value>) -> Result<Value> {
        self.check_keys(&args, &["data", "select", "include", "omit", "relationLoadStrategy"])?;
        self.parse_selection(model, &args)?;
        let data = self.data_arg(model, &args, "data", "CreateInput")?;
        let id = if Self::has_nested(model, &Value::Object(data.clone())) {
            self.in_tx(move |e| async move { e.create_record(model, data).await }.boxed()).await?
        } else {
            self.create_record(model, data).await?
        };
        self.read_back(model, &id, &args).await
    }

    pub(crate) async fn create_many_action(&self, model: &'static ModelMeta, args: Map<String, Value>, returning: bool) -> Result<Value> {
        if returning {
            self.check_keys(&args, &["data", "skipDuplicates", "select", "include", "omit"])?;
            self.parse_selection(model, &args)?;
        } else {
            self.check_keys(&args, &["data", "skipDuplicates"])?;
        }
        let items = list_or_one(args.get("data").unwrap_or(&Value::Null));
        let skip = args.get("skipDuplicates").and_then(|v| v.as_bool()).unwrap_or(false);
        let mut datas = Vec::new();
        for item in items {
            datas.push(self.obj(&item, "data", &format!("{}CreateManyInput", model.name))?.clone());
        }
        let (count, ids) = self
            .in_tx(move |e| {
                async move {
                    let mut n = 0;
                    let mut ids = Vec::new();
                    for d in &datas {
                        let (k, id) = e.insert_scalars(model, d, skip, returning).await?;
                        n += k;
                        ids.extend(id);
                    }
                    Ok((n, ids))
                }
                .boxed()
            })
            .await?;
        if returning {
            return self.read_back_many(model, &ids, &args).await;
        }
        Ok(json!({ "count": count }))
    }

    pub(crate) async fn update_action(&self, model: &'static ModelMeta, args: Map<String, Value>) -> Result<Value> {
        self.check_keys(&args, &["where", "data", "select", "include", "omit", "relationLoadStrategy"])?;
        self.parse_selection(model, &args)?;
        let w = args.get("where").cloned().unwrap_or(Value::Null);
        let w = self.check_unique_where(model, &w)?;
        let data = self.data_arg(model, &args, "data", "UpdateInput")?;
        let nested = Self::has_nested(model, &Value::Object(data.clone()));
        let run = move |e: Engine| {
            async move {
                let Some(id) = e.ids_where(model, &w, Some(1)).await?.into_iter().next() else {
                    return Err(e.not_found("No record was found for an update."));
                };
                e.update_record(model, id, data).await
            }
            .boxed()
        };
        let new_id = if nested { self.in_tx(run).await? } else { run(self.clone()).await? };
        self.read_back(model, &new_id, &args).await
    }

    pub(crate) async fn update_many_action(&self, model: &'static ModelMeta, args: Map<String, Value>, returning: bool) -> Result<Value> {
        if returning {
            self.check_keys(&args, &["where", "data", "limit", "select", "include", "omit"])?;
            self.parse_selection(model, &args)?;
        } else {
            self.check_keys(&args, &["where", "data", "limit"])?;
        }
        let data = self.data_arg(model, &args, "data", "UpdateManyMutationInput")?;
        let sets = self.scalar_sets(model, &data)?;
        let limit = self.int_arg(&args, "limit")?;
        let w = args.get("where").cloned().unwrap_or(Value::Null);
        if !returning {
            let n = self.update_where(model, &sets, &w, limit).await?;
            return Ok(json!({ "count": n }));
        }
        let ids = self
            .in_tx(move |e| {
                async move {
                    let ids = e.ids_where(model, &w, limit).await?;
                    if ids.is_empty() {
                        return Ok(ids);
                    }
                    e.update_where(model, &sets, &ids_filter(&ids), None).await?;
                    // Primary keys may have been updated.
                    let mut new_ids = Vec::new();
                    for id in ids {
                        let mut nid = id.clone();
                        for (k, v) in &data {
                            if nid.contains_key(k) {
                                let nv = v.as_object().and_then(|o| o.get("set")).cloned().unwrap_or_else(|| v.clone());
                                if !nv.is_object() {
                                    nid.insert(k.clone(), nv);
                                }
                            }
                        }
                        new_ids.push(nid);
                    }
                    Ok(new_ids)
                }
                .boxed()
            })
            .await?;
        self.read_back_many(model, &ids, &args).await
    }

    pub(crate) async fn upsert_action(&self, model: &'static ModelMeta, args: Map<String, Value>) -> Result<Value> {
        self.check_keys(&args, &["where", "create", "update", "select", "include", "omit", "relationLoadStrategy"])?;
        self.parse_selection(model, &args)?;
        let raw_where = args.get("where").cloned().unwrap_or(Value::Null);
        let w = self.check_unique_where(model, &raw_where)?;
        let create = self.data_arg(model, &args, "create", "CreateInput")?;
        let update = match args.get("update") {
            None | Some(Value::Null) => return Err(self.err("Argument `update` is missing.")),
            Some(v) => self.obj(v, "update", &format!("{}UpdateInput", model.name))?.clone(),
        };
        if let Some(target) = self.native_upsert_target(model, &raw_where, &create, &update, &args) {
            let id = self.native_upsert(model, &target, &w, &create, &update).await?;
            return self.read_back(model, &id, &args).await;
        }
        let id = self
            .in_tx(move |e| {
                async move {
                    match e.ids_where(model, &w, Some(1)).await?.into_iter().next() {
                        Some(id) => e.update_record(model, id, update).await,
                        None => e.create_record(model, create).await,
                    }
                }
                .boxed()
            })
            .await?;
        self.read_back(model, &id, &args).await
    }

    /// The conflict target of a native upsert (`INSERT ... ON CONFLICT (target) DO UPDATE`), when
    /// the reference query engine would use one (`can_use_connector_native_upsert`): exactly one unique
    /// field (or compound unique) in `where`, every `where` value equal to the one in `create`,
    /// no nested writes in `create` / `update`, a non-empty `update` and no relations or `_count`
    /// in `select` / `include`. Otherwise corvis-orm (like the reference engine) reads and then inserts or
    /// updates, which does not consume a sequence value when the record exists.
    fn native_upsert_target(
        &self,
        model: &'static ModelMeta,
        raw_where: &Value,
        create: &Map<String, Value>,
        update: &Map<String, Value>,
        args: &Map<String, Value>,
    ) -> Option<Vec<&'static FieldMeta>> {
        let w = raw_where.as_object()?;
        if update.is_empty() {
            return None;
        }
        let is_relation = |k: &String| model.field(k).is_some_and(|f| f.is_relation());
        if create.keys().any(is_relation) || update.keys().any(is_relation) {
            return None;
        }
        let nested_select = ["select", "include"].iter().any(|k| {
            args.get(*k).and_then(|v| v.as_object()).is_some_and(|o| {
                o.iter().any(|(name, v)| {
                    !matches!(v, Value::Null | Value::Bool(false)) && (name == "_count" || model.field(name).is_some_and(|f| f.is_relation()))
                })
            })
        });
        if nested_select {
            return None;
        }
        let single_pk = !model.has_compound_pk();
        let mut target = None;
        let mut uniques = 0;
        for k in w.keys() {
            match model.field(k) {
                Some(f) if f.is_scalar() && (f.is_unique || (f.is_id && single_pk)) => {
                    uniques += 1;
                    target = Some(vec![f]);
                }
                Some(_) => {}
                None => {
                    if let Some(fields) = model.compound_unique(k) {
                        uniques += 1;
                        target = Some(fields);
                    }
                }
            }
        }
        if uniques != 1 || !w.iter().all(|(k, v)| self.where_matches_create(model, k, v, create)) {
            return None;
        }
        target
    }

    /// The `where_and_create_equal` rule of the reference engine: objects are compared field by field (so `{ a_b: { a, b } }`
    /// needs `create.a` and `create.b`, and a filter like `{ equals }` never matches).
    fn where_matches_create(&self, model: &'static ModelMeta, k: &str, v: &Value, create: &Map<String, Value>) -> bool {
        let f = model.field(k);
        match v {
            Value::Object(inner) if f.is_none_or(|f| f.kind != FieldKind::Json) => {
                inner.iter().all(|(ik, iv)| self.where_matches_create(model, ik, iv, create))
            }
            _ => match (create.get(k), f) {
                (None, _) => false,
                (Some(c), Some(f)) if f.is_scalar() => match (self.value(f, v, true), self.value(f, c, true)) {
                    (Ok(a), Ok(b)) => a == b,
                    _ => c == v,
                },
                (Some(c), _) => c == v,
            },
        }
    }

    /// `INSERT ... ON CONFLICT (target) DO UPDATE SET ... WHERE <where> RETURNING <pk>`.
    async fn native_upsert(
        &self,
        model: &'static ModelMeta,
        target: &[&'static FieldMeta],
        w: &Value,
        create: &Map<String, Value>,
        update: &Map<String, Value>,
    ) -> Result<Id> {
        let mut cols = Vec::new();
        let mut provided = HashSet::new();
        for (f, v) in self.scalar_data(model, create)? {
            let v = match v.as_object().and_then(|o| o.get("set")) {
                Some(inner) if f.list => inner,
                _ => v,
            };
            cols.push((f, self.value(f, v, false)?));
            provided.insert(f.name);
        }
        self.complete_insert(model, &mut cols, &provided)?;
        let sets = self.scalar_sets(model, update)?;
        let mut q = Sql::new(self.dialect());
        q.push("INSERT INTO ").table(model.schema, model.table).push(" AS t0 (");
        for (i, (f, _)) in cols.iter().enumerate() {
            if i > 0 {
                q.push(", ");
            }
            q.ident(f.column);
        }
        q.push(") VALUES (");
        for (i, (_, v)) in cols.iter().enumerate() {
            if i > 0 {
                q.push(", ");
            }
            q.bind(v.clone());
        }
        q.push(") ON CONFLICT (");
        for (i, f) in target.iter().enumerate() {
            if i > 0 {
                q.push(", ");
            }
            q.ident(f.column);
        }
        q.push(") DO UPDATE SET ");
        self.set_list_sql(&mut q, &sets, "t0");
        q.push(" WHERE ");
        self.where_sql(&mut q, model, "t0", w)?;
        q.push(" RETURNING ");
        for (i, f) in model.pk().iter().enumerate() {
            if i > 0 {
                q.push(", ");
            }
            q.ident(f.column);
        }
        let rows = self.client.fetch(&q, Some(model)).await?;
        let d = self.dialect();
        rows.rows
            .into_iter()
            .next()
            .map(|r| model.pk().iter().zip(r).map(|(f, v)| (f.name.to_string(), crate::db::coerce_field_d(f, v, d))).collect())
            .ok_or_else(|| self.not_found("No record was found for an upsert."))
    }

    pub(crate) async fn delete_action(&self, model: &'static ModelMeta, args: Map<String, Value>) -> Result<Value> {
        self.check_keys(&args, &["where", "select", "include", "omit", "relationLoadStrategy"])?;
        self.parse_selection(model, &args)?;
        let w = args.get("where").cloned().unwrap_or(Value::Null);
        let w = self.check_unique_where(model, &w)?;
        let Some(id) = self.ids_where(model, &w, Some(1)).await?.into_iter().next() else {
            return Err(self.not_found("No record was found for a delete."));
        };
        let record = self.read_back(model, &id, &args).await?;
        self.delete_where(model, &Value::Object(id), None).await?;
        Ok(record)
    }

    pub(crate) async fn delete_many_action(&self, model: &'static ModelMeta, args: Map<String, Value>) -> Result<Value> {
        self.check_keys(&args, &["where", "limit"])?;
        let limit = self.int_arg(&args, "limit")?;
        let n = self.delete_where(model, args.get("where").unwrap_or(&Value::Null), limit).await?;
        Ok(json!({ "count": n }))
    }
}
