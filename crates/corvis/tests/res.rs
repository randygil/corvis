//! `@Res()` / `@Res({ passthrough: true })` with Express semantics. Expected responses were
//! captured from a real express 5 app (`fixtures/res_express.oracle.mjs`, which
//! declares the same controller) into `fixtures/res_express.json`.

use std::time::Duration;

use corvis::prelude::*;
use corvis::testing::request;
use corvis::{CookieOptions, Method, Res, StatusCode};

#[controller("res")]
pub struct ResController;

#[routes]
impl ResController {
    #[get("redirect")]
    fn redirect(&self, #[res] res: Res) {
        res.set_header("Set-Cookie", "sid=1; Path=/; HttpOnly");
        res.redirect_with(302, "/?auth=expired");
    }

    #[get("redirect-enc")]
    fn redirect_enc(&self, #[res] res: Res) {
        res.redirect("/a b/ñ?x=<1>");
    }

    #[post("login")]
    #[http_code(204)]
    async fn login(&self, #[res(passthrough = true)] res: Res) {
        res.set_header("Set-Cookie", "sid=2; Path=/; HttpOnly; SameSite=Lax; Max-Age=60");
    }

    #[post("login-fail")]
    #[http_code(204)]
    async fn login_fail(&self, #[res(passthrough)] res: Res) -> Result<(), HttpException> {
        res.set_header("Set-Cookie", "try=1");
        Err(UnauthorizedException::new("Clave incorrecta"))
    }

    #[get("send-text")]
    fn send_text(&self, #[res] res: Res) {
        res.status(201).send("hola");
    }

    #[get("json")]
    fn json(&self, #[res] res: Res) {
        res.json(json!({ "a": 1, "b": [true, null] }));
    }

    #[get("end")]
    fn end(&self, #[res] res: Res) {
        res.set_header("Content-Type", "text/plain; charset=windows-1252");
        res.set_header("Content-Disposition", "attachment; filename=\"r.txt\"");
        res.end(vec![0x41u8, 0xd1]);
    }

    #[get("send-status")]
    fn send_status(&self, #[res] res: Res) {
        res.send_status(418);
    }

    #[get("cookie")]
    fn cookie(&self, #[res] res: Res) {
        res.cookie("a", "b c", CookieOptions::new().http_only(true).same_site("lax"));
        res.clear_cookie("old", CookieOptions::new());
        res.send("ok");
    }

    #[post("pass")]
    #[header("X-A", "1")]
    #[header("Content-Type", "application/json")]
    fn pass(&self, #[res(passthrough)] res: Res) -> Value {
        res.status(202);
        json!({ "ok": true })
    }

    #[get("later")]
    fn later(&self, #[res] res: Res) -> &'static str {
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            res.json(json!({ "late": 1 }));
        });
        "ignored"
    }

    #[get("attachment")]
    fn attachment(&self, #[res] res: Res) {
        res.attachment(Some("informe 2026.pdf"));
        res.send(b"xyz".to_vec());
    }

    #[get("buffer")]
    fn buffer(&self, #[res] res: Res) {
        res.send(b"raw".to_vec());
    }

    #[get("handled-throw")]
    #[header("X-H", "h")]
    fn handled_throw(&self, #[res] res: Res) -> Result<(), HttpException> {
        res.set_header("X-Before", "1");
        Err(UnauthorizedException::default().into())
    }
}

#[module(controllers = [ResController])]
pub struct ResModule;

