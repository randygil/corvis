//! DDL generation for `db push`.
//!
//! [`push_plan`] builds what the reference client's migration builds for the same models: on Postgres the schemas
//! of `@@schema`, the enum types, the tables (every column with its type, nullability and default, the primary key
//! over all `@id` / `@@id` fields), the implicit many-to-many join tables, the unique and plain indexes, and then
//! the foreign keys as `ALTER TABLE .. ADD CONSTRAINT`, all named as that migration names them. SQLite has no
//! `ADD CONSTRAINT`, so there the foreign keys are written inside `CREATE TABLE` and the tables are ordered so that
//! referenced tables come first. What the plan cannot express is refused before any statement runs.
//!
//! Every identifier goes through [`quote_ident`] and every literal through [`string_literal`]. The only texts
//! written as they are come from the developer's own model definitions, never from a request: the type of an
//! `Unsupported("..")` column and the expression of `@default(dbgenerated(".."))`, as the reference does.

use std::collections::HashSet;

use crate::db::{Dialect, qualified, quote_ident};
use crate::error::{OrmError, Result};
use crate::meta::{DefaultValue, FieldKind, FieldMeta, ModelMeta, ReferentialAction, RelationKind, resolve_relation};

/// A database object a push may find already there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Object<'a> {
    /// A schema name (Postgres).
    Schema(&'a str),
    /// A quoted, possibly schema-qualified, type name (Postgres enums).
    Type(&'a str),
    /// A quoted, possibly schema-qualified, table name.
    Table(&'a str),
}

struct EnumDdl {
    qualified: String,
    create: String,
}

struct TableDdl {
    qualified: String,
    create: String,
    indexes: Vec<String>,
    foreign_keys: Vec<String>,
}

/// Everything `db push` creates for a set of models, in the reference migration's order.
pub(crate) struct PushPlan {
    dialect: Dialect,
    schemas: Vec<String>,
    enums: Vec<EnumDdl>,
    tables: Vec<TableDdl>,
}

impl PushPlan {
    /// The objects whose existence decides what [`PushPlan::statements`] leaves out (Postgres only: on SQLite every
    /// statement is `IF NOT EXISTS`).
    pub(crate) fn objects(&self) -> Vec<Object<'_>> {
        if self.dialect != Dialect::Postgres {
            return Vec::new();
        }
        let schemas = self.schemas.iter().map(|s| Object::Schema(s));
        let types = self.enums.iter().map(|e| Object::Type(&e.qualified));
        let tables = self.tables.iter().map(|t| Object::Table(&t.qualified));
        schemas.chain(types).chain(tables).collect()
    }

    /// The statements that create what does not exist yet: schemas, enum types, tables, indexes, foreign keys. A
    /// table that exists is left as it is, with its indexes and foreign keys.
    pub(crate) fn statements(&self, exists: impl Fn(Object<'_>) -> bool) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        out.extend(
            self.schemas.iter().filter(|s| !exists(Object::Schema(s))).map(|s| format!("CREATE SCHEMA IF NOT EXISTS {}", quote_ident(s))),
        );
        out.extend(self.enums.iter().filter(|e| !exists(Object::Type(&e.qualified))).map(|e| e.create.clone()));
        let new_tables: Vec<&TableDdl> = self.tables.iter().filter(|t| !exists(Object::Table(&t.qualified))).collect();
        out.extend(new_tables.iter().map(|t| t.create.clone()));
        out.extend(new_tables.iter().flat_map(|t| t.indexes.iter().cloned()));
        out.extend(new_tables.iter().flat_map(|t| t.foreign_keys.iter().cloned()));
        out
    }
}

/// A validation error for a construct `db push` cannot build.
fn refuse(m: &ModelMeta, field: &str, what: impl std::fmt::Display) -> OrmError {
    OrmError::validation(format!("`db push` cannot create `{}.{field}`: {what}. Create it with a migration instead.", m.name))
}

/// `'text'`, with quotes doubled.
fn string_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// `("a", "b")`, or `("a","b")` with an empty `sep` as the reference writes primary keys.
fn column_list<'a>(cols: impl IntoIterator<Item = &'a str>, sep: &str) -> String {
    format!("({})", cols.into_iter().map(quote_ident).collect::<Vec<_>>().join(sep))
}

