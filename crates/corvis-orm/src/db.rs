//! Low level database access: SQL builder, bound values, pools, transactions and row decoding.

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, SecondsFormat, Utc};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

use sqlx::TransactionManager;
use sqlx::pool::PoolConnection;
use sqlx::postgres::{PgConnection, PgPool, PgRow, PgTransactionManager, PgTypeKind};
use sqlx::sqlite::{SqlitePool, SqliteRow};
use sqlx::{Column, Postgres, Row, Sqlite, Transaction, TypeInfo, ValueRef};

use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};

use crate::decimal::Decimal;
use crate::error::{OrmError, Result};
use crate::meta::{FieldKind, FieldMeta};

/// Database provider (`datasource db { provider = "sqlite" | "postgresql" }`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Sqlite,
    Postgres,
}

impl Dialect {
    pub fn name(&self) -> &'static str {
        match self {
            Dialect::Sqlite => "sqlite",
            Dialect::Postgres => "postgresql",
        }
    }
}

/// A value bound to a query parameter.
#[derive(Debug, Clone, PartialEq)]
pub enum SqlValue {
    Null(FieldKind),
    Bool(bool),
    Int(i64),
    /// A 32-bit integer (`Int` columns).
    Int32(i32),
    Float(f64),
    Text(String),
    /// `timestamptz`
    DateTime(DateTime<Utc>),
    /// `timestamp` (without time zone, as UTC) – the default for `DateTime` on Postgres.
    NaiveDateTime(NaiveDateTime),
    /// `date`
    Date(NaiveDate),
    /// `time`
    Time(NaiveTime),
    Json(Value),
    Decimal(Decimal),
    Bytes(Vec<u8>),
    Uuid(uuid::Uuid),
    /// A Postgres array (scalar lists).
    Array(Vec<SqlValue>, FieldKind),
    /// A value followed by a `::type` cast (Postgres).
    Cast(Box<SqlValue>, String),
    /// An `Int` argument that does not fit into 32 bits: running the query fails with
    /// a conversion error, as the reference query engine does when it binds the parameter.
    OutOfRange(i64),
}

impl SqlValue {
    /// Converts an arbitrary JSON value (raw queries).
    pub fn from_json(v: &Value) -> SqlValue {
        match v {
            Value::Null => SqlValue::Null(FieldKind::String),
            Value::Bool(b) => SqlValue::Bool(*b),
            Value::Number(n) => match n.as_i64() {
                Some(i) => SqlValue::Int(i),
                None => SqlValue::Float(n.as_f64().unwrap_or_default()),
            },
            Value::String(s) => SqlValue::Text(s.clone()),
            other => SqlValue::Json(other.clone()),
        }
    }

    fn strip_cast(&self) -> &SqlValue {
        match self {
            SqlValue::Cast(v, _) => v.strip_cast(),
            v => v,
        }
    }
}

