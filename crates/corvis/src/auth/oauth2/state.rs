//! `state` and PKCE: generation, the records of the logins in progress, and where they live (a signed cookie or the
//! session).
//!
//! A browser can have several logins in progress at once (two tabs), so the record is a short list of flows, newest
//! last. A callback takes out the flow whose `state` it carries and leaves the others alone; a flow nobody claims
//! expires with its lifetime.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use rand::rngs::OsRng;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::options::{OAuth2Options, is_safe_relative_path};
use super::secret::ct_eq;
use crate::http::res::{CookieOptions, serialize_cookie};
use crate::http::signed_cookie::CookieKeys;

/// The bytes of randomness in a `state` and in a PKCE verifier: 256 bits each, 43 base64url characters.
const TOKEN_BYTES: usize = 32;
pub(crate) const TOKEN_CHARS: usize = 43;
/// The most logins in progress kept per browser and strategy.
pub(crate) const MAX_FLOWS: usize = 4;
/// A record from the future (clock skew between instances) is accepted up to this many seconds ahead.
pub(crate) const MAX_SKEW_SECONDS: i64 = 60;
/// A signed cookie value is kept under this size so the whole `Set-Cookie` stays within what browsers accept.
const MAX_COOKIE_VALUE: usize = 3500;

/// 32 bytes from the operating system's generator as 43 base64url characters. `None` when the generator fails: there is
/// no fallback to a weaker source.
pub(crate) fn random_token() -> Option<String> {
    let mut bytes = [0u8; TOKEN_BYTES];
    OsRng.try_fill_bytes(&mut bytes).ok()?;
    Some(URL_SAFE_NO_PAD.encode(bytes))
}

