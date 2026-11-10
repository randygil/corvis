//! Shared by the OAuth2 test binaries: a mock provider on `127.0.0.1` (raw HTTP/1.1 over TCP, optionally TLS) that
//! records what it receives and answers what the test scripts, and an application with the three kinds of strategy
//! (Google, GitHub and generic) pointed at it. Nothing here talks to a real provider.
#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use corvis::auth::AuthGuard;
use corvis::auth::oauth2::{self, OAuth2Endpoints, OAuth2Grant, OAuth2Options, OAuth2Strategy, OAuth2Validator};
use corvis::auth::session::{SessionModule, SessionOptions};
use corvis::http::CookieKeys;
use corvis::prelude::*;
use corvis::testing::{Test, TestResponse, request};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Notify;

pub const STATE_SECRET: &str = "oauth2-test-state-secret-0123456789abcdef";
pub const CLIENT_SECRET: &str = "the-client-secret-FAKE-4242";

pub fn keys() -> CookieKeys {
    CookieKeys::new([STATE_SECRET]).unwrap()
}

// ---- the mock provider ----

/// A request the mock received.
#[derive(Debug, Clone)]
pub struct Recorded {
    pub method: String,
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Recorded {
    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap()
    }

    pub fn query(&self) -> &str {
        self.target.split_once('?').map(|(_, q)| q).unwrap_or("")
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// The form fields of the body, in order.
    pub fn form(&self) -> Vec<(String, String)> {
        form_urlencoded::parse(self.body.as_bytes()).map(|(k, v)| (k.into_owned(), v.into_owned())).collect()
    }

    pub fn field(&self, name: &str) -> Option<String> {
        self.form().into_iter().find(|(k, _)| k == name).map(|(_, v)| v)
    }
}

/// What the mock answers.
pub enum Reply {
    Http {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    Raw(Vec<u8>),
    /// Accepts the request and never answers; the mock notes when the client closes the connection.
    Hang,
    /// Holds the answer until the notify is signalled.
    Wait(Arc<Notify>, Box<Reply>),
    /// A 200 with a body of `total` bytes written in chunks, with or without `Content-Length`; the mock counts the bytes
    /// it managed to write.
    Stream {
        content_length: bool,
        total: usize,
    },
}

impl Reply {
    pub fn json(status: u16, body: &str) -> Reply {
        Reply::Http { status, headers: vec![("Content-Type".into(), "application/json".into())], body: body.as_bytes().to_vec() }
    }

    pub fn form(status: u16, body: &str) -> Reply {
        Reply::Http {
            status,
            headers: vec![("Content-Type".into(), "application/x-www-form-urlencoded".into())],
            body: body.as_bytes().to_vec(),
        }
    }

    pub fn typed(status: u16, content_type: &str, body: &str) -> Reply {
        Reply::Http { status, headers: vec![("Content-Type".into(), content_type.into())], body: body.as_bytes().to_vec() }
    }

    pub fn redirect(location: &str) -> Reply {
        Reply::Http { status: 302, headers: vec![("Location".into(), location.into())], body: Vec::new() }
    }
}

pub type Script = Arc<dyn Fn(&Recorded) -> Reply + Send + Sync>;

pub struct Mock {
    pub addr: SocketAddr,
    pub hits: Arc<Mutex<Vec<Recorded>>>,
    /// Bytes of `Reply::Stream` bodies the mock wrote.
    pub written: Arc<AtomicUsize>,
    /// Signalled once per recorded request.
    pub arrived: Arc<Notify>,
    /// Signalled when the client closed a connection the mock was holding (`Reply::Hang`).
    pub closed: Arc<Notify>,
    scheme: &'static str,
    host: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Mock {
    pub async fn start(script: impl Fn(&Recorded) -> Reply + Send + Sync + 'static) -> Mock {
        Self::start_with(Arc::new(script), None, "127.0.0.1").await
    }

    pub async fn start_tls(
        script: impl Fn(&Recorded) -> Reply + Send + Sync + 'static,
        config: Arc<tokio_rustls::rustls::ServerConfig>,
        host: &str,
    ) -> Mock {
        Self::start_with(Arc::new(script), Some(tokio_rustls::TlsAcceptor::from(config)), host).await
    }

    async fn start_with(script: Script, tls: Option<tokio_rustls::TlsAcceptor>, host: &str) -> Mock {
        let scheme = if tls.is_some() { "https" } else { "http" };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits: Arc<Mutex<Vec<Recorded>>> = Arc::default();
        let written = Arc::new(AtomicUsize::new(0));
        let arrived = Arc::new(Notify::new());
        let closed = Arc::new(Notify::new());
        let (h, w, a, c) = (hits.clone(), written.clone(), arrived.clone(), closed.clone());
        let task = tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = listener.accept().await else { return };
                let (script, h, w, a, c, tls) = (script.clone(), h.clone(), w.clone(), a.clone(), c.clone(), tls.clone());
                tokio::spawn(async move {
                    match tls {
                        Some(acceptor) => {
                            if let Ok(stream) = acceptor.accept(tcp).await {
                                handle(stream, script, h, w, a, c).await;
                            }
                        }
                        None => handle(tcp, script, h, w, a, c).await,
                    }
                });
            }
        });
        Mock { addr, hits, written, arrived, closed, scheme, host: host.to_string(), task }
    }

