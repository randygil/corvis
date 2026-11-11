//! A minimal OAuth2 provider for `--dev` and the tests: an authorization endpoint that approves every request, a token
//! endpoint that checks the code, the PKCE verifier and the client secret, and a userinfo endpoint.
//!
//! It listens on `127.0.0.1` only, never contacts anything, and is started only by `Settings::dev` and by tests. A
//! deployment points the `OAUTH_*` variables at a real provider and never starts it.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use corvis::tokio::io::{AsyncReadExt, AsyncWriteExt};
use corvis::tokio::net::{TcpListener, TcpStream};
use corvis::tokio::task::JoinHandle;
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::settings::{OAuthClient, random_secret};

const CLIENT_ID: &str = "auth-app-demo";
const MAX_HEAD: usize = 16 * 1024;
const MAX_BODY: usize = 8 * 1024;

/// The user the provider reports, as the OpenID `userinfo` document: `sub`, `name`, `email`, `email_verified`.
#[derive(Debug, Clone)]
pub struct MockProfile {
    pub sub: String,
    pub name: String,
    pub email: String,
    pub email_verified: bool,
}

impl Default for MockProfile {
    fn default() -> Self {
        MockProfile { sub: "mock-1001".into(), name: "Mock Person".into(), email: "mock.person@example.test".into(), email_verified: true }
    }
}

#[derive(Default)]
struct State {
    /// code -> (PKCE challenge, redirect URI)
    codes: Mutex<HashMap<String, (String, String)>>,
    tokens: Mutex<HashSet<String>>,
    profile: Mutex<MockProfile>,
    token_requests: AtomicUsize,
    userinfo_requests: AtomicUsize,
    client_secret: String,
}

pub struct MockProvider {
    addr: SocketAddr,
    state: Arc<State>,
    task: JoinHandle<()>,
}

impl Drop for MockProvider {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl MockProvider {
    /// Binds a free port on `127.0.0.1` and serves until the value is dropped.
    pub async fn start() -> corvis::Result<MockProvider> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let state = Arc::new(State { client_secret: random_secret(), ..State::default() });
        let task = {
            let state = state.clone();
            corvis::tokio::spawn(async move {
                loop {
                    let Ok((stream, _)) = listener.accept().await else { return };
                    let state = state.clone();
                    corvis::tokio::spawn(async move {
                        let _ = serve(stream, state).await;
                    });
                }
            })
        };
        Ok(MockProvider { addr, state, task })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The OAuth2 client of an application whose callback is `redirect_uri`.
    pub fn client(&self, redirect_uri: String) -> OAuthClient {
        let base = format!("http://{}", self.addr);
        OAuthClient {
            authorize_url: format!("{base}/authorize"),
            token_url: format!("{base}/token"),
            userinfo_url: format!("{base}/userinfo"),
            client_id: CLIENT_ID.to_string(),
            client_secret: self.state.client_secret.clone(),
            redirect_uri,
        }
    }

    /// Changes the user the provider reports from now on.
    pub fn set_profile(&self, profile: MockProfile) {
        *self.state.profile.lock().unwrap_or_else(|e| e.into_inner()) = profile;
    }

    /// How many requests reached the token endpoint (the tests use it to show that a forged callback sent nothing).
    pub fn token_requests(&self) -> usize {
        self.state.token_requests.load(Ordering::SeqCst)
    }

    pub fn userinfo_requests(&self) -> usize {
        self.state.userinfo_requests.load(Ordering::SeqCst)
    }
}

struct Req {
    method: String,
    path: String,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

async fn serve(mut stream: TcpStream, state: Arc<State>) -> std::io::Result<()> {
    let Some(req) = read_request(&mut stream).await? else { return Ok(()) };
    let (status, headers, body) = route(&state, &req);
    let mut out = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
    for (name, value) in headers {
        out.push_str(&format!("{name}: {value}\r\n"));
    }
    out.push_str("\r\n");
    out.push_str(&body);
    stream.write_all(out.as_bytes()).await?;
    stream.shutdown().await
}

async fn read_request(stream: &mut TcpStream) -> std::io::Result<Option<Req>> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > MAX_HEAD {
            return Ok(None);
        }
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(None);
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or("").split(' ');
    let (method, target) = (first.next().unwrap_or("").to_string(), first.next().unwrap_or("").to_string());
    let headers: HashMap<String, String> =
        lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string())).collect();
    let wanted = headers.get("content-length").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
    if wanted > MAX_BODY {
        return Ok(None);
    }
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < wanted {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(None);
        }
        body.extend_from_slice(&chunk[..n]);
    }
    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    Ok(Some(Req { method, path: path.to_string(), query: form(query), headers, body }))
}