pub(crate) fn format_datetime(d: &DateTime<Utc>) -> String {
    d.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Parses `2024-01-01T10:00:00Z`, `2024-01-01T10:00:00.123+02:00`, `2024-01-01 10:00:00` or `2024-01-01`.
pub(crate) fn parse_datetime(s: &str) -> Option<DateTime<Utc>> {
    crate::DateTime::parse(s).map(|d| d.inner())
}

pub(crate) fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// `"schema"."table"` or `"table"`.
pub(crate) fn qualified(schema: Option<&str>, table: &str) -> String {
    match schema {
        Some(s) => format!("{}.{}", quote_ident(s), quote_ident(table)),
        None => quote_ident(table),
    }
}

/// Incrementally built SQL statement with positional parameters.
#[derive(Debug, Clone)]
pub struct Sql {
    pub dialect: Dialect,
    pub text: String,
    pub params: Vec<SqlValue>,
    aliases: usize,
    /// The parameter shared between several uses (`$n::cast`), see [`Sql::bind_shared`].
    shared: Option<(usize, String)>,
}

impl Sql {
    pub fn new(dialect: Dialect) -> Self {
        Sql { dialect, text: String::new(), params: Vec::new(), aliases: 0, shared: None }
    }
    /// Forgets the shared parameter: the next [`Sql::bind_shared`] binds a new one.
    pub fn reset_shared(&mut self) {
        self.shared = None;
    }
    /// Like [`Sql::bind`], but a later call (until [`Sql::reset_shared`]) writes a reference to the
    /// parameter bound by the first one instead of binding a copy; `make` runs only for the first.
    /// Postgres only: SQLite placeholders are positional, so there it always binds.
    pub fn bind_shared(&mut self, make: impl FnOnce() -> SqlValue) -> &mut Self {
        if self.dialect == Dialect::Postgres
            && let Some((n, cast)) = &self.shared
        {
            let text = format!("${n}::{cast}");
            self.text.push_str(&text);
            return self;
        }
        let v = make();
        let cast = match (&v, self.dialect) {
            (SqlValue::Cast(_, c), Dialect::Postgres) => Some(c.clone()),
            _ => None,
        };
        self.bind(v);
        if let Some(cast) = cast {
            self.shared = Some((self.params.len(), cast));
        }
        self
    }
    pub fn push(&mut self, s: &str) -> &mut Self {
        self.text.push_str(s);
        self
    }
    pub fn ident(&mut self, s: &str) -> &mut Self {
        self.text.push_str(&quote_ident(s));
        self
    }
    /// A table name, schema-qualified when needed.
    pub fn table(&mut self, schema: Option<&str>, table: &str) -> &mut Self {
        self.text.push_str(&qualified(schema, table));
        self
    }
    /// `alias."column"`
    pub fn col(&mut self, alias: &str, column: &str) -> &mut Self {
        if !alias.is_empty() {
            self.text.push_str(alias);
            self.text.push('.');
        }
        self.ident(column)
    }
    pub fn bind(&mut self, v: SqlValue) -> &mut Self {
        let cast = match (&v, self.dialect) {
            (SqlValue::Cast(_, c), Dialect::Postgres) => Some(c.clone()),
            _ => None,
        };
        let v = match v {
            SqlValue::Cast(inner, _) => *inner,
            other => other,
        };
        self.params.push(v);
        match self.dialect {
            Dialect::Sqlite => self.text.push('?'),
            Dialect::Postgres => {
                let n = self.params.len();
                self.text.push_str(&format!("${n}"));
                if let Some(c) = cast {
                    self.text.push_str("::");
                    self.text.push_str(&c);
                }
            }
        }
        self
    }
    /// A fresh table alias (`t1`, `t2`, ...). `t0` is reserved for the root table.
    pub fn alias(&mut self) -> String {
        self.aliases += 1;
        format!("t{}", self.aliases)
    }
}

#[derive(Clone)]
pub(crate) enum DbPool {
    Sqlite(SqlitePool),
    Postgres(PgHandle),
}

impl DbPool {
    pub(crate) async fn close(&self) {
        match self {
            DbPool::Sqlite(p) => p.close().await,
            DbPool::Postgres(h) => {
                // Requests still waiting for a slot fail at once instead of at their timeout.
                h.slots.close();
                h.pool.close().await
            }
        }
    }
    pub(crate) async fn begin(&self) -> Result<DbTx> {
        Ok(match self {
            DbPool::Sqlite(p) => DbTx::Sqlite(p.begin().await?),
            DbPool::Postgres(h) => {
                // The slot stays inside the `PgConn` while BEGIN runs, and until the connection is back in the pool.
                let conn = acquire_pg(h).await??;
                DbTx::Postgres(PgTx::begin(conn).await?)
            }
        })
    }
}

/// A Postgres pool. The sqlx pool has one deadline for waiting for a free connection and for opening
/// one; `slots` (as many permits as the pool has connections) is taken first, so the wait for a free
/// connection is timed by `pool_timeout` and, once a request holds a slot, opening or checking a
/// connection is timed by `connect_timeout`.
#[derive(Clone)]
pub(crate) struct PgHandle {
    pub(crate) pool: PgPool,
    pub(crate) slots: Arc<Semaphore>,
    /// `None` = no limit.
    pub(crate) pool_timeout: Option<Duration>,
    /// `None` = no limit.
    pub(crate) connect_timeout: Option<Duration>,
    /// `host:port`, for messages.
    pub(crate) server: Arc<str>,
}

/// A pooled connection together with the slot of the request that uses it.
pub(crate) struct PgConn {
    conn: Option<PoolConnection<Postgres>>,
    slot: Option<OwnedSemaphorePermit>,
}

impl PgConn {
    /// The raw connection, out of the pool, with the slot: the caller keeps the slot until the
    /// connection is closed, so the server never sees more than `max_connections` of this client.
    pub(crate) fn detach(mut self) -> (PgConnection, OwnedSemaphorePermit) {
        let conn = self.conn.take().expect("connection present").detach();
        (conn, self.slot.take().expect("slot present"))
    }
}

impl std::ops::Deref for PgConn {
    type Target = PgConnection;
    fn deref(&self) -> &PgConnection {
        self.conn.as_ref().expect("connection present")
    }
}

impl std::ops::DerefMut for PgConn {
    fn deref_mut(&mut self) -> &mut PgConnection {
        self.conn.as_mut().expect("connection present")
    }
}

impl Drop for PgConn {
    fn drop(&mut self) {
        let (Some(mut conn), Some(slot)) = (self.conn.take(), self.slot.take()) else { return };
        // The slot is released only after the connection is back in the sqlx pool (its liveness
        // check included), so the next request does not spend its `connect_timeout` waiting for that.
        let back = conn.return_to_pool();
        drop(conn);
        match tokio::runtime::Handle::try_current() {
            Ok(rt) => {
                rt.spawn(async move {
                    back.await;
                    drop(slot);
                });
            }
            Err(_) => drop(slot),
        }
    }
}

/// An interactive transaction on Postgres. It owns the [`PgConn`], so when it is dropped (committed, rolled
/// back, cancelled or timed out) the slot is released only after sqlx has finished the rollback and put the
/// connection back, as for any other use of the connection.
pub(crate) struct PgTx {
    conn: PgConn,
    open: bool,
}

impl PgTx {
    async fn begin(mut conn: PgConn) -> std::result::Result<PgTx, sqlx::Error> {
        PgTransactionManager::begin(&mut conn, None).await?;
        Ok(PgTx { conn, open: true })
    }
    async fn commit(mut self) -> std::result::Result<(), sqlx::Error> {
        let r = PgTransactionManager::commit(&mut self.conn).await;
        if r.is_ok() {
            self.open = false;
        }
        r
    }
    async fn rollback(mut self) -> std::result::Result<(), sqlx::Error> {
        let r = PgTransactionManager::rollback(&mut self.conn).await;
        if r.is_ok() {
            self.open = false;
        }
        r
    }
}

impl Drop for PgTx {
    fn drop(&mut self) {
        if self.open {
            // Queues the ROLLBACK; the connection's own drop then waits for it before the slot is released.
            PgTransactionManager::start_rollback(&mut self.conn);
        }
    }
}

impl std::ops::Deref for PgTx {
    type Target = PgConnection;
    fn deref(&self) -> &PgConnection {
        &self.conn
    }
}

impl std::ops::DerefMut for PgTx {
    fn deref_mut(&mut self) -> &mut PgConnection {
        &mut self.conn
    }
}

/// How many times a failed connection attempt is repeated (after the first one).
const CONNECT_RETRIES: u32 = 4;

/// Whether `e` is an I/O error of the socket layer while a connection was being established.
/// sqlx's pool already retries `ConnectionRefused` until the acquire timeout; the rest of the
/// socket errors (on Windows, `WSAEINVAL` / os error 10022 from `set_nodelay` on a socket that
/// the peer reset right after the handshake) used to reach the application.
fn is_transient_connect_error(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::Io(io) if io.kind() != std::io::ErrorKind::ConnectionRefused)
}

/// Runs `connect` again while it fails with a transient socket error, with a short backoff.
/// Nothing is sent to the server before a connection exists, so repeating is always safe. The
/// last error is returned unchanged when the attempts run out.
pub(crate) async fn retry_connect<T, F, Fut>(mut connect: F) -> std::result::Result<T, sqlx::Error>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = std::result::Result<T, sqlx::Error>>,
{
    let mut attempt = 0;
    loop {
        match connect().await {
            Err(e) if attempt < CONNECT_RETRIES && is_transient_connect_error(&e) => {
                attempt += 1;
                tokio::time::sleep(std::time::Duration::from_millis(10 << attempt)).await;
            }
            other => return other,
        }
    }
}