    /// `http://127.0.0.1:port/path`, or `https://host:port/path` for a TLS mock.
    pub fn url(&self, path: &str) -> String {
        format!("{}://{}:{}{path}", self.scheme, self.host, self.addr.port())
    }

    pub fn count(&self) -> usize {
        self.hits.lock().unwrap().len()
    }

    pub fn hits(&self) -> Vec<Recorded> {
        self.hits.lock().unwrap().clone()
    }

    pub fn hits_to(&self, path: &str) -> Vec<Recorded> {
        self.hits().into_iter().filter(|r| r.path() == path).collect()
    }
}

async fn read_request<S: AsyncRead + Unpin>(stream: &mut S) -> Option<Recorded> {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
        if buf.len() > 64 * 1024 {
            return None;
        }
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let (method, target) = (first.next()?.to_string(), first.next()?.to_string());
    let headers: Vec<(String, String)> =
        lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect();
    let length: usize =
        headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < length {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    Some(Recorded { method, target, headers, body: String::from_utf8_lossy(&body).into_owned() })
}

fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        302 => "Found",
        400 => "Bad Request",
        401 => "Unauthorized",
        500 => "Internal Server Error",
        _ => "Status",
    }
}

async fn handle<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    script: Script,
    hits: Arc<Mutex<Vec<Recorded>>>,
    written: Arc<AtomicUsize>,
    arrived: Arc<Notify>,
    closed: Arc<Notify>,
) {
    let Some(request) = read_request(&mut stream).await else { return };
    hits.lock().unwrap().push(request.clone());
    arrived.notify_one();
    let mut reply = script(&request);
    loop {
        match reply {
            Reply::Wait(gate, inner) => {
                gate.notified().await;
                reply = *inner;
            }
            Reply::Hang => {
                let mut sink = [0u8; 256];
                while matches!(stream.read(&mut sink).await, Ok(n) if n > 0) {}
                closed.notify_one();
                return;
            }
            Reply::Raw(bytes) => {
                let _ = stream.write_all(&bytes).await;
                let _ = stream.shutdown().await;
                return;
            }
            Reply::Http { status, headers, body } => {
                let mut out =
                    format!("HTTP/1.1 {status} {}\r\nContent-Length: {}\r\nConnection: close\r\n", status_text(status), body.len());
                for (k, v) in headers {
                    out.push_str(&format!("{k}: {v}\r\n"));
                }
                out.push_str("\r\n");
                let mut bytes = out.into_bytes();
                bytes.extend_from_slice(&body);
                let _ = stream.write_all(&bytes).await;
                let _ = stream.shutdown().await;
                return;
            }
            Reply::Stream { content_length, total } => {
                let mut head = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n".to_string();
                if content_length {
                    head.push_str(&format!("Content-Length: {total}\r\n"));
                }
                head.push_str("\r\n");
                if stream.write_all(head.as_bytes()).await.is_err() {
                    return;
                }
                let chunk = vec![b'a'; 8192];
                let mut sent = 0;
                while sent < total {
                    let n = chunk.len().min(total - sent);
                    if stream.write_all(&chunk[..n]).await.is_err() {
                        return;
                    }
                    sent += n;
                    written.fetch_add(n, Ordering::SeqCst);
                    tokio::task::yield_now().await;
                }
                let _ = stream.shutdown().await;
                return;
            }
        }
    }
}

