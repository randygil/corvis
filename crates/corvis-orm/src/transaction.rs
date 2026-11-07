//! `$transaction`: options of interactive transactions (`maxWait`, `timeout`,
//! `isolationLevel`), their expiry, and sequential (batch) transactions.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use futures::FutureExt;
use futures::future::BoxFuture;
use serde_json::Value;
use tokio::sync::{Mutex, Notify};

use crate::client::OrmClient;
use crate::db::DbTx;
use crate::error::{OrmError, Result};

/// Transaction isolation level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationLevel {
    ReadUncommitted,
    ReadCommitted,
    RepeatableRead,
    Serializable,
    /// SQL Server only.
    Snapshot,
}

impl IsolationLevel {
    /// The JSON name (`"ReadCommitted"`).
    pub fn as_str(&self) -> &'static str {
        match self {
            IsolationLevel::ReadUncommitted => "ReadUncommitted",
            IsolationLevel::ReadCommitted => "ReadCommitted",
            IsolationLevel::RepeatableRead => "RepeatableRead",
            IsolationLevel::Serializable => "Serializable",
            IsolationLevel::Snapshot => "Snapshot",
        }
    }

    /// Parses the JSON name (`"Serializable"`).
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "ReadUncommitted" => IsolationLevel::ReadUncommitted,
            "ReadCommitted" => IsolationLevel::ReadCommitted,
            "RepeatableRead" => IsolationLevel::RepeatableRead,
            "Serializable" => IsolationLevel::Serializable,
            "Snapshot" => IsolationLevel::Snapshot,
            _ => return None,
        })
    }

    pub(crate) fn sql(&self) -> &'static str {
        match self {
            IsolationLevel::ReadUncommitted => "READ UNCOMMITTED",
            IsolationLevel::ReadCommitted => "READ COMMITTED",
            IsolationLevel::RepeatableRead => "REPEATABLE READ",
            IsolationLevel::Serializable => "SERIALIZABLE",
            IsolationLevel::Snapshot => "SNAPSHOT",
        }
    }
}

/// Options of an interactive transaction: `{ maxWait, timeout, isolationLevel }`.
///
/// Unset values fall back to the client's `transactionOptions`, and then to the
/// defaults: `maxWait` 2000 ms and `timeout` 5000 ms.
///
/// ```ignore
/// let opts = TransactionOptions::new().max_wait(15_000).timeout(30_000);
/// let same: TransactionOptions = json!({ "maxWait": 15000, "timeout": 30000 }).into();
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TransactionOptions {
    /// Maximum time to wait for a connection to start the transaction.
    pub max_wait: Option<Duration>,
    /// Maximum time the interactive transaction may run before it is rolled back.
    pub timeout: Option<Duration>,
    pub isolation_level: Option<IsolationLevel>,
}

impl TransactionOptions {
    pub const DEFAULT_MAX_WAIT: Duration = Duration::from_millis(2000);
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_millis(5000);

    pub fn new() -> Self {
        Self::default()
    }
    /// `maxWait` in milliseconds.
    pub fn max_wait(mut self, ms: u64) -> Self {
        self.max_wait = Some(Duration::from_millis(ms));
        self
    }
    /// `timeout` in milliseconds.
    pub fn timeout(mut self, ms: u64) -> Self {
        self.timeout = Some(Duration::from_millis(ms));
        self
    }
    pub fn isolation_level(mut self, level: IsolationLevel) -> Self {
        self.isolation_level = Some(level);
        self
    }

    /// Fills the unset values from `fallback`.
    pub(crate) fn or(self, fallback: TransactionOptions) -> TransactionOptions {
        TransactionOptions {
            max_wait: self.max_wait.or(fallback.max_wait),
            timeout: self.timeout.or(fallback.timeout),
            isolation_level: self.isolation_level.or(fallback.isolation_level),
        }
    }
}

impl From<Value> for TransactionOptions {
    /// `{ maxWait, timeout, isolationLevel }` (unknown or invalid values are ignored).
    fn from(v: Value) -> Self {
        let ms = |k: &str| v.get(k).and_then(|x| x.as_u64()).map(Duration::from_millis);
        TransactionOptions {
            max_wait: ms("maxWait"),
            timeout: ms("timeout"),
            isolation_level: v.get("isolationLevel").and_then(|x| x.as_str()).and_then(IsolationLevel::parse),
        }
    }
}

impl From<()> for TransactionOptions {
    fn from(_: ()) -> Self {
        Self::default()
    }
}

/// The connection of an open interactive transaction.
pub(crate) struct TxState {
    pub(crate) conn: Mutex<Option<DbTx>>,
    started: Instant,
    timeout: Duration,
    expired: AtomicBool,
    done: Arc<Notify>,
}

pub(crate) type TxSlot = Arc<TxState>;