/// The last error of the per-connection setup statement of one request, reported by the
/// `after_connect` hook of the pool through [`report_setup_error`].
#[derive(Default)]
struct SetupFailure {
    last: std::sync::Mutex<Option<String>>,
    /// Notified when the error came from the database itself: the statement fails on every attempt.
    fatal: Notify,
}

tokio::task_local! {
    static SETUP: Arc<SetupFailure>;
}

/// Called by the `after_connect` hook when the setup statement fails. sqlx logs and retries
/// any hook error until its deadline; this makes the error visible to the request that is opening
/// the connection (the hook runs inline in that request's task). Without a request, it does nothing.
pub(crate) fn report_setup_error(e: &sqlx::Error) {
    let _ = SETUP.try_with(|s| {
        *s.last.lock().unwrap_or_else(|p| p.into_inner()) = Some(e.to_string());
        if matches!(e, sqlx::Error::Database(_)) {
            s.fatal.notify_one();
        }
    });
}

/// `P1001` for a failure to get a connection.
pub(crate) fn unreachable_error(server: &str, detail: impl std::fmt::Display) -> OrmError {
    OrmError::Initialization(crate::error::InitializationError {
        message: format!("Can't reach database server at `{server}`: {detail}"),
        error_code: Some("P1001".into()),
    })
}

/// `P1001` for a connection that was not obtained within `limit`.
pub(crate) fn connect_timed_out(server: &str, limit: Duration, last: Option<String>) -> OrmError {
    let mut detail = format!("no connection within {} s (`connect_timeout`)", limit.as_secs());
    if let Some(l) = last {
        detail.push_str("; last error: ");
        detail.push_str(&l);
    }
    unreachable_error(server, detail)
}

/// Step 2 of [`acquire_pg`]: gets a connection from `acquire` within `connect_timeout`. A setup error
/// that the database returned ends the wait at once; the other failures wait for the timeout. The
/// outer `Err` is the `P1001` of the timeout or of the setup; the inner one is an ordinary driver error.
pub(crate) async fn open_connection<T, F, Fut>(
    server: &str,
    connect_timeout: Option<Duration>,
    acquire: F,
) -> Result<std::result::Result<T, sqlx::Error>>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = std::result::Result<T, sqlx::Error>>,
{
    let setup = Arc::new(SetupFailure::default());
    let run = SETUP.scope(setup.clone(), async {
        tokio::select! {
            r = retry_connect(acquire) => Some(r),
            _ = setup.fatal.notified() => None,
        }
    });
    let outcome = match connect_timeout {
        Some(limit) => match tokio::time::timeout(limit, run).await {
            Ok(o) => o,
            Err(_) => return Err(connect_timed_out(server, limit, last_error(&setup))),
        },
        None => run.await,
    };
    match (outcome, connect_timeout) {
        // sqlx's own deadline (set to `connect_timeout`) may fire first.
        (Some(Err(sqlx::Error::PoolTimedOut)), Some(limit)) => Err(connect_timed_out(server, limit, last_error(&setup))),
        (Some(r), _) => Ok(r),
        (None, _) => Err(unreachable_error(server, last_error(&setup).unwrap_or_default())),
    }
}

