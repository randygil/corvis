//! Test-only statement recorder: [`capture`] runs a future and returns, with its output, every statement the
//! client sent while it ran (text and parameter types), so tests can assert SQL shapes and statement counts.

use std::future::Future;
use std::sync::{Arc, Mutex};

use crate::db::{Sql, SqlValue};

/// One statement as the client sent it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Recorded {
    pub sql: String,
    pub params: Vec<String>,
}

tokio::task_local! {
    static STATEMENTS: Arc<Mutex<Vec<Recorded>>>;
}

/// Runs `f`, recording the statements of the current task.
pub(crate) async fn capture<F: Future>(f: F) -> (F::Output, Vec<Recorded>) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let out = STATEMENTS.scope(log.clone(), f).await;
    let recorded = std::mem::take(&mut *log.lock().unwrap_or_else(|e| e.into_inner()));
    (out, recorded)
}

/// Called by the client for every statement it sends; a no-op outside [`capture`].
pub(crate) fn record(sql: &Sql) {
    let _ = STATEMENTS.try_with(|log| {
        let entry = Recorded { sql: sql.text.clone(), params: sql.params.iter().map(param_type).collect() };
        log.lock().unwrap_or_else(|e| e.into_inner()).push(entry);
    });
}

/// The type of a bound value (`Text`, `Int32`, `Array<Int>`, `Null<Decimal>`, ...).
fn param_type(v: &SqlValue) -> String {
    match v {
        SqlValue::Null(k) => format!("Null<{k:?}>"),
        SqlValue::Bool(_) => "Bool".into(),
        SqlValue::Int(_) => "Int".into(),
        SqlValue::Int32(_) => "Int32".into(),
        SqlValue::Float(_) => "Float".into(),
        SqlValue::Text(_) => "Text".into(),
        SqlValue::DateTime(_) => "DateTime".into(),
        SqlValue::NaiveDateTime(_) => "NaiveDateTime".into(),
        SqlValue::Date(_) => "Date".into(),
        SqlValue::Time(_) => "Time".into(),
        SqlValue::Json(_) => "Json".into(),
        SqlValue::Decimal(_) => "Decimal".into(),
        SqlValue::Bytes(_) => "Bytes".into(),
        SqlValue::Uuid(_) => "Uuid".into(),
        SqlValue::Array(items, k) => format!("Array<{k:?}>[{}]", items.iter().map(param_type).collect::<Vec<_>>().join(", ")),
        SqlValue::Cast(inner, c) => format!("{}::{c}", param_type(inner)),
        SqlValue::OutOfRange(_) => "OutOfRange".into(),
    }
}
