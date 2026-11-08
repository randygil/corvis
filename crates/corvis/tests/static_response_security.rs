//! Regression tests for the static files / responses security fixes (8.3 names, ADS, links,
//! `ServeStaticModule` dotfiles, streamed reads, SSE framing, outgoing cookies,
//! `StreamableFile::filename` and `Response::redirect`).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use corvis::prelude::*;
use corvis::http::res::serialize_cookie;
use corvis::testing::{Test, request};
use corvis::{Body, CookieOptions, Dotfiles, Res, serve_static};
use futures::StreamExt;

static SEQ: AtomicUsize = AtomicUsize::new(0);

/// A served folder plus a folder outside of it:
/// `root/{.env, .git/config, a.txt, index.html, sub/b.txt, big.bin}` and `outside/secret.txt`.
struct Fixture {
    base: PathBuf,
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let base = std::env::temp_dir().join(format!("corvis-sr-{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("public");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::create_dir_all(base.join("outside")).unwrap();
        std::fs::write(root.join(".env"), "DB_PASSWORD=supersecret").unwrap();
        std::fs::write(root.join(".git").join("config"), "[core]\nurl = https://user:token@example.com").unwrap();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        std::fs::write(root.join("index.html"), "<h1>index</h1>").unwrap();
        std::fs::write(root.join("sub").join("b.txt"), "bee").unwrap();
        std::fs::write(root.join("big.bin"), big_content()).unwrap();
        std::fs::write(base.join("outside").join("secret.txt"), "OUTSIDE ROOT SECRET").unwrap();
        Fixture { base, root }
    }

    /// `root/link` → `outside` (a junction on Windows, a symlink elsewhere).
    fn link_outside(&self) -> bool {
        let link = self.root.join("link");
        let target = self.base.join("outside");
        #[cfg(windows)]
        {
            std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&link)
                .arg(&target)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&target, &link).is_ok()
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn big_content() -> Vec<u8> {
    (0..1_048_576u32).map(|i| (i % 251) as u8).collect()
}

#[controller("api")]
pub struct ApiController;

#[routes]
impl ApiController {
    #[get("bad-cookie")]
    fn bad_cookie(&self) -> Response {
        Response::text("ok").with_cookie(Cookie::new("sid", "abc").path("/; Domain=evil.example"))
    }
    #[get("bad-res-cookie")]
    fn bad_res_cookie(&self, #[res] res: Res) {
        res.cookie("sid", "abc", CookieOptions::new().domain("example.com; Max-Age=99999999"));
        res.send("ok");
    }
    #[get("redirect")]
    fn redirect(&self) -> Redirect {
        Redirect::to("/a b<x>\"")
    }
}

async fn app_with(fx: &Fixture, configure: impl FnOnce(&mut CorvisApplication, &Path)) -> CorvisApplication {
    let module = Test::create_testing_module()
        .controller::<ApiController>()
        .import_dynamic(ServeStaticModule::for_root(vec![
            ServeStaticOptions::new(fx.root.to_string_lossy()).serve_root("/mod"),
        ]))
        .with_logger(false)
        .compile()
        .await
        .unwrap();
    let mut app = module.create_corvis_application();
    configure(&mut app, &fx.root);
    app.init().await.unwrap();
    app
}

async fn default_app(fx: &Fixture) -> CorvisApplication {
    app_with(fx, |app, root| {
        app.use_middleware(serve_static(root).mount("/static"));
        app.use_middleware(serve_static(root).mount("/allow").dotfiles(Dotfiles::Allow));
        app.use_middleware(serve_static(root).mount("/deny").dotfiles(Dotfiles::Deny).fallthrough(false));
        app.use_middleware(serve_static(root).mount("/nolinks").follow_symlinks(false));
    })
    .await
}

/// static-response #1: `ENV~1` / `GIT~1` are the 8.3 aliases of `.env` / `.git`.
#[cfg(windows)]
#[corvis::test]
async fn serve_static_short_names_do_not_bypass_dotfiles() {
    let fx = Fixture::new("short");
    if std::fs::metadata(fx.root.join("ENV~1")).is_err() {
        eprintln!("8.3 name generation is disabled on this volume; skipping");
        return;
    }
    let app = default_app(&fx).await;
    let c = request(&app);
    c.get("/static/.env").send().await.expect_status(404);
    for p in ["/static/ENV~1", "/static/env~1", "/static/GIT~1/config", "/static/ENV~1.", "/mod/ENV~1", "/mod/GIT~1/config"] {
        let r = c.get(p).send().await;
        assert_eq!(r.status(), 404, "{p}: {}", r.text());
        assert!(!r.text().contains("supersecret") && !r.text().contains("token"), "{p}");
    }
    c.get("/deny/ENV~1").send().await.expect_status(403);
    c.get("/deny/GIT~1/config").send().await.expect_status(403);
    // A directory alias does not even reveal the folder with a redirect.
    c.get("/static/GIT~1").send().await.expect_status(404);
    // `Dotfiles::Allow` keeps serving them; ordinary short names still work.
    assert_eq!(c.get("/allow/ENV~1").send().await.text(), "DB_PASSWORD=supersecret");
    assert_eq!(c.get("/static/INDEX~1.HTM").send().await.status(), 200);
}

/// static-response #7: NTFS alternate data streams.
#[cfg(windows)]
#[corvis::test]
async fn serve_static_rejects_alternate_data_streams() {
    let fx = Fixture::new("ads");
    std::fs::write(fx.root.join("a.txt:hidden"), "ads-content").unwrap();
    let app = default_app(&fx).await;
    let c = request(&app);
    for p in ["/static/a.txt:hidden", "/static/a.txt::$DATA", "/static/a.txt%3Ahidden", "/allow/a.txt:hidden", "/mod/a.txt:hidden"] {
        let r = c.get(p).send().await;
        assert_eq!(r.status(), 404, "{p}: {}", r.text());
    }
    c.get("/deny/a.txt:hidden").send().await.expect_status(404);
    assert_eq!(c.get("/static/a.txt").send().await.text(), "hello");
}

/// static-response #2: `ServeStaticModule` reuses `serve_static` (dotfiles, Range, ETag).
#[corvis::test]
async fn serve_static_module_ignores_dotfiles() {
    let fx = Fixture::new("module");
    let app = default_app(&fx).await;
    let c = request(&app);
    for p in ["/mod/.env", "/mod/.git/config", "/mod/sub/../.env", "/mod/%2eenv"] {
        let r = c.get(p).send().await;
        assert_eq!(r.status(), 404, "{p}: {}", r.text());
        assert!(!r.text().contains("supersecret") && !r.text().contains("token"), "{p}");
    }
    let r = c.get("/mod/a.txt").send().await;
    r.expect_status(200);
    assert_eq!(r.text(), "hello");
    assert_eq!(r.header("accept-ranges"), Some("bytes"));
    assert!(r.header("etag").is_some_and(|e| e.starts_with("W/\"5-")));
    let r = c.get("/mod/a.txt").header("range", "bytes=1-3").send().await;
    r.expect_status(206);
    assert_eq!(r.text(), "ell");
    assert_eq!(c.get("/mod/sub/b.txt").send().await.text(), "bee");
    assert_eq!(c.get("/mod/").send().await.text(), "<h1>index</h1>");
    c.get("/mod/missing.txt").send().await.expect_status(404);
}

/// static-response #8: `follow_symlinks(false)` keeps requests inside the canonical root.
#[corvis::test]
async fn serve_static_follow_symlinks_false_confines_to_root() {
    let fx = Fixture::new("links");
    if !fx.link_outside() {
        eprintln!("cannot create a link here; skipping");
        return;
    }
    let app = default_app(&fx).await;
    let c = request(&app);
    // Default: Express follows links (parity).
    assert_eq!(c.get("/static/link/secret.txt").send().await.text(), "OUTSIDE ROOT SECRET");
    let r = c.get("/nolinks/link/secret.txt").send().await;
    assert_eq!(r.status(), 404, "{}", r.text());
    c.get("/nolinks/link/").send().await.expect_status(404);
    assert_eq!(c.get("/nolinks/sub/b.txt").send().await.text(), "bee");
}

/// static-response #3: ranges and full bodies are read from disk in pieces (see the unit test
/// in `serve_static.rs` for the streamed body); the bytes must be exact.
#[corvis::test]
async fn serve_static_ranges_are_exact() {
    let fx = Fixture::new("range");
    let app = default_app(&fx).await;
    let c = request(&app);
    let big = big_content();
    let r = c.get("/static/big.bin").header("range", "bytes=1000-1009").send().await;
    r.expect_status(206);
    assert_eq!(r.header("content-length"), Some("10"));
    assert_eq!(r.header("content-range"), Some("bytes 1000-1009/1048576"));
    assert!(r.body[..] == big[1000..1010]);
    let r = c.get("/static/big.bin").header("range", "bytes=-70000").send().await;
    assert!(r.body[..] == big[big.len() - 70000..]);
    let r = c.get("/static/big.bin").send().await;
    r.expect_status(200);
    assert_eq!(r.header("content-length"), Some("1048576"));
    assert!(r.body[..] == big[..]);
    let r = c.head("/static/big.bin").header("range", "bytes=0-0").send().await;
    r.expect_status(206);
    assert_eq!(r.header("content-length"), Some("1"));
    assert!(r.body.is_empty());
    let r = c.get("/mod/big.bin").header("range", "bytes=5-6").send().await;
    assert!(r.body[..] == big[5..7]);
}

/// static-response #3: `StreamableFile::from_path` no longer loads the whole file.
#[corvis::test]
async fn streamable_file_from_path_streams() {
    let fx = Fixture::new("sfile");
    let path = fx.root.join("big.bin");
    let resp = StreamableFile::from_path(&path).await.unwrap().into_response().unwrap();
    assert_eq!(resp.default_headers().get("content-length").unwrap(), "1048576");
    let mut resp = resp;
    let body = std::mem::replace(resp.body_mut(), Body::Empty);
    let Body::Stream { mut stream, content_type } = body else { panic!("expected a streamed body, got {body:?}") };
    assert_eq!(content_type, "application/octet-stream");
    let mut got = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.unwrap();
        assert!(chunk.len() <= 256 * 1024);
        got.extend_from_slice(&chunk);
    }
    assert!(got == big_content());
    assert!(StreamableFile::from_path(fx.root.join("sub")).await.is_err());
    assert!(StreamableFile::from_path(fx.root.join("nope")).await.is_err());
}

/// static-response #4: SSE framing (`data` split on `\r\n`, `\r` and `\n`; CR/LF stripped from fields).
#[test]
fn sse_splits_cr_and_strips_newlines_from_fields() {
    let e = MessageEvent::new("hola\r\revent: admin\rdata: {\"cmd\":\"logout\"}\r");
    assert_eq!(e.to_frame(), "data: hola\ndata: \ndata: event: admin\ndata: data: {\"cmd\":\"logout\"}\ndata: \n\n");
    let e = MessageEvent::new("a\r\nb\nc").event("chat\ndata: injected\n\nevent: admin").id("1\r\n2");
    assert_eq!(e.to_frame(), "event: chatdata: injectedevent: admin\nid: 12\ndata: a\ndata: b\ndata: c\n\n");
    let e = MessageEvent::new(json!({ "a": 1 })).event("tick").id("7").retry(10);
    assert_eq!(e.to_frame(), "event: tick\nid: 7\nretry: 10\ndata: {\"a\":1}\n\n");
}

fn panic_message(f: impl FnOnce() + std::panic::UnwindSafe) -> Option<String> {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let r = std::panic::catch_unwind(f);
    std::panic::set_hook(prev);
    r.err().map(|p| {
        p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default()
    })
}

/// static-response #5 / auth-ws #11: outgoing cookie attributes are validated like cookie 0.7.
#[test]
fn outgoing_cookie_attributes_are_validated() {
    let err = |o: CookieOptions| panic_message(move || drop(serialize_cookie("sid", "abc", &o)));
    assert_eq!(err(CookieOptions::new().path("/; Domain=evil.example; SameSite=None")).as_deref(), Some("TypeError: option path is invalid"));
    assert_eq!(err(CookieOptions::new().domain("example.com; Max-Age=99999999")).as_deref(), Some("TypeError: option domain is invalid"));
    assert_eq!(err(CookieOptions::new().domain("a..b")).as_deref(), Some("TypeError: option domain is invalid"));
    assert_eq!(err(CookieOptions::new().domain(".example.com").path("/app/x y")), None);
    assert_eq!(serialize_cookie("sid", "abc", &CookieOptions::new().domain(".Example.com")), "sid=abc; Domain=.Example.com; Path=/");
    // cookie 0.7 name/value grammar.
    assert_eq!(panic_message(|| drop(serialize_cookie("a;b", "x", &CookieOptions::new()))).as_deref(), Some("TypeError: argument name is invalid"));
    assert_eq!(panic_message(|| drop(serialize_cookie("a=b", "x", &CookieOptions::new()))).as_deref(), Some("TypeError: argument name is invalid"));
    assert_eq!(serialize_cookie("a@b", "\"q\"", &CookieOptions::new().raw(true)), "a@b=\"q\"; Path=/");

    // The `Cookie` struct of `Response::with_cookie`.
    let bad = |c: Cookie| panic_message(move || drop(c.to_string()));
    assert_eq!(bad(Cookie::new("sid", "abc").path("/; Domain=evil.example")).as_deref(), Some("TypeError: option path is invalid"));
    assert_eq!(bad(Cookie::new("sid", "abc").domain("evil.example; Secure")).as_deref(), Some("TypeError: option domain is invalid"));
    assert_eq!(bad(Cookie::new("sid", "abc").same_site("None; Partitioned")).as_deref(), Some("TypeError: option sameSite is invalid"));
    assert_eq!(bad(Cookie::new("sid=x; Domain=evil", "abc")).as_deref(), Some("TypeError: argument name is invalid"));
    assert!(Cookie::new("sid", "abc").path("/; x").try_to_string().is_err());
    assert_eq!(
        Cookie::new("sid", "a b").domain("app.example").max_age(60).http_only(true).secure(true).same_site("lax").to_string(),
        "sid=a%20b; Path=/; Domain=app.example; Max-Age=60; HttpOnly; Secure; SameSite=lax"
    );
}

#[corvis::test]
async fn invalid_cookies_answer_500_without_the_header() {
    let fx = Fixture::new("cookie");
    let app = default_app(&fx).await;
    let c = request(&app);
    for p in ["/api/bad-cookie", "/api/bad-res-cookie"] {
        let r = c.get(p).send().await;
        assert_eq!(r.status(), 500, "{p}");
        assert!(r.header("set-cookie").is_none(), "{p}");
    }
}

/// static-response #6.
#[test]
fn streamable_file_filename_is_escaped() {
    let f = StreamableFile::new(b"x".to_vec()).filename("a.pdf\"; filename*=UTF-8''evil.html");
    assert_eq!(f.get_headers().1.as_deref(), Some("attachment; filename=\"a.pdf\\\"; filename*=UTF-8''evil.html\""));
    let f = StreamableFile::new(b"x".to_vec()).filename("informe.pdf");
    assert_eq!(f.get_headers().1.as_deref(), Some("attachment; filename=\"informe.pdf\""));
    let f = StreamableFile::new(b"x".to_vec()).filename("año.pdf");
    assert_eq!(f.get_headers().1.as_deref(), Some("attachment; filename=\"año.pdf\""));
    let f = StreamableFile::new(b"x".to_vec()).filename("€.pdf");
    assert_eq!(f.get_headers().1.as_deref(), Some("attachment; filename=\"?.pdf\"; filename*=UTF-8''%E2%82%AC.pdf"));
}

/// static-response #10: `Response::redirect` / `Redirect::to` use `encodeurl` like `res.redirect`.
#[corvis::test]
async fn redirect_location_is_encoded() {
    let r = Response::redirect("/a b<x>\"", 302);
    assert_eq!(r.headers().get("location").unwrap(), "/a%20b%3Cx%3E%22");
    let r = Response::redirect("/x\r\nSet-Cookie: a=b", 302);
    assert_eq!(r.headers().get("location").unwrap(), "/x%0D%0ASet-Cookie:%20a=b");
    let r = Response::redirect("/?q=%20ñ", 301);
    assert_eq!(r.headers().get("location").unwrap(), "/?q=%20%C3%B1");
    let fx = Fixture::new("redirect");
    let app = default_app(&fx).await;
    let r = request(&app).get("/api/redirect").send().await;
    r.expect_status(302);
    assert_eq!(r.header("location"), Some("/a%20b%3Cx%3E%22"));
}