/// The Postgres keyword of a `@db.*` native type, or `None` when `db push` does not know it.
fn pg_native_keyword(name: &str) -> Option<&'static str> {
    Some(match name {
        "Text" => "TEXT",
        "Char" => "CHAR",
        "VarChar" => "VARCHAR",
        "Bit" => "BIT",
        "VarBit" => "VARBIT",
        "Uuid" => "UUID",
        "Xml" => "XML",
        "Inet" => "INET",
        "Citext" => "CITEXT",
        "Boolean" => "BOOLEAN",
        "Integer" => "INTEGER",
        "SmallInt" => "SMALLINT",
        "BigInt" => "BIGINT",
        "Oid" => "OID",
        "DoublePrecision" => "DOUBLE PRECISION",
        "Real" => "REAL",
        "Decimal" => "DECIMAL",
        "Money" => "MONEY",
        "Timestamp" => "TIMESTAMP",
        "Timestamptz" => "TIMESTAMPTZ",
        "Date" => "DATE",
        "Time" => "TIME",
        "Timetz" => "TIMETZ",
        "Json" => "JSON",
        "JsonB" => "JSONB",
        "ByteA" => "BYTEA",
        _ => return None,
    })
}

/// The arguments of a native type as the reference writes them (`(20,2)`); anything but numbers is refused.
fn native_args(m: &ModelMeta, f: &FieldMeta, native: &str) -> Result<String> {
    let Some((_, rest)) = native.split_once('(') else { return Ok(String::new()) };
    let args: Vec<&str> = rest.trim_end().strip_suffix(')').unwrap_or(rest).split(',').map(str::trim).collect();
    if args.iter().any(|a| a.is_empty() || !a.bytes().all(|b| b.is_ascii_digit())) {
        return Err(refuse(m, f.name, format_args!("the arguments of the native type `@db.{native}` are not numbers")));
    }
    Ok(format!("({})", args.join(",")))
}

/// Column type of a scalar field. With `serial`, an autoincrement id becomes `SERIAL` / `BIGSERIAL` / `SMALLSERIAL`
/// on Postgres; a column that only references such an id gets the plain integer type.
fn column_type(d: Dialect, m: &ModelMeta, f: &FieldMeta, serial: bool) -> Result<String> {
    let autoincrement = serial && !f.list && f.default == Some(DefaultValue::Autoincrement);
    if d == Dialect::Sqlite {
        // The types of the reference client's SQLite migration (`tests/fixtures/oracle_sqlite/migration.sql`).
        return Ok(match (f.kind, f.list) {
            // Scalar lists are JSON text on SQLite (a Corvis extension: the reference client has no lists there).
            (_, true) => "TEXT",
            (FieldKind::Int, _) => "INTEGER",
            // `INTEGER PRIMARY KEY AUTOINCREMENT` takes no other spelling.
            (FieldKind::BigInt, _) if autoincrement => "INTEGER",
            (FieldKind::BigInt, _) => "BIGINT",
            (FieldKind::Float, _) => "REAL",
            (FieldKind::Boolean, _) => "BOOLEAN",
            (FieldKind::DateTime, _) => "DATETIME",
            // Numeric affinity, like the reference migration: values compare and sort as numbers.
            (FieldKind::Decimal, _) => "DECIMAL",
            (FieldKind::Json, _) => "JSONB",
            _ => "TEXT",
        }
        .to_string());
    }
    let base = match (f.native_type, f.native_name()) {
        (Some(native), Some(name)) => {
            let keyword = pg_native_keyword(name)
                .ok_or_else(|| refuse(m, f.name, format_args!("the native type `@db.{native}` is not one `db push` knows")))?;
            match (keyword, autoincrement) {
                ("SMALLINT", true) => "SMALLSERIAL".to_string(),
                ("INTEGER", true) => "SERIAL".to_string(),
                ("BIGINT", true) => "BIGSERIAL".to_string(),
                _ => format!("{keyword}{}", native_args(m, f, native)?),
            }
        }
        _ => match f.kind {
            FieldKind::Int if autoincrement => "SERIAL".to_string(),
            FieldKind::BigInt if autoincrement => "BIGSERIAL".to_string(),
            FieldKind::Int => "INTEGER".to_string(),
            FieldKind::BigInt => "BIGINT".to_string(),
            FieldKind::Float => "DOUBLE PRECISION".to_string(),
            FieldKind::Boolean => "BOOLEAN".to_string(),
            FieldKind::String => "TEXT".to_string(),
            FieldKind::DateTime => "TIMESTAMP(3)".to_string(),
            FieldKind::Json => "JSONB".to_string(),
            FieldKind::Decimal => "DECIMAL(65,30)".to_string(),
            FieldKind::Bytes => "BYTEA".to_string(),
            FieldKind::Enum => match f.enum_meta() {
                Some(e) => qualified(e.schema, e.db_name),
                None => return Err(refuse(m, f.name, "the enum of the field is not known")),
            },
            FieldKind::Relation => return Err(refuse(m, f.name, "it is a relation, not a column")),
        },
    };
    Ok(if f.list { format!("{base}[]") } else { base })
}

