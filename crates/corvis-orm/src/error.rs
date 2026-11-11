//! Client errors: `KnownRequestError` (with `P2xxx` codes),
//! `QueryValidationError`, `InitializationError` and
//! `UnknownRequestError`.

use std::fmt;

use serde_json::{Value, json};

use crate::meta::ModelMeta;

/// Version reported in errors (`clientVersion`).
pub const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// A known request error (`KnownRequestError`): `{ code, message, meta }`.
#[derive(Debug, Clone)]
pub struct KnownRequestError {
    pub code: String,
    pub message: String,
    pub meta: Value,
    pub client_version: &'static str,
}

/// Invalid arguments (`QueryValidationError`).
#[derive(Debug, Clone)]
pub struct QueryValidationError {
    pub message: String,
}

/// Connection / configuration problem (`InitializationError`).
#[derive(Debug, Clone)]
pub struct InitializationError {
    pub message: String,
    pub error_code: Option<String>,
}

/// Any other database error (`UnknownRequestError`).
#[derive(Debug, Clone)]
pub struct UnknownRequestError {
    pub message: String,
}

/// All the errors produced by corvis-orm.
#[derive(Debug, Clone)]
pub enum OrmError {
    KnownRequest(KnownRequestError),
    Validation(QueryValidationError),
    Initialization(InitializationError),
    UnknownRequest(UnknownRequestError),
}

pub type Result<T, E = OrmError> = std::result::Result<T, E>;

impl OrmError {
    pub fn known(code: &str, message: impl Into<String>, meta: Value) -> Self {
        OrmError::KnownRequest(KnownRequestError { code: code.to_string(), message: message.into(), meta, client_version: CLIENT_VERSION })
    }
    pub fn validation(message: impl Into<String>) -> Self {
        OrmError::Validation(QueryValidationError { message: message.into() })
    }
    pub fn init(message: impl Into<String>) -> Self {
        OrmError::Initialization(InitializationError { message: message.into(), error_code: None })
    }
    pub fn unknown(message: impl Into<String>) -> Self {
        OrmError::UnknownRequest(UnknownRequestError { message: message.into() })
    }

    /// `P2025` – record required but not found.
    pub fn not_found(model: &ModelMeta, cause: &str) -> Self {
        Self::known(
            "P2025",
            format!("An operation failed because it depends on one or more records that were required but not found. {cause}"),
            json!({ "modelName": model.name, "cause": cause }),
        )
    }