#[corvis::test]
async fn res_matches_express() {
    let mut app = CorvisFactory::create::<ResModule>().await.unwrap();
    app.silence_route_logs();
    app.set_x_powered_by("Express");
    app.init().await.unwrap();
    let c = request(&app);

    let cases: Vec<Value> = serde_json::from_str(include_str!("fixtures/res_express.json")).unwrap();
    for case in cases {
        let method = case["method"].as_str().unwrap();
        let path = format!("/res/{}", case["path"].as_str().unwrap());
        let mut req = c.method(method.parse().unwrap(), &path);
        for (k, v) in case["headers"].as_object().unwrap() {
            req = req.header(k, v.as_str().unwrap());
        }
        let r = req.send().await;
        let label = format!("{method} {path} {}", case["headers"]);
        assert_eq!(r.status() as u64, case["status"].as_u64().unwrap(), "{label}: {}", r.text());
        assert_eq!(r.body.as_ref(), case["body"].as_str().unwrap().chars().map(|ch| ch as u8).collect::<Vec<_>>(), "{label}");

        let expected = case["h"].as_object().unwrap();
        let cookies: Vec<String> =
            r.headers.get_all("set-cookie").iter().map(|v| v.to_str().unwrap().to_string()).collect();
        assert_eq!(serde_json::to_value(&cookies).unwrap(), expected["set-cookie"], "{label}: set-cookie");
        let mut names: Vec<String> = r
            .headers
            .keys()
            .map(|k| k.as_str().to_string())
            .filter(|k| !["date", "set-cookie"].contains(&k.as_str()))
            .collect();
        names.dedup();
        let mut want: Vec<String> = expected.keys().filter(|k| *k != "set-cookie").cloned().collect();
        names.sort();
        want.sort();
        assert_eq!(names, want, "{label}: header names");
        for k in want {
            assert_eq!(r.header(&k), expected[&k].as_str(), "{label}: {k}");
        }
    }
}

#[corvis::test]
async fn res_helpers() {
    let res = Res::new(Method::GET, None, StatusCode::OK, corvis::ResMode::Handled);
    res.type_("json");
    assert_eq!(res.get("content-type").as_deref(), Some("application/json; charset=utf-8"));
    res.type_("text/csv");
    assert_eq!(res.get("Content-Type").as_deref(), Some("text/csv; charset=utf-8"));
    res.append("X-List", "a").append("x-list", "b");
    assert_eq!(res.get_header_values("x-list"), vec!["a", "b"]);
    assert_eq!(res.get("x-list").as_deref(), Some("a, b"));
    res.vary("Origin").vary("accept").vary("origin");
    assert_eq!(res.get("vary").as_deref(), Some("Origin, accept"));
    res.links(&[("next", "/p/2")]).links(&[("last", "/p/5")]);
    assert_eq!(res.get("link").as_deref(), Some("</p/2>; rel=\"next\", </p/5>; rel=\"last\""));
    res.cookie_json("cart", &json!({ "n": 1 }), CookieOptions::new());
    assert_eq!(res.get("set-cookie").as_deref(), Some("cart=j%3A%7B%22n%22%3A1%7D; Path=/"));
    res.remove_header("x-list");
    assert!(!res.has_header("x-list"));
    assert!(!res.headers_sent());
    res.end_empty();
    assert!(res.headers_sent() && res.writable_ended());

    // The `content-disposition` package.
    assert_eq!(corvis::http::res::content_disposition("informe ñ.pdf"), "attachment; filename=\"informe ñ.pdf\"");
    assert_eq!(corvis::http::res::content_disposition("a€b.pdf"), "attachment; filename=\"a?b.pdf\"; filename*=UTF-8''a%E2%82%ACb.pdf");
    assert_eq!(corvis::http::res::content_disposition("a%41.pdf"), "attachment; filename=\"a%41.pdf\"; filename*=UTF-8''a%2541.pdf");
    assert_eq!(corvis::http::res::content_disposition("dir/a\"b.pdf"), "attachment; filename=\"a\\\"b.pdf\"");
}

#[controller("plain")]
pub struct PlainController;

#[routes]
impl PlainController {
    /// Taking `Res` without `#[res]` is a 500 (the response object was not requested).
    #[get]
    fn get(&self, res: Res) -> Value {
        json!({ "status": res.status_code() })
    }
}

#[module(controllers = [PlainController])]
pub struct PlainModule;

#[corvis::test]
async fn res_without_attribute_is_an_error() {
    let mut app = CorvisFactory::create::<PlainModule>().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    let r = request(&app).get("/plain").send().await;
    assert_eq!(r.status(), 500);
}