fn last_error(s: &SetupFailure) -> Option<String> {
    s.last.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

/// Takes a connection from the pool in two steps: a slot (waiting at most `pool_timeout`, `P2024`
/// otherwise), then a connection (at most `connect_timeout`, `P1001` otherwise), repeating attempts
/// that fail with a transient socket error. The slot stays with the returned connection.
pub(crate) async fn acquire_pg(h: &PgHandle) -> Result<std::result::Result<PgConn, sqlx::Error>> {
    let slot = match h.pool_timeout {
        Some(limit) => match tokio::time::timeout(limit, h.slots.clone().acquire_owned()).await {
            Ok(r) => r,
            Err(_) => return Err(OrmError::from(sqlx::Error::PoolTimedOut)),
        },
        None => h.slots.clone().acquire_owned().await,
    };
    let Ok(slot) = slot else { return Ok(Err(sqlx::Error::PoolClosed)) };
    let conn = open_connection(&h.server, h.connect_timeout, || h.pool.acquire()).await?;
    Ok(conn.map(|c| PgConn { conn: Some(c), slot: Some(slot) }))
}

pub(crate) enum DbTx {
    Sqlite(Transaction<'static, Sqlite>),
    Postgres(PgTx),
}

impl DbTx {
    pub(crate) async fn commit(self) -> Result<()> {
        match self {
            DbTx::Sqlite(t) => t.commit().await?,
            DbTx::Postgres(t) => t.commit().await?,
        }
        Ok(())
    }
    pub(crate) async fn rollback(self) -> Result<()> {
        match self {
            DbTx::Sqlite(t) => t.rollback().await?,
            DbTx::Postgres(t) => t.rollback().await?,
        }
        Ok(())
    }
}

/// Decoded rows: column names + JSON values.
#[derive(Debug, Default, Clone)]
pub struct Rows {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

impl Rows {
    /// Rows as JSON objects keyed by column name.
    pub fn into_objects(self) -> Vec<Value> {
        let cols = self.columns;
        self.rows.into_iter().map(|r| Value::Object(cols.iter().cloned().zip(r).collect())).collect()
    }
}

fn sqlite_query(sql: &Sql) -> sqlx::query::Query<'_, Sqlite, sqlx::sqlite::SqliteArguments<'_>> {
    let mut q = sqlx::query::<Sqlite>(&sql.text);
    for p in &sql.params {
        q = match p.strip_cast() {
            SqlValue::Null(_) => q.bind(Option::<String>::None),
            SqlValue::Bool(b) => q.bind(*b),
            SqlValue::Int(i) => q.bind(*i),
            SqlValue::Int32(i) => q.bind(*i),
            SqlValue::OutOfRange(i) => q.bind(*i),
            SqlValue::Float(f) => q.bind(*f),
            SqlValue::Text(s) => q.bind(s.clone()),
            SqlValue::DateTime(d) => q.bind(format_datetime(d)),
            SqlValue::NaiveDateTime(d) => q.bind(format_datetime(&d.and_utc())),
            SqlValue::Date(d) => q.bind(format_datetime(&d.and_hms_opt(0, 0, 0).unwrap().and_utc())),
            SqlValue::Time(t) => q.bind(t.format("%H:%M:%S%.3f").to_string()),
            SqlValue::Json(j) => q.bind(j.to_string()),
            SqlValue::Decimal(d) => q.bind(d.to_fixed_plain()),
            SqlValue::Bytes(b) => q.bind(b.clone()),
            SqlValue::Uuid(u) => q.bind(u.to_string()),
            SqlValue::Array(items, _) => q.bind(Value::Array(items.iter().map(sql_value_json).collect()).to_string()),
            SqlValue::Cast(..) => unreachable!(),
        };
    }
    q
}

/// JSON representation of a bound value (SQLite scalar lists, logs).
pub(crate) fn sql_value_json(v: &SqlValue) -> Value {
    match v.strip_cast() {
        SqlValue::Null(_) => Value::Null,
        SqlValue::Bool(b) => Value::Bool(*b),
        SqlValue::Int(i) => Value::from(*i),
        SqlValue::Int32(i) => Value::from(*i),
        SqlValue::OutOfRange(i) => Value::from(*i),
        SqlValue::Float(f) => Value::from(*f),
        SqlValue::Text(s) => Value::String(s.clone()),
        SqlValue::DateTime(d) => Value::String(format_datetime(d)),
        SqlValue::NaiveDateTime(d) => Value::String(format_datetime(&d.and_utc())),
        SqlValue::Date(d) => Value::String(format_datetime(&d.and_hms_opt(0, 0, 0).unwrap().and_utc())),
        SqlValue::Time(t) => Value::String(t.format("%H:%M:%S%.3f").to_string()),
        SqlValue::Json(j) => j.clone(),
        SqlValue::Decimal(d) => Value::String(d.value_of()),
        SqlValue::Bytes(b) => Value::Array(b.iter().map(|x| Value::from(*x)).collect()),
        SqlValue::Uuid(u) => Value::String(u.to_string()),
        SqlValue::Array(items, _) => Value::Array(items.iter().map(sql_value_json).collect()),
        SqlValue::Cast(..) => unreachable!(),
    }
}

macro_rules! bind_array {
    ($q:expr, $items:expr, $variant:ident, $t:ty, $conv:expr) => {{
        let v: Vec<Option<$t>> = $items
            .iter()
            .map(|x| match x.strip_cast() {
                SqlValue::$variant(inner) => Some(($conv)(inner)),
                _ => None,
            })
            .collect();
        $q.bind(v)
    }};
}

fn pg_query(sql: &Sql) -> sqlx::query::Query<'_, Postgres, sqlx::postgres::PgArguments> {
    let mut q = sqlx::query::<Postgres>(&sql.text);
    for p in &sql.params {
        q = match p.strip_cast() {
            SqlValue::Null(kind) => match kind {
                FieldKind::Int => q.bind(Option::<i32>::None),
                FieldKind::BigInt => q.bind(Option::<i64>::None),
                FieldKind::Float => q.bind(Option::<f64>::None),
                FieldKind::Boolean => q.bind(Option::<bool>::None),
                FieldKind::DateTime => q.bind(Option::<NaiveDateTime>::None),
                FieldKind::Json => q.bind(Option::<sqlx::types::Json<Value>>::None),
                FieldKind::Decimal => q.bind(Option::<Decimal>::None),
                FieldKind::Bytes => q.bind(Option::<Vec<u8>>::None),
                _ => q.bind(Option::<String>::None),
            },
            SqlValue::Bool(b) => q.bind(*b),
            SqlValue::Int(i) => q.bind(*i),
            SqlValue::Int32(i) => q.bind(*i),
            SqlValue::OutOfRange(i) => q.bind(*i),
            SqlValue::Float(f) => q.bind(*f),
            SqlValue::Text(s) => q.bind(s.clone()),
            SqlValue::DateTime(d) => q.bind(*d),
            SqlValue::NaiveDateTime(d) => q.bind(*d),
            SqlValue::Date(d) => q.bind(*d),
            SqlValue::Time(t) => q.bind(*t),
            SqlValue::Json(j) => q.bind(sqlx::types::Json(j.clone())),
            SqlValue::Decimal(d) => q.bind(d.clone()),
            SqlValue::Bytes(b) => q.bind(b.clone()),
            SqlValue::Uuid(u) => q.bind(*u),
            SqlValue::Array(items, kind) => {
                // Element representation: from the first non-null element, else from the kind.
                let first = items.iter().map(|x| x.strip_cast()).find(|x| !matches!(x, SqlValue::Null(_)));
                match (first, kind) {
                    (Some(SqlValue::Bool(_)), _) | (None, FieldKind::Boolean) => bind_array!(q, items, Bool, bool, |b: &bool| *b),
                    (Some(SqlValue::Int32(_)), _) | (None, FieldKind::Int) => bind_array!(q, items, Int32, i32, |b: &i32| *b),
                    (Some(SqlValue::Int(_)), _) | (None, FieldKind::BigInt) => bind_array!(q, items, Int, i64, |b: &i64| *b),
                    (Some(SqlValue::Float(_)), _) | (None, FieldKind::Float) => bind_array!(q, items, Float, f64, |b: &f64| *b),
                    (Some(SqlValue::Decimal(_)), _) | (None, FieldKind::Decimal) => {
                        bind_array!(q, items, Decimal, Decimal, |b: &Decimal| b.clone())
                    }
                    (Some(SqlValue::DateTime(_)), _) => bind_array!(q, items, DateTime, DateTime<Utc>, |b: &DateTime<Utc>| *b),
                    (Some(SqlValue::NaiveDateTime(_)), _) | (None, FieldKind::DateTime) => {
                        bind_array!(q, items, NaiveDateTime, NaiveDateTime, |b: &NaiveDateTime| *b)
                    }
                    (Some(SqlValue::Date(_)), _) => bind_array!(q, items, Date, NaiveDate, |b: &NaiveDate| *b),
                    (Some(SqlValue::Uuid(_)), _) => bind_array!(q, items, Uuid, uuid::Uuid, |b: &uuid::Uuid| *b),
                    (Some(SqlValue::Json(_)), _) | (None, FieldKind::Json) => {
                        let v: Vec<sqlx::types::Json<Value>> = items.iter().map(|x| sqlx::types::Json(sql_value_json(x))).collect();
                        q.bind(v)
                    }
                    (Some(SqlValue::Bytes(_)), _) | (None, FieldKind::Bytes) => {
                        bind_array!(q, items, Bytes, Vec<u8>, |b: &Vec<u8>| b.clone())
                    }
                    _ => bind_array!(q, items, Text, String, |b: &String| b.clone()),
                }
            }
            SqlValue::Cast(..) => unreachable!(),
        };
    }
    q
}

pub(crate) async fn sqlite_fetch<'c, E>(e: E, sql: &Sql) -> std::result::Result<Rows, sqlx::Error>
where
    E: sqlx::Executor<'c, Database = Sqlite>,
{
    let rows = sqlite_query(sql).fetch_all(e).await?;
    decode_sqlite(&rows)
}

pub(crate) async fn sqlite_execute<'c, E>(e: E, sql: &Sql) -> std::result::Result<u64, sqlx::Error>
where
    E: sqlx::Executor<'c, Database = Sqlite>,
{
    Ok(sqlite_query(sql).execute(e).await?.rows_affected())
}

/// sqlx caches prepared statements per connection by their SQL text, with the parameter types
/// of the first execution. The same text executed later with other parameter types would reuse
/// a statement prepared for the wrong types: a `NULL` is bound with a guessed type (`text`,
/// `timestamp`, ...) and later receives a `uuid` or a `date`, or a raw query receives an `int8`
/// and then a `text` (`invalid byte sequence ...`, `insufficient data left in message`).
/// Every statement with parameters therefore carries a comment with the bound type of each
/// parameter, which gives each type signature its own cache entry.
fn pg_type_signature(sql: &Sql) -> Option<Sql> {
    if sql.params.is_empty() {
        return None;
    }
    let tags: Vec<String> = sql.params.iter().map(pg_bind_tag).collect();
    let mut tagged = sql.clone();
    tagged.text = format!("{} /* corvis-types {} */", sql.text, tags.join(","));
    Some(tagged)
}

/// The Postgres type a value is bound with by [`pg_query`].
fn pg_bind_tag(v: &SqlValue) -> String {
    match v.strip_cast() {
        SqlValue::Null(kind) => format!("null:{kind:?}"),
        SqlValue::Bool(_) => "bool".into(),
        SqlValue::Int(_) | SqlValue::OutOfRange(_) => "int8".into(),
        SqlValue::Int32(_) => "int4".into(),
        SqlValue::Float(_) => "float8".into(),
        SqlValue::Text(_) => "text".into(),
        SqlValue::DateTime(_) => "timestamptz".into(),
        SqlValue::NaiveDateTime(_) => "timestamp".into(),
        SqlValue::Date(_) => "date".into(),
        SqlValue::Time(_) => "time".into(),
        SqlValue::Json(_) => "jsonb".into(),
        SqlValue::Decimal(_) => "numeric".into(),
        SqlValue::Bytes(_) => "bytea".into(),
        SqlValue::Uuid(_) => "uuid".into(),
        SqlValue::Array(items, kind) => match items.iter().map(|x| x.strip_cast()).find(|x| !matches!(x, SqlValue::Null(_))) {
            Some(first) => format!("{}[]", pg_bind_tag(first)),
            None => format!("{kind:?}[]"),
        },
        SqlValue::Cast(..) => unreachable!(),
    }
}

pub(crate) async fn pg_fetch<'c, E>(e: E, sql: &Sql) -> std::result::Result<Rows, sqlx::Error>
where
    E: sqlx::Executor<'c, Database = Postgres>,
{
    let tagged = pg_type_signature(sql);
    let rows = pg_query(tagged.as_ref().unwrap_or(sql)).fetch_all(e).await?;
    decode_pg(&rows)
}

pub(crate) async fn pg_execute<'c, E>(e: E, sql: &Sql) -> std::result::Result<u64, sqlx::Error>
where
    E: sqlx::Executor<'c, Database = Postgres>,
{
    let tagged = pg_type_signature(sql);
    Ok(pg_query(tagged.as_ref().unwrap_or(sql)).execute(e).await?.rows_affected())
}

/// Runs a script of several statements without parameters (migrations).
pub(crate) async fn pg_execute_script<'c, E>(e: E, sql: &'c str) -> std::result::Result<(), sqlx::Error>
where
    E: sqlx::Executor<'c, Database = Postgres>,
{
    use futures::TryStreamExt;
    let mut s = sqlx::raw_sql(sql).execute_many(e);
    while s.try_next().await?.is_some() {}
    Ok(())
}

pub(crate) async fn sqlite_execute_script<'c, E>(e: E, sql: &'c str) -> std::result::Result<(), sqlx::Error>
where
    E: sqlx::Executor<'c, Database = Sqlite>,
{
    use futures::TryStreamExt;
    let mut s = sqlx::raw_sql(sql).execute_many(e);
    while s.try_next().await?.is_some() {}
    Ok(())
}