/// One literal of `@default(..)`: enum values are written as stored (`@map`).
fn literal_item(d: Dialect, m: &ModelMeta, f: &FieldMeta, v: &serde_json::Value) -> Result<String> {
    use serde_json::Value;
    Ok(match v {
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) if f.kind == FieldKind::Enum => {
            let e = f.enum_meta().ok_or_else(|| refuse(m, f.name, "the enum of the field is not known"))?;
            let db = e.to_db(s).ok_or_else(|| refuse(m, f.name, format_args!("`{s}` is not a value of the enum `{}`", e.name)))?;
            string_literal(db)
        }
        Value::String(s) => string_literal(s),
        Value::Array(_) | Value::Object(_) if d == Dialect::Sqlite && f.kind == FieldKind::Json => string_literal(&v.to_string()),
        other => return Err(refuse(m, f.name, format_args!("the default `{other}` has no SQL literal"))),
    })
}

/// The `DEFAULT` expression of a column of type `ty`, when the database computes one (`now()`, `dbgenerated(..)`,
/// literals); client-generated defaults (`uuid()`, `cuid()`, ...) have none.
fn default_sql(d: Dialect, m: &ModelMeta, f: &FieldMeta, ty: &str) -> Result<Option<String>> {
    let literal = match f.default {
        Some(DefaultValue::Now) => return Ok(Some("CURRENT_TIMESTAMP".to_string())),
        Some(DefaultValue::DbGenerated(expr)) => return Ok(Some(expr.to_string())),
        Some(DefaultValue::Literal(l)) => l,
        _ => return Ok(None),
    };
    let v: serde_json::Value =
        serde_json::from_str(literal).map_err(|e| refuse(m, f.name, format_args!("the default `{literal}` is not valid: {e}")))?;
    Ok(Some(match (&v, f.list, d) {
        (serde_json::Value::Array(items), true, Dialect::Postgres) => {
            let items = items.iter().map(|x| literal_item(d, m, f, x)).collect::<Result<Vec<_>>>()?;
            format!("ARRAY[{}]::{ty}", items.join(", "))
        }
        (serde_json::Value::Array(_), true, Dialect::Sqlite) => string_literal(&v.to_string()),
        (_, true, _) => return Err(refuse(m, f.name, format_args!("the default `{literal}` of a list must be a list"))),
        _ => literal_item(d, m, f, &v)?,
    }))
}

/// `true` when the model declares an `@id` or `@@id` (a model with only `@unique` keys has no primary key).
fn has_primary_key(m: &ModelMeta) -> bool {
    !m.primary_key.is_empty() || m.fields.iter().any(|f| f.is_id)
}

