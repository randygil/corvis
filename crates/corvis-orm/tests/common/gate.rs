//! A server-side gate that keeps queries in flight until the test releases them.
//!
//! A holder transaction takes an exclusive Postgres advisory lock. The queries under test run
//! `SELECT 1 FROM pg_advisory_xact_lock_shared(<key>)` (see [`Gate::query_sql`]) and block at the server
//! while the lock is held, so each pooled connection stays busy for as long as the test needs, however
//! slow the machine is. An observer connection counts the blocked queries.
//!
//! Bound policy for the timing tests of the pool (so later tests follow it):
//! - Hold connections with a gate or a handshake, never with a fixed sleep that has to overlap another one.
//! - A lower bound stays tight: timers never fire early and load only adds time.
//! - An upper bound sits below the nearest value a wrong implementation would produce (the other value of a
//!   repeated key, the 10 s default `pool_timeout`, the 5 s default `connect_timeout`, no limit), and the
//!   assertion message names that value.
//! - A deadline is only a hang guard: a correct pool never reaches it.
#![allow(dead_code)]

use std::time::{Duration, Instant};

use corvis_orm::TransactionOptions;
use corvis_orm::prelude::*;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// Longest the observer waits for the expected number of blocked queries.
const DEADLINE: Duration = Duration::from_secs(30);
/// How long sampling continues after the expected count is reached, to catch any excess.
const SETTLE: Duration = Duration::from_millis(250);
const POLL: Duration = Duration::from_millis(20);

/// Handle of a spawned query under test.
pub type Task = JoinHandle<std::result::Result<Vec<Value>, OrmError>>;

pub struct Gate {
    key: i64,
    holder_pid: i64,
    observer: OrmClient,
    release: Option<oneshot::Sender<()>>,
    holder: JoinHandle<std::result::Result<(), OrmError>>,
}

impl Gate {
    /// Closes the gate on database `db_url` (the URL of the database the queries run on) with advisory key `key`.
    pub async fn close(db_url: &str, key: i64) -> Gate {
        let url = |q: &str| if db_url.contains('?') { format!("{db_url}&{q}") } else { format!("{db_url}?{q}") };
        let holder_client = OrmClient::new(url("connection_limit=1")).unwrap();
        let (ready_tx, ready_rx) = oneshot::channel::<i64>();
        let (release_tx, release_rx) = oneshot::channel::<()>();
        let holder = tokio::spawn(async move {
            let opts = TransactionOptions::new().max_wait(30_000).timeout(120_000);
            let res = holder_client
                ._transaction_with(opts, |tx| async move {
                    tx.query_raw(&format!("SELECT 1 FROM pg_advisory_xact_lock({key})"), vec![]).await?;
                    let rows = tx.query_raw("SELECT pg_backend_pid()::int8 AS pid", vec![]).await?;
                    let _ = ready_tx.send(rows[0]["pid"].as_i64().unwrap());
                    let _ = release_rx.await;
                    Ok::<_, OrmError>(())
                })
                .await;
            let _ = holder_client.disconnect().await;
            res
        });
        let holder_pid = ready_rx.await.expect("the gate holder failed before it took the lock");
        let observer = OrmClient::new(url("connection_limit=1")).unwrap();
        observer.connect().await.unwrap();
        Gate { key, holder_pid, observer, release: Some(release_tx), holder }
    }

    /// The statement the queries under test run: it blocks while the gate is closed and returns at once after.
    pub fn query_sql(key: i64) -> String {
        format!("SELECT 1 FROM pg_advisory_xact_lock_shared({key})")
    }

    pub fn sql(&self) -> String {
        Self::query_sql(self.key)
    }

    /// One observation: `(queries blocked on the gate, client backends of the database)`, the observer and
    /// the holder not counted.
    pub async fn sample(&self) -> (i64, i64) {
        let sql = format!(
            "SELECT (SELECT count(*) FROM pg_locks WHERE locktype = 'advisory' AND NOT granted AND objid::int8 = {key} \
                     AND database = (SELECT oid FROM pg_database WHERE datname = current_database()))::int8 AS blocked, \
                    (SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND backend_type = 'client backend' \
                     AND pid <> pg_backend_pid() AND pid <> {holder})::int8 AS backends",
            key = self.key,
            holder = self.holder_pid
        );
        let rows = tokio::time::timeout(Duration::from_secs(10), self.observer.query_raw(&sql, vec![]))
            .await
            .expect("the gate observer's query did not return within 10 s")
            .unwrap();
        (rows[0]["blocked"].as_i64().unwrap(), rows[0]["backends"].as_i64().unwrap())
    }

    /// Samples until `expected` queries are blocked, keeps sampling for the settle window and returns the
    /// maxima `(blocked, backends)`. Fails at once if one of `tasks` finishes first (a gated query can only
    /// end early with an error), and after 30 s if the count is never reached.
    pub async fn wait_for(&self, expected: i64, tasks: &mut [Task]) -> (i64, i64) {
        let start = Instant::now();
        let (mut max_blocked, mut max_backends) = (0, 0);
        let mut settled_at: Option<Instant> = None;
        loop {
            let (blocked, backends) = self.sample().await;
            max_blocked = max_blocked.max(blocked);
            max_backends = max_backends.max(backends);
            if settled_at.is_none() && blocked >= expected {
                settled_at = Some(Instant::now());
            }
            if let Some(at) = settled_at {
                if at.elapsed() >= SETTLE {
                    return (max_blocked, max_backends);
                }
            } else {
                if let Some(t) = tasks.iter_mut().find(|t| t.is_finished()) {
                    let res = t.await.unwrap();
                    panic!("a gated query finished before {expected} were blocked (blocked {blocked}, backends {backends}): {res:?}");
                }
                if start.elapsed() >= DEADLINE {
                    panic!("only {max_blocked} of the expected {expected} queries reached the server within 30 s");
                }
            }
            tokio::time::sleep(POLL).await;
        }
    }

    /// Releases the gate: the blocked queries go on.
    pub async fn open(mut self) {
        let _ = self.release.take().unwrap().send(());
        self.holder.await.unwrap().unwrap();
        let _ = self.observer.disconnect().await;
    }
}