// ---- documents ----

pub const GOOGLE_DOC: &str = r#"{"sub":"110169484474386276334","name":"Ana Perez","given_name":"Ana","family_name":"Perez","picture":"https://lh3.example/a.png","email":"ana@example.com","email_verified":true}"#;
pub const GITHUB_USER: &str = r#"{"login":"octocat","id":583231,"node_id":"MDQ6VXNlcjU4MzIzMQ==","avatar_url":"https://avatars.example/u/583231","html_url":"https://github.com/octocat","name":"The Octocat","email":null}"#;
pub const GITHUB_EMAILS: &str = r#"[{"email":"octocat@users.noreply.github.com","primary":false,"verified":true,"visibility":null},{"email":"octocat@example.com","primary":true,"verified":true,"visibility":"public"}]"#;

/// The standard provider: a token for any code, the Google document, the GitHub documents.
pub fn standard(request: &Recorded) -> Reply {
    match (request.method.as_str(), request.path()) {
        ("POST", "/token") => Reply::json(
            200,
            r#"{"access_token":"ACCESS-TOKEN-FAKE","token_type":"Bearer","refresh_token":"REFRESH-TOKEN-FAKE","expires_in":3600,"scope":"email profile","id_token":"ID-TOKEN-FAKE"}"#,
        ),
        ("GET", "/userinfo") => Reply::json(200, GOOGLE_DOC),
        ("GET", "/user") => Reply::json(200, GITHUB_USER),
        ("GET", "/user/emails") => Reply::json(200, GITHUB_EMAILS),
        _ => Reply::json(404, "{}"),
    }
}

// ---- the application ----

/// What a validator saw.
#[derive(Debug, Clone)]
pub struct Seen {
    pub access: String,
    pub refresh: Option<String>,
    pub expires_in: Option<u64>,
    pub scope: Option<String>,
    pub id_token: Option<String>,
    pub profile: Option<Value>,
    pub verified_email: Option<String>,
    pub debug: String,
}

#[derive(Clone)]
pub enum Verdict {
    User,
    Refuse,
    Forbidden,
    /// A user that serializes to JSON `null`.
    Empty,
}

pub struct Cfg {
    pub options: OAuth2Options,
    pub seen: Mutex<Vec<Seen>>,
    pub verdict: Mutex<Verdict>,
    pub options_calls: AtomicUsize,
}

impl Cfg {
    pub fn new(options: OAuth2Options) -> Arc<Cfg> {
        Arc::new(Cfg { options, seen: Mutex::default(), verdict: Mutex::new(Verdict::User), options_calls: AtomicUsize::new(0) })
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    pub fn set_verdict(&self, verdict: Verdict) {
        *self.verdict.lock().unwrap() = verdict;
    }

    fn validate(&self, grant: OAuth2Grant) -> Result<Option<Value>, HttpException> {
        let profile = grant.profile.as_ref();
        self.seen.lock().unwrap().push(Seen {
            access: grant.access_token.expose().to_string(),
            refresh: grant.refresh_token.as_ref().map(|t| t.expose().to_string()),
            expires_in: grant.params.expires_in(),
            scope: grant.params.scope().map(str::to_string),
            id_token: grant.params.id_token().map(|t| t.expose().to_string()),
            profile: profile.map(|p| p.to_json()),
            verified_email: profile.and_then(|p| p.verified_email()).map(str::to_string),
            debug: format!("{grant:?}"),
        });
        match self.verdict.lock().unwrap().clone() {
            Verdict::User => {
                Ok(Some(json!({ "id": profile.map(|p| format!("{}:{}", p.provider, p.id)).unwrap_or_else(|| "anon".into()) })))
            }
            Verdict::Refuse => Ok(None),
            Verdict::Forbidden => Err(ForbiddenException::new("nope")),
            Verdict::Empty => Ok(Some(Value::Null)),
        }
    }
}

macro_rules! validator {
    ($cfg:ident, $name:ident, $label:literal) => {
        pub struct $cfg(pub Arc<Cfg>);

        #[injectable]
        pub struct $name {
            cfg: Arc<$cfg>,
        }

        #[async_trait]
        impl OAuth2Validator for $name {
            const NAME: &'static str = $label;
            type User = Value;

            fn options(&self) -> OAuth2Options {
                self.cfg.0.options_calls.fetch_add(1, Ordering::SeqCst);
                self.cfg.0.options.clone()
            }

            async fn validate(&self, _req: &Request, grant: OAuth2Grant) -> Result<Option<Value>, HttpException> {
                self.cfg.0.validate(grant)
            }
        }
    };
}

validator!(GoogleCfg, GoogleAuth, "google");
validator!(GithubCfg, GithubAuth, "github");
validator!(GenericCfg, GenericAuth, "corp");

fn view(req: &Request) -> Value {
    json!({ "user": req.user(), "return_to": oauth2::return_to(req), "auth_info": req.auth_info() })
}

/// Runs the guard from an interceptor, after the guard phase, the way a careless wrapper might.
#[injectable]
pub struct GuardInInterceptor;

#[async_trait]
impl corvis::CorvisInterceptor for GuardInInterceptor {
    async fn intercept(&self, ctx: &ExecutionContext, next: corvis::CallHandler) -> HandlerResult {
        corvis::CanActivate::can_activate(&AuthGuard::new("google"), ctx).await?;
        next.handle().await
    }
}

#[controller("")]
pub struct GoogleRoutes;

#[routes]
impl GoogleRoutes {
    #[get("auth/google")]
    #[use_guards(AuthGuard::new("google"))]
    fn login(&self, req: Request) -> Value {
        view(&req)
    }

