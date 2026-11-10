//! Login with a session cookie: the `local` strategy checks the credentials, the response sets a session cookie, the
//! user opens `/session/me` with it, a second login changes the session identifier, and a logout ends the session so
//! the old cookie is anonymous.

use corvis::prelude::*;
use corvis::testing::request;

async fn app() -> CorvisApplication {
    // SAFETY: set before the application reads the environment; this binary has a single test.
    unsafe {
        std::env::set_var("JWT_SECRET", "cats-test-secret-0123456789abcdef");
        std::env::set_var("SESSION_SECRET", "cats-session-secret-0123456789abcdef");
    }
    let mut app = cats_app::create_app().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    app
}

fn cookie_of(response: &corvis::testing::TestResponse) -> String {
    let header = response.headers.get("set-cookie").expect("a session cookie").to_str().unwrap();
    header.split(';').next().unwrap().to_string()
}

#[corvis::test]
async fn a_session_login_opens_the_protected_route_and_a_logout_closes_it() {
    let app = app().await;
    let c = request(&app);
    let credentials = json!({ "username": "john", "password": "changeme" });

    // Anonymous: the plain 401, and the session cookie only after a login.
    let anonymous = c.get("/api/v1/session/me").send().await;
    assert_eq!(anonymous.status, 401);
    assert!(anonymous.headers.get("set-cookie").is_none());

    let wrong = c.post("/api/v1/session/login").json(&json!({ "username": "john", "password": "wrong-password" })).send().await;
    assert_eq!(wrong.status, 401);
    assert!(wrong.headers.get("set-cookie").is_none());

    let login = c.post("/api/v1/session/login").json(&credentials).send().await;
    assert_eq!(login.status, 200, "{}", login.text());
    let header = login.headers.get("set-cookie").unwrap().to_str().unwrap().to_string();
    assert!(
        header.starts_with("connect.sid=s%3A")
            && header.contains("HttpOnly")
            && header.contains("Secure")
            && header.contains("SameSite=Lax"),
        "{header}"
    );
    let first = cookie_of(&login);

    let me = c.get("/api/v1/session/me").header("cookie", &first).send().await;
    assert_eq!(me.status, 200, "{}", me.text());
    assert_eq!(me.json_value(), json!({ "user_id": 1, "username": "john" }));
    assert!(!me.text().contains("argon2"), "the hash never reaches a response");

    // Logging in again with the cookie changes the identifier, and the old cookie is anonymous.
    let again = c.post("/api/v1/session/login").header("cookie", &first).json(&credentials).send().await;
    assert_eq!(again.status, 200);
    let second = cookie_of(&again);
    assert_ne!(second, first);
    assert_eq!(c.get("/api/v1/session/me").header("cookie", &first).send().await.status, 401);
    assert_eq!(c.get("/api/v1/session/me").header("cookie", &second).send().await.status, 200);

    // The token routes are not opened by the cookie.
    assert_eq!(c.get("/api/v1/auth/profile").header("cookie", &second).send().await.status, 401);

    // Logout destroys the session: its cookie is anonymous from then on.
    let out = c.post("/api/v1/session/logout").header("cookie", &second).send().await;
    assert_eq!(out.status, 204, "{}", out.text());
    assert_eq!(c.get("/api/v1/session/me").header("cookie", &second).send().await.status, 401);
    assert_eq!(c.get("/api/v1/session/me").header("cookie", &cookie_of(&out)).send().await.status, 401);
}

#[corvis::test]
async fn a_login_from_another_site_is_refused_before_the_credentials_are_checked() {
    let app = app().await;
    let c = request(&app);
    let form = [("username", "john"), ("password", "changeme")];

    // The attack: a cross-site form POST. The browser says so with Sec-Fetch-Site, and sends Origin.
    let by_fetch_metadata = c
        .post("/api/v1/session/login")
        .header("host", "cats.test")
        .header("sec-fetch-site", "cross-site")
        .header("origin", "https://evil.test")
        .form(&form)
        .send()
        .await;
    assert_eq!(by_fetch_metadata.status, 403, "{}", by_fetch_metadata.text());
    assert!(by_fetch_metadata.headers.get("set-cookie").is_none(), "no session cookie reaches the victim");

    // A browser without Sec-Fetch-Site: the Origin is not the host.
    let by_origin =
        c.post("/api/v1/session/login").header("host", "cats.test").header("origin", "https://evil.test").form(&form).send().await;
    assert_eq!(by_origin.status, 403);
    assert!(by_origin.headers.get("set-cookie").is_none());

    // A sibling subdomain is `same-site`: still another site for a login (the login route is throttled to 5 a minute, so unknown values are covered by the unit tests).
    for site in ["same-site"] {
        let r = c
            .post("/api/v1/session/login")
            .header("host", "cats.test")
            .header("sec-fetch-site", site)
            .header("origin", "https://evil.cats.test")
            .form(&form)
            .send()
            .await;
        assert_eq!(r.status, 403, "{site}: {}", r.text());
        assert!(r.headers.get("set-cookie").is_none(), "{site}");
    }

    // The same login from the site itself, and from a client that sends neither header.
    let same_origin = c
        .post("/api/v1/session/login")
        .header("host", "cats.test")
        .header("sec-fetch-site", "same-origin")
        .header("origin", "http://cats.test")
        .form(&form)
        .send()
        .await;
    assert_eq!(same_origin.status, 200, "{}", same_origin.text());
    assert!(same_origin.headers.get("set-cookie").is_some());
    let plain = c.post("/api/v1/session/login").form(&form).send().await;
    assert_eq!(plain.status, 200, "{}", plain.text());
}