fn decode_sqlite(rows: &[SqliteRow]) -> std::result::Result<Rows, sqlx::Error> {
    let mut out = Rows::default();
    if let Some(first) = rows.first() {
        out.columns = first.columns().iter().map(|c| c.name().to_string()).collect();
    }
    for row in rows {
        let mut values = Vec::with_capacity(row.len());
        for i in 0..row.len() {
            let raw = row.try_get_raw(i)?;
            if raw.is_null() {
                values.push(Value::Null);
                continue;
            }
            let ty = raw.type_info().name().to_uppercase();
            let v = match ty.as_str() {
                "INTEGER" | "INT" | "BIGINT" | "INT8" | "BOOLEAN" | "BOOL" => Value::from(row.try_get_unchecked::<i64, _>(i)?),
                "REAL" | "FLOAT" | "DOUBLE" | "NUMERIC" => Value::from(row.try_get_unchecked::<f64, _>(i)?),
                "BLOB" => Value::from(row.try_get_unchecked::<Vec<u8>, _>(i)?),
                _ => Value::from(row.try_get_unchecked::<String, _>(i)?),
            };
            values.push(v);
        }
        out.rows.push(values);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Postgres binary decoding (by type OID), with the JavaScript client conversions.
// ---------------------------------------------------------------------------

fn pg_epoch() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2000, 1, 1).unwrap().and_hms_opt(0, 0, 0).unwrap()
}