    #[get("auth/google/callback")]
    #[use_guards(AuthGuard::new("google"))]
    fn callback(&self, req: Request) -> Value {
        view(&req)
    }

    #[post("auth/google/callback")]
    #[http_code(200)]
    #[use_guards(AuthGuard::new("google"))]
    fn callback_post(&self, req: Request) -> Value {
        view(&req)
    }

    #[get("auth/google/optional")]
    #[use_guards(AuthGuard::new("google").optional())]
    fn optional(&self, req: Request) -> Value {
        view(&req)
    }

    #[get("auth/google/scoped")]
    #[use_guards(AuthGuard::new("google").with_options(json!({ "scope": ["drive.readonly"] })))]
    fn scoped(&self, req: Request) -> Value {
        view(&req)
    }

    #[get("auth/google/interceptor")]
    #[use_interceptors(GuardInInterceptor)]
    fn via_interceptor(&self) -> Value {
        json!({ "handler": "ran" })
    }

    #[get("auth/google/bad-scope")]
    #[use_guards(AuthGuard::new("google").with_options(json!({ "scope": "not a list" })))]
    fn bad_scope(&self, req: Request) -> Value {
        view(&req)
    }

    #[get("auth/google/challenge")]
    #[use_guards(AuthGuard::new("google").on_failure(|failures| BadRequestException::new(failures[0].challenge.clone().unwrap_or(Value::Null))))]
    fn challenge(&self, req: Request) -> Value {
        view(&req)
    }
}

#[controller("")]
pub struct GithubRoutes;

#[routes]
impl GithubRoutes {
    #[get("auth/github")]
    #[use_guards(AuthGuard::new("github"))]
    fn login(&self, req: Request) -> Value {
        view(&req)
    }

    #[get("auth/github/callback")]
    #[use_guards(AuthGuard::new("github"))]
    fn callback(&self, req: Request) -> Value {
        view(&req)
    }

    #[get("auth/github/scoped")]
    #[use_guards(AuthGuard::new("github").with_options(json!({ "scope": ["read:org"] })))]
    fn scoped(&self, req: Request) -> Value {
        view(&req)
    }

    #[get("auth/github/scoped/callback")]
    #[use_guards(AuthGuard::new("github").with_options(json!({ "scope": ["read:org"] })))]
    fn scoped_callback(&self, req: Request) -> Value {
        view(&req)
    }
}

#[controller("")]
pub struct GenericRoutes;

#[routes]
impl GenericRoutes {
    #[get("auth/corp")]
    #[use_guards(AuthGuard::new("corp"))]
    fn login(&self, req: Request) -> Value {
        view(&req)
    }

