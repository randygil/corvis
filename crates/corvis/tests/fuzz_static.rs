//! Property-based fuzzing of static file serving, end to end through a running application:
//! generated request paths against a served folder with a secret file next to it.
//!
//! `PROPTEST_CASES` overrides the case count (2,000 by default).

use std::path::PathBuf;

use corvis::prelude::*;
use corvis::testing::{Test, TestClient, request};
use corvis::{Dotfiles, serve_static};
use proptest::prelude::*;
use proptest::test_runner::{FileFailurePersistence, TestCaseError, TestRunner};

const ENV_TEXT: &str = "DB_PASSWORD=supersecret";
const GIT_TEXT: &str = "url = https://user:token@example.com";
const OUTSIDE_TEXT: &str = "OUTSIDE ROOT SECRET";

/// `base/public/{.env, .git/config, a.txt, index.html, sub/b.txt}` and `base/outside/secret.txt`.
struct Fixture {
    base: PathBuf,
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let base = std::env::temp_dir().join(format!("corvis-fz-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("public");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::create_dir_all(base.join("outside")).unwrap();
        std::fs::write(root.join(".env"), ENV_TEXT).unwrap();
        std::fs::write(root.join(".git").join("config"), GIT_TEXT).unwrap();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        std::fs::write(root.join("index.html"), "<h1>index</h1>").unwrap();
        std::fs::write(root.join("sub").join("b.txt"), "bee").unwrap();
        std::fs::write(base.join("outside").join("secret.txt"), OUTSIDE_TEXT).unwrap();
        Fixture { base, root }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

async fn build_app(fx: &Fixture) -> CorvisApplication {
    let module = Test::create_testing_module()
        .import_dynamic(ServeStaticModule::for_root(vec![
            ServeStaticOptions::new(fx.root.to_string_lossy()).serve_root("/mod").spa_fallback(true).exclude(&["/mod/api/(.*)"]),
            ServeStaticOptions::new(fx.root.to_string_lossy()).serve_root("/modplain"),
        ]))
        .with_logger(false)
        .compile()
        .await
        .unwrap();
    let mut app = module.create_corvis_application();
    let root = fx.root.clone();
    app.use_middleware(serve_static(&root).mount("/static"));
    app.use_middleware(serve_static(&root).mount("/allow").dotfiles(Dotfiles::Allow));
    app.use_middleware(serve_static(&root).mount("/deny").dotfiles(Dotfiles::Deny).fallthrough(false));
    // A mount at the root answers every path that no other mount took.
    app.use_middleware(serve_static(&root));
    app.init().await.unwrap();
    app
}

/// One path segment, valid in a URI: real names, traversal, encodings and platform tricks.
fn segment() -> impl Strategy<Value = String> {
    let fixed = prop::sample::select(vec![
        "..", ".", "%2e%2e", "%2E%2E", ".%2e", "%2e.", "%2e", "%2f", "%2F", "%5c", "%5C", "%00", "%0a", "%25", "%252e%252e",
        "%c0%af", "%c0%ae", "%e0%80%af", "%ff", "C:", "c:", "D%3a", "ENV~1", "env~1", "GIT~1", "ENV~1.", "PUBLIC~1", "CON", "NUL",
        "aux", "COM1", "LPT1", "a.txt::$DATA", "a.txt:hidden", "a.txt%3Ahidden", "a.txt.", "a.txt%20", "%2eenv",
        ".env", ".env.", ".env%20", ".ENV", ".git", ".git%2fconfig", "config", "GIT~1.", "a.txt", "sub", "b.txt", "index.html",
        "outside", "secret.txt", "public", "static", "allow", "deny", "", "", "*", "?", "%3f", "%23", "~", "$", "..;", "...",
        "....//", "%2e%2e%2f", "%2e%2e%5c", "..%2f", "..%5c", "%2e%2e%2foutside%2fsecret.txt", "..%5coutside%5csecret.txt",
        "%C3%A9", "%E2%80%AE", r"\", r"..\outside\secret.txt", r"a.txt\", r"..\..\", r".env\", r"C:\x", r"\\host\share",
    ]);
    prop_oneof![
        8 => fixed.prop_map(String::from),
        2 => "([a-zA-Z0-9._~:$-]|%[0-9a-fA-F]{2}){0,6}",
    ]
}

fn request_path() -> impl Strategy<Value = String> {
    let mount = prop::sample::select(vec!["/static", "/allow", "/deny", "/mod", "/modplain", "/mod/api", "/static/", "/allow//", "/Static", "", "/"]);
    let segments = prop::collection::vec(segment(), 0..8);
    let query = prop::sample::select(vec!["", "", "", "?x=1", "?", "?a=../outside"]);
    (mount, segments, query).prop_map(|(m, s, q)| format!("{m}/{}{q}", s.join("/")))
}

/// The segments of a request path that is valid as a URI; the test client replaces an invalid one with `/`.
fn is_valid_uri(path: &str) -> bool {
    path.parse::<http::Uri>().is_ok()
}

async fn check(c: &TestClient, path: &str) -> Result<(), String> {
    for method in ["GET", "HEAD"] {
        let r = if method == "GET" { c.get(path).send().await } else { c.head(path).send().await };
        let text = r.text();
        let allow_mount = path.starts_with("/allow");
        if r.status() >= 500 {
            return Err(format!("{method} {path}: status {} body {text}", r.status()));
        }
        if text.contains("OUTSIDE ROOT") {
            return Err(format!("{method} {path}: leaked the file outside the folder"));
        }
        if !allow_mount && (text.contains("supersecret") || text.contains("token@") || text.contains("[core]")) {
            return Err(format!("{method} {path}: leaked a dotfile ({})", r.status()));
        }
    }
    Ok(())
}

fn run_property(strategy: impl Strategy<Value = String>) {
    let fx = Fixture::new("prop");
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let app = rt.block_on(build_app(&fx));
    let cases = std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(2000);
    let mut runner = TestRunner::new(config(cases));
    let result = runner.run(&strategy, |path| {
        // The test client turns an invalid URI into `/`, which would test nothing: a generator
        // that produces one is a bug of the test, not a case to skip.
        prop_assert!(is_valid_uri(&path), "the generator produced an invalid URI: {path:?}");
        let c = request(&app);
        rt.block_on(check(&c, &path)).map_err(TestCaseError::fail)
    });
    if let Err(e) = result {
        panic!("{e}");
    }
}

#[test]
fn static_paths_never_leak_and_never_answer_5xx() {
    run_property(request_path());
}

/// Fixed attack paths, so a regression shows up without the generator.
#[test]
fn known_hostile_paths() {
    let fx = Fixture::new("known");
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let app = rt.block_on(build_app(&fx));
    let paths = [
        "/static/../outside/secret.txt",
        "/static/%2e%2e%2foutside%2fsecret.txt",
        "/static/..%5coutside%5csecret.txt",
        "/static/C:/x",
        "/static/C:",
        "/static/%00",
        "/static/a.txt%00.png",
        "/static/a.txt::$DATA",
        "/static/CON",
        "/static/NUL",
        "/static/nul.txt",
        "/static/con/x",
        "/static/AUX",
        "/static/COM1",
        "/static/LPT1.txt",
        "/static/.env",
        "/static/ENV~1",
        "/static/GIT~1/config",
        "/deny/.env",
        "/deny/CON",
        "/deny/%00",
        "/allow/CON",
        "/allow/%00",
        "/allow/C:/x",
        "/allow/../outside/secret.txt",
        "/static//..//outside//secret.txt",
        r"/static/..\outside\secret.txt",
        r"/static/.env\",
        r"/deny/..\.env",
        r"/allow/..\..\outside\secret.txt",
        r"/mod/..\outside\secret.txt",
        "/mod/%00",
        "/mod/api/x",
        "/modplain/C:/x",
        "/../outside/secret.txt",
        "/%2e%2e/outside/secret.txt",
    ];
    rt.block_on(async {
        let c = request(&app);
        for p in paths {
            if let Err(e) = check(&c, p).await {
                panic!("{e}");
            }
        }
        assert_eq!(c.get("/static/a.txt").send().await.text(), "hello");
        assert_eq!(c.get("/static/sub/b.txt").send().await.text(), "bee");
    });
}

/// Failing seeds go to `proptest-regressions/fuzz_static.txt` (committed) and are replayed on every run.
fn config(cases: u32) -> ProptestConfig {
    ProptestConfig {
        cases,
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct("proptest-regressions/fuzz_static.txt"))),
        ..ProptestConfig::default()
    }
}
