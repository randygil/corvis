//! Sessions against the original: every step recorded from `express-session` 1.19.0 with `passport` 0.7.0
//! (`tests/fixtures/auth/session.mjs`) is replayed against the same routes with Corvis: status, body, the attributes
//! of `Set-Cookie`, the calls the store received and the records it holds afterwards. No Node is needed at test time.
//! A step with a `divergence` is asserted against the specification for the part that differs.
#![cfg(feature = "auth-session")]

mod session_common;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use corvis::auth::session::{SessionCookie, SessionOptions, SessionStore};
use corvis::http::{CookieKeys, Method};
use corvis::prelude::*;
use corvis::testing::request;
use session_common::*;

const FIXTURE: &str = include_str!("fixtures/auth/session.json");

fn fixture() -> Value {
    serde_json::from_str(FIXTURE).unwrap()
}

fn secret(name: &str) -> &'static str {
    match name {
        "new" => NEW,
        "old" => OLD,
        other => panic!("unknown secret {other}"),
    }
}

/// The options of an app of the fixture, as the original was configured.
fn options_of(cfg: &Value, store: Arc<RecordingStore>) -> SessionOptions {
    let secrets: Vec<&str> = cfg["secrets"].as_array().unwrap().iter().map(|s| secret(s.as_str().unwrap())).collect();
    let mut cookie = original_cookie();
    if let Some(ms) = cfg["maxAge"].as_u64() {
        cookie = cookie.max_age(Duration::from_millis(ms));
    }
    SessionOptions::with_secrets(secrets)
        .unwrap()
        .cookie(cookie)
        .resave(cfg["resave"].as_bool().unwrap())
        .save_uninitialized(cfg["saveUninitialized"].as_bool().unwrap())
        .rolling(cfg["rolling"].as_bool().unwrap_or(false))
        .keep_session_info_on_login(cfg["keepSessionInfo"].as_bool().unwrap_or(false))
        .store_arc(store)
}

/// A `Set-Cookie` header as the fixture records it: flags, attributes (`Expires` as `<date>`) and what the value is.
fn parse_set_cookie(header: &str) -> Value {
    let mut parts = header.split("; ");
    let pair = parts.next().unwrap();
    let mut flags: Vec<String> = Vec::new();
    let mut attributes = serde_json::Map::new();
    for attr in parts {
        match attr.split_once('=') {
            Some((k, _)) if k == "Expires" => {
                attributes.insert(k.into(), json!("<date>"));
            }
            Some((k, v)) => {
                attributes.insert(k.into(), json!(v));
            }
            None => flags.push(attr.to_string()),
        }
    }
    let _ = pair;
    json!({ "flags": flags, "attributes": attributes })
}

fn comparable(recorded: &Value) -> Value {
    json!({ "flags": recorded["flags"], "attributes": recorded["attributes"] })
}

/// The binding between the placeholders of the fixture and the identifiers of this run.
#[derive(Default)]
struct Binding {
    ids: HashMap<String, String>,
}

impl Binding {
    fn bind(&mut self, placeholder: &str, id: &str, context: &str) {
        match self.ids.get(placeholder) {
            Some(bound) => assert_eq!(bound, id, "{context}: {placeholder} is another session"),
            None => {
                assert!(!self.ids.values().any(|v| v == id), "{context}: the identifier of {placeholder} is bound to another placeholder");
                self.ids.insert(placeholder.to_string(), id.to_string());
            }
        }
    }
}

type Normalized = Vec<(String, String)>;

/// Drops what has no lasting effect and differs by design: a `destroy` of an identifier that had no record, a `set`
/// of a record destroyed later in the same step, and a repeated `set` of one record.
fn normalize(ops: Vec<(String, String, bool)>) -> Normalized {
    let mut kept: Vec<(String, String, bool)> = ops.into_iter().filter(|(op, _, existed)| !(op == "destroy" && !existed)).collect();
    let mut i = 0;
    while i < kept.len() {
        let (op, id, _) = kept[i].clone();
        let destroyed_later = kept[i + 1..].iter().any(|(o, other, _)| o == "destroy" && *other == id);
        if op == "set" && destroyed_later {
            kept.remove(i);
        } else {
            i += 1;
        }
    }
    kept.dedup_by(|a, b| a.0 == "set" && b.0 == "set" && a.1 == b.1);
    kept.into_iter().map(|(op, id, _)| (op, id)).collect()
}

