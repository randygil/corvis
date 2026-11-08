//! `OrmClient`: connection management, transactions, raw queries and `db push`.

use std::future::Future;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tokio::sync::RwLock;

use crate::db::{self, DbPool, DbTx, Dialect, Rows, Sql, SqlValue};
use crate::error::{OrmError, Result};
use crate::meta::{Model, ModelMeta, ModelSet};
use crate::query::Delegate;
use crate::raw::RawSql;
use crate::transaction::{Batch, IsolationLevel, TransactionOptions, TxSlot, TxState, tx_error};
use crate::url_params::{PoolTimeout, UrlParams, split_url_params};

/// Options of a [`OrmClient`] (`new OrmClient({ datasourceUrl, log })`).
#[derive(Debug, Clone)]
pub struct OrmClientOptions {
    /// `file:./dev.db`, `sqlite::memory:`, `sqlite://dev.db`, `postgresql://user:pass@host/db`.
    pub url: String,
    ///
    /// A Postgres URL may carry `?schema=name`: every connection then runs `SET search_path TO "name"`, and
    /// `migrate deploy`, `migrate status` and `db push` create the schema when it is missing.
    /// The parameter never reaches the driver (it is removed with `connection_limit` and `pool_timeout`).
    /// Log every query (`log: ['query']`) through the corvis logger (`[OrmClient]`).
    pub log_queries: bool,
    /// Maximum number of pooled connections. Overrides the `connection_limit` URL parameter; without
    /// either it is `2 * physical CPU cores + 1`. In-memory SQLite always uses 1.
    pub max_connections: Option<u32>,
    /// Timeout to acquire a connection. Overrides the `pool_timeout` URL parameter (whole seconds,
    /// `0` for no limit); without either it is 10 seconds.
    pub acquire_timeout: Option<Duration>,
    /// Global `omit` (`new OrmClient({ omit: { user: { password: true } } })`):
    /// model (client name like `user`, or model name like `User`) -> omitted fields.
    pub omit: std::collections::HashMap<String, Vec<String>>,
    /// Default options of interactive transactions (`transactionOptions`).
    pub transaction_options: TransactionOptions,
    /// Maximum nesting of the query arguments (objects and lists, the arguments object being
    /// level 1). Deeper arguments are rejected with a validation error before they are walked,
    /// so a hostile `where` cannot overflow the stack. Default
    /// [`OrmClientOptions::DEFAULT_MAX_QUERY_DEPTH`] (125, the same limit as the reference
    /// query engine).
    pub max_query_depth: usize,
}

impl OrmClientOptions {
    pub fn new(url: impl Into<String>) -> Self {
        OrmClientOptions {
            url: url.into(),
            log_queries: false,
            max_connections: None,
            acquire_timeout: None,
            omit: Default::default(),
            transaction_options: TransactionOptions::default(),
            max_query_depth: Self::DEFAULT_MAX_QUERY_DEPTH,
        }
    }
    /// Default of [`OrmClientOptions::max_query_depth`].
    pub const DEFAULT_MAX_QUERY_DEPTH: usize = 125;
    /// Maximum nesting of query arguments (see [`OrmClientOptions::max_query_depth`]).
    /// Raising it much beyond the default can overflow the stack with hostile input.
    pub fn max_query_depth(mut self, depth: usize) -> Self {
        self.max_query_depth = depth;
        self
    }
    /// Default `{ maxWait, timeout, isolationLevel }` of interactive transactions.
    pub fn transaction_options(mut self, o: impl Into<TransactionOptions>) -> Self {
        self.transaction_options = o.into();
        self
    }
    pub fn log_queries(mut self, v: bool) -> Self {
        self.log_queries = v;
        self
    }
    pub fn max_connections(mut self, n: u32) -> Self {
        self.max_connections = Some(n);
        self
    }
    /// Global omit, as a `{ model: { field: true } }` object: `json!({ "user": { "password": true } })`.
    pub fn omit(mut self, v: Value) -> Self {
        if let Value::Object(models) = v {
            for (m, fields) in models {
                if let Value::Object(fields) = fields {
                    let list = fields.into_iter().filter(|(_, v)| v == &Value::Bool(true)).map(|(k, _)| k).collect();
                    self.omit.insert(m, list);
                }
            }
        }
        self
    }
}