type Answer = (&'static str, Vec<(&'static str, String)>, String);

fn route(state: &State, req: &Req) -> Answer {
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/authorize") => authorize(state, req),
        ("POST", "/token") => token(state, req),
        ("GET", "/userinfo") => userinfo(state, req),
        _ => ("404 Not Found", Vec::new(), String::new()),
    }
}

/// Approves everything: a real provider would show a consent page here. The state travels back as it came.
fn authorize(state: &State, req: &Req) -> Answer {
    let q = &req.query;
    let ok = q.get("response_type").map(String::as_str) == Some("code")
        && q.get("client_id").map(String::as_str) == Some(CLIENT_ID)
        && q.get("code_challenge_method").map(String::as_str) == Some("S256");
    let (Some(redirect), Some(state_value), Some(challenge)) = (q.get("redirect_uri"), q.get("state"), q.get("code_challenge")) else {
        return ("400 Bad Request", Vec::new(), String::new());
    };
    if !ok {
        return ("400 Bad Request", Vec::new(), String::new());
    }
    let mut raw = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut raw);
    let code = URL_SAFE_NO_PAD.encode(raw);
    state.codes.lock().unwrap_or_else(|e| e.into_inner()).insert(code.clone(), (challenge.clone(), redirect.clone()));
    let location = format!("{redirect}?code={code}&state={state_value}");
    ("302 Found", vec![("Location", location)], String::new())
}

/// Checks the client secret, the redirect URI and the PKCE verifier, and spends the code.
fn token(state: &State, req: &Req) -> Answer {
    state.token_requests.fetch_add(1, Ordering::SeqCst);
    let body = form(&String::from_utf8_lossy(&req.body));
    let refuse = || ("400 Bad Request", vec![("Content-Type", "application/json".to_string())], r#"{"error":"invalid_grant"}"#.to_string());
    let field = |name: &str| body.get(name).map(String::as_str);
    if field("client_id") != Some(CLIENT_ID) || field("client_secret") != Some(state.client_secret.as_str()) {
        return refuse();
    }
    let Some(code) = field("code") else { return refuse() };
    let Some((challenge, redirect)) = state.codes.lock().unwrap_or_else(|e| e.into_inner()).remove(code) else { return refuse() };
    let verifier_ok = field("code_verifier").is_some_and(|v| URL_SAFE_NO_PAD.encode(Sha256::digest(v.as_bytes())) == challenge);
    if !verifier_ok || field("redirect_uri") != Some(redirect.as_str()) {
        return refuse();
    }
    let access = format!("mock-{}", random_secret());
    state.tokens.lock().unwrap_or_else(|e| e.into_inner()).insert(access.clone());
    (
        "200 OK",
        vec![("Content-Type", "application/json".to_string())],
        format!(r#"{{"access_token":"{access}","token_type":"Bearer","expires_in":3600}}"#),
    )
}

fn userinfo(state: &State, req: &Req) -> Answer {
    state.userinfo_requests.fetch_add(1, Ordering::SeqCst);
    let token = req.headers.get("authorization").and_then(|v| v.strip_prefix("Bearer ")).unwrap_or("");
    if !state.tokens.lock().unwrap_or_else(|e| e.into_inner()).contains(token) {
        return ("401 Unauthorized", Vec::new(), String::new());
    }
    let p = state.profile.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let document = corvis::serde_json::json!({ "sub": p.sub, "name": p.name, "email": p.email, "email_verified": p.email_verified });
    ("200 OK", vec![("Content-Type", "application/json".to_string())], document.to_string())
}

/// `a=1&b=%2F` as a map (the last value of a repeated name wins: this is a test double).
fn form(text: &str) -> HashMap<String, String> {
    text.split('&').filter(|p| !p.is_empty()).map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (percent_decode(k), percent_decode(v))
    }).collect()
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if bytes.len() >= i + 3 => {
                match std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                    Some(b) => {
                        out.push(b);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