/// The S256 `code_challenge` of a verifier: the base64url SHA-256 of its ASCII bytes.
pub(crate) fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Whether a callback's `state` has the shape of one this module issues.
pub(crate) fn is_state_shape(state: &str) -> bool {
    state.len() == TOKEN_CHARS && state.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The PKCE verifier of a cookie-stored flow. It is not written to the cookie: it is derived from the state with the
/// first cookie secret, so the browser (which knows the state) cannot compute it and a party that sees only the callback
/// URL cannot either. A key rotation between the redirect and the callback makes the derivation differ, and the
/// provider then rejects the exchange (one failed login, for logins that straddle a rotation).
pub(crate) fn derive_verifier(keys: &CookieKeys, strategy: &str, state: &str, issued: i64) -> String {
    let signed = keys.sign(&format!("corvis-oauth2-pkce-v1|{strategy}|{state}|{issued}"));
    let signature = signed.rsplit('.').next().unwrap_or("");
    URL_SAFE_NO_PAD.encode(Sha256::digest(signature.as_bytes()))
}

/// One login in progress.
#[derive(Clone)]
pub(crate) struct Flow {
    pub(crate) state: String,
    /// The verifier, kept only by the session storage (the cookie storage derives it).
    pub(crate) verifier: Option<String>,
    /// Seconds since the epoch when the login started.
    pub(crate) issued: i64,
    pub(crate) return_to: Option<String>,
    /// The session the flow started under (see [`session_binding`]): `None` when no session is in use for the
    /// application, so the callback does not check one.
    pub(crate) binding: Option<String>,
    /// The user the request that started the flow carried (see [`user_binding`]): `None` in a record written before
    /// this field existed, which counts as a flow started by nobody.
    pub(crate) user: Option<String>,
}

/// The binding of a flow that started without a session.
const NO_SESSION: &str = "-";

/// What ties a cookie-stored flow to the browser session it started under: a keyed digest of the session identifier
/// (of the state and the strategy too, so it is useless anywhere else), or [`NO_SESSION`] when the request had no
/// session that will be kept. `None` when no `SessionModule` runs for the request. The identifier cannot be recovered from the digest: it is a SHA-256 of an HMAC made with a cookie
/// secret.
#[cfg(feature = "auth-session")]
pub(crate) fn session_binding(
    keys: &CookieKeys,
    strategy: &str,
    state: &str,
    session: Option<&crate::auth::session::Session>,
) -> Option<String> {
    let session = session?;
    // A new session is not kept unless `save_uninitialized` is on: the browser has none yet, and the callback will not
    // have one either. With it on, the session of this request is written with the response and the callback carries it.
    if session.is_new() && !session.manager().options.save_uninitialized {
        return Some(NO_SESSION.to_string());
    }
    let signed = keys.sign(&format!("corvis-oauth2-session-v1|{strategy}|{state}|{}", session.id()));
    let signature = signed.rsplit('.').next().unwrap_or("");
    Some(URL_SAFE_NO_PAD.encode(Sha256::digest(signature.as_bytes())))
}

/// `true` when the callback's binding (see [`session_binding`]) is the one the flow was started with. A flow without a
/// binding, and a callback that has no session machinery, are not checked.
pub(crate) fn binding_matches(flow: &Flow, current: Option<&str>) -> bool {
    match (&flow.binding, current) {
        (Some(started), Some(now)) => ct_eq(started.as_bytes(), now.as_bytes()),
        _ => true,
    }
}

/// The binding of a flow that started with no user on the request.
const NO_USER: &str = "-";

/// What ties a flow to the user the request that started it carried, whatever authenticated that user (a token, an
/// API key, the session strategy): a digest of the serialized user, of the state and of the strategy, or [`NO_USER`].
/// The session binding does not cover an identity that does not live in the session: a logout that only drops a token
/// and another user's login with theirs leave the session untouched. With `keys` (the cookie storage, whose record the
/// browser can read) the digest is keyed, so it is no oracle for guesses of the user; the session storage keeps its
/// record on the server and uses a plain digest.
pub(crate) fn user_binding(keys: Option<&CookieKeys>, strategy: &str, state: &str, user: Option<&Value>) -> String {
    let Some(user) = user else { return NO_USER.to_string() };
    let input = format!("corvis-oauth2-user-v1|{strategy}|{state}|{user}");
    let digest = match keys {
        Some(keys) => {
            let signed = keys.sign(&input);
            Sha256::digest(signed.rsplit('.').next().unwrap_or("").as_bytes())
        }
        None => Sha256::digest(input.as_bytes()),
    };
    URL_SAFE_NO_PAD.encode(digest)
}

/// `true` when the callback may complete the flow for the user its request carries. A callback with no user has
/// nobody to attach the provider's identity to, so it is a plain login and passes. A callback WITH a user must be the
/// user the flow was started by: a flow started by someone else, or by nobody (which cannot be told apart from
/// someone else's on a shared browser), is refused, so the provider account of one user is never linked to another.
pub(crate) fn user_matches(flow: &Flow, current: &str) -> bool {
    if current == NO_USER {
        return true;
    }
    let started = flow.user.as_deref().unwrap_or(NO_USER);
    ct_eq(started.as_bytes(), current.as_bytes())
}

/// Prints nothing: a flow holds a state and a verifier.
impl std::fmt::Debug for Flow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Flow(..)")
    }
}

/// The logins in progress of one browser for one strategy.
#[derive(Clone, Default)]
pub(crate) struct FlowSet {
    pub(crate) flows: Vec<Flow>,
}

impl std::fmt::Debug for FlowSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FlowSet({} flow(s))", self.flows.len())
    }
}

impl FlowSet {
    /// Adds a flow, dropping the oldest ones past [`MAX_FLOWS`].
    pub(crate) fn push(&mut self, flow: Flow) {
        self.flows.push(flow);
        while self.flows.len() > MAX_FLOWS {
            self.flows.remove(0);
        }
    }

    /// Drops the flows that are older than `ttl` seconds or more than a minute ahead of `now`.
    pub(crate) fn prune(&mut self, now: i64, ttl: i64) {
        // A timestamp that cannot be compared without overflow is not a time this server issued: the flow is invalid.
        self.flows.retain(|f| {
            let not_ahead = now.checked_add(MAX_SKEW_SECONDS).is_some_and(|limit| f.issued <= limit);
            let age = now.checked_sub(f.issued);
            not_ahead && age.is_some_and(|age| age <= ttl)
        });
    }

