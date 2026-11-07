//! Typed raw SQL: the `sql`, `join`, `raw` and `empty` helpers of tagged-template SQL,
//! used by `db._query_raw(...)` / `db._execute_raw(...)`
//! (`$queryRaw` / `$executeRaw` with tagged templates).
//!
//! ```ignore
//! use corvis_orm::{sql, RawSql};
//! let rows = tx._query_raw(sql!(
//!     "SELECT * FROM \"Wallet\" WHERE \"merchantId\" = {merchant_id} AND \"currency\" = {currency}::\"Currency\" LIMIT 1 FOR UPDATE"
//! )).await?;
//! let ids = RawSql::join([1, 2, 3], ",", "", "");
//! let n = db._execute_raw(sql!("UPDATE t SET hits = hits + 1 WHERE id IN ({ids})")).await?;
//! ```
//!
//! Values are always sent as bound parameters (never spliced into the text), with the
//! types of the JavaScript clients: integers as `int8`, other numbers as `numeric`, strings as `text`,
//! `Decimal` as `numeric`, dates as `timestamptz`, `serde_json::Value` objects and
//! arrays as `jsonb`, `Vec<T>` as Postgres arrays. Add casts in the SQL when the
//! column needs another type (`{id}::uuid`, `{kind}::"Kind"`).

use serde_json::Value;

use crate::db::{Dialect, Sql, SqlValue};
use crate::meta::FieldKind;

/// One interpolated value of a [`RawSql`]: a bound parameter or a nested fragment.
#[derive(Debug, Clone, PartialEq)]
pub enum RawArg {
    Value(SqlValue),
    Sql(RawSql),
}

/// A SQL statement with bound values. Build it with [`sql!`](crate::sql),
/// [`RawSql::new`], [`RawSql::join`], [`RawSql::raw`] or [`RawSql::empty`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RawSql {
    /// The literal parts; always `values.len() + 1` of them.
    strings: Vec<String>,
    values: Vec<SqlValue>,
}

impl RawSql {
    /// `sql(strings, ...values)`: `strings` must have one more element than `values`.
    /// Nested [`RawSql`] values are inlined.
    pub fn new<S: Into<String>>(strings: Vec<S>, values: Vec<RawArg>) -> Self {
        let strings: Vec<String> = strings.into_iter().map(Into::into).collect();
        assert_eq!(strings.len(), values.len() + 1, "RawSql::new: expected one more string than values");
        let mut out = RawSql { strings: vec![String::new()], values: Vec::new() };
        let mut strings = strings.into_iter();
        out.push_text(&strings.next().unwrap_or_default());
        for (v, s) in values.into_iter().zip(strings) {
            match v {
                RawArg::Value(v) => {
                    out.values.push(v);
                    out.strings.push(String::new());
                }
                RawArg::Sql(inner) => out.append(inner),
            }
            out.push_text(&s);
        }
        out
    }

    /// `raw(text)`: literal SQL without parameters. Never use it with user input.
    pub fn raw(text: impl Into<String>) -> Self {
        RawSql { strings: vec![text.into()], values: Vec::new() }
    }

    /// `empty`
    pub fn empty() -> Self {
        RawSql { strings: vec![String::new()], values: Vec::new() }
    }

    /// `join(values, separator = ",", prefix = "", suffix = "")`.
    ///
    /// # Panics
    /// When `values` is empty ("Expected `join([])` to be called with an array of multiple elements").
    pub fn join<I>(values: I, separator: &str, prefix: &str, suffix: &str) -> Self
    where
        I: IntoIterator,
        I::Item: ToSqlArg,
    {
        let args: Vec<RawArg> = values.into_iter().map(|v| v.to_sql_arg()).collect();
        assert!(!args.is_empty(), "Expected `join([])` to be called with an array of multiple elements, but got an empty array");
        let mut strings = vec![prefix.to_string()];
        strings.extend(std::iter::repeat_n(separator.to_string(), args.len() - 1));
        strings.push(suffix.to_string());
        RawSql::new(strings, args)
    }

    fn push_text(&mut self, s: &str) {
        if let Some(last) = self.strings.last_mut() {
            last.push_str(s);
        }
    }