    /// The error code (`P2002`, `P2025`, ...) of a known request error.
    pub fn code(&self) -> Option<&str> {
        match self {
            OrmError::KnownRequest(e) => Some(&e.code),
            OrmError::Initialization(e) => e.error_code.as_deref(),
            _ => None,
        }
    }
    pub fn message(&self) -> &str {
        match self {
            OrmError::KnownRequest(e) => &e.message,
            OrmError::Validation(e) => &e.message,
            OrmError::Initialization(e) => &e.message,
            OrmError::UnknownRequest(e) => &e.message,
        }
    }
    pub fn meta(&self) -> Option<&Value> {
        match self {
            OrmError::KnownRequest(e) => Some(&e.meta),
            _ => None,
        }
    }
    /// The name of the error kind (`"KnownRequestError"`, `"QueryValidationError"`, ...).
    pub fn name(&self) -> &'static str {
        match self {
            OrmError::KnownRequest(_) => "KnownRequestError",
            OrmError::Validation(_) => "QueryValidationError",
            OrmError::Initialization(_) => "InitializationError",
            OrmError::UnknownRequest(_) => "UnknownRequestError",
        }
    }
    pub fn is_validation(&self) -> bool {
        matches!(self, OrmError::Validation(_))
    }
    /// `P2025`
    pub fn is_not_found(&self) -> bool {
        self.code() == Some("P2025")
    }
    /// `P2002`
    pub fn is_unique_violation(&self) -> bool {
        self.code() == Some("P2002")
    }
    /// `P2003`
    pub fn is_foreign_key_violation(&self) -> bool {
        self.code() == Some("P2003")
    }

    /// Maps an sqlx error of a model query into an [`OrmError`], using the model to translate
    /// column names. Database errors without a `P2xxx` code become an
    /// `UnknownRequestError` whose message carries the SQLSTATE, like the
    /// reference query engine's `ConnectorError(...)` text.
    pub(crate) fn from_sqlx(err: sqlx::Error, model: Option<&ModelMeta>) -> Self {
        let sqlx::Error::Database(db) = &err else {
            return Self::from_sqlx_other(err);
        };
        let msg = db.message().to_string();
        let pg = db.try_downcast_ref::<sqlx::postgres::PgDatabaseError>();
        if pg.is_none()
            && let Some(e) = sqlite_range_error(&msg)
        {
            return e;
        }
        let code = db.code().map(|c| c.to_string()).unwrap_or_default();
        let model_name = model.map(|m| m.name);
        let columns_to_fields = |cols: Vec<String>| -> Vec<String> {
            cols.into_iter().map(|c| model.and_then(|m| m.field_by_column(&c)).map(|f| f.name.to_string()).unwrap_or(c)).collect()
        };
        match db.kind() {
            sqlx::error::ErrorKind::UniqueViolation => {
                let detail = pg.and_then(|e| e.detail()).map(|d| d.to_string());
                let fields = columns_to_fields(unique_columns(&msg, detail.as_deref(), db.constraint(), model.map(|m| m.table)));
                let list = fields.iter().map(|f| format!("`{f}`")).collect::<Vec<_>>().join(",");
                Self::known(
                    "P2002",
                    format!("Unique constraint failed on the fields: ({list})"),
                    json!({ "modelName": model_name, "target": fields }),
                )
            }
            sqlx::error::ErrorKind::ForeignKeyViolation => {
                // SQLite does not name the constraint; the reference client then reports `constraint: null` there.
                let (text, constraint) = match db.constraint() {
                    Some(c) => (format!("constraint: `{c}`"), json!(c)),
                    None if pg.is_none() => ("foreign key".to_string(), Value::Null),
                    None => ("foreign key".to_string(), json!("foreign key")),
                };
                Self::known(
                    "P2003",
                    format!("Foreign key constraint violated on the {text}"),
                    json!({ "modelName": model_name, "constraint": constraint }),
                )
            }
            sqlx::error::ErrorKind::NotNullViolation => {
                let cols = columns_to_fields(not_null_columns(&msg));
                // Recorded on SQLite: "(`views`)".
                let c = if pg.is_none() { cols.iter().map(|c| format!("`{c}`")).collect::<Vec<_>>().join(", ") } else { cols.join(", ") };
                Self::known(
                    "P2011",
                    format!("Null constraint violation on the fields: ({c})"),
                    json!({ "modelName": model_name, "constraint": cols }),
                )
            }
            sqlx::error::ErrorKind::CheckViolation if pg.is_none() => Self::known(
                "P2004",
                format!("A constraint failed on the database: `{msg}`"),
                json!({ "modelName": model_name, "database_error": msg }),
            ),
            _ if code == "22001" => Self::known(
                "P2000",
                "The provided value for the column is too long for the column's type. Column: (not available)",
                json!({ "modelName": model_name, "column_name": "(not available)" }),
            ),
            _ if code == "40001" => Self::known(
                "P2034",
                "Transaction failed due to a write conflict or a deadlock. Please retry your transaction",
                json!({ "modelName": model_name }),
            ),
            _ if code == "42P01" || msg.contains("no such table") => {
                let table = model.map(|m| m.table.to_string()).unwrap_or_else(|| quoted_name(&msg).unwrap_or_default());
                Self::known(
                    "P2021",
                    format!("The table `{table}` does not exist in the current database."),
                    json!({ "modelName": model_name, "table": table }),
                )
            }
            _ if code == "42703" || msg.contains("no such column") => {
                let column =
                    quoted_name(&msg).or_else(|| msg.split("no such column:").nth(1).map(|c| c.trim().to_string())).unwrap_or_default();
                Self::known(
                    "P2022",
                    format!("The column `{column}` does not exist in the current database."),
                    json!({ "modelName": model_name, "column": column }),
                )
            }
            _ => {
                let kind = match pg {
                    Some(e) => format!(
                        "PostgresError {{ code: {:?}, message: {:?}, severity: {:?}, detail: {:?}, column: {:?}, hint: {:?} }}",
                        e.code(),
                        e.message(),
                        pg_severity(e.severity()),
                        e.detail(),
                        e.column(),
                        e.hint()
                    ),
                    None => format!("Error {{ code: {code:?}, message: {msg:?} }}"),
                };
                Self::unknown(format!(
                    "Error occurred during query execution:\nConnectorError(ConnectorError {{ user_facing_error: None, kind: QueryError({kind}), transient: false }})"
                ))
            }
        }
    }

    /// Maps an sqlx error of `$queryRaw` / `$executeRaw`: every database error is a
    /// `P2010` ("Raw query failed") with the SQLSTATE and the database message.
    pub(crate) fn from_sqlx_raw(err: sqlx::Error) -> Self {
        let sqlx::Error::Database(db) = &err else {
            return Self::from_sqlx_other(err);
        };
        let pg = db.try_downcast_ref::<sqlx::postgres::PgDatabaseError>();
        let code = db.code().map(|c| c.to_string()).unwrap_or_else(|| "N/A".to_string());
        // The reference query engine reports the detail for unique / not-null violations, the bare
        // message for the errors it recognizes, and the full driver text for the rest.
        let message = match (pg, code.as_str()) {
            (Some(e), "23505" | "23502") => e.detail().unwrap_or(e.message()).to_string(),
            (Some(e), "22001" | "23503" | "3D000" | "28000" | "28P01" | "40001" | "42P01" | "42703" | "42P04" | "53300") => {
                e.message().to_string()
            }
            (Some(e), _) => {
                let mut m = format!("{}: {}", pg_severity(e.severity()), e.message());
                if let Some(d) = e.detail() {
                    m.push_str(&format!("\nDETAIL: {d}"));
                }
                if let Some(h) = e.hint() {
                    m.push_str(&format!("\nHINT: {h}"));
                }
                m
            }
            (None, _) => db.message().to_string(),
        };
        Self::known("P2010", format!("Raw query failed. Code: `{code}`. Message: `{message}`"), json!({ "code": code, "message": message }))
    }

    /// The error Postgres gives for a value out of the range of its column (SQLSTATE 22003), worded like the reference
    /// query engine reports it.
    fn out_of_range(message: &str, detail: Option<String>) -> Self {
        Self::unknown(format!(
            "Error occurred during query execution:\nConnectorError(ConnectorError {{ user_facing_error: None, kind: QueryError(PostgresError {{ code: \"22003\", message: {message:?}, severity: \"ERROR\", detail: {detail:?}, column: None, hint: None }}), transient: false }})"
        ))
    }

    /// The error raised when a query has an `Int` argument outside the 32-bit range
    /// (parameter `index`, 0-based), worded like the reference query engine.
    pub(crate) fn int_overflow(index: usize, value: i64) -> Self {
        Self::unknown(format!(
            "Error occurred during query execution:\nConnectorError(ConnectorError {{ user_facing_error: None, kind: QueryError(Error {{ kind: ToSql({index}), cause: Some(Error {{ kind: ConversionError(\"Unable to fit integer value '{value}' into an INT4 (32-bit signed integer).\"), original_code: None, original_message: None }}) }}), transient: false }})"
        ))
    }

    /// A parameter that cannot be converted to its database type (parameter `index`, 0-based),
    /// worded like the reference query engine.
    pub(crate) fn conversion(index: usize, cause: &str) -> Self {
        Self::unknown(format!(
            "Error occurred during query execution:
ConnectorError(ConnectorError {{ user_facing_error: None, kind: QueryError(Error {{ kind: ToSql({index}), cause: Some(Error {{ kind: ConversionError({cause:?}), original_code: None, original_message: None }}) }}), transient: false }})"
        ))
    }

    fn from_sqlx_other(err: sqlx::Error) -> Self {
        match err {
            sqlx::Error::PoolTimedOut => Self::known(
                "P2024",
                "Timed out fetching a new connection from the connection pool. More info: http://pris.ly/d/connection-pool",
                json!({}),
            ),
            sqlx::Error::Io(_) | sqlx::Error::Tls(_) | sqlx::Error::Configuration(_) | sqlx::Error::PoolClosed => {
                OrmError::Initialization(InitializationError {
                    message: format!("Can't reach database server: {err}"),
                    error_code: Some("P1001".into()),
                })
            }
            other => Self::unknown(other.to_string()),
        }
    }
}

