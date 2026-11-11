//! The mock OAuth2 provider is dev tooling, but people copy it: `/authorize` sends the browser only to the redirect URI the
//! client registered, refuses control characters, and encodes the `state` it puts in the `Location` header.

mod common;

use auth_app::Settings;
use common::http_get;

const REDIRECT: &str = "http%3A%2F%2F127.0.0.1%3A3000%2Fauth%2Fprovider%2Fcallback";

fn authorize(redirect: &str, state: &str) -> String {
    format!(
        "/authorize?response_type=code&client_id=auth-app-demo&code_challenge_method=S256&code_challenge=abc&state={state}&redirect_uri={redirect}"
    )
}

#[corvis::test]
async fn the_registered_redirect_uri_is_accepted() {
    let settings = Settings::dev(3000).await.unwrap();
    let (status, location, _) = http_get(settings.mock().unwrap().addr(), &authorize(REDIRECT, "s-1_2")).await;
    assert_eq!(status, 302);
    let location = location.unwrap();
    assert!(location.starts_with("http://127.0.0.1:3000/auth/provider/callback?code="), "{location}");
    assert!(location.ends_with("&state=s-1_2"), "{location}");
}

#[corvis::test]
async fn another_redirect_uri_is_refused() {
    let settings = Settings::dev(3000).await.unwrap();
    let addr = settings.mock().unwrap().addr();
    for evil in [
        "https%3A%2F%2Fevil.test%2Fsteal",
        "http%3A%2F%2F127.0.0.1%3A3000%2Fauth%2Fprovider%2Fcallback%2Fx",
        "http%3A%2F%2F127.0.0.1%3A3000%2Fauth%2Fprovider%2Fcallback%3Fnext%3Dhttps%3A%2F%2Fevil.test",
        "http%3A%2F%2F127.0.0.1%3A3001%2Fauth%2Fprovider%2Fcallback",
    ] {
        let (status, location, _) = http_get(addr, &authorize(evil, "s")).await;
        assert_eq!(status, 400, "{evil}");
        assert!(location.is_none(), "{evil}");
    }
}

#[corvis::test]
async fn control_characters_cannot_inject_headers() {
    let settings = Settings::dev(3000).await.unwrap();
    let addr = settings.mock().unwrap().addr();
    // A CR LF in the state, and in the redirect URI.
    let (status, location, body) = http_get(addr, &authorize(REDIRECT, "x%0D%0ASet-Cookie:%20injected=1")).await;
    assert_eq!(status, 400);
    assert!(location.is_none() && !body.contains("injected"));
    let (status, _, _) = http_get(addr, &authorize(&format!("{REDIRECT}%0D%0AX-Evil:%201"), "s")).await;
    assert_eq!(status, 400);
}

#[corvis::test]
async fn the_state_is_encoded_in_the_location() {
    let settings = Settings::dev(3000).await.unwrap();
    let (status, location, _) = http_get(settings.mock().unwrap().addr(), &authorize(REDIRECT, "a%26b%20c%23d")).await;
    assert_eq!(status, 302);
    let location = location.unwrap();
    // `&`, space and `#` stay encoded: the state cannot add a parameter or end the query.
    assert!(location.ends_with("&state=a%26b%20c%23d"), "{location}");
}