impl TxState {
    /// Opens the state and starts the timer that rolls the transaction back after `timeout`.
    pub(crate) fn start(tx: DbTx, timeout: Duration) -> TxSlot {
        let state = Arc::new(TxState {
            conn: Mutex::new(Some(tx)),
            started: Instant::now(),
            timeout,
            expired: AtomicBool::new(false),
            done: Arc::new(Notify::new()),
        });
        let weak = Arc::downgrade(&state);
        let done = state.done.clone();
        tokio::spawn(async move {
            let deadline = tokio::time::sleep(timeout);
            let notified = done.notified();
            tokio::select! {
                _ = deadline => {
                    if let Some(s) = weak.upgrade() {
                        // `expired` is set while the lock is held: `finish()` either takes the
                        // connection first, or sees it gone *and* the flag set.
                        let tx = {
                            let mut guard = s.conn.lock().await;
                            let tx = guard.take();
                            if tx.is_some() {
                                s.expired.store(true, Ordering::SeqCst);
                            }
                            tx
                        };
                        if let Some(tx) = tx {
                            let _ = tx.rollback().await;
                        }
                    }
                }
                _ = notified => {}
            }
        });
        state
    }

    /// Takes the connection to commit or roll back, stopping the timer.
    pub(crate) async fn finish(&self) -> Option<DbTx> {
        let tx = self.conn.lock().await.take();
        self.done.notify_one();
        tx
    }

    pub(crate) fn is_expired(&self) -> bool {
        self.expired.load(Ordering::SeqCst)
    }

    /// P2028 for an operation (`"query"`, `"commit"`) on a closed transaction.
    pub(crate) fn closed_error(&self, what: &str) -> OrmError {
        let msg = if self.is_expired() {
            format!(
                "Transaction already closed: A {what} cannot be executed on an expired transaction. The timeout for this transaction was {} ms, however {} ms passed since the start of the transaction. Consider increasing the interactive transaction timeout or doing less work in the transaction.",
                self.timeout.as_millis(),
                self.started.elapsed().as_millis()
            )
        } else {
            format!("Transaction already closed: A {what} cannot be executed on a committed transaction.")
        };
        tx_error(msg)
    }
}

impl Drop for TxState {
    /// Stops the timer when the transaction is dropped without `finish()` (its future was
    /// cancelled): otherwise the timer task would sleep until the timeout (a year for the
    /// transactions of nested writes). The connection itself is rolled back by sqlx on drop.
    fn drop(&mut self) {
        self.done.notify_one();
    }
}

/// `P2028` ("Transaction API error: ...").
pub(crate) fn tx_error(msg: impl Into<String>) -> OrmError {
    let msg = msg.into();
    OrmError::known("P2028", format!("Transaction API error: {msg}"), serde_json::json!({ "error": msg }))
}

/// A query that can run inside a sequential transaction (`$transaction([q1, q2])`).
/// Implemented by every model query (`db.user().create(...)`).
pub trait BatchQuery: Send + 'static {
    type Output: Send + 'static;
    /// Runs the query on `client` (the transaction).
    fn exec_on(self, client: OrmClient) -> BoxFuture<'static, Result<Self::Output>>;
}

/// The list of queries of a sequential transaction: a `Vec` of queries of the same
/// type, or a tuple of up to 12 queries of any type.
pub trait Batch: Send + 'static {
    type Output: Send + 'static;
    fn run(self, client: OrmClient) -> BoxFuture<'static, Result<Self::Output>>;
}

impl<Q: BatchQuery> Batch for Vec<Q> {
    type Output = Vec<Q::Output>;
    fn run(self, client: OrmClient) -> BoxFuture<'static, Result<Self::Output>> {
        async move {
            let mut out = Vec::with_capacity(self.len());
            for q in self {
                out.push(q.exec_on(client.clone()).await?);
            }
            Ok(out)
        }
        .boxed()
    }
}

macro_rules! batch_tuple {
    ($($q:ident),+) => {
        impl<$($q: BatchQuery),+> Batch for ($($q,)+) {
            type Output = ($($q::Output,)+);
            #[allow(non_snake_case)]
            fn run(self, client: OrmClient) -> BoxFuture<'static, Result<Self::Output>> {
                let ($($q,)+) = self;
                async move { Ok(($($q.exec_on(client.clone()).await?,)+)) }.boxed()
            }
        }
    };
}

batch_tuple!(A);
batch_tuple!(A, B);
batch_tuple!(A, B, C);
batch_tuple!(A, B, C, D);
batch_tuple!(A, B, C, D, E);
batch_tuple!(A, B, C, D, E, F);
batch_tuple!(A, B, C, D, E, F, G);
batch_tuple!(A, B, C, D, E, F, G, H);
batch_tuple!(A, B, C, D, E, F, G, H, I);
batch_tuple!(A, B, C, D, E, F, G, H, I, J);
batch_tuple!(A, B, C, D, E, F, G, H, I, J, K);
batch_tuple!(A, B, C, D, E, F, G, H, I, J, K, L);
