//! DDL generation for `db push`.

use crate::db::{Dialect, quote_ident};
use crate::error::{OrmError, Result};
use crate::meta::{DefaultValue, FieldKind, FieldMeta, ModelMeta, ReferentialAction, RelationKind, resolve_relation};

fn column_type(d: Dialect, f: &FieldMeta) -> String {
    if d == Dialect::Postgres && f.kind == FieldKind::Decimal {
        // Like the reference migrations: `@db.Decimal(p, s)` or `DECIMAL(65,30)` (a `TEXT`
        // column would compare as text and fail against `numeric` parameters).
        return match f.native_type.filter(|_| f.native_name() == Some("Decimal")) {
            Some(t) => format!("DECIMAL{}", t.trim_start_matches("Decimal").replace(' ', "")),
            None => "DECIMAL(65,30)".to_string(),
        };
    }
    column_type_base(d, f).to_string()
}

fn column_type_base(d: Dialect, f: &FieldMeta) -> &'static str {
    match (d, f.kind) {
        (Dialect::Sqlite, FieldKind::Int | FieldKind::BigInt) => "INTEGER",
        (Dialect::Sqlite, FieldKind::Float) => "REAL",
        (Dialect::Sqlite, FieldKind::Boolean) => "BOOLEAN",
        (Dialect::Sqlite, FieldKind::DateTime) => "DATETIME",
        (Dialect::Sqlite, FieldKind::Json) => "TEXT",
        // Numeric affinity, like the reference migration: values compare and sort as numbers.
        (Dialect::Sqlite, FieldKind::Decimal) if !f.list => "DECIMAL",
        (Dialect::Sqlite, _) => "TEXT",
        (Dialect::Postgres, FieldKind::Int) => {
            if f.default == Some(DefaultValue::Autoincrement) {
                "SERIAL"
            } else {
                "INTEGER"
            }
        }
        (Dialect::Postgres, FieldKind::BigInt) => {
            if f.default == Some(DefaultValue::Autoincrement) {
                "BIGSERIAL"
            } else {
                "BIGINT"
            }
        }
        (Dialect::Postgres, FieldKind::Float) => "DOUBLE PRECISION",
        (Dialect::Postgres, FieldKind::Boolean) => "BOOLEAN",
        (Dialect::Postgres, FieldKind::DateTime) => "TIMESTAMPTZ(3)",
        (Dialect::Postgres, FieldKind::Json) => "JSONB",
        (Dialect::Postgres, _) => "TEXT",
    }
}

fn literal_sql(json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    Some(match v {
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => format!("'{}'", s.replace('\'', "''")),
        _ => return None,
    })
}

fn create_table(d: Dialect, m: &'static ModelMeta) -> Result<String> {
    let mut cols = Vec::new();
    let id = m.id();
    let sqlite_autoinc = d == Dialect::Sqlite && id.default == Some(DefaultValue::Autoincrement);
    for f in m.scalars() {
        let mut c = format!("{} {}", quote_ident(f.column), column_type(d, f));
        if !f.optional {
            c.push_str(" NOT NULL");
        }
        if f.is_id && sqlite_autoinc {
            c.push_str(" PRIMARY KEY AUTOINCREMENT");
        }
        if let Some(DefaultValue::Literal(l)) = f.default
            && let Some(sql) = literal_sql(l)
        {
            c.push_str(&format!(" DEFAULT {sql}"));
        }
        cols.push(c);
    }
    if !sqlite_autoinc {
        cols.push(format!("CONSTRAINT {} PRIMARY KEY ({})", quote_ident(&format!("{}_pkey", m.table)), quote_ident(id.column)));
    }
    for f in m.relations() {
        let rel = f.relation.as_ref().unwrap();
        if rel.kind != RelationKind::BelongsTo {
            continue;
        }
        let r = resolve_relation(m, f).map_err(OrmError::validation)?;
        let local: Vec<String> = r.parent_fields.iter().map(|x| quote_ident(x.column)).collect();
        let foreign: Vec<String> = r.target_fields.iter().map(|x| quote_ident(x.column)).collect();
        let optional = r.parent_fields.iter().any(|x| x.optional);
        let on_delete = rel.on_delete.unwrap_or(if optional { ReferentialAction::SetNull } else { ReferentialAction::Restrict });
        let on_update = rel.on_update.unwrap_or(ReferentialAction::Cascade);
        let name = format!("{}_{}_fkey", m.table, r.parent_fields.iter().map(|x| x.column).collect::<Vec<_>>().join("_"));
        cols.push(format!(
            "CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({}) ON DELETE {} ON UPDATE {}",
            quote_ident(&name),
            local.join(", "),
            quote_ident(r.target.table),
            foreign.join(", "),
            on_delete.sql(),
            on_update.sql()
        ));
    }
    Ok(format!("CREATE TABLE IF NOT EXISTS {} (\n    {}\n)", quote_ident(m.table), cols.join(",\n    ")))
}

/// Orders models so that referenced tables are created first.
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
            // Cycle: keep declaration order.
            out.append(&mut pending);
        }
    }
    out
}

/// `CREATE TABLE IF NOT EXISTS` + `CREATE UNIQUE INDEX IF NOT EXISTS` statements.
pub(crate) fn push_statements(d: Dialect, models: &[&'static ModelMeta]) -> Result<Vec<String>> {
    let mut stmts = Vec::new();
    for m in sorted(models) {
        stmts.push(create_table(d, m)?);
        for f in m.scalars().filter(|f| f.is_unique && !f.is_id) {
            stmts.push(format!(
                "CREATE UNIQUE INDEX IF NOT EXISTS {} ON {}({})",
                quote_ident(&format!("{}_{}_key", m.table, f.column)),
                quote_ident(m.table),
                quote_ident(f.column)
            ));
        }
    }
    Ok(stmts)
}

/// `DROP TABLE IF EXISTS` statements (dependents first).
pub(crate) fn drop_statements(d: Dialect, models: &[&'static ModelMeta]) -> Result<Vec<String>> {
    let mut ordered = sorted(models);
    ordered.reverse();
    Ok(ordered
        .into_iter()
        .map(|m| match d {
            Dialect::Sqlite => format!("DROP TABLE IF EXISTS {}", quote_ident(m.table)),
            Dialect::Postgres => format!("DROP TABLE IF EXISTS {} CASCADE", quote_ident(m.table)),
        })
        .collect())
}