fn pg_severity(s: sqlx::postgres::PgSeverity) -> &'static str {
    use sqlx::postgres::PgSeverity::*;
    match s {
        Panic => "PANIC",
        Fatal => "FATAL",
        Error => "ERROR",
        Warning => "WARNING",
        Notice => "NOTICE",
        Debug => "DEBUG",
        Info => "INFO",
        Log => "LOG",
    }
}

/// The text an SQLite arithmetic update puts in the JSON path error it raises for a result out of the range of `f`'s
/// column (`Engine::checked_arith`); [`sqlite_range_error`] reads it back from the error message. Made of letters,
/// digits and punctuation only, so it can be written as an SQL string literal.
pub(crate) fn sqlite_range_path(f: &crate::meta::FieldMeta) -> String {
    match f.kind {
        crate::meta::FieldKind::Decimal => {
            let (p, s) = crate::engine::decimal_precision(f);
            format!("{SQLITE_RANGE}decimal:{p}:{s}")
        }
        _ => format!("{SQLITE_RANGE}int"),
    }
}

const SQLITE_RANGE: &str = "corvis_orm:out_of_range:";

/// The error of an SQLite message raised by [`sqlite_range_path`]: the one Postgres gives for the same update.
fn sqlite_range_error(msg: &str) -> Option<OrmError> {
    let rest = &msg[msg.find(SQLITE_RANGE)? + SQLITE_RANGE.len()..];
    if rest.starts_with("int") {
        return Some(OrmError::out_of_range("integer out of range", None));
    }
    let mut it = rest.strip_prefix("decimal:")?.split(|c: char| !c.is_ascii_digit()).map(str::parse::<u32>);
    let (p, s) = (it.next()?.ok()?, it.next()?.ok()?);
    let detail = format!("A field with precision {p}, scale {s} must round to an absolute value less than 10^{}.", p.saturating_sub(s));
    Some(OrmError::out_of_range("numeric field overflow", Some(detail)))
}