    /// Removes and returns the flow whose state equals `state`. Every stored state is compared in constant time and
    /// the loop does not stop at a match.
    pub(crate) fn take(&mut self, state: &str) -> Option<Flow> {
        let mut found: Option<usize> = None;
        for (i, flow) in self.flows.iter().enumerate() {
            if ct_eq(flow.state.as_bytes(), state.as_bytes()) {
                found = Some(i);
            }
        }
        found.map(|i| self.flows.remove(i))
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.flows.is_empty()
    }

    /// The JSON record. `with_verifier` is the session form.
    fn encode(&self, strategy: &str, with_verifier: bool) -> Value {
        let flows: Vec<Value> = self
            .flows
            .iter()
            .map(|f| {
                let mut m = serde_json::Map::new();
                m.insert("s".into(), json!(f.state));
                if with_verifier && let Some(v) = &f.verifier {
                    m.insert("v".into(), json!(v));
                }
                m.insert("t".into(), json!(f.issued));
                if let Some(r) = &f.return_to {
                    m.insert("r".into(), json!(r));
                }
                if let Some(b) = &f.binding {
                    m.insert("b".into(), json!(b));
                }
                if let Some(u) = &f.user {
                    m.insert("u".into(), json!(u));
                }
                Value::Object(m)
            })
            .collect();
        json!({ "n": strategy, "f": flows })
    }

    /// Reads a record. A record of another strategy, a malformed one and a malformed entry give no flow.
    fn decode(value: &Value, strategy: &str, with_verifier: bool) -> FlowSet {
        let mut set = FlowSet::default();
        if value.get("n").and_then(Value::as_str) != Some(strategy) {
            return set;
        }
        let Some(list) = value.get("f").and_then(Value::as_array) else { return set };
        for entry in list.iter().take(MAX_FLOWS) {
            let Some(state) = entry.get("s").and_then(Value::as_str).filter(|s| is_state_shape(s)) else { continue };
            let Some(issued) = entry.get("t").and_then(Value::as_i64) else { continue };
            let verifier = if with_verifier {
                match entry.get("v").and_then(Value::as_str) {
                    Some(v) if is_state_shape(v) => Some(v.to_string()),
                    _ => continue,
                }
            } else {
                None
            };
            let return_to = entry.get("r").and_then(Value::as_str).filter(|r| is_safe_relative_path(r)).map(str::to_string);
            // A binding that is not one this module writes is a malformed entry (a record with no binding is a flow
            // that was not bound).
            let binding = match entry.get("b") {
                None => None,
                Some(Value::String(b)) if b == NO_SESSION || is_state_shape(b) => Some(b.clone()),
                Some(_) => continue,
            };
            let user = match entry.get("u") {
                None => None,
                Some(Value::String(u)) if u == NO_USER || is_state_shape(u) => Some(u.clone()),
                Some(_) => continue,
            };
            set.flows.push(Flow { state: state.to_string(), verifier, issued, return_to, binding, user });
        }
        set
    }
}

/// The attributes of the state cookie.
pub(crate) struct StateCookie {
    pub(crate) name: String,
    pub(crate) keys: CookieKeys,
    pub(crate) secure: bool,
    pub(crate) max_age_seconds: i64,
}

impl StateCookie {
    pub(crate) fn new(name: String, keys: CookieKeys, options: &OAuth2Options) -> Self {
        StateCookie { name, keys, secure: options.cookie_secure, max_age_seconds: options.state_ttl.as_secs() as i64 }
    }

    fn options(&self) -> CookieOptions {
        let mut o = CookieOptions::new().path("/").http_only(true);
        o.secure = self.secure;
        o.same_site = Some("lax".into());
        o
    }