#[cfg(test)]
#[path = "client_pool_tests.rs"]
mod pool_tests;

pub(crate) struct ClientInner {
    pub(crate) options: OrmClientOptions,
    pub(crate) dialect: Dialect,
    /// `options.url` without the corvis-only parameters (`connection_limit`, `pool_timeout`): what the driver gets.
    pub(crate) driver_url: String,
    params: UrlParams,
    pool: RwLock<Option<DbPool>>,
}

/// `pool_timeout=0` ("no limit") as a finite wait of about 136 years: sqlx adds the timeout to
/// `Instant::now()`, which panics for `Duration::MAX`.
const UNLIMITED_POOL_TIMEOUT: Duration = Duration::from_secs(u32::MAX as u64);
/// Wait for a free connection when neither the code nor the URL set one (6.x engine default).
const DEFAULT_POOL_TIMEOUT: Duration = Duration::from_secs(10);

/// Default pool size: `2 * physical CPU cores + 1` (6.x engine default).
fn default_pool_size() -> u32 {
    u32::try_from(num_cpus::get_physical().saturating_mul(2).saturating_add(1)).unwrap_or(u32::MAX)
}

/// The database client. Cheap to clone; clones share the connection pool.
///
/// ```ignore
/// let db = OrmClient::new("file:./dev.db")?;
/// db.db_push::<(User, Post)>().await?;
/// let users: Vec<User> = db.user().find_many(json!({ "take": 10 })).await?;
/// ```
#[derive(Clone)]
pub struct OrmClient {
    pub(crate) inner: Arc<ClientInner>,
    tx: Option<TxSlot>,
}

impl std::fmt::Debug for OrmClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OrmClient").field("provider", &self.inner.dialect.name()).field("in_transaction", &self.tx.is_some()).finish()
    }
}

fn dialect_of(url: &str) -> Result<Dialect> {
    let u = url.trim();
    if u.starts_with("file:") || u.starts_with("sqlite:") {
        Ok(Dialect::Sqlite)
    } else if u.starts_with("postgres://") || u.starts_with("postgresql://") {
        Ok(Dialect::Postgres)
    } else {
        // The URL is not echoed: it usually carries the password.
        Err(OrmError::init(
            "error: Error validating datasource `db`: the URL must start with the protocol `file:`, `sqlite:`, `postgresql://` or `postgres://`.",
        ))
    }
}

fn is_memory(url: &str) -> bool {
    url.contains(":memory:") || url.contains("mode=memory")
}

impl OrmClient {
    /// Creates a client. It connects lazily on the first query, or explicitly with [`OrmClient::connect`].
    pub fn new(url: impl Into<String>) -> Result<Self> {
        Self::with_options(OrmClientOptions::new(url))
    }

    pub fn with_options(options: OrmClientOptions) -> Result<Self> {
        let dialect = dialect_of(&options.url)?;
        let (driver_url, params) = split_url_params(&options.url, dialect)?;
        Ok(OrmClient { inner: Arc::new(ClientInner { options, dialect, driver_url, params, pool: RwLock::new(None) }), tx: None })
    }

    /// The database provider.
    pub fn dialect(&self) -> Dialect {
        self.inner.dialect
    }

    pub fn options(&self) -> &OrmClientOptions {
        &self.inner.options
    }

