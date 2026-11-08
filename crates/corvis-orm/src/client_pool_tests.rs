//! Pool size and wait taken from the URL, the defaults and the precedence (unit tests of `client.rs`).

use super::*;

fn sqlite_pool(db: &DbPool) -> &sqlx::SqlitePool {
    match db {
        DbPool::Sqlite(p) => p,
        DbPool::Postgres(_) => unreachable!("sqlite client"),
    }
}

async fn limits(opts: OrmClientOptions) -> (u32, Duration) {
    let c = OrmClient::with_options(opts).unwrap();
    let pool = c.pool().await.unwrap();
    let p = sqlite_pool(&pool);
    (p.options().get_max_connections(), p.options().get_acquire_timeout())
}

/// A throw-away directory removed on drop (no extra dev-dependency).
struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("corvis-orm-pool-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    fn path(&self) -> String {
        self.0.to_string_lossy().replace('\\', "/")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn debug_output_redacts_the_password() {
    let o = OrmClientOptions::new("postgresql://app:S3cretPw@db.example:5432/main?sslmode=disable&password=Other");
    let d = format!("{o:?}");
    assert!(d.contains("postgresql://app:***@db.example:5432/main?..."), "{d}");
    for leak in ["S3cretPw", "Other", "sslmode"] {
        assert!(!d.contains(leak), "{leak} leaked in {d}");
    }
    // Unencoded `/` and `@` in the password.
    let d = format!("{:?}", OrmClientOptions::new("postgres://app:p/w@d@host/db"));
    assert!(d.contains("postgres://app:***@host/db") && !d.contains("p/w"), "{d}");
    assert_eq!(crate::client::redact_url("file:./dev.db?connection_limit=1"), "file:./dev.db?...");
    assert_eq!(crate::client::redact_url("postgres://host/db"), "postgres://host/db");
}

#[tokio::test]
async fn acquire_timeout_in_code_is_clamped_like_the_url_value() {
    let mut o = OrmClientOptions::new("sqlite::memory:");
    o.acquire_timeout = Some(Duration::MAX);
    let (_, t) = limits(o).await;
    assert_eq!(t, UNLIMITED_POOL_TIMEOUT);
}

#[test]
fn pool_size_in_code_is_bounded() {
    for n in [0, MAX_POOL_SIZE + 1, u32::MAX] {
        let e = OrmClient::with_options(OrmClientOptions::new("sqlite::memory:").max_connections(n)).unwrap_err();
        assert_eq!(e.name(), "InitializationError", "{n}");
        assert!(e.to_string().contains("`max_connections`") && e.to_string().contains(&format!("got `{n}`")), "{n}: {e}");
    }
    for n in [1, MAX_POOL_SIZE] {
        assert!(OrmClient::with_options(OrmClientOptions::new("sqlite::memory:").max_connections(n)).is_ok(), "{n}");
    }
}

#[test]
fn schema_search_path_statement_is_a_quoted_identifier() {
    assert_eq!(set_search_path_sql("tenant_a"), r#"SET search_path TO "tenant_a""#);
    assert_eq!(set_search_path_sql(r#"a"b"#), r#"SET search_path TO "a""b""#);
    assert_eq!(set_search_path_sql(r#"x"; DROP TABLE canary; --"#), r#"SET search_path TO "x""; DROP TABLE canary; --""#);
    assert_eq!(set_search_path_sql("My Schema"), r#"SET search_path TO "My Schema""#);
}

#[test]
fn pool_invalid_value_fails_at_creation_without_connecting() {
    let t = std::time::Instant::now();
    let e = OrmClient::new("postgresql://app:secret@192.0.2.1/db?connection_limit=abc").unwrap_err();
    assert!(t.elapsed() < Duration::from_secs(1));
    assert_eq!(e.code(), Some("P1013"));
    assert!(e.to_string().contains("connection_limit") && !e.to_string().contains("secret"));
    assert!(OrmClient::new("file:./dev.db?connection_limit=0").is_err());
    assert!(OrmClient::new("postgresql://h/db?pool_timeout=-1").is_err());
}

#[test]
fn pool_options_keep_the_url_as_given() {
    let c = OrmClient::new("postgresql://h/db?connection_limit=2&sslmode=disable").unwrap();
    assert_eq!(c.options().url, "postgresql://h/db?connection_limit=2&sslmode=disable");
    assert_eq!(c.inner.driver_url, "postgresql://h/db?sslmode=disable");
}

#[tokio::test]
async fn pool_defaults_url_and_builder_precedence_on_sqlite_files() {
    let dir = TempDir::new("prec");
    let url = |q: &str| format!("file:{}/pool.db{q}", dir.path());

    let (n, t) = limits(OrmClientOptions::new(url(""))).await;
    assert_eq!((n, t), (default_pool_size(), Duration::from_secs(10)));
    assert_eq!(default_pool_size() as usize, num_cpus::get_physical() * 2 + 1);

    let (n, t) = limits(OrmClientOptions::new(url("?connection_limit=3&pool_timeout=7"))).await;
    assert_eq!((n, t), (3, Duration::from_secs(7)));

    let mut o = OrmClientOptions::new(url("?connection_limit=3&pool_timeout=7")).max_connections(4);
    o.acquire_timeout = Some(Duration::from_secs(2));
    assert_eq!(limits(o).await, (4, Duration::from_secs(2)));

    let (_, t) = limits(OrmClientOptions::new(url("?pool_timeout=0"))).await;
    assert_eq!(t, UNLIMITED_POOL_TIMEOUT);
    let (_, t) = limits(OrmClientOptions::new(url("?pool_timeout=18446744073709551615"))).await;
    assert_eq!(t, UNLIMITED_POOL_TIMEOUT);
}

#[tokio::test]
async fn pool_in_memory_sqlite_keeps_one_connection() {
    let (n, t) = limits(OrmClientOptions::new("file::memory:?connection_limit=4")).await;
    assert_eq!((n, t), (1, Duration::from_secs(10)));
    let (n, _) = limits(OrmClientOptions::new("sqlite::memory:").max_connections(8)).await;
    assert_eq!(n, 1);
}

#[tokio::test]
async fn pool_sqlite_scheme_url_with_limit_connects() {
    let dir = TempDir::new("scheme");
    let url = format!("sqlite://{}/pool2.db?connection_limit=1", dir.path());
    let (n, _) = limits(OrmClientOptions::new(url)).await;
    assert_eq!(n, 1);
}

// ---- connect_timeout (Postgres), against a listener that accepts sockets and never answers ----

/// A loopback listener that accepts connections and keeps them open without writing.
async fn silent_server() -> (u16, tokio::task::JoinHandle<()>) {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    let h = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((s, _)) = l.accept().await {
            held.push(s);
        }
    });
    (port, h)
}

fn silent_url(port: u16, q: &str) -> String {
    format!("postgres://u:topsecret@127.0.0.1:{port}/x{q}")
}

#[tokio::test]
async fn connect_timeout_one_second_fails_with_p1001_although_pool_timeout_is_unlimited() {
    let (port, _srv) = silent_server().await;
    let c = OrmClient::new(silent_url(port, "?connect_timeout=1&pool_timeout=0")).unwrap();
    let t = std::time::Instant::now();
    let e = c.connect().await.unwrap_err();
    assert!(t.elapsed() >= Duration::from_millis(900) && t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
    assert_eq!(e.code(), Some("P1001"));
    assert!(e.message().starts_with(&format!("Can't reach database server at `127.0.0.1:{port}`")), "{e}");
    assert!(!e.message().contains("topsecret"), "{e}");
    assert!(!c.is_connected().await);
}

#[tokio::test]
async fn connect_timeout_default_is_five_seconds() {
    let (port, _srv) = silent_server().await;
    let c = OrmClient::new(silent_url(port, "?pool_timeout=0")).unwrap();
    assert_eq!(c.connect_timeout(), Some(Duration::from_secs(5)));
    let t = std::time::Instant::now();
    let e = c.connect().await.unwrap_err();
    assert!(t.elapsed() >= Duration::from_millis(4900) && t.elapsed() < Duration::from_secs(8), "{:?}", t.elapsed());
    assert_eq!(e.code(), Some("P1001"));
}

#[tokio::test]
async fn connect_timeout_zero_has_no_limit() {
    let (port, _srv) = silent_server().await;
    let c = OrmClient::new(silent_url(port, "?connect_timeout=0&pool_timeout=0")).unwrap();
    assert_eq!(c.connect_timeout(), None);
    assert!(tokio::time::timeout(Duration::from_secs(3), c.connect()).await.is_err());
    let c = OrmClient::new(silent_url(port, "?connect_timeout=99999999999999999999")).unwrap();
    assert_eq!(c.connect_timeout(), None);
}

#[tokio::test]
async fn connect_timeout_repeated_key_last_wins() {
    let c = OrmClient::new("postgresql://h/db?connect_timeout=1&connect_timeout=3").unwrap();
    assert_eq!(c.connect_timeout(), Some(Duration::from_secs(3)));
    assert_eq!(c.inner.driver_url, "postgresql://h/db");
}

#[tokio::test]
async fn connect_timeout_refused_port_with_unlimited_pool_timeout_fails() {
    let c = OrmClient::new("postgres://u:p@127.0.0.1:1/x?pool_timeout=0&connect_timeout=1").unwrap();
    let t = std::time::Instant::now();
    let e = c.connect().await.unwrap_err();
    assert!(t.elapsed() < Duration::from_secs(4), "{:?}", t.elapsed());
    assert_eq!(e.code(), Some("P1001"));
    assert!(e.message().starts_with("Can't reach database server at `127.0.0.1:1`"), "{e}");
}

#[tokio::test]
async fn connect_timeout_is_not_the_acquire_timeout_in_code() {
    // `acquire_timeout` bounds the wait for a slot only: a short one does not cut the connect.
    let (port, _srv) = silent_server().await;
    let mut o = OrmClientOptions::new(silent_url(port, "?connect_timeout=1"));
    o.acquire_timeout = Some(Duration::from_millis(100));
    let t = std::time::Instant::now();
    let e = OrmClient::with_options(o).unwrap().connect().await.unwrap_err();
    assert!(t.elapsed() >= Duration::from_millis(900), "{:?}", t.elapsed());
    assert_eq!(e.code(), Some("P1001"));
}

#[tokio::test]
async fn connect_timeout_migrate_deploy_is_bounded() {
    let (port, _srv) = silent_server().await;
    let dir = TempDir::new("migrate");
    let c = OrmClient::new(silent_url(port, "?connect_timeout=1&pool_timeout=0")).unwrap();
    let t = std::time::Instant::now();
    let e = c.migrate_deploy(dir.path()).await.unwrap_err();
    assert!(t.elapsed() >= Duration::from_millis(900) && t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
    assert_eq!(e.code(), Some("P1001"));
    assert!(e.message().starts_with(&format!("Can't reach database server at `127.0.0.1:{port}`")), "{e}");
    assert!(!e.message().contains("topsecret"), "{e}");
}

#[tokio::test]
async fn connect_timeout_is_ignored_on_sqlite() {
    let dir = TempDir::new("sqlite-ct");
    for v in ["abc", "1", "0", "-1", ""] {
        let url = format!("file:{}/ct.db?connect_timeout={v}", dir.path());
        let c = OrmClient::new(url).unwrap();
        assert_eq!(c.inner.driver_url, format!("file:{}/ct.db", dir.path()));
        c.connect().await.unwrap();
    }
}