    /// The flows the request's cookie holds (none for a missing, unsigned, tampered, oversized or foreign cookie).
    pub(crate) fn read(&self, req: &crate::http::Request, strategy: &str) -> FlowSet {
        let Some(raw) = req.cookie(&self.name) else { return FlowSet::default() };
        let Some(encoded) = self.keys.unsign(&raw) else { return FlowSet::default() };
        let Some(bytes) = URL_SAFE_NO_PAD.decode(encoded.as_bytes()).ok() else { return FlowSet::default() };
        let Some(value) = super::json::parse(&bytes) else { return FlowSet::default() };
        FlowSet::decode(&value, strategy, false)
    }

    /// The `Set-Cookie` value that stores `set`, dropping the oldest flows until it fits.
    pub(crate) fn write(&self, set: &FlowSet, strategy: &str) -> String {
        let mut set = set.clone();
        loop {
            let record = set.encode(strategy, false).to_string();
            let signed = self.keys.sign(&URL_SAFE_NO_PAD.encode(record.as_bytes()));
            if signed.len() <= MAX_COOKIE_VALUE || set.flows.len() <= 1 {
                // Name, path and attributes were validated at startup, and the value is `s:` + base64url + `.` +
                // base64, so the panics of `serialize_cookie` cannot be reached.
                return serialize_cookie(&self.name, &signed, &self.options().max_age(self.max_age_seconds * 1000));
            }
            set.flows.remove(0);
        }
    }

    /// The `Set-Cookie` value that makes the browser drop the cookie.
    pub(crate) fn clear(&self) -> String {
        let epoch = chrono::DateTime::from_timestamp_millis(0).unwrap_or_default();
        serialize_cookie(&self.name, "", &self.options().expires(epoch))
    }
}

/// The key of a strategy's record in the session.
pub(crate) fn session_key(strategy: &str) -> String {
    format!("oauth2:{strategy}")
}

/// The flows the session holds for the strategy.
#[cfg(feature = "auth-session")]
pub(crate) fn read_session(session: &crate::auth::session::Session, strategy: &str) -> FlowSet {
    match session.get_value(&session_key(strategy)) {
        Some(value) => FlowSet::decode(&value, strategy, true),
        None => FlowSet::default(),
    }
}

