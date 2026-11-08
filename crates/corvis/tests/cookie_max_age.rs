//! `max_age` of both cookie APIs is in milliseconds and writes the same `Max-Age`.

use corvis::http::res::serialize_cookie;
use corvis::{Cookie, CookieOptions};

#[test]
fn same_value_same_attribute() {
    let a = serialize_cookie("m", "1", &CookieOptions::new().max_age(90_500));
    let b = Cookie::new("m", "1").max_age(90_500).to_string();
    assert!(a.contains("; Max-Age=90;"), "{a}");
    assert_eq!(b, "m=1; Path=/; Max-Age=90");
}

#[test]
fn one_hour() {
    let c = Cookie::new("theme", "dark").max_age(3_600_000).to_string();
    assert_eq!(c, "theme=dark; Path=/; Max-Age=3600");
}

#[test]
fn negative_value_rounds_down() {
    let c = Cookie::new("sid", "").max_age(-1).to_string();
    assert_eq!(c, "sid=; Path=/; Max-Age=-1");
    assert!(serialize_cookie("sid", "", &CookieOptions::new().max_age(-1)).contains("Max-Age=-1;"));
}