fn actions(on_delete: ReferentialAction, on_update: ReferentialAction) -> String {
    format!("ON DELETE {} ON UPDATE {}", on_delete.sql(), on_update.sql())
}

fn model_table(d: Dialect, m: &'static ModelMeta) -> Result<TableDdl> {
    let pg = d == Dialect::Postgres;
    let table = qualified(m.schema, m.table);
    let pk = m.pk();
    let sqlite_autoinc = !pg && pk.len() == 1 && pk[0].default == Some(DefaultValue::Autoincrement);
    let mut parts = Vec::new();
    for f in m.scalars() {
        let ty = column_type(d, m, f, true)?;
        let mut c = format!("{} {ty}", quote_ident(f.column));
        // Scalar lists are nullable columns, like the reference migration's.
        if !f.optional && !f.list {
            c.push_str(" NOT NULL");
        }
        if sqlite_autoinc && f.is_id {
            c.push_str(" PRIMARY KEY AUTOINCREMENT");
        }
        if let Some(default) = default_sql(d, m, f, &ty)? {
            c.push_str(" DEFAULT ");
            c.push_str(&default);
        }
        parts.push(c);
    }
    for u in m.unsupported {
        parts.push(format!("{} {}{}", quote_ident(u.column), u.db_type, if u.optional { "" } else { " NOT NULL" }));
    }
    if has_primary_key(m) && !sqlite_autoinc {
        let name = m.primary_key_map.map(str::to_string).unwrap_or_else(|| format!("{}_pkey", m.table));
        parts.push(format!("CONSTRAINT {} PRIMARY KEY {}", quote_ident(&name), column_list(pk.iter().map(|f| f.column), ",")));
    }

    let mut indexes = Vec::new();
    let unique = |name: &str, cols: &[&str]| {
        format!("CREATE UNIQUE INDEX IF NOT EXISTS {} ON {table}{}", quote_ident(name), column_list(cols.iter().copied(), ", "))
    };
    for f in m.scalars().filter(|f| f.is_unique && !f.is_id) {
        indexes.push(unique(&format!("{}_{}_key", m.table, f.column), &[f.column]));
    }
    let columns_of = |names: &[&str]| -> Result<Vec<&'static str>> {
        names.iter().map(|n| m.field_any(n).map(|f| f.column).ok_or_else(|| refuse(m, n, "the field of the index is not known"))).collect()
    };
    for u in m.uniques {
        let cols = columns_of(u.fields)?;
        let name = u.map.map(str::to_string).unwrap_or_else(|| format!("{}_{}_key", m.table, cols.join("_")));
        indexes.push(unique(&name, &cols));
    }
    for i in m.indexes {
        let cols = columns_of(i.fields)?;
        let name = i.map.map(str::to_string).unwrap_or_else(|| format!("{}_{}_idx", m.table, cols.join("_")));
        let using = match (d, i.kind) {
            // SQLite has one index type; the type is ignored there, like native types.
            (_, None | Some("BTree")) | (Dialect::Sqlite, _) => String::new(),
            (Dialect::Postgres, Some(k @ ("Hash" | "Gin" | "Gist" | "SpGist" | "Brin"))) => format!(" USING {} ", k.to_uppercase()),
            (Dialect::Postgres, Some(k)) => {
                return Err(refuse(m, &i.fields.join(", "), format_args!("the index type `{k}` is not one `db push` knows")));
            }
        };
        indexes.push(format!("CREATE INDEX IF NOT EXISTS {} ON {table}{using}{}", quote_ident(&name), column_list(cols, ", ")));
    }

    let mut foreign_keys = Vec::new();
    for f in m.relations() {
        let Some(rel) = f.relation.filter(|r| r.kind == RelationKind::BelongsTo) else { continue };
        let r = resolve_relation(m, f).map_err(OrmError::validation)?;
        let optional = r.parent_fields.iter().any(|x| x.optional);
        let on_delete = rel.on_delete.unwrap_or(if optional { ReferentialAction::SetNull } else { ReferentialAction::Restrict });
        let on_update = rel.on_update.unwrap_or(ReferentialAction::Cascade);
        let local: Vec<&str> = r.parent_fields.iter().map(|x| x.column).collect();
        let name = quote_ident(&rel.map.map(str::to_string).unwrap_or_else(|| format!("{}_{}_fkey", m.table, local.join("_"))));
        let target = qualified(r.target.schema, r.target.table);
        let foreign = column_list(r.target_fields.iter().map(|x| x.column), ", ");
        let local = column_list(local, ", ");
        let acts = actions(on_delete, on_update);
        if pg {
            foreign_keys.push(format!("ALTER TABLE {table} ADD CONSTRAINT {name} FOREIGN KEY {local} REFERENCES {target}{foreign} {acts}"));
        } else {
            parts.push(format!("CONSTRAINT {name} FOREIGN KEY {local} REFERENCES {target} {foreign} {acts}"));
        }
    }
    let create = format!("CREATE TABLE IF NOT EXISTS {table} (\n    {}\n)", parts.join(",\n    "));
    Ok(TableDdl { qualified: table, create, indexes, foreign_keys })
}