fn cookie_header(spec: &Value, jar: &HashMap<String, String>) -> Option<String> {
    let new_keys = CookieKeys::new([NEW]).unwrap();
    let old_keys = CookieKeys::new([OLD]).unwrap();
    if spec == "unknown" {
        let id = fixture()["unknownId"].as_str().unwrap().to_string();
        return Some(format!("connect.sid={}", urlencode(&new_keys.sign(&id))));
    }
    if spec.is_null() {
        return None;
    }
    let raw = jar.get(spec["ref"].as_str().unwrap()).unwrap_or_else(|| panic!("no cookie bound to {}", spec["ref"])).clone();
    Some(match spec["mutate"].as_str() {
        None => raw,
        Some("flip-signature") => {
            let last = raw.chars().last().unwrap();
            format!("{}{}", &raw[..raw.len() - 1], if last == 'A' { 'B' } else { 'A' })
        }
        Some("unsigned") => format!("connect.sid={}", id_of(&raw)),
        Some("retired-secret") => format!("connect.sid={}", urlencode(&old_keys.sign(&id_of(&raw)))),
        Some(other) => panic!("unknown mutation {other}"),
    })
}

fn urlencode(s: &str) -> String {
    corvis::http::res::encode_uri_component(s)
}

fn normalized_record(record: &Value) -> Value {
    let mut record = record.clone();
    let object = record.as_object_mut().unwrap();
    object.remove("__corvis");
    if let Some(expires) = object.get_mut("cookie").and_then(|c| c.get_mut("expires"))
        && !expires.is_null()
    {
        *expires = json!("<date>");
    }
    record
}