fn be_i16(b: &[u8]) -> i16 {
    i16::from_be_bytes([b[0], b[1]])
}
fn be_i32(b: &[u8]) -> i32 {
    i32::from_be_bytes([b[0], b[1], b[2], b[3]])
}
fn be_i64(b: &[u8]) -> i64 {
    i64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

fn ts_from_micros(us: i64) -> Option<NaiveDateTime> {
    pg_epoch().checked_add_signed(chrono::Duration::microseconds(us))
}

/// Decodes one binary value. `oid` is the type OID; `kind` the type kind (for enums/arrays).
fn decode_pg_value(oid: u32, kind: Option<&PgTypeKind>, b: &[u8], type_name: &str) -> std::result::Result<Value, String> {
    let err = |m: &str| format!("Failed to deserialize column of type '{type_name}': {m}");
    Ok(match oid {
        16 => Value::Bool(b.first().copied().unwrap_or(0) != 0),
        17 => Value::Array(b.iter().map(|x| Value::from(*x)).collect()),
        18 => Value::String(String::from_utf8_lossy(b).to_string()),
        20 => Value::from(be_i64(b)),
        21 => Value::from(be_i16(b)),
        23 => Value::from(be_i32(b)),
        26 => Value::from(u32::from_be_bytes([b[0], b[1], b[2], b[3]])),
        700 => js_number(f32::from_be_bytes([b[0], b[1], b[2], b[3]]) as f64),
        701 => js_number(f64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])),
        790 => Value::from(be_i64(b) as f64 / 100.0),
        1700 => Value::String(crate::decimal::decode_numeric(b).map_err(|e| err(&e.to_string()))?.value_of()),
        114 | 3802 => {
            let text = if oid == 3802 { &b[1..] } else { b };
            serde_json::from_slice(text).map_err(|e| err(&e.to_string()))?
        }
        2950 => Value::String(uuid::Uuid::from_slice(b).map_err(|e| err(&e.to_string()))?.to_string()),
        1082 => {
            let days = be_i32(b);
            let d = pg_epoch().date().checked_add_signed(chrono::Duration::days(days as i64)).ok_or_else(|| err("date out of range"))?;
            Value::String(format_datetime(&d.and_hms_opt(0, 0, 0).unwrap().and_utc()))
        }
        1114 | 1184 => {
            let us = be_i64(b);
            let t = ts_from_micros(us).ok_or_else(|| err("timestamp out of range"))?;
            Value::String(format_datetime(&t.and_utc()))
        }
        1083 => {
            let us = be_i64(b);
            let t = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap().and_hms_opt(0, 0, 0).unwrap() + chrono::Duration::microseconds(us);
            Value::String(format_datetime(&t.and_utc()))
        }
        1266 => {
            let us = be_i64(b);
            let off = be_i32(&b[8..]) as i64;
            let t = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap().and_hms_opt(0, 0, 0).unwrap()
                + chrono::Duration::microseconds(us)
                + chrono::Duration::seconds(off);
            Value::String(format_datetime(&t.and_utc()))
        }
        1186 => {
            let us = be_i64(b);
            let days = be_i32(&b[8..]);
            let months = be_i32(&b[12..]);
            Value::String(format!(
                "{months} mons {days} days {}",
                NaiveTime::from_num_seconds_from_midnight_opt(0, 0).unwrap() + chrono::Duration::microseconds(us)
            ))
        }
        2278 => Value::Null,
        25 | 19 | 1042 | 1043 | 142 => Value::String(String::from_utf8_lossy(b).to_string()),
        _ => match kind {
            Some(PgTypeKind::Enum(_)) => Value::String(String::from_utf8_lossy(b).to_string()),
            Some(PgTypeKind::Array(elem)) => decode_pg_array(b, Some(elem), type_name)?,
            Some(PgTypeKind::Domain(base)) => {
                let boid = base.oid().map(|o| o.0).unwrap_or(25);
                decode_pg_value(boid, Some(base.kind()), b, type_name)?
            }
            _ => {
                if let Some(arr) = known_array_elem(oid) {
                    decode_pg_array_oid(b, arr, type_name)?
                } else {
                    // citext and other text-like types: valid UTF-8 is taken as text.
                    match std::str::from_utf8(b) {
                        Ok(s) if !s.contains('\0') => Value::String(s.to_string()),
                        _ => return Err(err("unsupported type")),
                    }
                }
            }
        },
    })
}

fn known_array_elem(oid: u32) -> Option<u32> {
    Some(match oid {
        1000 => 16,
        1001 => 17,
        1002 => 18,
        1005 => 21,
        1007 => 23,
        1016 => 20,
        1009 => 25,
        1015 => 1043,
        1014 => 1042,
        1021 => 700,
        1022 => 701,
        1231 => 1700,
        2951 => 2950,
        1182 => 1082,
        1115 => 1114,
        1185 => 1184,
        1183 => 1083,
        199 => 114,
        3807 => 3802,
        1003 => 19,
        _ => return None,
    })
}

fn decode_pg_array(b: &[u8], elem: Option<&sqlx::postgres::PgTypeInfo>, type_name: &str) -> std::result::Result<Value, String> {
    let elem_oid = elem.and_then(|e| e.oid()).map(|o| o.0);
    let kind = elem.map(|e| e.kind());
    decode_array_inner(b, elem_oid, kind, type_name)
}

fn decode_pg_array_oid(b: &[u8], elem_oid: u32, type_name: &str) -> std::result::Result<Value, String> {
    decode_array_inner(b, Some(elem_oid), None, type_name)
}

fn decode_array_inner(b: &[u8], elem_oid: Option<u32>, kind: Option<&PgTypeKind>, type_name: &str) -> std::result::Result<Value, String> {
    if b.len() < 12 {
        return Err(format!("Failed to deserialize column of type '{type_name}': invalid array"));
    }
    let ndim = be_i32(b) as usize;
    let header_oid = u32::from_be_bytes([b[8], b[9], b[10], b[11]]);
    let oid = elem_oid.unwrap_or(header_oid);
    if ndim == 0 {
        return Ok(Value::Array(vec![]));
    }
    let mut dims = Vec::new();
    let mut pos = 12;
    for _ in 0..ndim {
        dims.push(be_i32(&b[pos..]) as usize);
        pos += 8;
    }
    let total: usize = dims.iter().product();
    let mut flat = Vec::with_capacity(total);
    for _ in 0..total {
        let len = be_i32(&b[pos..]);
        pos += 4;
        if len < 0 {
            flat.push(Value::Null);
            continue;
        }
        let len = len as usize;
        flat.push(decode_pg_value(oid, kind, &b[pos..pos + len], type_name)?);
        pos += len;
    }
    // Rebuild nested dimensions.
    fn reshape(flat: &mut std::vec::IntoIter<Value>, dims: &[usize]) -> Value {
        if dims.len() == 1 {
            return Value::Array(flat.take(dims[0]).collect());
        }
        Value::Array((0..dims[0]).map(|_| reshape(flat, &dims[1..])).collect())
    }
    let mut it = flat.into_iter();
    Ok(reshape(&mut it, &dims))
}