    fn append(&mut self, other: RawSql) {
        let mut parts = other.strings.into_iter();
        self.push_text(&parts.next().unwrap_or_default());
        for (v, s) in other.values.into_iter().zip(parts) {
            self.values.push(v);
            self.strings.push(s);
        }
    }

    /// The literal parts.
    pub fn strings(&self) -> &[String] {
        &self.strings
    }

    /// The bound values.
    pub fn values(&self) -> &[SqlValue] {
        &self.values
    }

    /// `sql.text`: the statement with Postgres placeholders (`$1`, `$2`, ...).
    pub fn text(&self) -> String {
        self.render(Dialect::Postgres)
    }

    /// `sql.sql`: the statement with `?` placeholders (SQLite / MySQL).
    pub fn sql(&self) -> String {
        self.render(Dialect::Sqlite)
    }

    fn render(&self, dialect: Dialect) -> String {
        let mut out = String::new();
        for (i, s) in self.strings.iter().enumerate() {
            out.push_str(s);
            if i < self.values.len() {
                match dialect {
                    Dialect::Postgres => out.push_str(&format!("${}", i + 1)),
                    Dialect::Sqlite => out.push('?'),
                }
            }
        }
        out
    }

    /// The statement for `dialect`, with its parameters.
    pub(crate) fn to_sql(&self, dialect: Dialect) -> Sql {
        let mut q = Sql::new(dialect);
        q.push(&self.render(dialect));
        q.params = self.values.iter().cloned().map(|v| raw_param(v, dialect)).collect();
        q
    }
}

/// Non-integer numbers of raw queries are sent as `numeric` on Postgres.
pub(crate) fn raw_param(v: SqlValue, dialect: Dialect) -> SqlValue {
    match v {
        SqlValue::Float(f) if dialect == Dialect::Postgres && f.is_finite() => SqlValue::Decimal(crate::Decimal::from(f)),
        other => other,
    }
}

/// A value that can be interpolated in [`sql!`](crate::sql) / [`RawSql::join`].
pub trait ToSqlArg {
    fn to_sql_arg(&self) -> RawArg;
    /// The type of a `NULL` of this type (`Option::<T>::None`).
    fn null_kind() -> FieldKind
    where
        Self: Sized,
    {
        FieldKind::String
    }
}

macro_rules! arg_impl {
    ($($t:ty => $kind:ident, |$v:ident| $e:expr;)*) => {
        $(
            impl ToSqlArg for $t {
                fn to_sql_arg(&self) -> RawArg {
                    let $v = self;
                    RawArg::Value($e)
                }
                fn null_kind() -> FieldKind {
                    FieldKind::$kind
                }
            }
        )*
    };
}

arg_impl! {
    i8 => BigInt, |v| SqlValue::Int(*v as i64);
    i16 => BigInt, |v| SqlValue::Int(*v as i64);
    i32 => BigInt, |v| SqlValue::Int(*v as i64);
    i64 => BigInt, |v| SqlValue::Int(*v);
    u8 => BigInt, |v| SqlValue::Int(*v as i64);
    u16 => BigInt, |v| SqlValue::Int(*v as i64);
    u32 => BigInt, |v| SqlValue::Int(*v as i64);
    // Beyond i64 the value is bound as `numeric`, never wrapped to a negative number.
    u64 => BigInt, |v| i64::try_from(*v).map(SqlValue::Int).unwrap_or_else(|_| SqlValue::Decimal(crate::Decimal::from(*v)));
    usize => BigInt, |v| i64::try_from(*v).map(SqlValue::Int).unwrap_or_else(|_| SqlValue::Decimal(crate::Decimal::from(*v)));
    isize => BigInt, |v| SqlValue::Int(*v as i64);
    f32 => Float, |v| SqlValue::Float(*v as f64);
    f64 => Float, |v| SqlValue::Float(*v);
    bool => Boolean, |v| SqlValue::Bool(*v);
    String => String, |v| SqlValue::Text(v.clone());
    crate::Decimal => Decimal, |v| SqlValue::Decimal(v.clone());
    crate::DateTime => DateTime, |v| SqlValue::DateTime(v.inner());
    chrono::DateTime<chrono::Utc> => DateTime, |v| SqlValue::DateTime(*v);
    chrono::NaiveDateTime => DateTime, |v| SqlValue::NaiveDateTime(*v);
    chrono::NaiveDate => DateTime, |v| SqlValue::Date(*v);
    chrono::NaiveTime => DateTime, |v| SqlValue::Time(*v);
    uuid::Uuid => String, |v| SqlValue::Uuid(*v);
}