#[tokio::test]
async fn every_recorded_step_is_replayed() {
    let f = fixture();
    assert_eq!(f["versions"]["express-session"], "1.19.0");
    assert_eq!(f["versions"]["passport"], "0.7.0");
    let steps = f["steps"].as_array().unwrap();
    assert!(steps.len() >= 40, "{} steps", steps.len());

    let mut stores: HashMap<String, Arc<RecordingStore>> = HashMap::new();
    let mut apps = HashMap::new();
    for (name, cfg) in f["apps"].as_object().unwrap() {
        let group = cfg["store"].as_str().unwrap_or(name).to_string();
        let store = stores.entry(group).or_insert_with(RecordingStore::new).clone();
        apps.insert(name.clone(), (app(options_of(cfg, store.clone())).await, store));
    }

    let mut jar: HashMap<String, String> = HashMap::new();
    let mut binding = Binding::default();
    let mut diverged = 0;
    for step in steps {
        let name = step["name"].as_str().unwrap();
        let (app, store) = &apps[step["app"].as_str().unwrap()];
        store.take_ops();

        let req = &step["request"];
        let method = Method::from_bytes(req["method"].as_str().unwrap().as_bytes()).unwrap();
        let mut call = request(app).method(method, req["path"].as_str().unwrap());
        for (k, v) in req["headers"].as_object().unwrap() {
            call = call.header(k, v.as_str().unwrap());
        }
        if let Some(cookie) = cookie_header(&req["cookie"], &jar) {
            call = call.header("cookie", &cookie);
        }
        let r = call.send().await;
        let recorded = &step["response"];
        assert_eq!(r.status as u64, recorded["status"].as_u64().unwrap(), "{name}: status, body {}", r.text());
        assert_eq!(r.json_value(), recorded["body"], "{name}: body");

        let cookies = set_cookies(&r);
        let divergence = step.get("divergence").and_then(Value::as_str);
        if divergence.is_some() {
            diverged += 1;
        }
        let recorded_cookies = recorded["setCookie"].as_array().unwrap();
        match name {
            // Corvis clears the cookie after destroy(); the original sends nothing.
            "strict: destroy" => {
                assert_eq!(cookies.len(), 1, "{name}: {cookies:?}");
                assert!(cookies[0].starts_with("connect.sid=;"), "{name}: {}", cookies[0]);
                assert!(cookies[0].contains("Expires=Thu, 01 Jan 1970"), "{name}: {}", cookies[0]);
            }
            // Corvis signs the cookie again with the current secret after a rotation; the original sends nothing.
            "rotation: the rotated app accepts it and signs again" => {
                assert_eq!(cookies.len(), 1, "{name}: {cookies:?}");
                let value = percent_encoding::percent_decode_str(cookies[0].split_once('=').unwrap().1.split(';').next().unwrap())
                    .decode_utf8()
                    .unwrap()
                    .into_owned();
                assert!(CookieKeys::new([NEW]).unwrap().verified_with_current(&value), "{name}: signed with the current secret");
                binding.bind("sid7", &id_of(&pair(&cookies[0])), name);
            }
            _ => {
                assert_eq!(cookies.len(), recorded_cookies.len(), "{name}: Set-Cookie headers {cookies:?}");
                for (actual, recorded) in cookies.iter().zip(recorded_cookies) {
                    assert_eq!(parse_set_cookie(actual), comparable(recorded), "{name}: {actual}");
                    if let Some(placeholder) = recorded["sid"].as_str() {
                        binding.bind(placeholder, &id_of(&pair(actual)), name);
                    }
                }
            }
        }
        if let Some(bound) = req["as"].as_str()
            && let Some(header) = cookies.iter().find(|c| !c.starts_with("connect.sid=;"))
        {
            jar.insert(bound.to_string(), pair(header));
        }

        // The calls the store received.
        let actual_ops = normalize(store.take_ops().into_iter().map(|o| (o.op.to_string(), o.id, o.existed)).collect());
        let recorded_ops = normalize(
            step["ops"]
                .as_array()
                .unwrap()
                .iter()
                .map(|o| {
                    (o["op"].as_str().unwrap().to_string(), o["sid"].as_str().unwrap().to_string(), o["existed"].as_bool().unwrap_or(true))
                })
                .collect(),
        );
        assert_eq!(actual_ops.len(), recorded_ops.len(), "{name}: store calls {actual_ops:?} against the recorded {recorded_ops:?}");
        for ((op, id), (recorded_op, placeholder)) in actual_ops.iter().zip(&recorded_ops) {
            assert_eq!(op, recorded_op, "{name}: store calls {actual_ops:?} against the recorded {recorded_ops:?}");
            if placeholder == "unknown" {
                continue;
            }
            binding.bind(placeholder, id, name);
        }

        // The records the store holds afterwards.
        let held: BTreeMap<String, Value> = store.records().await.into_iter().map(|(id, rec)| (id, normalized_record(&rec))).collect();
        let expected: BTreeMap<String, Value> = step["store"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(placeholder, record)| {
                let id = binding.ids.get(placeholder).unwrap_or_else(|| panic!("{name}: {placeholder} was never seen")).clone();
                (id, record.clone())
            })
            .collect();
        assert_eq!(held, expected, "{name}: the store after the step");
    }
    assert_eq!(diverged, 4, "the steps that diverge on purpose");
}

#[tokio::test]
async fn corvis_logout_and_login_without_a_session_use_the_calls_of_the_specification() {
    // The original also saves the old record before it destroys it (logout) and destroys an identifier that never had a
    // record (login without a session); Corvis destroys the old record, then creates the new one.
    let store = RecordingStore::new();
    let app = app(options().cookie(original_cookie()).store_arc(store.clone())).await;
    let login = request(&app).post("/login").header("x-user", "3").send().await;
    assert_eq!(store.take_ops().iter().map(|o| o.op).collect::<Vec<_>>(), ["set"]);
    let cookie = pair(&set_cookies(&login)[0]);
    let out = request(&app).post("/logout").header("cookie", &cookie).send().await;
    assert_eq!(out.status, 200);
    let ops = store.take_ops();
    assert_eq!(ops.iter().map(|o| o.op).collect::<Vec<_>>(), ["get", "destroy", "set"]);
    assert_eq!(ops[1].id, id_of(&cookie));
    assert_ne!(ops[2].id, id_of(&cookie));
}

