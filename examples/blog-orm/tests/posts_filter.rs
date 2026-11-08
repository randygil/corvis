//! `GET /posts?published=` is parsed with `z::stringbool()`: `false` must not select published posts.

use corvis::prelude::*;
use corvis::testing::{TestClient, request};

async fn titles(c: &TestClient, query: &str) -> Vec<String> {
    let r = c.get(&format!("/posts{query}")).send().await;
    assert_eq!(r.status, 200, "{query}: {}", r.text());
    let body = r.json_value();
    let rows = body.as_array().cloned().or_else(|| body["data"].as_array().cloned()).unwrap_or_else(|| panic!("unexpected body: {body}"));
    let mut t: Vec<String> = rows.iter().map(|p| p["title"].as_str().unwrap().to_string()).collect();
    t.sort();
    t
}

#[corvis::test]
async fn published_flag_is_read_as_a_text_flag() {
    // SAFETY: this test binary has a single test and sets the environment before the app reads it.
    unsafe {
        std::env::set_var("DATABASE_URL", "sqlite::memory:");
    }
    let mut app = blog_orm::create_app().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    let c = request(&app);

    let r = c.post("/users").json(&json!({ "email": "ana@corvis.dev", "name": "Ana" })).send().await;
    assert_eq!(r.status, 201, "{}", r.text());
    for (title, published) in [("Draft", false), ("Live", true)] {
        let r = c.post("/posts").json(&json!({ "title": title, "published": published, "authorEmail": "ana@corvis.dev" })).send().await;
        assert_eq!(r.status, 201, "{}", r.text());
    }

    assert_eq!(titles(&c, "?published=false").await, ["Draft"]);
    assert_eq!(titles(&c, "?published=0").await, ["Draft"]);
    assert_eq!(titles(&c, "?published=true").await, ["Live"]);
    assert_eq!(titles(&c, "?published=yes").await, ["Live"]);
    assert_eq!(titles(&c, "").await, ["Draft", "Live"]);
    let r = c.get("/posts?published=maybe").send().await;
    assert_eq!(r.status, 400, "{}", r.text());
}