impl ToSqlArg for str {
    fn to_sql_arg(&self) -> RawArg {
        RawArg::Value(SqlValue::Text(self.to_string()))
    }
}

impl ToSqlArg for Value {
    /// Strings, numbers, booleans and `null` are bound as such; objects and arrays as `jsonb`.
    fn to_sql_arg(&self) -> RawArg {
        RawArg::Value(match self {
            Value::Object(_) | Value::Array(_) => SqlValue::Json(self.clone()),
            other => SqlValue::from_json(other),
        })
    }
    fn null_kind() -> FieldKind {
        FieldKind::Json
    }
}

impl ToSqlArg for RawSql {
    fn to_sql_arg(&self) -> RawArg {
        RawArg::Sql(self.clone())
    }
}

impl<T: ToSqlArg + ?Sized> ToSqlArg for &T {
    fn to_sql_arg(&self) -> RawArg {
        (**self).to_sql_arg()
    }
}

impl<T: ToSqlArg> ToSqlArg for Option<T> {
    fn to_sql_arg(&self) -> RawArg {
        match self {
            Some(v) => v.to_sql_arg(),
            None => RawArg::Value(SqlValue::Null(T::null_kind())),
        }
    }
    fn null_kind() -> FieldKind {
        T::null_kind()
    }
}

fn array_arg<T: ToSqlArg>(items: &[T]) -> RawArg {
    let values = items
        .iter()
        .map(|x| match x.to_sql_arg() {
            RawArg::Value(v) => v,
            RawArg::Sql(s) => SqlValue::Text(s.text()),
        })
        .collect();
    RawArg::Value(SqlValue::Array(values, T::null_kind()))
}

/// A Postgres array parameter (`= ANY({ids})`).
impl<T: ToSqlArg> ToSqlArg for Vec<T> {
    fn to_sql_arg(&self) -> RawArg {
        array_arg(self)
    }
}

impl<T: ToSqlArg> ToSqlArg for [T] {
    fn to_sql_arg(&self) -> RawArg {
        array_arg(self)
    }
}

/// `Bytes` / `Buffer` parameter (`bytea`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bytes(pub Vec<u8>);

impl ToSqlArg for Bytes {
    fn to_sql_arg(&self) -> RawArg {
        RawArg::Value(SqlValue::Bytes(self.0.clone()))
    }
    fn null_kind() -> FieldKind {
        FieldKind::Bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composes_like_tagged_sql() {
        let ids = RawSql::join([1, 2, 3], ",", "(", ")");
        let cond = RawSql::new(vec!["\"status\" = ", ""], vec!["open".to_sql_arg()]);
        let q = RawSql::new(vec!["SELECT * FROM t WHERE id IN ", " AND ", "", " LIMIT ", ""], vec![
            ids.to_sql_arg(),
            cond.to_sql_arg(),
            RawSql::empty().to_sql_arg(),
            10.to_sql_arg(),
        ]);
        assert_eq!(q.text(), "SELECT * FROM t WHERE id IN ($1,$2,$3) AND \"status\" = $4 LIMIT $5");
        assert_eq!(q.sql(), "SELECT * FROM t WHERE id IN (?,?,?) AND \"status\" = ? LIMIT ?");
        assert_eq!(q.values().len(), 5);
        assert_eq!(q.values()[3], SqlValue::Text("open".into()));
        assert_eq!(RawSql::raw("NOW()").text(), "NOW()");
        let none: Option<i32> = None;
        assert_eq!(none.to_sql_arg(), RawArg::Value(SqlValue::Null(FieldKind::BigInt)));
    }
}