#[tokio::test]
async fn a_cookie_created_by_the_original_is_read_by_corvis() {
    let f = fixture();
    let seed = &f["original"];
    let store = RecordingStore::new();
    // The record exactly as the original stored it: no `__corvis` timestamps, which Corvis adds at the first load.
    store.inner.create(seed["id"].as_str().unwrap(), &seed["record"], Duration::from_secs(3600)).await.unwrap();
    let app = app(options().cookie(original_cookie()).store_arc(store.clone())).await;
    let r = request(&app).get("/sess").header("cookie", seed["cookie"].as_str().unwrap()).send().await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json_value(), json!({ "data": { "views": "7" } }));
    // It is touched: the timestamps are written next to the record the original wrote.
    let held = store.inner.load(seed["id"].as_str().unwrap()).await.unwrap().unwrap();
    assert!(held["__corvis"]["createdAt"].is_i64(), "{held}");
    assert_eq!(held["views"], "7");
}

#[tokio::test]
async fn a_cookie_created_by_corvis_has_the_recorded_shape_and_verifies_like_the_original() {
    let f = fixture();
    let app = app(options().cookie(original_cookie())).await;
    let r = request(&app).post("/set?k=views&v=7").send().await;
    let header = set_cookies(&r).remove(0);
    let recorded = f["original"]["cookie"].as_str().unwrap();
    // `connect.sid=s%3A<32 characters>.<43 characters>`.
    let shape = |s: &str| {
        let value = s.split(';').next().unwrap().split_once('=').unwrap().1.to_string();
        let value = value.strip_prefix("s%3A").unwrap().to_string();
        let (id, sig) = value.rsplit_once('.').unwrap();
        (id.len(), sig.replace("%2F", "/").replace("%2B", "+").len())
    };
    assert_eq!(shape(&header), shape(recorded));
    assert_eq!(shape(&header), (32, 43));
    // The signature is the one `cookie-signature` computes over the identifier with the secret.
    let value = percent_encoding::percent_decode_str(header.split(';').next().unwrap().split_once('=').unwrap().1)
        .decode_utf8()
        .unwrap()
        .into_owned();
    assert_eq!(CookieKeys::new([NEW]).unwrap().unsign(&value).as_deref(), Some(id_of(&pair(&header)).as_str()));
    // The same secret and identifier give the recorded cookie.
    let id = f["original"]["id"].as_str().unwrap();
    assert_eq!(format!("connect.sid={}", urlencode(&CookieKeys::new([NEW]).unwrap().sign(id))), recorded);
}

#[tokio::test]
async fn the_record_has_the_shape_of_the_original() {
    let f = fixture();
    let store = RecordingStore::new();
    let app = app(options().cookie(original_cookie()).store_arc(store.clone())).await;
    let r = request(&app).post("/set?k=views&v=7").send().await;
    let id = id_of(&pair(&set_cookies(&r)[0]));
    let record = store.inner.load(&id).await.unwrap().unwrap();
    assert_eq!(normalized_record(&record), f["original"]["record"]);
    // The timestamps live under one reserved key.
    let meta = record["__corvis"].as_object().unwrap();
    assert_eq!(meta.len(), 2);
    assert!(meta["createdAt"].as_i64().unwrap() <= meta["lastAccess"].as_i64().unwrap());
}

#[tokio::test]
async fn corvis_defaults_diverge_from_the_original_as_the_specification_says() {
    // SameSite=Lax and Secure by default; the original sends neither.
    let app = app(options()).await;
    let r = request(&app).post("/set?k=a&v=1").send().await;
    let header = set_cookies(&r).remove(0);
    let attrs: Vec<&str> = header.split("; ").skip(1).collect();
    assert_eq!(attrs, ["Path=/", "HttpOnly", "Secure", "SameSite=Lax"], "{header}");
    // An anonymous visit creates nothing with the defaults, where the original defaults to saving it.
    let r = request(&app).get("/sess").send().await;
    assert!(set_cookies(&r).is_empty());
    let _ = SessionCookie::default();
}