/// Stores `set` in the session (or removes the key when it is empty). `false` when the session refused the value.
#[cfg(feature = "auth-session")]
pub(crate) fn write_session(session: &crate::auth::session::Session, strategy: &str, set: &FlowSet) -> bool {
    let key = session_key(strategy);
    if set.is_empty() {
        session.remove(&key);
        return true;
    }
    session.set(&key, set.encode(strategy, true)).is_ok()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    const KEY: &str = "0123456789abcdef0123456789abcdef";

    fn keys() -> CookieKeys {
        CookieKeys::new([KEY]).unwrap()
    }

    fn flow(n: u8, issued: i64) -> Flow {
        Flow {
            state: format!("{:0>43}", n),
            verifier: Some(format!("{:0>43}", n + 100)),
            issued,
            return_to: None,
            binding: None,
            user: None,
        }
    }

    #[test]
    fn tokens_are_43_base64url_characters_and_differ() {
        let mut seen = HashSet::new();
        for _ in 0..200 {
            let t = random_token().unwrap();
            assert_eq!(t.len(), TOKEN_CHARS);
            assert!(is_state_shape(&t), "{t}");
            assert!(seen.insert(t));
        }
    }

    #[test]
    fn the_challenge_is_the_rfc_7636_one() {
        assert_eq!(challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        let v = random_token().unwrap();
        assert_eq!(challenge(&v).len(), 43);
    }

    #[test]
    fn the_derived_verifier_is_stable_unique_and_keyed() {
        let a = derive_verifier(&keys(), "google", "S1", 10);
        assert_eq!(a, derive_verifier(&keys(), "google", "S1", 10));
        assert_eq!(a.len(), 43);
        assert!(is_state_shape(&a));
        assert_ne!(a, derive_verifier(&keys(), "google", "S2", 10));
        assert_ne!(a, derive_verifier(&keys(), "github", "S1", 10));
        assert_ne!(a, derive_verifier(&keys(), "google", "S1", 11));
        let other = CookieKeys::new(["fedcba9876543210fedcba9876543210"]).unwrap();
        assert_ne!(a, derive_verifier(&other, "google", "S1", 10));
        assert!(!a.contains("S1"));
    }

    #[test]
    fn a_record_round_trips_in_both_forms() {
        let mut set = FlowSet::default();
        set.push(Flow { return_to: Some("/cats/7?tab=toys".into()), ..flow(1, 1000) });
        set.push(flow(2, 1001));
        let session_form = FlowSet::decode(&set.encode("google", true), "google", true);
        assert_eq!(session_form.flows.len(), 2);
        assert_eq!(session_form.flows[0].verifier, set.flows[0].verifier);
        assert_eq!(session_form.flows[0].return_to.as_deref(), Some("/cats/7?tab=toys"));
        let cookie_form = FlowSet::decode(&set.encode("google", false), "google", false);
        assert_eq!(cookie_form.flows.len(), 2);
        assert!(cookie_form.flows[0].verifier.is_none());
        assert!(
            !set.encode("google", false).to_string().contains(set.flows[0].verifier.as_ref().unwrap()),
            "the cookie form has no verifier"
        );
        assert!(FlowSet::decode(&set.encode("google", true), "github", true).is_empty(), "another strategy's record");
    }

    #[test]
    fn malformed_records_give_no_flow() {
        for bad in [
            json!(null),
            json!([]),
            json!({ "n": "g" }),
            json!({ "n": "g", "f": "x" }),
            json!({ "n": "g", "f": [{ "s": "short", "t": 1 }] }),
            json!({ "n": "g", "f": [{ "s": format!("{:0>43}", 1), "t": "x" }] }),
        ] {
            assert!(FlowSet::decode(&bad, "g", false).is_empty(), "{bad}");
        }
        let unsafe_return = json!({ "n": "g", "f": [{ "s": format!("{:0>43}", 1), "t": 1, "r": "//evil.example" }] });
        let set = FlowSet::decode(&unsafe_return, "g", false);
        assert_eq!(set.flows.len(), 1);
        assert!(set.flows[0].return_to.is_none(), "an unsafe return_to is dropped when reading");
        let no_verifier = json!({ "n": "g", "f": [{ "s": format!("{:0>43}", 1), "t": 1 }] });
        assert!(FlowSet::decode(&no_verifier, "g", true).is_empty(), "the session form needs the verifier");
    }

    #[test]
    fn at_most_four_flows_and_the_oldest_goes_first() {
        let mut set = FlowSet::default();
        for i in 0..6 {
            set.push(flow(i, 1000 + i as i64));
        }
        assert_eq!(set.flows.len(), MAX_FLOWS);
        assert_eq!(set.flows[0].state, flow(2, 0).state);
    }

    #[test]
    fn take_removes_only_the_matching_flow() {
        let mut set = FlowSet::default();
        set.push(flow(1, 1000));
        set.push(flow(2, 1000));
        assert!(set.take(&flow(9, 0).state).is_none());
        assert_eq!(set.flows.len(), 2);
        assert!(set.take(&flow(1, 0).state).is_some());
        assert!(set.take(&flow(1, 0).state).is_none(), "a flow is used once");
        assert_eq!(set.flows.len(), 1);
        assert!(set.take("").is_none());
        assert!(set.take(&format!("{}x", flow(2, 0).state)).is_none(), "different length");
    }

    #[test]
    fn an_impossible_timestamp_is_an_invalid_flow_and_never_overflows() {
        for issued in [i64::MIN, i64::MIN + 1, i64::MAX, i64::MAX - 1] {
            for now in [0, 1_700_000_000, i64::MAX, i64::MIN, -1] {
                let mut set = FlowSet::default();
                set.push(flow(1, issued));
                set.prune(now, 600);
                assert!(set.is_empty() || now.checked_sub(issued).is_some(), "issued {issued}, now {now}");
            }
        }
        let mut set = FlowSet::default();
        set.push(flow(1, i64::MIN));
        set.prune(1_700_000_000, 600);
        assert!(set.is_empty(), "an ancient flow is not accepted, whatever the arithmetic wraps to");
    }

    #[test]
    fn prune_drops_expired_and_future_flows() {
        let mut set = FlowSet::default();
        set.push(flow(1, 1000)); // 601 s old
        set.push(flow(2, 1001)); // exactly 600 s old: kept
        set.push(flow(3, 1700 + 60)); // 60 s ahead: kept
        set.push(flow(4, 1700 + 61)); // too far ahead
        set.prune(1601, 600);
        let kept: Vec<_> = set.flows.iter().map(|f| f.state.clone()).collect();
        assert_eq!(kept, vec![flow(2, 0).state]);
        let mut set = FlowSet::default();
        set.push(flow(3, 1660));
        set.prune(1600, 600);
        assert_eq!(set.flows.len(), 1);
    }

    #[test]
    fn debug_holds_no_value() {
        let f = flow(7, 5);
        assert!(!format!("{f:?}").contains("0000"));
        let mut set = FlowSet::default();
        set.push(f);
        assert!(!format!("{set:?}").contains("0000"));
    }

    fn cookie(secure: bool) -> StateCookie {
        StateCookie {
            name: if secure { "__Host-oauth2.google".into() } else { "oauth2.google".into() },
            keys: keys(),
            secure,
            max_age_seconds: 600,
        }
    }

    fn value_of(set_cookie: &str) -> String {
        let raw = set_cookie.split(';').next().unwrap().split_once('=').unwrap().1;
        percent_encoding::percent_decode_str(raw).decode_utf8().unwrap().into_owned()
    }

    #[test]
    fn the_cookie_has_the_documented_attributes() {
        let mut set = FlowSet::default();
        set.push(flow(1, 1000));
        let header = cookie(true).write(&set, "google");
        assert!(header.starts_with("__Host-oauth2.google=s%3A"), "{header}");
        for attr in ["Max-Age=600", "Path=/", "HttpOnly", "Secure", "SameSite=Lax"] {
            assert!(header.contains(attr), "{attr} in {header}");
        }
        assert!(!header.contains("Domain"), "{header}");
        let plain = cookie(false).write(&set, "google");
        assert!(plain.starts_with("oauth2.google=") && !plain.contains("Secure"), "{plain}");
        assert!(keys().unsign(&value_of(&header)).is_some());
        let cleared = cookie(true).clear();
        assert!(cleared.contains("Expires=Thu, 01 Jan 1970") && cleared.contains("Secure") && cleared.contains("HttpOnly"), "{cleared}");
    }

    #[test]
    fn the_cookie_round_trips_and_refuses_tampering_and_other_strategies() {
        use crate::http::Request;
        let mut set = FlowSet::default();
        set.push(Flow { return_to: Some("/x".into()), ..flow(1, 1000) });
        let c = cookie(true);
        let header = c.write(&set, "google");
        let req_with = |value: &str| {
            let mut headers = http::HeaderMap::new();
            headers.insert("cookie", http::HeaderValue::from_str(&format!("{}={value}", c.name)).unwrap());
            Request::new(http::Method::GET, http::Uri::from_static("/"), headers, bytes::Bytes::new()).unwrap()
        };
        let raw = header.split(';').next().unwrap().split_once('=').unwrap().1.to_string();
        let read = c.read(&req_with(&raw), "google");
        assert_eq!(read.flows.len(), 1);
        assert_eq!(read.flows[0].return_to.as_deref(), Some("/x"));
        assert!(c.read(&req_with(&raw), "github").is_empty(), "a record of another strategy");
        let mut tampered = raw.clone().into_bytes();
        let last = tampered.len() - 1;
        tampered[last] = if tampered[last] == b'A' { b'B' } else { b'A' };
        assert!(c.read(&req_with(&String::from_utf8(tampered).unwrap()), "google").is_empty());
        assert!(c.read(&req_with("garbage"), "google").is_empty());
        assert!(c.read(&Request::empty(), "google").is_empty());
        assert!(c.read(&req_with(&"a".repeat(5000)), "google").is_empty());
    }

    #[test]
    fn a_cookie_with_long_return_paths_drops_the_oldest_flows_to_fit() {
        let c = cookie(true);
        let mut set = FlowSet::default();
        for i in 0..4 {
            set.push(Flow { return_to: Some(format!("/{}", "a".repeat(1000))), ..flow(i, 1000 + i as i64) });
        }
        let header = c.write(&set, "google");
        assert!(keys().unsign(&value_of(&header)).unwrap().len() <= MAX_COOKIE_VALUE);
        let value = keys().unsign(&value_of(&header)).unwrap();
        let record = super::super::json::parse(&URL_SAFE_NO_PAD.decode(value).unwrap()).unwrap();
        let kept = FlowSet::decode(&record, "google", false);
        assert!(!kept.is_empty() && kept.flows.len() < 4);
        assert_eq!(kept.flows.last().unwrap().state, flow(3, 0).state, "the newest flow is kept");
    }

    #[test]
    fn a_flow_is_bound_to_the_user_that_started_it() {
        let (ana, bob) = (json!({ "id": 1 }), json!({ "id": 2 }));
        let state = flow(1, 0).state;
        let by_ana = user_binding(Some(&keys()), "link", &state, Some(&ana));
        let by_bob = user_binding(Some(&keys()), "link", &state, Some(&bob));
        let by_nobody = user_binding(Some(&keys()), "link", &state, None);
        assert_eq!(by_nobody, NO_USER);
        assert!(is_state_shape(&by_ana), "a digest survives the record's own check: {by_ana}");
        assert_ne!(by_ana, by_bob);
        assert_eq!(by_ana, user_binding(Some(&keys()), "link", &state, Some(&ana)), "the same user gives the same digest");
        // Useless for another flow, another strategy or without the key.
        assert_ne!(by_ana, user_binding(Some(&keys()), "link", &flow(2, 0).state, Some(&ana)));
        assert_ne!(by_ana, user_binding(Some(&keys()), "other", &state, Some(&ana)));
        assert_ne!(by_ana, user_binding(None, "link", &state, Some(&ana)));

        let started_by = |user: Option<&str>| Flow { user: user.map(str::to_string), ..flow(1, 0) };
        // The user that started it completes it; another user, or a user on a flow nobody started, does not.
        assert!(user_matches(&started_by(Some(&by_ana)), &by_ana));
        assert!(!user_matches(&started_by(Some(&by_ana)), &by_bob));
        assert!(!user_matches(&started_by(Some(NO_USER)), &by_bob));
        assert!(!user_matches(&started_by(None), &by_bob), "a record without the field was started by nobody");
        // A callback that carries no user is a plain login: nobody to link the provider's identity to.
        assert!(user_matches(&started_by(Some(&by_ana)), NO_USER));
        assert!(user_matches(&started_by(Some(NO_USER)), NO_USER));
        assert!(user_matches(&started_by(None), NO_USER));
    }

    #[test]
    fn the_user_binding_travels_with_the_record_and_a_malformed_one_drops_the_flow() {
        let mut set = FlowSet::default();
        let bound = user_binding(Some(&keys()), "google", &flow(1, 1000).state, Some(&json!({ "id": 1 })));
        set.push(Flow { user: Some(bound.clone()), ..flow(1, 1000) });
        set.push(Flow { user: Some(NO_USER.to_string()), ..flow(2, 1000) });
        let record = set.encode("google", false);
        let back = FlowSet::decode(&record, "google", false);
        assert_eq!(back.flows.len(), 2);
        assert_eq!(back.flows[0].user.as_deref(), Some(bound.as_str()));
        assert_eq!(back.flows[1].user.as_deref(), Some(NO_USER));
        let mut tampered = record.clone();
        tampered["f"][0]["u"] = json!("not a digest!");
        assert_eq!(FlowSet::decode(&tampered, "google", false).flows.len(), 1, "the entry with a malformed binding is no flow");
    }
}