/// The join table of an implicit many-to-many relation, written from the side of its column `A` (the model whose
/// name sorts first); `None` from the other side.
fn join_table(d: Dialect, a: &'static ModelMeta, f: &'static FieldMeta) -> Result<Option<TableDdl>> {
    let r = resolve_relation(a, f).map_err(OrmError::validation)?;
    let Some(join) = r.join.as_ref().filter(|j| j.parent_column == "A") else { return Ok(None) };
    let (a_id, b, b_id) = (r.parent_fields[0], r.target, r.target_fields[0]);
    let name = &join.table;
    let table = qualified(join.schema, name);
    let cascade = actions(ReferentialAction::Cascade, ReferentialAction::Cascade);
    let fk = |col: &str, m: &ModelMeta, id: &FieldMeta| {
        let constraint = quote_ident(&format!("{name}_{col}_fkey"));
        let (target, local, foreign) = (qualified(m.schema, m.table), column_list([col], ""), column_list([id.column], ""));
        match d {
            Dialect::Postgres => {
                format!("ALTER TABLE {table} ADD CONSTRAINT {constraint} FOREIGN KEY {local} REFERENCES {target}{foreign} {cascade}")
            }
            Dialect::Sqlite => format!("CONSTRAINT {constraint} FOREIGN KEY {local} REFERENCES {target} {foreign} {cascade}"),
        }
    };
    let mut parts =
        vec![format!("\"A\" {} NOT NULL", column_type(d, a, a_id, false)?), format!("\"B\" {} NOT NULL", column_type(d, b, b_id, false)?)];
    let mut indexes = Vec::new();
    let mut foreign_keys = Vec::new();
    match d {
        Dialect::Postgres => {
            parts.push(format!("CONSTRAINT {} PRIMARY KEY (\"A\",\"B\")", quote_ident(&format!("{name}_AB_pkey"))));
            foreign_keys.push(fk("A", a, a_id));
            foreign_keys.push(fk("B", b, b_id));
        }
        Dialect::Sqlite => {
            parts.push(fk("A", a, a_id));
            parts.push(fk("B", b, b_id));
            indexes
                .push(format!("CREATE UNIQUE INDEX IF NOT EXISTS {} ON {table}(\"A\", \"B\")", quote_ident(&format!("{name}_AB_unique"))));
        }
    }
    indexes.push(format!("CREATE INDEX IF NOT EXISTS {} ON {table}(\"B\")", quote_ident(&format!("{name}_B_index"))));
    let create = format!("CREATE TABLE IF NOT EXISTS {table} (\n    {}\n)", parts.join(",\n    "));
    Ok(Some(TableDdl { qualified: table, create, indexes, foreign_keys }))
}