    #[get("auth/corp/callback")]
    #[use_guards(AuthGuard::new("corp"))]
    fn callback(&self, req: Request) -> Value {
        view(&req)
    }
}

/// Presets pointed at `mock`.
pub fn endpoints(mock: &Mock) -> OAuth2Endpoints {
    OAuth2Endpoints::new(mock.url("/authorize"), mock.url("/token")).userinfo_url(mock.url("/userinfo"))
}

pub fn google_options(mock: &Mock) -> OAuth2Options {
    OAuth2Options::google("google-client-id", CLIENT_SECRET, "https://app.example/auth/google/callback")
        .endpoints(endpoints(mock))
        .scope(["email", "profile"])
        .state_cookie(keys())
}

pub fn github_options(mock: &Mock) -> OAuth2Options {
    OAuth2Options::github("github-client-id", CLIENT_SECRET, "https://app.example/auth/github/callback")
        .endpoints(
            OAuth2Endpoints::new(mock.url("/authorize"), mock.url("/token"))
                .userinfo_url(mock.url("/user"))
                .emails_url(mock.url("/user/emails")),
        )
        .state_cookie(keys())
}

pub fn generic_options(mock: &Mock) -> OAuth2Options {
    OAuth2Options::new(endpoints(mock), "corp-client-id", CLIENT_SECRET, "https://app.example/auth/corp/callback").state_cookie(keys())
}

#[derive(Default)]
pub struct AppBuilder {
    pub google: Option<Arc<Cfg>>,
    pub github: Option<Arc<Cfg>>,
    pub generic: Option<Arc<Cfg>>,
    pub session: Option<SessionOptions>,
    pub logger: bool,
    pub logging_interceptor: bool,
}

impl AppBuilder {
    pub fn google(mut self, cfg: &Arc<Cfg>) -> Self {
        self.google = Some(cfg.clone());
        self
    }

    pub fn github(mut self, cfg: &Arc<Cfg>) -> Self {
        self.github = Some(cfg.clone());
        self
    }

    pub fn generic(mut self, cfg: &Arc<Cfg>) -> Self {
        self.generic = Some(cfg.clone());
        self
    }

    pub fn session(mut self, options: SessionOptions) -> Self {
        self.session = Some(options);
        self
    }

    pub fn logger(mut self) -> Self {
        self.logger = true;
        self
    }

    /// Installs `LoggingInterceptor` globally, which logs every request line.
    pub fn logging_interceptor(mut self) -> Self {
        self.logging_interceptor = true;
        self
    }

    /// The application, or the text of the error that stopped it.
    pub async fn try_build(self) -> Result<corvis::CorvisApplication, String> {
        let mut builder = Test::create_testing_module().with_logger(self.logger);
        if let Some(options) = self.session {
            builder = builder.import_dynamic(SessionModule::for_root(options));
        }
        if let Some(cfg) = self.google {
            builder = builder
                .providers(vec![
                    Provider::value(GoogleCfg(cfg)),
                    Provider::class::<GoogleAuth>(),
                    Provider::strategy::<OAuth2Strategy<GoogleAuth>>(),
                ])
                .controller::<GoogleRoutes>();
        }
        if let Some(cfg) = self.github {
            builder = builder
                .providers(vec![
                    Provider::value(GithubCfg(cfg)),
                    Provider::class::<GithubAuth>(),
                    Provider::strategy::<OAuth2Strategy<GithubAuth>>(),
                ])
                .controller::<GithubRoutes>();
        }
        if let Some(cfg) = self.generic {
            builder = builder
                .providers(vec![
                    Provider::value(GenericCfg(cfg)),
                    Provider::class::<GenericAuth>(),
                    Provider::strategy::<OAuth2Strategy<GenericAuth>>(),
                ])
                .controller::<GenericRoutes>();
        }
        let module = builder.compile().await.map_err(|e| format!("{e:#}"))?;
        let mut app = module.create_corvis_application();
        if self.logging_interceptor {
            app.use_global_interceptors(corvis::LoggingInterceptor);
        }
        app.init().await.map_err(|e| format!("{e:#}"))?;
        Ok(app)
    }

