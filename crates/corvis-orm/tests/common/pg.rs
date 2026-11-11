//! Postgres helpers for the integration tests.
//!
//! Tests use `CORVIS_ORM_PG` (default `postgres://postgres:postgres@127.0.0.1:55432`)
//! and scratch databases named `orm_test_<name>_<run>` and roles named `orm_<name>_<run>`, where `<run>` is
//! `<unix seconds>_<process id>_<random hex>`: two runs against one server never share a name. What a process
//! created is dropped when it exits, and the first Postgres test of a process also drops leftovers of runs that
//! died, once they are older than [`STALE_AFTER_SECS`]. When the server is not reachable they are skipped,
//! unless `CORVIS_ORM_PG_REQUIRED=1` is set (CI sets it): then the test fails instead, so
//! a green run means the Postgres tests really ran. Failure messages name the server's
//! host and port, never the password.
#![allow(dead_code)]

use sqlx::{ConnectOptions, Connection, Executor};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

/// Leftover scratch databases and roles of dead runs are dropped once they are older than this (3 hours).
pub const STALE_AFTER_SECS: u64 = 3 * 3600;

/// Postgres truncates identifiers longer than this many bytes.
const MAX_IDENT: usize = 63;

/// The per-run part of every scratch name: `<unix seconds>_<process id>_<4 hex digits>`.
pub fn run_id() -> &'static str {
    static RUN: OnceLock<String> = OnceLock::new();
    RUN.get_or_init(|| {
        use std::hash::{BuildHasher, Hasher};
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let random = std::collections::hash_map::RandomState::new().build_hasher().finish() & 0xffff;
        format!("{secs}_{}_{random:04x}", std::process::id())
    })
}

/// `<prefix><name>_<run>`, with `name` cut so that the whole stays within Postgres' identifier limit.
pub fn scratch_name(prefix: &str, name: &str) -> String {
    let budget = MAX_IDENT - prefix.len() - 1 - run_id().len();
    let name: String = name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(budget).collect();
    format!("{prefix}{name}_{}", run_id())
}

/// The unique database name for `name` in this run (`orm_test_<name>_<run>`), registered for the drop at exit.
pub fn scratch_db(name: &str) -> String {
    let db = scratch_name("orm_test_", name);
    register(Scratch::Database(db.clone()));
    db
}

/// The unique role name for `name` in this run (`orm_<name>_<run>`), registered for the drop at exit.
pub fn scratch_role(name: &str) -> String {
    let role = scratch_name("orm_", name);
    register(Scratch::Role(role.clone()));
    role
}

/// The database name of a URL such as `postgres://u:p@host:port/db?x=1`.
pub fn db_of(url: &str) -> &str {
    let tail = url.rsplit_once('/').map_or(url, |(_, t)| t);
    tail.split('?').next().unwrap_or(tail)
}

enum Scratch {
    Database(String),
    Role(String),
}

static CREATED: Mutex<Vec<Scratch>> = Mutex::new(Vec::new());

fn register(item: Scratch) {
    static HOOK: AtomicBool = AtomicBool::new(false);
    if !HOOK.swap(true, Ordering::SeqCst) {
        unsafe extern "C" {
            fn atexit(callback: extern "C" fn()) -> i32;
        }
        extern "C" fn at_exit() {
            let items = std::mem::take(&mut *CREATED.lock().unwrap_or_else(|e| e.into_inner()));
            // A fresh runtime on its own thread: the test runtimes are gone by now.
            let _ = std::thread::spawn(move || {
                if let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    rt.block_on(drop_scratch(&base_url(), items));
                }
            })
            .join();
        }
        // SAFETY: `atexit` is the C runtime's; `at_exit` is a plain function that does not unwind into it.
        unsafe { atexit(at_exit) };
    }
    CREATED.lock().unwrap_or_else(|e| e.into_inner()).push(item);
}