/// Orders models so that referenced tables are created first (SQLite writes foreign keys inline).
fn sorted(models: &[&'static ModelMeta]) -> Vec<&'static ModelMeta> {
    let mut out: Vec<&'static ModelMeta> = Vec::new();
    let mut pending: Vec<&'static ModelMeta> = models.to_vec();
    while !pending.is_empty() {
        let before = pending.len();
        pending.retain(|m| {
            let deps_ready = m.relations().all(|f| {
                let rel = f.relation.unwrap();
                let t = (rel.target)();
                rel.kind != RelationKind::BelongsTo
                    || t.name == m.name
                    || out.iter().any(|o| o.name == t.name)
                    || !models.iter().any(|x| x.name == t.name)
            });
            if deps_ready {
                out.push(m);
            }
            !deps_ready
        });
        if pending.len() == before {
            // Cycle: keep declaration order (SQLite checks foreign keys when rows are written, not here).
            out.append(&mut pending);
        }
    }
    out
}

/// The models of `models` that are tables, each once (views are not created, as the reference's push skips them).
fn tables_of(models: &[&'static ModelMeta]) -> Vec<&'static ModelMeta> {
    let mut seen = HashSet::new();
    models.iter().copied().filter(|m| !m.is_view && seen.insert(m.name)).collect()
}

/// The implicit many-to-many join tables between two models of `models`.
fn join_tables(d: Dialect, models: &[&'static ModelMeta]) -> Result<Vec<TableDdl>> {
    let names: HashSet<&str> = models.iter().map(|m| m.name).collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for m in models {
        for f in m.relations() {
            let Some(rel) = f.relation.filter(|r| r.kind == RelationKind::ManyToMany) else { continue };
            if !names.contains((rel.target)().name) {
                continue;
            }
            if let Some(t) = join_table(d, m, f)?
                && seen.insert(t.qualified.clone())
            {
                out.push(t);
            }
        }
    }
    Ok(out)
}

/// What `db push` creates for `models`, or a validation error naming the first construct it cannot build.
pub(crate) fn push_plan(d: Dialect, models: &[&'static ModelMeta]) -> Result<PushPlan> {
    let models = tables_of(models);
    let ordered = if d == Dialect::Sqlite { sorted(&models) } else { models.clone() };
    let mut tables = ordered.into_iter().map(|m| model_table(d, m)).collect::<Result<Vec<_>>>()?;
    tables.extend(join_tables(d, &models)?);
    let mut schemas: Vec<String> = Vec::new();
    let mut enums: Vec<EnumDdl> = Vec::new();
    if d == Dialect::Postgres {
        let mut add_schema = |s: Option<&str>| {
            if let Some(s) = s.filter(|s| !schemas.iter().any(|x| x == s)) {
                schemas.push(s.to_string());
            }
        };
        for m in &models {
            add_schema(m.schema);
            for e in m.scalars().filter_map(|f| f.enum_meta()) {
                add_schema(e.schema);
                let name = qualified(e.schema, e.db_name);
                if !enums.iter().any(|x| x.qualified == name) {
                    let values = e.values.iter().map(|v| string_literal(v.db_name)).collect::<Vec<_>>().join(", ");
                    enums.push(EnumDdl { create: format!("CREATE TYPE {name} AS ENUM ({values})"), qualified: name });
                }
            }
        }
    }
    Ok(PushPlan { dialect: d, schemas, enums, tables })
}

/// `DROP TABLE IF EXISTS` statements: join tables, then dependents first. Enum types stay, other tables may use them.
pub(crate) fn drop_statements(d: Dialect, models: &[&'static ModelMeta]) -> Result<Vec<String>> {
    let models = tables_of(models);
    let mut tables: Vec<String> = join_tables(d, &models)?.into_iter().map(|t| t.qualified).collect();
    tables.extend(sorted(&models).into_iter().rev().map(|m| qualified(m.schema, m.table)));
    Ok(tables
        .into_iter()
        .map(|t| match d {
            Dialect::Sqlite => format!("DROP TABLE IF EXISTS {t}"),
            Dialect::Postgres => format!("DROP TABLE IF EXISTS {t} CASCADE"),
        })
        .collect())
}
