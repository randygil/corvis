//! The protected header is parsed before the key provider and the signature, on bytes an attacker controls. These
//! tests run with `serde_json/raw_value` on (a dev-dependency of `corvis`, as sqlx turns it on in a real app): the
//! magic key of `RawValue` must not make the parser re-enter with a fresh recursion budget.

#![cfg(feature = "jwt")]

use std::sync::atomic::{AtomicUsize, Ordering};

use base64::Engine as _;
use corvis::auth::AuthGuard;
use corvis::auth::jwt::{ExtractJwt, JwtKey, JwtKeyProvider, JwtStrategy, JwtStrategyOptions, JwtValidator};
use corvis::prelude::*;
use corvis::testing::request;
use corvis::{JwtModuleOptions, JwtService};
use jsonwebtoken::{Algorithm, EncodingKey};

const SECRET: &str = "header-test-secret-0123456789abcdef-01";
const RAW: &str = "$serde_json::private::RawValue";
static CALLS: AtomicUsize = AtomicUsize::new(0);
/// The tests that count provider calls run one at a time.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn b64(b: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

fn sign(header: &str) -> String {
    let exp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 3600;
    let message = format!("{}.{}", b64(header.as_bytes()), b64(format!(r#"{{"sub":"u","exp":{exp}}}"#).as_bytes()));
    let sig = jsonwebtoken::crypto::sign(message.as_bytes(), &EncodingKey::from_secret(SECRET.as_bytes()), Algorithm::HS256).unwrap();
    format!("{message}.{sig}")
}

fn raw_nest(levels: usize, arrays: usize) -> String {
    let mut inner = "0".to_string();
    for _ in 0..levels {
        inner = format!("{}{{\"{RAW}\":{}}}{}", "[".repeat(arrays), serde_json::to_string(&inner).unwrap(), "]".repeat(arrays));
    }
    format!(r#"{{"alg":"HS256","z":{inner}}}"#)
}

struct Counting;

#[async_trait]
impl JwtKeyProvider for Counting {
    async fn key(&self, _req: &Request, _raw: &str) -> anyhow::Result<JwtKey> {
        CALLS.fetch_add(1, Ordering::SeqCst);
        Ok(JwtKey::secret(SECRET))
    }
}

#[injectable]
pub struct Hs;

#[async_trait]
impl JwtValidator for Hs {
    const NAME: &'static str = "hs";
    type User = Value;

    fn options(&self) -> JwtStrategyOptions {
        JwtStrategyOptions::new().jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token()).secret_or_key_provider(Counting)
    }

    async fn validate(&self, _req: &Request, payload: Value) -> Result<Option<Value>, HttpException> {
        Ok(Some(payload))
    }
}

#[controller("")]
pub struct Api;

#[routes]
impl Api {
    #[get("hs")]
    #[use_guards(AuthGuard::new("hs"))]
    fn hs(&self) -> Value {
        json!({ "ok": true })
    }
}

#[module(providers = [Provider::strategy::<JwtStrategy<Hs>>()], controllers = [Api])]
pub struct M;

async fn status(token: &str) -> u16 {
    let mut app = CorvisFactory::create::<M>().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    request(&app).get("/hs").bearer(token).send().await.status
}

/// Runs `f` on a thread with a 2 MiB stack.
fn small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new().stack_size(2 * 1024 * 1024).spawn(f).unwrap().join().expect("the thread must not crash")
}

#[test]
fn nested_raw_value_keys_do_not_exhaust_the_stack() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // Plain JSON now: the magic key is an ordinary member whose string is never parsed again, so this correctly signed
    // header (about 126 levels deep, under the parser's limit of 128) is valid and accepted, on a small stack.
    let token = sign(&raw_nest(8, 125));
    assert!(token.len() < 8192);
    let t = token.clone();
    let verified = small_stack(move || JwtService::new(JwtModuleOptions::new(SECRET)).verify::<Value>(&t).is_ok());
    assert!(verified);
    let code = small_stack(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(status(&token))
    });
    assert_eq!(code, 200);
}

#[test]
fn a_header_over_the_parser_limit_is_refused_on_a_small_stack() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let deep = sign(&format!(r#"{{"alg":"HS256","z":{}0{}}}"#, "[".repeat(200), "]".repeat(200)));
    CALLS.store(0, Ordering::SeqCst);
    let (verified, code) = small_stack(move || {
        let ok = JwtService::new(JwtModuleOptions::new(SECRET)).verify::<Value>(&deep).is_ok();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        (ok, rt.block_on(status(&deep)))
    });
    assert!(!verified);
    assert_eq!(code, 401);
    assert_eq!(CALLS.load(Ordering::SeqCst), 0, "the key provider must not be reached");
}

#[test]
fn the_magic_key_is_an_ordinary_member_whatever_serde_json_features_are_on() {
    // Accepted without `raw_value` and rejected with it before the fix; now one fixed outcome.
    let token = sign(&format!(r#"{{"alg":"HS256","z":{{"{RAW}":"[1"}}}}"#));
    let svc = JwtService::new(JwtModuleOptions::new(SECRET));
    assert!(svc.verify::<Value>(&token).is_ok());
    // A crit member beside it is still found.
    let crit = sign(&format!(r#"{{"alg":"HS256","{RAW}":"{{}}","crit":["x"]}}"#));
    assert!(svc.verify::<Value>(&crit).is_err());
}

#[test]
fn decode_does_not_follow_the_magic_key() {
    let svc = JwtService::new(JwtModuleOptions::new(SECRET));
    let nested = b64(raw_nest(8, 125).as_bytes());
    let s = svc.clone();
    small_stack(move || {
        s.decode(&format!("a.{nested}.c"));
    });
    let payload = b64(format!(r#"{{"z":{{"{RAW}":"[1"}}}}"#).as_bytes());
    assert!(svc.decode(&format!("a.{payload}.c")).is_some());
}
