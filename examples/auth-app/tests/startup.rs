//! Without `--dev` there are no default secrets: a missing or short one stops the application, and the error names the
//! variable and never prints its value.

use std::collections::HashMap;

use auth_app::Settings;

const SECRET: &str = "0123456789abcdef0123456789abcdef";

fn valid() -> HashMap<String, String> {
    [
        ("JWT_SECRET", SECRET),
        ("SESSION_SECRET", "1123456789abcdef0123456789abcdef"),
        ("OAUTH_STATE_SECRET", "2123456789abcdef0123456789abcdef"),
        ("SERVICE_API_KEY", "3123456789abcdef0123456789abcdef"),
        ("OAUTH_AUTHORIZE_URL", "https://idp.example/authorize"),
        ("OAUTH_TOKEN_URL", "https://idp.example/token"),
        ("OAUTH_USERINFO_URL", "https://idp.example/userinfo"),
        ("OAUTH_CLIENT_ID", "client"),
        ("OAUTH_CLIENT_SECRET", "client-secret"),
        ("OAUTH_REDIRECT_URI", "https://app.example/auth/provider/callback"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

fn load(vars: &HashMap<String, String>) -> corvis::Result<Settings> {
    Settings::from_lookup(|name| vars.get(name).cloned())
}

#[test]
fn a_complete_environment_starts() {
    let settings = load(&valid()).unwrap();
    assert!(!settings.dev);
    assert!(settings.mock().is_none(), "the mock provider is never started outside --dev");
}

#[test]
fn a_missing_secret_names_the_variable() {
    for name in ["JWT_SECRET", "SESSION_SECRET", "OAUTH_STATE_SECRET", "SERVICE_API_KEY"] {
        let mut vars = valid();
        vars.remove(name);
        let err = format!("{:#}", load(&vars).unwrap_err());
        assert!(err.contains(name), "{name}: {err}");
        // An empty value is a missing one.
        vars.insert(name.to_string(), String::new());
        assert!(format!("{:#}", load(&vars).unwrap_err()).contains(name));
    }
}

#[test]
fn a_missing_oauth_setting_names_the_variable() {
    for name in
        ["OAUTH_AUTHORIZE_URL", "OAUTH_TOKEN_URL", "OAUTH_USERINFO_URL", "OAUTH_CLIENT_ID", "OAUTH_CLIENT_SECRET", "OAUTH_REDIRECT_URI"]
    {
        let mut vars = valid();
        vars.remove(name);
        let err = format!("{:#}", load(&vars).unwrap_err());
        assert!(err.contains(name), "{name}: {err}");
    }
}

#[test]
fn a_short_secret_is_refused_without_printing_it() {
    let short = "only-16-bytes-ok";
    assert_eq!(short.len(), 16);
    for name in ["JWT_SECRET", "SESSION_SECRET", "OAUTH_STATE_SECRET", "SERVICE_API_KEY"] {
        let mut vars = valid();
        vars.insert(name.to_string(), short.to_string());
        let err = format!("{:#}", load(&vars).unwrap_err());
        assert!(err.contains(name) && err.contains("32"), "{name}: {err}");
        assert!(!err.contains(short), "the error must not hold the value: {err}");
    }
}

#[test]
fn the_debug_form_of_the_settings_holds_no_secret() {
    let printed = format!("{:?}", load(&valid()).unwrap());
    for value in valid().into_iter().filter(|(k, _)| k.contains("SECRET") || k.contains("KEY")).map(|(_, v)| v) {
        assert!(!printed.contains(&value), "{printed}");
    }
}

#[test]
fn copying_the_example_environment_file_does_not_start_the_application() {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/.env.example")).unwrap();
    let vars: HashMap<String, String> = text
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    // Every variable the application reads is listed, none has a value.
    for name in valid().keys() {
        assert_eq!(vars.get(name).map(String::as_str), Some(""), "{name} must be listed and empty");
    }
    assert!(load(&vars).is_err());
}

#[test]
fn two_secrets_with_the_same_value_are_refused_without_printing_it() {
    let names = ["JWT_SECRET", "SESSION_SECRET", "OAUTH_STATE_SECRET", "SERVICE_API_KEY"];
    for a in names {
        for b in names.into_iter().filter(|b| *b != a) {
            let mut vars = valid();
            let shared = vars[a].clone();
            vars.insert(b.to_string(), shared.clone());
            let err = format!("{:#}", load(&vars).unwrap_err());
            assert!(err.contains(a) && err.contains(b), "{a} {b}: {err}");
            assert!(!err.contains(&shared), "the error must not hold the value: {err}");
        }
    }
}