    pub async fn build(self) -> corvis::CorvisApplication {
        self.try_build().await.unwrap_or_else(|e| panic!("the application did not start: {e}"))
    }
}

/// The application with Google only, pointed at `mock`.
pub async fn google_app(mock: &Mock) -> (corvis::CorvisApplication, Arc<Cfg>) {
    let cfg = Cfg::new(google_options(mock));
    (AppBuilder::default().google(&cfg).build().await, cfg)
}

// ---- driving the flow ----

pub fn set_cookies(r: &TestResponse) -> Vec<String> {
    r.headers.get_all("set-cookie").iter().map(|v| v.to_str().unwrap().to_string()).collect()
}

/// The `name=value` pair of a `Set-Cookie` header.
pub fn pair(set_cookie: &str) -> String {
    set_cookie.split(';').next().unwrap().to_string()
}

pub async fn get(app: &corvis::CorvisApplication, path: &str, cookie: Option<&str>) -> TestResponse {
    let mut call = request(app).get(path);
    if let Some(cookie) = cookie {
        call = call.header("cookie", cookie);
    }
    call.send().await
}

/// A started login.
pub struct Login {
    pub response: TestResponse,
    pub location: url::Url,
    pub params: Vec<(String, String)>,
    /// The `name=value` pair of the state cookie, when there is one.
    pub cookie: Option<String>,
}

impl Login {
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn state(&self) -> String {
        self.param("state").expect("the redirect has a state").to_string()
    }

    pub fn names(&self) -> Vec<&str> {
        self.params.iter().map(|(k, _)| k.as_str()).collect()
    }
}

pub fn parse_login(response: TestResponse) -> Login {
    assert_eq!(response.status, 302, "{}", response.text());
    let location = url::Url::parse(response.header("location").expect("a Location header")).unwrap();
    let params = location.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
    let cookie = set_cookies(&response).first().map(|c| pair(c));
    Login { response, location, params, cookie }
}

pub async fn login(app: &corvis::CorvisApplication, path: &str) -> Login {
    parse_login(get(app, path, None).await)
}

pub async fn login_with_cookie(app: &corvis::CorvisApplication, path: &str, cookie: Option<&str>) -> Login {
    parse_login(get(app, path, cookie).await)
}

/// The callback of a login: `?code=<code>&state=<its state>`.
pub async fn callback(app: &corvis::CorvisApplication, path: &str, login: &Login, code: &str) -> TestResponse {
    let target = format!("{path}?code={code}&state={}", login.state());
    get(app, &target, login.cookie.as_deref()).await
}

/// The cookie pair a response's last `Set-Cookie` sets for `name`, or `None` when it expires it.
pub fn cookie_after(r: &TestResponse, name: &str) -> Option<String> {
    set_cookies(r).iter().rev().find(|c| c.starts_with(&format!("{name}="))).map(|c| pair(c))
}

/// The JSON of a 401 of the original: `{"message":"Unauthorized","statusCode":401}`.
pub fn unauthorized() -> Value {
    json!({ "message": "Unauthorized", "statusCode": 401 })
}

pub fn internal_error() -> Value {
    json!({ "statusCode": 500, "message": "Internal server error" })
}

// ---- TLS ----

pub struct Pki {
    pub root_der: Vec<u8>,
    pub server: Arc<tokio_rustls::rustls::ServerConfig>,
}

/// A throwaway root and a leaf under it valid for `names` (and 127.0.0.1 when `ip`), generated per run.
pub fn pki(names: &[&str], ip: bool) -> Pki {
    use rcgen::{BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair, KeyUsagePurpose, SanType};
    use tokio_rustls::rustls::crypto::ring;
    use tokio_rustls::rustls::pki_types::PrivateKeyDer;
    let mut root_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    root_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    root_params.distinguished_name.push(DnType::CommonName, "Corvis oauth2 test root");
    root_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign, KeyUsagePurpose::DigitalSignature];
    let root_key = KeyPair::generate().unwrap();
    let root = root_params.self_signed(&root_key).unwrap();
    let issuer = Issuer::new(root_params, root_key);
    let mut leaf_params = CertificateParams::new(names.iter().map(|n| n.to_string()).collect::<Vec<_>>()).unwrap();
    if ip {
        leaf_params.subject_alt_names.push(SanType::IpAddress(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)));
    }
    leaf_params.distinguished_name.push(DnType::CommonName, names[0]);
    leaf_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    leaf_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let leaf_key = KeyPair::generate().unwrap();
    let leaf = leaf_params.signed_by(&leaf_key, &issuer).unwrap();
    let key = PrivateKeyDer::try_from(leaf_key.serialize_der()).unwrap();
    let server = tokio_rustls::rustls::ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![leaf.der().clone()], key)
        .unwrap();
    Pki { root_der: root.der().to_vec(), server: Arc::new(server) }
}