/// Drops the databases first (which ends their connections), then the roles. Best effort.
async fn drop_scratch(base: &str, items: Vec<Scratch>) {
    let Ok(opts) = sqlx::postgres::PgConnectOptions::from_str(&format!("{base}/postgres")) else { return };
    let Ok(mut c) = opts.connect().await else { return };
    for item in &items {
        if let Scratch::Database(d) = item {
            let _ = c.execute(format!("DROP DATABASE IF EXISTS \"{d}\" WITH (FORCE)").as_str()).await;
        }
    }
    for item in &items {
        if let Scratch::Role(r) = item {
            let _ = c.execute(format!("DROP ROLE IF EXISTS \"{r}\"").as_str()).await;
        }
    }
    let _ = c.close().await;
}

/// The unix time embedded in a scratch name of the shape `<name>_<secs>_<pid>_<4 hex digits>`.
fn created_at(name: &str) -> Option<u64> {
    let mut parts = name.rsplitn(4, '_');
    let (random, pid, secs) = (parts.next()?, parts.next()?, parts.next()?);
    parts.next()?;
    let secs: u64 = secs.parse().ok().filter(|s| *s > 1_000_000_000)?;
    pid.parse::<u32>().ok()?;
    (random.len() == 4 && random.bytes().all(|b| b.is_ascii_hexdigit())).then_some(secs)
}

/// Drops `orm_test_*` databases and `orm_*` roles of runs older than [`STALE_AFTER_SECS`]. The age is the
/// timestamp in the name, so it works on any server with no file access. Once per process, best effort.
async fn sweep(c: &mut sqlx::PgConnection) {
    static DONE: AtomicBool = AtomicBool::new(false);
    if !DONE.swap(true, Ordering::SeqCst) {
        sweep_stale(c).await;
    }
}

/// The sweep itself: every call drops the stale scratch objects it finds.
pub async fn sweep_stale(c: &mut sqlx::PgConnection) {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let stale = |n: &str| created_at(n).is_some_and(|t| now.saturating_sub(t) > STALE_AFTER_SECS);
    let dbs: Vec<String> = sqlx::query_scalar(r"SELECT datname::text FROM pg_database WHERE datname LIKE 'orm\_test\_%'")
        .fetch_all(&mut *c)
        .await
        .unwrap_or_default();
    for d in dbs.iter().filter(|d| stale(d)) {
        let _ = c.execute(format!("DROP DATABASE IF EXISTS \"{d}\" WITH (FORCE)").as_str()).await;
    }
    let roles: Vec<String> =
        sqlx::query_scalar(r"SELECT rolname::text FROM pg_roles WHERE rolname LIKE 'orm\_%'").fetch_all(&mut *c).await.unwrap_or_default();
    for r in roles.iter().filter(|r| stale(r)) {
        let _ = c.execute(format!("DROP ROLE IF EXISTS \"{r}\"").as_str()).await;
    }
}

pub fn base_url() -> String {
    std::env::var("CORVIS_ORM_PG").unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1:55432".to_string())
}

/// Whether `CORVIS_ORM_PG_REQUIRED=1` turns a missing server into a test failure.
pub fn required() -> bool {
    std::env::var("CORVIS_ORM_PG_REQUIRED").is_ok_and(|v| v == "1")
}

/// Creates the database `orm_test_<name>_<run>` (a stale one of that name is dropped first); returns its URL, or `None` when Postgres is down
/// (a panic instead when `CORVIS_ORM_PG_REQUIRED=1`).
pub async fn fresh_db(name: &str) -> Option<String> {
    fresh_db_at(&base_url(), required(), name).await
}

/// Drops its database when it goes out of scope, also when the test panics (declare it before the clients that use
/// the database, so that they are dropped first). A process that is killed still leaves the database behind: the
/// sweep of a later run drops it once it is stale.
pub struct DropOnExit(String);

impl Drop for DropOnExit {
    fn drop(&mut self) {
        let db = std::mem::take(&mut self.0);
        // A fresh runtime on its own thread: the test's runtime may be the one unwinding.
        let _ = std::thread::spawn(move || {
            if let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() {
                rt.block_on(drop_scratch(&base_url(), vec![Scratch::Database(db)]));
            }
        })
        .join();
    }
}