/// The first `"quoted"` name of a database message (`relation "x" does not exist`).
fn quoted_name(msg: &str) -> Option<String> {
    let start = msg.find('"')?;
    let rest = &msg[start + 1..];
    rest.find('"').map(|end| rest[..end].to_string())
}

/// `UNIQUE constraint failed: User.email, User.name` (SQLite) or
/// `Key (email)=(a@b.c) already exists.` (Postgres detail is not in the message;
/// we fall back to the constraint name `User_email_key`).
fn unique_columns(msg: &str, detail: Option<&str>, constraint: Option<&str>, table: Option<&str>) -> Vec<String> {
    if let Some(d) = detail
        && let Some(start) = d.find("Key (")
        && let Some(end) = d[start..].find(")=")
    {
        return d[start + 5..start + end].split(',').map(|s| s.trim().trim_matches('"').to_string()).collect();
    }
    if let Some(rest) = msg.split("failed:").nth(1) {
        return rest
            .split(',')
            .map(|s| s.trim().rsplit('.').next().unwrap_or("").trim_matches('"').to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
    if let Some(c) = constraint {
        // `<Table>_<col>_key`, `<Table>_<a>_<b>_key` or `<Table>_pkey`.
        let c = c.trim_end_matches("_key").trim_end_matches("_pkey");
        if let Some(t) = table
            && let Some(cols) = c.strip_prefix(&format!("{t}_"))
        {
            return vec![cols.to_string()];
        }
        if let Some((_, cols)) = c.split_once('_') {
            return vec![cols.to_string()];
        }
        return vec![c.to_string()];
    }
    vec![]
}

fn not_null_columns(msg: &str) -> Vec<String> {
    if let Some(rest) = msg.split("failed:").nth(1) {
        return rest.split(',').map(|s| s.trim().rsplit('.').next().unwrap_or("").to_string()).collect();
    }
    if let Some(start) = msg.find("column \"") {
        let rest = &msg[start + 8..];
        if let Some(end) = rest.find('"') {
            return vec![rest[..end].to_string()];
        }
    }
    vec![]
}

impl fmt::Display for OrmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OrmError::KnownRequest(e) => write!(f, "{}", e.message),
            other => write!(f, "{}", other.message()),
        }
    }
}

impl std::error::Error for OrmError {}

impl From<sqlx::Error> for OrmError {
    fn from(e: sqlx::Error) -> Self {
        OrmError::from_sqlx(e, None)
    }
}

impl From<serde_json::Error> for OrmError {
    fn from(e: serde_json::Error) -> Self {
        OrmError::unknown(format!("Failed to convert the result: {e}"))
    }
}

/// Levenshtein distance (for "Did you mean ...?").
pub(crate) fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = if a[i - 1].eq_ignore_ascii_case(&b[j - 1]) { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Best suggestion among `options` for the unknown `name`.
pub(crate) fn suggest<'a>(name: &str, options: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    options
        .into_iter()
        .map(|o| (levenshtein(name, o), o))
        .filter(|(d, o)| *d <= (o.len().max(name.len()) / 2).max(2))
        .min_by_key(|(d, _)| *d)
        .map(|(_, o)| o)
}
