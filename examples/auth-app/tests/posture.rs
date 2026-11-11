//! How the strategies combine: an optional guard that personalizes a public route, a role required after the guard that
//! authenticates, and route posture enforced at startup.

mod common;

use common::*;
use corvis::prelude::*;
use corvis::{PostureEntry, PostureKind, PostureStatus, PublicSource, RoutePostureReport};

fn find<'a>(report: &'a RoutePostureReport, path: &str, method: RequestMethod) -> &'a PostureEntry {
    report
        .entries()
        .iter()
        .find(|e| e.kind == PostureKind::Http { method } && e.path == path)
        .unwrap_or_else(|| panic!("no entry for {method:?} {path}:\n{report}"))
}

#[corvis::test]
async fn an_optional_guard_personalizes_a_public_route() {
    let h = Harness::start().await;
    let c = h.client();
    let anonymous = c.get("/feed").send().await;
    assert_eq!(anonymous.status, 200);
    assert_eq!(anonymous.json_value(), json!({ "message": "Hello, stranger" }));

    let token = token_of(&c, "maria", "maria-demo-pass").await;
    let named = c.get("/feed").bearer(&token).send().await;
    assert_eq!(named.status, 200);
    assert_eq!(named.json_value(), json!({ "message": "Hello, maria" }));

    // A wrong token is not "no credentials": it is a failed attempt, and the optional guard lets it through as
    // anonymous only because every strategy failed; the route never says who the caller is.
    let bad = c.get("/feed").bearer("not-a-token").send().await;
    assert_eq!(bad.status, 200);
    assert_eq!(bad.json_value(), json!({ "message": "Hello, stranger" }));

    // The session cookie is the other credential of the chain.
    let cookie = session_of(&c, "john", "john-demo-pass").await;
    assert_eq!(c.get("/feed").header("cookie", &cookie).send().await.json_value(), json!({ "message": "Hello, john" }));
}

#[corvis::test]
async fn a_role_is_required_after_the_user_is_authenticated() {
    let h = Harness::start().await;
    let c = h.client();

    let anonymous = c.get("/admin/users").send().await;
    assert_eq!(anonymous.status, 401);

    let maria = token_of(&c, "maria", "maria-demo-pass").await;
    let user = c.get("/admin/users").bearer(&maria).send().await;
    assert_eq!(user.status, 403, "{}", user.text());

    let john = token_of(&c, "john", "john-demo-pass").await;
    let admin = c.get("/admin/users").bearer(&john).send().await;
    assert_eq!(admin.status, 200, "{}", admin.text());
    let names: Vec<String> = admin.json_value().as_array().unwrap().iter().map(|u| u["username"].as_str().unwrap().to_string()).collect();
    assert!(names.contains(&"john".to_string()) && names.contains(&"maria".to_string()));
    assert!(!admin.text().contains("argon2") && !admin.text().contains("@example.test"), "no hash or e-mail address is listed");

    // The chain accepts the session as well.
    let cookie = session_of(&c, "john", "john-demo-pass").await;
    assert_eq!(c.get("/admin/users").header("cookie", &cookie).send().await.status, 200);
    let maria_cookie = session_of(&c, "maria", "maria-demo-pass").await;
    assert_eq!(c.get("/admin/users").header("cookie", &maria_cookie).send().await.status, 403);
}

#[corvis::test]
async fn every_route_is_guarded_or_marked_public() {
    // `create_app` enforces route posture: `init()` in the harness would have failed otherwise.
    let h = Harness::start().await;
    let report = h.app.route_posture().expect("the report exists after init()");
    assert!(report.unprotected().is_empty(), "{report}");

    let home = find(&report, "/", RequestMethod::Get);
    assert_eq!((&home.status, home.public_source), (&PostureStatus::Public, Some(PublicSource::Handler)));
    // The optional guard does not count as access control: the route is public because it says so.
    let feed = find(&report, "/feed", RequestMethod::Get);
    assert_eq!((&feed.status, feed.public_source), (&PostureStatus::Public, Some(PublicSource::Handler)));
    // The login routes are protected by the guard of their strategy.
    for (path, method) in [("/auth/login", RequestMethod::Post), ("/session/login", RequestMethod::Post), ("/auth/provider", RequestMethod::Get), ("/auth/provider/callback", RequestMethod::Get), ("/me", RequestMethod::Get)] {
        assert_eq!(find(&report, path, method).status, PostureStatus::Protected, "{path}");
    }
    // `RolesGuard` and the throttler are not access control: the `AuthGuard` before them is.
    let admin = find(&report, "/admin/users", RequestMethod::Get);
    let access: Vec<&str> = admin.guards.iter().filter(|g| g.controls_access).map(|g| g.name.as_str()).collect();
    assert_eq!(access, ["AuthGuard"]);
}
