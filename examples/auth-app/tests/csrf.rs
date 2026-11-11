//! A cookie is an ambient credential: a login or a logout that a browser makes on behalf of another site is refused
//! before the credentials are checked (`SameSite=Lax` does not stop a cross-site top-level form `POST`).

mod common;

use common::*;

#[corvis::test]
async fn a_login_from_another_site_is_refused_before_the_credentials_are_checked() {
    let h = Harness::start().await;
    let c = h.client();
    let form = [("username", "john"), ("password", "john-demo-pass")];

    // The attack: a cross-site form POST. The browser says so with Sec-Fetch-Site, and sends Origin.
    let by_fetch_metadata = c
        .post("/session/login")
        .header("host", "app.test")
        .header("sec-fetch-site", "cross-site")
        .header("origin", "https://evil.test")
        .form(&form)
        .send()
        .await;
    assert_eq!(by_fetch_metadata.status, 403, "{}", by_fetch_metadata.text());
    assert!(by_fetch_metadata.headers.get("set-cookie").is_none(), "no session cookie reaches the victim");

    // A browser without Sec-Fetch-Site: the Origin is not the host.
    let by_origin = c.post("/session/login").header("host", "app.test").header("origin", "https://evil.test").form(&form).send().await;
    assert_eq!(by_origin.status, 403);
    assert!(by_origin.headers.get("set-cookie").is_none());

    // A sibling subdomain is `same-site`, and still another site for a login.
    let sibling = c
        .post("/session/login")
        .header("host", "app.test")
        .header("sec-fetch-site", "same-site")
        .header("origin", "https://evil.app.test")
        .form(&form)
        .send()
        .await;
    assert_eq!(sibling.status, 403);

    // The same login from the site itself, and from a client that sends neither header (curl).
    let same_origin = c
        .post("/session/login")
        .header("host", "app.test")
        .header("sec-fetch-site", "same-origin")
        .header("origin", "http://app.test")
        .form(&form)
        .send()
        .await;
    assert_eq!(same_origin.status, 200, "{}", same_origin.text());
    assert!(same_origin.headers.get("set-cookie").is_some());
    assert_eq!(c.post("/session/login").form(&form).send().await.status, 200);
}

#[corvis::test]
async fn a_logout_from_another_site_keeps_the_session() {
    let h = Harness::start().await;
    let c = h.client();
    let cookie = session_of(&c, "john", "john-demo-pass").await;
    let attack = c.post("/session/logout").header("cookie", &cookie).header("sec-fetch-site", "cross-site").send().await;
    assert_eq!(attack.status, 403);
    assert_eq!(c.get("/session/me").header("cookie", &cookie).send().await.status, 200);
}