fn decode_pg(rows: &[PgRow]) -> std::result::Result<Rows, sqlx::Error> {
    let mut out = Rows::default();
    if let Some(first) = rows.first() {
        out.columns = first.columns().iter().map(|c| c.name().to_string()).collect();
    }
    for row in rows {
        let mut values = Vec::with_capacity(row.len());
        for i in 0..row.len() {
            let raw = row.try_get_raw(i)?;
            if raw.is_null() {
                values.push(Value::Null);
                continue;
            }
            let ti = raw.type_info().into_owned();
            let oid = ti.oid().map(|o| o.0).unwrap_or(0);
            let v = match raw.format() {
                sqlx::postgres::PgValueFormat::Text => Value::String(raw.as_str().map_err(sqlx::Error::Decode)?.to_string()),
                sqlx::postgres::PgValueFormat::Binary => {
                    let bytes = raw.as_bytes().map_err(sqlx::Error::Decode)?;
                    decode_pg_value(oid, Some(ti.kind()), bytes, ti.name()).map_err(|m| sqlx::Error::Decode(m.into()))?
                }
            };
            values.push(v);
        }
        out.rows.push(values);
    }
    Ok(out)
}

/// Normalizes a decoded column value according to the field kind
/// (SQLite booleans are integers, JSON is stored as text, enums use their client name, ...).
/// Normalizes a decoded column value for a field (enum mapping, lists, Json).
pub(crate) fn coerce_field_d(f: &FieldMeta, v: Value, dialect: Dialect) -> Value {
    if f.list {
        return match v {
            // A NULL scalar list is returned as `[]`.
            Value::Null => Value::Array(vec![]),
            Value::Array(items) => Value::Array(items.into_iter().map(|x| coerce_scalar(f, x, dialect)).collect()),
            Value::String(s) => match serde_json::from_str::<Value>(&s) {
                Ok(Value::Array(items)) => Value::Array(items.into_iter().map(|x| coerce_scalar(f, x, dialect)).collect()),
                _ => Value::String(s),
            },
            other => other,
        };
    }
    coerce_scalar(f, v, dialect)
}

fn coerce_scalar(f: &FieldMeta, v: Value, dialect: Dialect) -> Value {
    if f.kind == FieldKind::Enum
        && let (Some(e), Value::String(s)) = (f.enum_meta(), &v)
    {
        return Value::String(e.from_db(s).unwrap_or(s).to_string());
    }
    if f.kind == FieldKind::Json && dialect == Dialect::Postgres {
        // jsonb is decoded natively; a JSON string stays a string.
        return v;
    }
    coerce(f.kind, v)
}

/// Normalizes a decoded value by kind.
pub(crate) fn coerce(kind: FieldKind, v: Value) -> Value {
    match (kind, v) {
        (_, Value::Null) => Value::Null,
        (FieldKind::Boolean, Value::Number(n)) => Value::Bool(n.as_i64().unwrap_or(0) != 0),
        (FieldKind::Boolean, Value::String(s)) => Value::Bool(matches!(s.as_str(), "1" | "true" | "TRUE" | "t")),
        (FieldKind::Float, Value::Number(n)) => n.as_f64().map(js_number).unwrap_or(Value::Null),
        (FieldKind::Float, Value::String(s)) => s.parse::<f64>().ok().map(js_number).unwrap_or(Value::String(s)),
        (FieldKind::Int | FieldKind::BigInt, Value::Number(n)) => match n.as_i64() {
            Some(i) => Value::from(i),
            None => Value::from(n.as_f64().unwrap_or_default() as i64),
        },
        (FieldKind::Int | FieldKind::BigInt, Value::String(s)) => s.parse::<i64>().map(Value::from).unwrap_or(Value::String(s)),
        (FieldKind::Decimal, Value::Number(n)) => {
            let d = match (n.as_i64(), n.as_u64()) {
                (Some(i), _) => Decimal::from(i),
                (_, Some(u)) => Decimal::from(u),
                _ => Decimal::from(n.as_f64().unwrap_or_default()),
            };
            Value::String(d.value_of())
        }
        (FieldKind::Decimal, Value::String(s)) => match Decimal::parse(&s) {
            Ok(d) => Value::String(d.value_of()),
            Err(_) => Value::String(s),
        },
        (FieldKind::Json, Value::String(s)) => serde_json::from_str(&s).unwrap_or(Value::String(s)),
        (FieldKind::DateTime, Value::String(s)) => match parse_datetime(&s) {
            Some(d) => Value::String(format_datetime(&d)),
            None => Value::String(s),
        },
        (FieldKind::DateTime, Value::Number(n)) => {
            // Unix milliseconds (the storage format of `DateTime` on SQLite).
            match n.as_i64().and_then(DateTime::<Utc>::from_timestamp_millis) {
                Some(d) => Value::String(format_datetime(&d)),
                None => Value::Number(n),
            }
        }
        (_, v) => v,
    }
}

/// A float as JavaScript prints it: integral values without a fraction (`2`, not `2.0`).
pub(crate) fn js_number(f: f64) -> Value {
    if f.fract() == 0.0 && f.abs() < 9_007_199_254_740_992.0 {
        // `-0` prints as `0` in JSON.stringify.
        Value::from(f as i64)
    } else {
        serde_json::Number::from_f64(f).map(Value::Number).unwrap_or(Value::Null)
    }
}

pub(crate) fn map_sqlx(e: sqlx::Error, model: Option<&crate::meta::ModelMeta>) -> OrmError {
    OrmError::from_sqlx(e, model)
}

#[cfg(test)]
mod connect_retry_tests {
    use super::*;
    use std::io::{Error as IoError, ErrorKind};
    use std::sync::atomic::{AtomicU32, Ordering::SeqCst};

    fn io(kind: ErrorKind) -> sqlx::Error {
        sqlx::Error::Io(IoError::new(kind, "boom"))
    }