    /// Fields omitted by default for `model` (the client's global `omit` option).
    pub(crate) fn global_omit(&self, model: &'static ModelMeta) -> std::collections::HashSet<&'static str> {
        let omit = &self.inner.options.omit;
        let list = omit.get(model.client_name).or_else(|| omit.get(model.name));
        match list {
            None => Default::default(),
            Some(names) => model.fields.iter().filter(|f| names.iter().any(|n| n == f.name)).map(|f| f.name).collect(),
        }
    }

    /// `true` when this client is bound to an interactive transaction.
    pub fn in_transaction(&self) -> bool {
        self.tx.is_some()
    }

    /// `db.model::<User>()` – the model delegate (same as `db.user()`).
    pub fn model<M: Model>(&self) -> Delegate<M> {
        Delegate::new(self.clone())
    }

    /// Opens the connection pool now instead of on the first query.
    pub async fn connect(&self) -> Result<()> {
        self.pool().await.map(|_| ())
    }

    /// Closes the connection pool.
    pub async fn disconnect(&self) -> Result<()> {
        let pool = self.inner.pool.write().await.take();
        if let Some(p) = pool {
            p.close().await;
        }
        Ok(())
    }

    /// `true` once connected.
    pub async fn is_connected(&self) -> bool {
        self.inner.pool.read().await.is_some()
    }

    pub(crate) async fn pool(&self) -> Result<DbPool> {
        if let Some(p) = self.inner.pool.read().await.as_ref() {
            return Ok(p.clone());
        }
        let mut slot = self.inner.pool.write().await;
        if let Some(p) = slot.as_ref() {
            return Ok(p.clone());
        }
        let opts = &self.inner.options;
        let url = self.inner.driver_url.as_str();
        let params = &self.inner.params;
        let max_connections = opts.max_connections.or(params.connection_limit).unwrap_or_else(default_pool_size);
        let acquire_timeout = opts.acquire_timeout.unwrap_or(match params.pool_timeout {
            Some(PoolTimeout::Seconds(s)) => Duration::from_secs(s).min(UNLIMITED_POOL_TIMEOUT),
            Some(PoolTimeout::Unlimited) => UNLIMITED_POOL_TIMEOUT,
            None => DEFAULT_POOL_TIMEOUT,
        });
        let pool = match self.inner.dialect {
            Dialect::Sqlite => {
                let connect = if let Some(path) = url.strip_prefix("file:") {
                    let (path, query) = path.split_once('?').unwrap_or((path, ""));
                    if path == ":memory:" || query.contains("mode=memory") {
                        SqliteConnectOptions::from_str("sqlite::memory:")
                    } else {
                        Ok(SqliteConnectOptions::new().filename(path))
                    }
                } else {
                    SqliteConnectOptions::from_str(url)
                }
                .map_err(|e| OrmError::init(format!("Invalid SQLite URL `{url}`: {e}")))?
                .create_if_missing(true)
                .foreign_keys(true);
                let mut pool_opts = SqlitePoolOptions::new().acquire_timeout(acquire_timeout);
                if is_memory(url) {
                    // Every connection to `:memory:` is a different database: keep exactly one, forever.
                    pool_opts = pool_opts.max_connections(1).min_connections(1).idle_timeout(None).max_lifetime(None);
                } else {
                    pool_opts = pool_opts.max_connections(max_connections);
                }
                DbPool::Sqlite(pool_opts.connect_with(connect).await.map_err(|e| init_error(e, url))?)
            }
            Dialect::Postgres => {
                let mut pool_opts = PgPoolOptions::new().max_connections(max_connections).acquire_timeout(acquire_timeout);
                if let Some(schema) = &params.schema {
                    // Runs on every new connection before it is handed out, so cached statements are planned against it.
                    let set: Arc<str> = set_search_path_sql(schema).into();
                    pool_opts = pool_opts.after_connect(move |conn, _meta| {
                        let set = set.clone();
                        Box::pin(async move { sqlx::Executor::execute(&mut *conn, &*set).await.map(|_| ()) })
                    });
                }
                DbPool::Postgres(db::retry_connect(|| pool_opts.clone().connect(url)).await.map_err(|e| init_error(e, url))?)
            }
        };
        *slot = Some(pool.clone());
        Ok(pool)
    }

    fn log(&self, sql: &Sql) {
        if self.inner.options.log_queries {
            let params = if sql.params.is_empty() { String::new() } else { format!(" -- params: {:?}", sql.params) };
            #[cfg(feature = "corvis")]
            corvis::Logger::new("OrmClient").debug(format!("orm:query {}{params}", sql.text));
            #[cfg(not(feature = "corvis"))]
            eprintln!("orm:query {}{params}", sql.text);
        }
    }

    /// Fails like the reference query engine when a parameter cannot be converted: an `Int`
    /// out of range, or a `Decimal` that does not fit a `NUMERIC` (it would be truncated).
    fn check_params(sql: &Sql) -> Result<()> {
        fn bad(p: &SqlValue, i: usize) -> Option<OrmError> {
            match p {
                SqlValue::OutOfRange(v) => Some(OrmError::int_overflow(i, *v)),
                SqlValue::Decimal(d) if !d.fits_numeric() => Some(OrmError::conversion(i, crate::decimal::NUMERIC_OVERFLOW)),
                SqlValue::Cast(v, _) => bad(v, i),
                SqlValue::Array(items, _) => items.iter().find_map(|x| bad(x, i)),
                _ => None,
            }
        }
        match sql.params.iter().enumerate().find_map(|(i, p)| bad(p, i)) {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    async fn fetch_driver(&self, sql: &Sql) -> Result<std::result::Result<Rows, sqlx::Error>> {
        Self::check_params(sql)?;
        self.log(sql);
        Ok(match &self.tx {
            Some(state) => {
                let mut guard = state.conn.lock().await;
                match guard.as_mut() {
                    Some(DbTx::Sqlite(tx)) => db::sqlite_fetch(&mut **tx, sql).await,
                    Some(DbTx::Postgres(tx)) => db::pg_fetch(&mut **tx, sql).await,
                    None => return Err(state.closed_error("query")),
                }
            }
            None => match self.pool().await? {
                DbPool::Sqlite(p) => db::sqlite_fetch(&p, sql).await,
                DbPool::Postgres(p) => match db::acquire_pg(&p).await {
                    Ok(mut c) => db::pg_fetch(&mut *c, sql).await,
                    Err(e) => Err(e),
                },
            },
        })
    }

    async fn execute_driver(&self, sql: &Sql) -> Result<std::result::Result<u64, sqlx::Error>> {
        Self::check_params(sql)?;
        self.log(sql);
        Ok(match &self.tx {
            Some(state) => {
                let mut guard = state.conn.lock().await;
                match guard.as_mut() {
                    Some(DbTx::Sqlite(tx)) => db::sqlite_execute(&mut **tx, sql).await,
                    Some(DbTx::Postgres(tx)) => db::pg_execute(&mut **tx, sql).await,
                    None => return Err(state.closed_error("query")),
                }
            }
            None => match self.pool().await? {
                DbPool::Sqlite(p) => db::sqlite_execute(&p, sql).await,
                DbPool::Postgres(p) => match db::acquire_pg(&p).await {
                    Ok(mut c) => db::pg_execute(&mut *c, sql).await,
                    Err(e) => Err(e),
                },
            },
        })
    }

    /// Runs a query returning rows.
    pub(crate) async fn fetch(&self, sql: &Sql, model: Option<&ModelMeta>) -> Result<Rows> {
        self.fetch_driver(sql).await?.map_err(|e| db::map_sqlx(e, model))
    }

    /// Runs a statement returning the number of affected rows.
    pub(crate) async fn execute(&self, sql: &Sql, model: Option<&ModelMeta>) -> Result<u64> {
        self.execute_driver(sql).await?.map_err(|e| db::map_sqlx(e, model))
    }

    /// Runs a script of several statements without parameters (simple query protocol).
    pub(crate) async fn execute_script(&self, script: &str) -> Result<()> {
        self.execute_script_raw(script).await?.map_err(|e| db::map_sqlx(e, None))
    }

    /// Like [`Self::execute_script`], keeping the driver error.
    pub(crate) async fn execute_script_raw(&self, script: &str) -> Result<std::result::Result<(), sqlx::Error>> {
        if self.inner.options.log_queries {
            let mut q = Sql::new(self.inner.dialect);
            q.push(script);
            self.log(&q);
        }
        Ok(match &self.tx {
            Some(state) => {
                let mut guard = state.conn.lock().await;
                match guard.as_mut() {
                    Some(DbTx::Sqlite(tx)) => db::sqlite_execute_script(&mut **tx, script).await,
                    Some(DbTx::Postgres(tx)) => db::pg_execute_script(&mut **tx, script).await,
                    None => return Err(state.closed_error("query")),
                }
            }
            None => match self.pool().await? {
                DbPool::Sqlite(p) => db::sqlite_execute_script(&p, script).await,
                DbPool::Postgres(p) => match db::acquire_pg(&p).await {
                    Ok(mut c) => db::pg_execute_script(&mut *c, script).await,
                    Err(e) => Err(e),
                },
            },
        })
    }

    /// `db execute --file`: runs a SQL script with several statements and no
    /// parameters (simple query protocol).
    pub async fn db_execute(&self, script: &str) -> Result<()> {
        self.execute_script(script).await
    }

    /// Runs any model action with JSON query arguments and returns the JSON result
    /// (`client[model][action](args)` in JavaScript terms).
    ///
    /// `action` is one of `findMany`, `findFirst`, `findFirstOrThrow`, `findUnique`,
    /// `findUniqueOrThrow`, `create`, `createMany`, `createManyAndReturn`, `update`,
    /// `updateMany`, `updateManyAndReturn`, `upsert`, `delete`, `deleteMany`, `count`,
    /// `aggregate` and `groupBy`.
    pub async fn query_json(&self, model: &'static ModelMeta, action: &str, args: Value) -> Result<Value> {
        let action = crate::engine::static_action(action)
            .ok_or_else(|| OrmError::validation(format!("Unknown action `{action}` for model `{}`", model.name)))?;
        crate::engine::run(self.clone(), model, action, args).await
    }

    /// Interactive transaction.
    ///
    /// The closure receives a client bound to the transaction. It is committed when the
    /// closure returns `Ok`, rolled back when it returns `Err`. Nested calls reuse the
    /// outer transaction. It waits at most `maxWait` (2 s) for a connection
    /// and is rolled back after `timeout` (5 s): use [`Self::_transaction_with`] or the
    /// client's `transaction_options` to change them.
    ///
    /// ```ignore
    /// let (user, post) = db._transaction(|tx| async move {
    ///     let user = tx.user().create(json!({ "data": { "email": "a@b.c" } })).await?;
    ///     let post = tx.post().create(json!({ "data": { "title": "Hi", "author_id": user.id } })).await?;
    ///     Ok::<_, OrmError>((user, post))
    /// }).await?;
    /// ```
    pub async fn _transaction<T, E, F, Fut>(&self, f: F) -> std::result::Result<T, E>
    where
        F: FnOnce(OrmClient) -> Fut,
        Fut: Future<Output = std::result::Result<T, E>>,
        E: From<OrmError>,
    {
        self._transaction_with(TransactionOptions::default(), f).await
    }

    /// Interactive transaction with `{ maxWait, timeout, isolationLevel }` options.
    ///
    /// ```ignore
    /// db._transaction_with(TransactionOptions::new().max_wait(15_000).timeout(30_000), |tx| async move { ... }).await?;
    /// db._transaction_with(json!({ "isolationLevel": "Serializable" }), |tx| async move { ... }).await?;
    /// ```
    pub async fn _transaction_with<T, E, F, Fut>(&self, options: impl Into<TransactionOptions>, f: F) -> std::result::Result<T, E>
    where
        F: FnOnce(OrmClient) -> Fut,
        Fut: Future<Output = std::result::Result<T, E>>,
        E: From<OrmError>,
    {
        if self.tx.is_some() {
            return f(self.clone()).await;
        }
        let options = options.into().or(self.inner.options.transaction_options);
        let state = self.begin_tx(options).await?;
        let client = OrmClient { inner: self.inner.clone(), tx: Some(state.clone()) };
        let result = f(client).await;
        match (state.finish().await, &result) {
            (Some(tx), Ok(_)) => tx.commit().await?,
            (Some(tx), Err(_)) => tx.rollback().await?,
            // The connection is gone: the timer rolled the transaction back. Never report
            // success for work that was not committed.
            (None, Ok(_)) => return Err(E::from(state.closed_error("commit"))),
            (None, Err(_)) => {}
        }
        result
    }

    /// Opens a transaction honouring `maxWait`, `timeout` and `isolationLevel`.
    async fn begin_tx(&self, options: TransactionOptions) -> Result<TxSlot> {
        let max_wait = options.max_wait.unwrap_or(TransactionOptions::DEFAULT_MAX_WAIT);
        let timeout = options.timeout.unwrap_or(TransactionOptions::DEFAULT_TIMEOUT);
        let pool = self.pool().await?;
        let mut tx = match tokio::time::timeout(max_wait, pool.begin()).await {
            Ok(tx) => tx?,
            Err(_) => return Err(tx_error("Unable to start a transaction in the given time.")),
        };
        if let Some(level) = options.isolation_level {
            match (&mut tx, level) {
                (DbTx::Postgres(t), l) if l != IsolationLevel::Snapshot => {
                    let mut q = Sql::new(Dialect::Postgres);
                    q.push("SET TRANSACTION ISOLATION LEVEL ").push(l.sql());
                    db::pg_execute(&mut **t, &q).await.map_err(|e| db::map_sqlx(e, None))?;
                }
                // SQLite transactions are always serializable.
                (DbTx::Sqlite(_), IsolationLevel::Serializable) => {}
                (_, l) => {
                    let _ = tx.rollback().await;
                    return Err(tx_error(format!(
                        "Invalid isolation level `{}` for the {} provider.",
                        l.as_str(),
                        self.inner.dialect.name()
                    )));
                }
            }
        }
        Ok(TxState::start(tx, timeout))
    }

    /// Sequential transaction (`$transaction([q1, q2, ...])` in JavaScript): runs the queries in
    /// order inside one transaction and returns their results; if one fails, the
    /// transaction is rolled back and its error returned.
    ///
    /// Takes a `Vec` of queries of the same type or a tuple of up to 12 queries:
    ///
    /// ```ignore
    /// let (total, rows) = db._transaction_batch((
    ///     db.invoice().count(json!({ "where": w })),
    ///     db.invoice().find_many(json!({ "where": w, "take": 20 })),
    /// )).await?;
    /// let wallets = db._transaction_batch(ids.iter().map(|id| db.wallet().update(...)).collect::<Vec<_>>()).await?;
    /// ```
    pub async fn _transaction_batch<B: Batch>(&self, batch: B) -> Result<B::Output> {
        self._transaction_batch_with(TransactionOptions::default(), batch).await
    }

    /// Sequential transaction with an `{ isolationLevel }` option.
    pub async fn _transaction_batch_with<B: Batch>(&self, options: impl Into<TransactionOptions>, batch: B) -> Result<B::Output> {
        if self.tx.is_some() {
            return batch.run(self.clone()).await;
        }
        let mut options = options.into().or(self.inner.options.transaction_options);
        // Batches only take `isolationLevel`: no `maxWait` / `timeout`.
        let forever = Duration::from_secs(60 * 60 * 24 * 365);
        options.max_wait = Some(forever);
        options.timeout = Some(forever);
        let state = self.begin_tx(options).await?;
        let client = OrmClient { inner: self.inner.clone(), tx: Some(state.clone()) };
        let result = batch.run(client).await;
        match (state.finish().await, &result) {
            (Some(tx), Ok(_)) => tx.commit().await?,
            (Some(tx), Err(_)) => tx.rollback().await?,
            (None, Ok(_)) => return Err(state.closed_error("commit")),
            (None, Err(_)) => {}
        }
        result
    }

    /// Runs a raw statement, mapping database errors to `P2010`.
    async fn raw_fetch(&self, q: &Sql) -> Result<Vec<Value>> {
        Ok(self.fetch_driver(q).await?.map_err(OrmError::from_sqlx_raw)?.into_objects())
    }

    async fn raw_execute(&self, q: &Sql) -> Result<u64> {
        self.execute_driver(q).await?.map_err(OrmError::from_sqlx_raw)
    }

    /// `$queryRaw` with a typed statement built with [`sql!`](crate::sql) (or
    /// [`RawSql`]): rows as JSON objects (`numeric` as decimal strings, `bigint` as
    /// numbers, timestamps as ISO strings).
    ///
    /// ```ignore
    /// let rows = tx._query_raw(sql!("SELECT * FROM \"Wallet\" WHERE \"merchantId\" = {id} FOR UPDATE")).await?;
    /// ```
    pub async fn _query_raw(&self, sql: RawSql) -> Result<Vec<Value>> {
        self.raw_fetch(&sql.to_sql(self.inner.dialect)).await
    }

    /// Like [`Self::_query_raw`], deserializing each row into `T` (`$queryRaw<T>`).
    pub async fn _query_raw_as<T: DeserializeOwned>(&self, sql: RawSql) -> Result<Vec<T>> {
        let rows = self._query_raw(sql).await?;
        rows.into_iter().map(|r| serde_json::from_value(r).map_err(OrmError::from)).collect()
    }

    /// `$executeRaw` with a typed statement: the number of affected rows.
    pub async fn _execute_raw(&self, sql: RawSql) -> Result<u64> {
        self.raw_execute(&sql.to_sql(self.inner.dialect)).await
    }

    /// `$queryRawUnsafe(sql, ...values)`: rows as JSON objects. Use the native
    /// placeholders of the database (`?` for SQLite, `$1` for Postgres).
    pub async fn query_raw(&self, sql: &str, params: Vec<Value>) -> Result<Vec<Value>> {
        let q = self.raw_sql(sql, &params);
        self.raw_fetch(&q).await
    }

    /// Like [`OrmClient::query_raw`] but deserializing each row into `T`.
    pub async fn query_raw_as<T: DeserializeOwned>(&self, sql: &str, params: Vec<Value>) -> Result<Vec<T>> {
        let rows = self.query_raw(sql, params).await?;
        rows.into_iter().map(|r| serde_json::from_value(r).map_err(OrmError::from)).collect()
    }

    /// `$executeRawUnsafe(sql, ...values)`: returns the number of affected rows.
    pub async fn execute_raw(&self, sql: &str, params: Vec<Value>) -> Result<u64> {
        let q = self.raw_sql(sql, &params);
        self.raw_execute(&q).await
    }

    /// `$queryRawUnsafe(sql, ...values)` (alias of [`Self::query_raw`]).
    pub async fn _query_raw_unsafe(&self, sql: &str, params: Vec<Value>) -> Result<Vec<Value>> {
        self.query_raw(sql, params).await
    }

    /// `$executeRawUnsafe(sql, ...values)` (alias of [`Self::execute_raw`]).
    pub async fn _execute_raw_unsafe(&self, sql: &str, params: Vec<Value>) -> Result<u64> {
        self.execute_raw(sql, params).await
    }

    fn raw_sql(&self, sql: &str, params: &[Value]) -> Sql {
        let mut q = Sql::new(self.inner.dialect);
        q.push(sql);
        q.params = params.iter().map(|v| crate::raw::raw_param(SqlValue::from_json(v), self.inner.dialect)).collect();
        q
    }

    /// `db push`: creates the missing tables, unique indexes and foreign keys.
    pub async fn db_push<S: ModelSet>(&self) -> Result<()> {
        self.db_push_models(&S::metas()).await
    }

    /// `db push` for a list of model metadata.
    pub async fn db_push_models(&self, models: &[&'static ModelMeta]) -> Result<()> {
        self.ensure_schema().await?;
        let statements = crate::schema::push_statements(self.inner.dialect, models)?;
        for stmt in statements {
            let mut q = Sql::new(self.inner.dialect);
            q.push(&stmt);
            self.execute(&q, None).await?;
        }
        Ok(())
    }

    /// `db push --force-reset`: drops the tables of the models and recreates them.
    pub async fn db_push_force_reset<S: ModelSet>(&self) -> Result<()> {
        let models = S::metas();
        for stmt in crate::schema::drop_statements(self.inner.dialect, &models)? {
            let mut q = Sql::new(self.inner.dialect);
            q.push(&stmt);
            self.execute(&q, None).await?;
        }
        self.db_push_models(&models).await
    }

    /// The SQL that `db_push` would run (as a `migrate diff --script` would).
    pub fn schema_sql<S: ModelSet>(&self) -> Result<Vec<String>> {
        crate::schema::push_statements(self.inner.dialect, &S::metas())
    }
}

/// `SET search_path` for the URL schema: the name is always a quoted identifier, never raw text.
pub(crate) fn set_search_path_sql(schema: &str) -> String {
    format!("SET search_path TO {}", db::quote_ident(schema))
}

impl OrmClient {
    /// Creates the URL schema (`?schema=`) on Postgres when it does not exist. The catalog lookup comes
    /// first because `CREATE SCHEMA IF NOT EXISTS` checks the database `CREATE` privilege before it checks
    /// existence, which would fail for a role that only has rights on an existing schema.
    pub(crate) async fn ensure_schema(&self) -> Result<()> {
        let Some(schema) = self.inner.params.schema.as_deref() else { return Ok(()) };
        if self.inner.dialect != Dialect::Postgres {
            return Ok(());
        }
        let found =
            self.query_raw("SELECT 1 AS one FROM pg_namespace WHERE nspname::text = $1", vec![Value::String(schema.to_string())]).await?;
        if found.is_empty() {
            self.execute_script(&format!("CREATE SCHEMA IF NOT EXISTS {}", db::quote_ident(schema))).await?;
        }
        Ok(())
    }
}

fn init_error(e: sqlx::Error, url: &str) -> OrmError {
    OrmError::Initialization(crate::error::InitializationError {
        message: format!("Can't reach database server at `{}`: {e}", server_of(url)),
        error_code: Some("P1001".into()),
    })
}

/// `host:port` of a connection URL (like `P1001` of the reference client), never the user,
/// the password or the query string. SQLite URLs give the file.
pub(crate) fn server_of(url: &str) -> String {
    let u = url.trim();
    if u.starts_with("file:") || u.starts_with("sqlite:") {
        return u.split('?').next().unwrap_or(u).to_string();
    }
    let rest = u.split_once("://").map(|(_, r)| r).unwrap_or("");
    // Authority: up to the first `/`, `?` or `#`; the host follows the last `@`.
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or("");
    if host.is_empty() {
        return "<unknown host>".to_string();
    }
    if host.contains(':') && !host.ends_with(']') { host.to_string() } else { format!("{host}:5432") }
}