/// [`fresh_db`], with a guard that drops the database at the end of the test and `statement_timeout` set on it
/// (`timeout_secs`), so that a statement that would run for long fails instead of holding the shared server.
pub async fn fresh_db_bounded(name: &str, timeout_secs: u32) -> Option<(String, DropOnExit)> {
    let url = fresh_db(name).await?;
    let guard = DropOnExit(db_of(&url).to_string());
    let opts = sqlx::postgres::PgConnectOptions::from_str(&url).expect("a scratch database URL");
    let mut c = opts.connect().await.expect("the scratch database accepts connections");
    c.execute(format!("ALTER DATABASE \"{}\" SET statement_timeout = '{timeout_secs}s'", db_of(&url)).as_str()).await.unwrap();
    let _ = c.close().await;
    Some((url, guard))
}

/// [`fresh_db`] against an explicit server URL and policy.
pub async fn fresh_db_at(base: &str, required: bool, name: &str) -> Option<String> {
    let db = scratch_name("orm_test_", name);
    let admin = match sqlx::postgres::PgConnectOptions::from_str(&format!("{base}/postgres")) {
        Ok(o) => o,
        Err(e) => return unavailable(base, required, "has an invalid URL", e),
    };
    // A refused connection means Postgres is down (skip, or panic when required); any other socket
    // error is retried a few times first, so a transient error (Windows `WSAEINVAL`) cannot silently
    // skip a test.
    let mut attempt = 0;
    let mut c = loop {
        match admin.connect().await {
            Ok(c) => break c,
            Err(sqlx::Error::Io(e)) if e.kind() != std::io::ErrorKind::ConnectionRefused && attempt < 4 => {
                attempt += 1;
                tokio::time::sleep(std::time::Duration::from_millis(10 << attempt)).await;
            }
            Err(e) => return unavailable(base, required, "is not reachable", e),
        }
    };
    if let Err(e) = c.execute(format!("DROP DATABASE IF EXISTS \"{db}\" WITH (FORCE)").as_str()).await {
        return unavailable(base, required, &format!("could not drop database {db}"), e);
    }
    if let Err(e) = c.execute(format!("CREATE DATABASE \"{db}\"").as_str()).await {
        return unavailable(base, required, &format!("could not create database {db}"), e);
    }
    register(Scratch::Database(db.clone()));
    sweep(&mut c).await;
    let _ = c.close().await;
    Some(format!("{base}/{db}"))
}

/// Skips (returns `None`) or, when `required`, panics with the host and the error.
fn unavailable(base: &str, required: bool, what: &str, e: impl std::fmt::Display) -> Option<String> {
    let e = redact(base, &e.to_string());
    if required {
        panic!("CORVIS_ORM_PG_REQUIRED=1 but the Postgres server at {} (CORVIS_ORM_PG) {what}: {e}", host(base));
    }
    eprintln!("skipping: Postgres not reachable ({} {what}: {e})", host(base));
    None
}

/// The `user:password@` part of a `postgres://` URL, split at the last `@` before the path.
fn userinfo(base: &str) -> Option<&str> {
    let rest = base.split_once("://").map_or(base, |(_, r)| r);
    let authority = rest.split(['/', '?']).next().unwrap_or(rest);
    authority.rsplit_once('@').map(|(u, _)| u)
}

/// `host:port` of a server URL, without credentials.
pub fn host(base: &str) -> &str {
    let rest = base.split_once("://").map_or(base, |(_, r)| r);
    let rest = match userinfo(base) {
        Some(u) => &rest[u.len() + 1..],
        None => rest,
    };
    rest.split(['/', '?']).next().unwrap_or(rest)
}

/// Removes the URL's password from `msg`, in case an error message quotes it.
fn redact(base: &str, msg: &str) -> String {
    match userinfo(base).and_then(|u| u.split_once(':')).map(|(_, p)| p) {
        Some(p) if !p.is_empty() => msg.replace(p, "***"),
        _ => msg.to_string(),
    }
}

/// The folder of an original reference application (`corvis-tests/nest/<app>`, outside
/// this repository), when it is present.
pub fn app_dir(rel: &str) -> Option<std::path::PathBuf> {
    let base = std::env::var("CORVIS_TESTS_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../corvis-tests"));
    let p = base.join("nest").join(rel);
    if p.exists() { Some(p) } else { None }
}