    #[tokio::test]
    async fn transient_socket_errors_are_retried_until_a_connection_opens() {
        let calls = AtomicU32::new(0);
        let r = retry_connect(|| async { if calls.fetch_add(1, SeqCst) < 3 { Err(io(ErrorKind::InvalidInput)) } else { Ok(7) } }).await;
        assert_eq!(r.unwrap(), 7);
        assert_eq!(calls.load(SeqCst), 4);
    }

    #[tokio::test]
    async fn a_persistent_error_is_returned_unchanged_after_the_attempts() {
        let calls = AtomicU32::new(0);
        let r: std::result::Result<(), _> = retry_connect(|| async {
            calls.fetch_add(1, SeqCst);
            Err(io(ErrorKind::InvalidInput))
        })
        .await;
        assert!(matches!(r, Err(sqlx::Error::Io(e)) if e.kind() == ErrorKind::InvalidInput));
        assert_eq!(calls.load(SeqCst), CONNECT_RETRIES + 1);
    }

    #[tokio::test]
    async fn refused_connections_and_other_errors_are_not_repeated_here() {
        for e in [io(ErrorKind::ConnectionRefused), sqlx::Error::PoolTimedOut, sqlx::Error::PoolClosed] {
            let calls = AtomicU32::new(0);
            let mut e = Some(e);
            let r: std::result::Result<(), _> = retry_connect(|| {
                calls.fetch_add(1, SeqCst);
                let e = e.take().unwrap();
                async move { Err(e) }
            })
            .await;
            assert!(r.is_err());
            assert_eq!(calls.load(SeqCst), 1);
        }
    }
}

#[cfg(test)]
mod setup_failure_tests {
    use super::*;
    use std::time::Instant;

    #[derive(Debug)]
    struct FakeDbError(&'static str);

    impl std::fmt::Display for FakeDbError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }

    impl std::error::Error for FakeDbError {}

    impl sqlx::error::DatabaseError for FakeDbError {
        fn message(&self) -> &str {
            self.0
        }
        fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
            self
        }
        fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
            self
        }
        fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
            self
        }
        fn kind(&self) -> sqlx::error::ErrorKind {
            sqlx::error::ErrorKind::Other
        }
    }

    fn pending() -> impl std::future::Future<Output = std::result::Result<(), sqlx::Error>> {
        std::future::pending()
    }

    #[tokio::test]
    async fn setup_failure_from_the_database_ends_the_wait_at_once() {
        let t = Instant::now();
        let r = open_connection("db:5432", Some(Duration::from_secs(30)), || async {
            report_setup_error(&sqlx::Error::Database(Box::new(FakeDbError("invalid byte sequence"))));
            pending().await
        })
        .await;
        let e = r.map(|_| ()).unwrap_err();
        assert!(t.elapsed() < Duration::from_secs(1));
        assert_eq!(e.code(), Some("P1001"));
        assert!(e.message().starts_with("Can't reach database server at `db:5432`: "), "{e}");
        assert!(e.message().contains("invalid byte sequence"), "{e}");
    }

    #[tokio::test]
    async fn setup_failure_from_the_database_ends_an_unlimited_wait() {
        let r = open_connection("db:5432", None, || async {
            report_setup_error(&sqlx::Error::Database(Box::new(FakeDbError("boom"))));
            pending().await
        })
        .await;
        assert!(r.map(|_| ()).unwrap_err().message().contains("boom"));
    }

    #[tokio::test]
    async fn setup_failure_on_the_socket_is_retried_until_the_timeout_and_shown() {
        let t = Instant::now();
        let r = open_connection("db:5432", Some(Duration::from_millis(300)), || async {
            report_setup_error(&sqlx::Error::Io(std::io::Error::new(std::io::ErrorKind::ConnectionReset, "reset by peer")));
            pending().await
        })
        .await;
        let e = r.map(|_| ()).unwrap_err();
        assert!(t.elapsed() >= Duration::from_millis(250) && t.elapsed() < Duration::from_secs(3));
        assert_eq!(e.code(), Some("P1001"));
        assert!(e.message().contains("no connection within 0 s (`connect_timeout`)"), "{e}");
        assert!(e.message().contains("last error:") && e.message().contains("reset by peer"), "{e}");
    }

    #[tokio::test]
    async fn setup_failure_with_nothing_reported_ends_with_the_plain_timeout() {
        let r = open_connection("db:5432", Some(Duration::from_millis(200)), pending).await;
        let e = r.map(|_| ()).unwrap_err();
        assert_eq!(e.message(), "Can't reach database server at `db:5432`: no connection within 0 s (`connect_timeout`)");
    }

    #[tokio::test]
    async fn setup_failure_reports_of_other_requests_are_not_seen() {
        // A report outside of any request is ignored, and a request never sees another request's error.
        report_setup_error(&sqlx::Error::Database(Box::new(FakeDbError("elsewhere"))));
        let slow = open_connection("db:5432", Some(Duration::from_millis(300)), pending);
        let failing = open_connection("db:5432", Some(Duration::from_secs(30)), || async {
            report_setup_error(&sqlx::Error::Database(Box::new(FakeDbError("mine"))));
            pending().await
        });
        let (a, b) = tokio::join!(slow, failing);
        let (a, b) = (a.map(|_| ()).unwrap_err(), b.map(|_| ()).unwrap_err());
        assert!(!a.message().contains("mine") && !a.message().contains("elsewhere"), "{a}");
        assert!(b.message().contains("mine"), "{b}");
    }

    #[tokio::test]
    async fn setup_success_and_driver_errors_pass_through() {
        let r = open_connection("db:5432", Some(Duration::from_secs(5)), || async { Ok::<_, sqlx::Error>(7) }).await;
        assert_eq!(r.unwrap().unwrap(), 7);
        let r = open_connection("db:5432", Some(Duration::from_secs(5)), || async { Err::<(), _>(sqlx::Error::PoolClosed) }).await;
        assert!(matches!(r.unwrap(), Err(sqlx::Error::PoolClosed)));
        // sqlx's own deadline, equal to ours, may fire first: still the connect error.
        let r = open_connection("db:5432", Some(Duration::from_secs(5)), || async { Err::<(), _>(sqlx::Error::PoolTimedOut) }).await;
        assert_eq!(r.map(|_| ()).unwrap_err().code(), Some("P1001"));
    }
}
