//! The private HTTP client of the OAuth2 strategies: one request per connection over `hyper` and `tokio-rustls`.
//!
//! What it never does: follow a redirect, read a proxy from the environment, send a request over plain `http` to
//! anything but a loopback address, skip certificate or host name verification, read a body past its limit, wait past
//! its deadline, or put text received from the provider in an error.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::Semaphore;
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::crypto::{CryptoProvider, ring};
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName};
use tokio_rustls::rustls::{ClientConfig, RootCertStore};

use super::options::{ClientAuth, OAuth2HttpOptions, OAuth2Options, host_is_loopback};
use super::profile::{self, OAuth2Profile};
use super::secret::{ClientSecret, OAuth2Token};
use crate::logger::Logger;

/// The header block of a response may not be larger than this.
const MAX_HEADER_BLOCK: usize = 16 * 1024;
/// The longest access, refresh or ID token accepted (bytes): a token ends up in a header and in the application's store.
const MAX_TOKEN_BYTES: usize = 16 * 1024;
const USER_AGENT: &str = concat!("corvis-oauth2/", env!("CARGO_PKG_VERSION"));

/// Why a request to the provider failed. No variant holds text received from the provider: a code is kept only when it
/// is 1 to 64 characters of `[A-Za-z0-9_.-]`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum OAuth2Error {
    /// A deadline passed (connect or total).
    #[error("the provider did not answer in time")]
    Timeout,
    /// The provider could not be reached.
    #[error("the provider could not be reached")]
    Connect,
    /// The TLS handshake failed: the certificate chain or the host name was not accepted, or the provider spoke no TLS.
    #[error("the TLS handshake with the provider failed")]
    Tls,
    /// A URL that is not https and does not point at a loopback address.
    #[error("the URL is not https and does not point at a loopback address")]
    InsecureUrl,
    /// The provider answered with a redirect, which is never followed.
    #[error("the provider answered with a redirect")]
    Redirected,
    /// A response was larger than its limit.
    #[error("the provider's response is larger than the limit")]
    TooLarge,
    /// A status the strategy has no use for.
    #[error("the provider answered with the status {0}")]
    Status(u16),
    /// A body, header or protocol detail that cannot be used.
    #[error("the provider's response cannot be used")]
    InvalidResponse,
    /// The provider refused the request (a 400 or 401 answer, or an `error` in a successful one). The sanitized OAuth
    /// error code (`invalid_grant`), when there is one.
    #[error("the provider rejected the request{}", .code.as_deref().map(|c| format!(" ({c})")).unwrap_or_default())]
    Rejected { code: Option<String> },
    /// Too many requests to the provider are in flight.
    #[error("too many requests to the provider are in flight")]
    Busy,
}

/// The OAuth error code of a provider answer, only when it is 1 to 64 characters of `[A-Za-z0-9_.-]`.
pub(crate) fn sanitize_error_code(code: &str) -> Option<String> {
    let ok = !code.is_empty() && code.len() <= 64 && code.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'));
    ok.then(|| code.to_string())
}

/// The token response of the provider, without the tokens (they are typed values of their own). `Debug` prints the
/// keys only.
#[derive(Clone)]
pub struct OAuth2TokenResponse {
    params: serde_json::Map<String, serde_json::Value>,
    id_token: Option<OAuth2Token>,
}

impl std::fmt::Debug for OAuth2TokenResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuth2TokenResponse").field("keys", &self.params.keys().collect::<Vec<_>>()).finish_non_exhaustive()
    }
}

impl OAuth2TokenResponse {
    /// A member of the response other than `access_token`, `refresh_token` and `id_token` (JSON members keep their
    /// type; members of a form-encoded response are strings).
    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        self.params.get(key)
    }

    /// `expires_in` in seconds, when the provider sent a number or a numeric string.
    pub fn expires_in(&self) -> Option<u64> {
        match self.params.get("expires_in")? {
            serde_json::Value::Number(n) => n.as_u64(),
            serde_json::Value::String(s) => s.parse().ok(),
            _ => None,
        }
    }

    /// The scope the provider granted, when it said.
    pub fn scope(&self) -> Option<&str> {
        self.params.get("scope").and_then(|v| v.as_str())
    }

    /// The raw OpenID Connect `id_token`. **Unverified**: its signature, issuer, audience, expiry and nonce are not
    /// checked, so read no claim from it; identify the user with the profile.
    pub fn id_token(&self) -> Option<OAuth2Token> {
        self.id_token.clone()
    }
}

/// The tokens of a successful refresh.
pub struct OAuth2TokenSet {
    pub access_token: OAuth2Token,
    pub refresh_token: Option<OAuth2Token>,
    pub params: OAuth2TokenResponse,
}

impl std::fmt::Debug for OAuth2TokenSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuth2TokenSet").field("params", &self.params).finish_non_exhaustive()
    }
}

/// What the callback exchange or a refresh produced.
pub(crate) struct TokenResult {
    pub(crate) access_token: OAuth2Token,
    pub(crate) refresh_token: Option<OAuth2Token>,
    pub(crate) response: OAuth2TokenResponse,
}

/// What a request carries.
pub(crate) struct Call<'a> {
    pub(crate) url: &'a url::Url,
    pub(crate) post_form: Option<String>,
    pub(crate) headers: Vec<(&'static str, String)>,
    pub(crate) max_bytes: usize,
}

pub(crate) struct Answer {
    pub(crate) status: u16,
    pub(crate) content_type: Option<String>,
    pub(crate) body: Vec<u8>,
}

/// The HTTP client of one strategy: its limits, its TLS configuration and its concurrency permits.
pub(crate) struct ProviderHttp {
    connect_timeout: Duration,
    timeout: Duration,
    permits: Arc<Semaphore>,
    tls: Arc<ClientConfig>,
}

impl ProviderHttp {
    pub(crate) fn new(options: &OAuth2HttpOptions) -> Result<Self, String> {
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        for der in &options.extra_roots {
            roots
                .add(CertificateDer::from(der.clone()))
                .map_err(|_| "extra_root_certificate: is not a valid DER certificate".to_string())?;
        }
        // The installed provider, or ring: `ClientConfig::builder()` panics when two providers are compiled in and none
        // is installed.
        let provider = CryptoProvider::get_default().cloned().unwrap_or_else(|| Arc::new(ring::default_provider()));
        let tls = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|_| "the TLS configuration could not be built".to_string())?
            .with_root_certificates(roots)
            .with_no_client_auth();
        Ok(ProviderHttp {
            connect_timeout: options.connect_timeout,
            timeout: options.timeout,
            permits: Arc::new(Semaphore::new(options.max_in_flight)),
            tls: Arc::new(tls),
        })
    }

    /// Sends one request: the permit is taken at once or the call is `Busy`, and the whole exchange (resolution,
    /// connection, request, response body) has the total timeout.
    pub(crate) async fn execute(&self, call: Call<'_>) -> Result<Answer, OAuth2Error> {
        let _permit = self.permits.clone().try_acquire_owned().map_err(|_| OAuth2Error::Busy)?;
        match tokio::time::timeout(self.timeout, self.run(call)).await {
            Ok(result) => result,
            Err(_) => Err(OAuth2Error::Timeout),
        }
    }

    async fn run(&self, call: Call<'_>) -> Result<Answer, OAuth2Error> {
        let url = call.url;
        let https = match url.scheme() {
            "https" => true,
            "http" => false,
            _ => return Err(OAuth2Error::InsecureUrl),
        };
        let host = url.host().ok_or(OAuth2Error::InvalidResponse)?;
        // The rule is checked again here, whatever the options said: `http` goes to loopback only.
        if !https && !host_is_loopback(&host) {
            return Err(OAuth2Error::InsecureUrl);
        }
        let port = url.port_or_known_default().ok_or(OAuth2Error::InvalidResponse)?;
        let addrs: Vec<SocketAddr> = match host {
            url::Host::Ipv4(a) => vec![SocketAddr::new(a.into(), port)],
            url::Host::Ipv6(a) => vec![SocketAddr::new(a.into(), port)],
            url::Host::Domain(d) => tokio::net::lookup_host((d, port)).await.map_err(|_| OAuth2Error::Connect)?.collect(),
        };
        if addrs.is_empty() {
            return Err(OAuth2Error::Connect);
        }
        if !addresses_allowed(https, &addrs) {
            return Err(OAuth2Error::InsecureUrl);
        }
        let connect = async {
            let mut last = OAuth2Error::Connect;
            for addr in addrs {
                match TcpStream::connect(addr).await {
                    Ok(tcp) => {
                        let _ = tcp.set_nodelay(true);
                        if !https {
                            return Ok(Stream::Plain(tcp));
                        }
                        let name = match url.host() {
                            Some(url::Host::Domain(d)) => ServerName::try_from(d.to_string()).map_err(|_| OAuth2Error::Tls)?,
                            Some(url::Host::Ipv4(a)) => ServerName::IpAddress(std::net::IpAddr::V4(a).into()),
                            Some(url::Host::Ipv6(a)) => ServerName::IpAddress(std::net::IpAddr::V6(a).into()),
                            None => return Err(OAuth2Error::InvalidResponse),
                        };
                        let tls = TlsConnector::from(self.tls.clone()).connect(name, tcp).await.map_err(|_| OAuth2Error::Tls)?;
                        return Ok(Stream::Tls(Box::new(tls)));
                    }
                    Err(_) => last = OAuth2Error::Connect,
                }
            }
            Err(last)
        };
        let stream = match tokio::time::timeout(self.connect_timeout, connect).await {
            Ok(result) => result?,
            Err(_) => return Err(OAuth2Error::Timeout),
        };
        exchange(stream, &call).await
    }
}

/// Whether the addresses a name resolved to may be connected to: `http` only when every one of them is loopback.
fn addresses_allowed(https: bool, addrs: &[SocketAddr]) -> bool {
    https || addrs.iter().all(|a| a.ip().to_canonical().is_loopback())
}

/// A connected stream, plain (loopback) or TLS.
enum Stream {
    Plain(TcpStream),
    Tls(Box<tokio_rustls::client::TlsStream<TcpStream>>),
}

async fn exchange(stream: Stream, call: &Call<'_>) -> Result<Answer, OAuth2Error> {
    match stream {
        Stream::Plain(s) => exchange_io(s, call).await,
        Stream::Tls(s) => exchange_io(*s, call).await,
    }
}

/// Aborts the connection driver when the request ends, whatever way it ends.
struct Driver(tokio::task::JoinHandle<()>);

impl Drop for Driver {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn exchange_io<S>(io: S, call: &Call<'_>) -> Result<Answer, OAuth2Error>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut sender, conn) = hyper::client::conn::http1::Builder::new()
        .max_buf_size(MAX_HEADER_BLOCK)
        .handshake::<_, Full<Bytes>>(TokioIo::new(io))
        .await
        .map_err(|_| OAuth2Error::InvalidResponse)?;
    let _driver = Driver(tokio::spawn(async move {
        let _ = conn.await;
    }));
    let url = call.url;
    let mut target = url.path().to_string();
    if let Some(q) = url.query() {
        target.push('?');
        target.push_str(q);
    }
    let host_header = match url.port() {
        Some(p) => format!("{}:{p}", url.host_str().unwrap_or("")),
        None => url.host_str().unwrap_or("").to_string(),
    };
    let mut builder = http::Request::builder()
        .method(if call.post_form.is_some() { "POST" } else { "GET" })
        .uri(target)
        .header("host", host_header)
        .header("user-agent", USER_AGENT)
        .header("accept-encoding", "identity")
        .header("connection", "close");
    for (name, value) in &call.headers {
        let mut value = http::HeaderValue::from_str(value).map_err(|_| OAuth2Error::InvalidResponse)?;
        if name.eq_ignore_ascii_case("authorization") {
            value.set_sensitive(true);
        }
        builder = builder.header(*name, value);
    }
    let body = call.post_form.clone().map(|b| Full::new(Bytes::from(b))).unwrap_or_else(|| Full::new(Bytes::new()));
    let request = builder.body(body).map_err(|_| OAuth2Error::InvalidResponse)?;
    let response = sender
        .send_request(request)
        .await
        .map_err(|e| if e.is_parse_too_large() { OAuth2Error::TooLarge } else { OAuth2Error::InvalidResponse })?;
    let status = response.status();
    // A redirect is a failed request: `Location` is never read.
    if status.is_redirection() {
        return Err(OAuth2Error::Redirected);
    }
    let headers = response.headers();
    if let Some(len) = headers.get(http::header::CONTENT_LENGTH) {
        let len = len.to_str().ok().and_then(|s| s.parse::<u64>().ok()).ok_or(OAuth2Error::InvalidResponse)?;
        if len > call.max_bytes as u64 {
            return Err(OAuth2Error::TooLarge);
        }
    }
    if let Some(encoding) = headers.get(http::header::CONTENT_ENCODING)
        && !encoding.as_bytes().eq_ignore_ascii_case(b"identity")
    {
        return Err(OAuth2Error::InvalidResponse);
    }
    let content_type = headers.get(http::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(str::to_string);
    let mut body = response.into_body();
    let mut data: Vec<u8> = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| OAuth2Error::InvalidResponse)?;
        if let Some(chunk) = frame.data_ref() {
            if data.len() + chunk.len() > call.max_bytes {
                return Err(OAuth2Error::TooLarge);
            }
            data.extend_from_slice(chunk);
        }
    }
    Ok(Answer { status: status.as_u16(), content_type, body: data })
}

/// The media type of a `Content-Type` value, lowercase, without parameters.
pub(crate) fn media_type(content_type: Option<&str>) -> String {
    content_type.and_then(|c| c.split(';').next()).map(|m| m.trim().to_ascii_lowercase()).unwrap_or_default()
}

fn is_json(media: &str) -> bool {
    media == "application/json" || (media.starts_with("application/") && media.ends_with("+json"))
}

/// Parses a form-encoded body into a map of strings; `None` for a repeated key or an oversized value count.
fn parse_form(body: &[u8]) -> Option<serde_json::Map<String, serde_json::Value>> {
    let mut map = serde_json::Map::new();
    for (k, v) in form_urlencoded::parse(body) {
        if map.len() >= 64 || map.contains_key(k.as_ref()) {
            return None;
        }
        map.insert(k.into_owned(), serde_json::Value::String(v.into_owned()));
    }
    Some(map)
}

/// The members of a token or error answer: JSON for a JSON media type, a form body for the form and `text/plain` types
/// (what GitHub sends unless asked for JSON).
fn parse_members(answer: &Answer) -> Option<serde_json::Map<String, serde_json::Value>> {
    let media = media_type(answer.content_type.as_deref());
    if is_json(&media) {
        super::json::parse_object(&answer.body)
    } else if media == "application/x-www-form-urlencoded" || media == "text/plain" {
        parse_form(&answer.body)
    } else {
        None
    }
}

/// The sanitized `error` of a provider answer, when its body can be read.
fn error_code(answer: &Answer) -> Option<String> {
    parse_members(answer).and_then(|m| m.get("error").and_then(|v| v.as_str()).and_then(sanitize_error_code))
}

/// A token answer: a non-empty string `access_token`, a `bearer` `token_type` when present.
fn read_token(answer: Answer) -> Result<TokenResult, OAuth2Error> {
    match answer.status {
        200..=299 => {}
        400 | 401 => return Err(OAuth2Error::Rejected { code: error_code(&answer) }),
        other => return Err(OAuth2Error::Status(other)),
    }
    let mut members = parse_members(&answer).ok_or(OAuth2Error::InvalidResponse)?;
    // Some providers answer a refused code with a 200 and an `error` member (GitHub does).
    if members.get("access_token").is_none()
        && let Some(error) = members.get("error")
    {
        return Err(OAuth2Error::Rejected { code: error.as_str().and_then(sanitize_error_code) });
    }
    let access = match members.remove("access_token") {
        Some(serde_json::Value::String(s)) if !s.is_empty() && s.len() <= MAX_TOKEN_BYTES => OAuth2Token::new(s),
        _ => return Err(OAuth2Error::InvalidResponse),
    };
    if let Some(kind) = members.get("token_type") {
        match kind.as_str() {
            Some(k) if k.eq_ignore_ascii_case("bearer") => {}
            _ => return Err(OAuth2Error::InvalidResponse),
        }
    }
    let refresh = match members.remove("refresh_token") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(s)) if s.is_empty() => None,
        Some(serde_json::Value::String(s)) if s.len() <= MAX_TOKEN_BYTES => Some(OAuth2Token::new(s)),
        Some(_) => return Err(OAuth2Error::InvalidResponse),
    };
    let id_token = match members.remove("id_token") {
        Some(serde_json::Value::String(s)) if !s.is_empty() && s.len() <= MAX_TOKEN_BYTES => Some(OAuth2Token::new(s)),
        Some(serde_json::Value::String(s)) if s.len() > MAX_TOKEN_BYTES => return Err(OAuth2Error::InvalidResponse),
        _ => None,
    };
    Ok(TokenResult { access_token: access, refresh_token: refresh, response: OAuth2TokenResponse { params: members, id_token } })
}

/// `application/x-www-form-urlencoded` encoding of a string (RFC 6749 appendix B), for the Basic credentials.
fn form_encode(value: &str) -> String {
    form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

/// What the token endpoint is asked.
pub(crate) enum Grant<'a> {
    AuthorizationCode { code: &'a str, verifier: Option<&'a str> },
    Refresh { refresh_token: &'a str },
}

/// A bearer value that fits in a header: visible ASCII only.
fn bearer(token: &OAuth2Token) -> Result<String, OAuth2Error> {
    let t = token.expose();
    if t.is_empty() || !t.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(OAuth2Error::InvalidResponse);
    }
    Ok(format!("Bearer {t}"))
}

/// The client of a strategy: the options it was built from and the HTTP client.
pub(crate) struct Provider {
    pub(crate) http: ProviderHttp,
    token_url: url::Url,
    userinfo_url: Option<url::Url>,
    emails_url: Option<url::Url>,
    client_id: String,
    client_secret: ClientSecret,
    client_auth: ClientAuth,
    redirect_uri: String,
    refresh_scope: Option<Vec<String>>,
    scope_separator: char,
    max_token_bytes: usize,
    max_profile_bytes: usize,
    kind: super::options::ProfileKind,
    preset: Option<&'static str>,
    issuer: String,
}

impl Provider {
    /// Builds the client of validated options (see `OAuth2Options::problems`).
    pub(crate) fn new(options: &OAuth2Options) -> Result<Self, String> {
        let parse = |s: &str| url::Url::parse(s).map_err(|_| "an endpoint URL is not valid".to_string());
        Ok(Provider {
            http: ProviderHttp::new(&options.http)?,
            token_url: parse(&options.endpoints.token_url)?,
            userinfo_url: options.endpoints.userinfo_url.as_deref().map(parse).transpose()?,
            emails_url: options.endpoints.emails_url.as_deref().map(parse).transpose()?,
            client_id: options.client_id.clone(),
            client_secret: options.client_secret.clone(),
            client_auth: options.client_auth,
            redirect_uri: options.redirect_uri.clone(),
            refresh_scope: options.refresh_scope.clone(),
            scope_separator: options.scope_separator,
            max_token_bytes: options.http.max_token_bytes,
            max_profile_bytes: options.http.max_profile_bytes,
            kind: options.kind,
            preset: options.preset_name(),
            issuer: options.issuer(),
        })
    }

    pub(crate) fn token_host(&self) -> String {
        self.token_url.host_str().unwrap_or("").to_string()
    }

    /// The form fields of a token request, in the order `passport-oauth2` writes them, and the headers of the call.
    fn token_request(&self, grant: &Grant<'_>) -> (String, Vec<(&'static str, String)>) {
        let mut form = form_urlencoded::Serializer::new(String::new());
        let mut headers =
            vec![("content-type", "application/x-www-form-urlencoded".to_string()), ("accept", "application/json".to_string())];
        match grant {
            Grant::AuthorizationCode { verifier, .. } => {
                form.append_pair("grant_type", "authorization_code");
                form.append_pair("redirect_uri", &self.redirect_uri);
                if let Some(v) = verifier {
                    form.append_pair("code_verifier", v);
                }
            }
            Grant::Refresh { .. } => {
                form.append_pair("grant_type", "refresh_token");
                if let Some(scope) = &self.refresh_scope
                    && !scope.is_empty()
                {
                    form.append_pair("scope", &scope.join(&self.scope_separator.to_string()));
                }
            }
        }
        match self.client_auth {
            ClientAuth::RequestBody => {
                form.append_pair("client_id", &self.client_id);
                form.append_pair("client_secret", self.client_secret.expose());
            }
            ClientAuth::Basic => {
                use base64::Engine as _;
                let raw = format!("{}:{}", form_encode(&self.client_id), form_encode(self.client_secret.expose()));
                headers.push(("authorization", format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(raw))));
            }
        }
        match grant {
            Grant::AuthorizationCode { code, .. } => form.append_pair("code", code),
            Grant::Refresh { refresh_token } => form.append_pair("refresh_token", refresh_token),
        };
        (form.finish(), headers)
    }

    /// One token request. A rejection is logged (rate limited by the caller's log filter) with the strategy name, the
    /// host and the sanitized code only.
    pub(crate) async fn token(&self, grant: Grant<'_>) -> Result<TokenResult, OAuth2Error> {
        let (body, headers) = self.token_request(&grant);
        let answer =
            self.http.execute(Call { url: &self.token_url, post_form: Some(body), headers, max_bytes: self.max_token_bytes }).await?;
        read_token(answer)
    }

    /// The profile of the user: the preset's mapping over its requests. `scope` is the effective scope of the login.
    pub(crate) async fn profile(
        &self,
        provider_name: &str,
        token: &OAuth2Token,
        scope: &[String],
    ) -> Result<Option<OAuth2Profile>, OAuth2Error> {
        let Some(url) = &self.userinfo_url else { return Ok(None) };
        let auth = bearer(token)?;
        fn get_call<'a>(url: &'a url::Url, auth: &str, accept: &str, max_bytes: usize) -> Call<'a> {
            Call { url, post_form: None, headers: vec![("authorization", auth.to_string()), ("accept", accept.to_string())], max_bytes }
        }
        let max = self.max_profile_bytes;
        let accept = if self.kind == super::options::ProfileKind::GitHub { "application/vnd.github+json" } else { "application/json" };
        let answer = self.http.execute(get_call(url, &auth, accept, max)).await?;
        let document = json_answer(answer)?;
        let mut profile = match self.kind {
            super::options::ProfileKind::GitHub => {
                let mut profile = profile::github_user(&document).ok_or(OAuth2Error::InvalidResponse)?;
                if OAuth2Options::can_read_emails(scope)
                    && let Some(emails_url) = &self.emails_url
                {
                    let answer = self.http.execute(get_call(emails_url, &auth, accept, max)).await?;
                    let emails = json_answer_any(answer)?;
                    profile::github_emails(&mut profile, &emails).ok_or(OAuth2Error::InvalidResponse)?;
                }
                profile
            }
            super::options::ProfileKind::Google | super::options::ProfileKind::Generic => {
                profile::openid(&document, provider_name).ok_or(OAuth2Error::InvalidResponse)?
            }
        };
        // The namespace of the id: the preset's name only while the preset's own endpoints are in use.
        profile.provider = super::options::provider_namespace(self.preset, provider_name, &self.issuer);
        profile.issuer = self.issuer.clone();
        Ok(Some(profile))
    }
}

/// A profile answer that must be a JSON object with a 2xx status.
fn json_answer(answer: Answer) -> Result<serde_json::Map<String, serde_json::Value>, OAuth2Error> {
    match json_answer_any(answer)? {
        serde_json::Value::Object(map) => Ok(map),
        _ => Err(OAuth2Error::InvalidResponse),
    }
}

fn json_answer_any(answer: Answer) -> Result<serde_json::Value, OAuth2Error> {
    match answer.status {
        200..=299 => {}
        other => return Err(OAuth2Error::Status(other)),
    }
    if !is_json(&media_type(answer.content_type.as_deref())) {
        return Err(OAuth2Error::InvalidResponse);
    }
    super::json::parse(&answer.body).ok_or(OAuth2Error::InvalidResponse)
}

/// Exchanges refresh tokens with the same client and limits as the login: build it from the options of a strategy and
/// call [`refresh`](OAuth2Client::refresh) when an access token has expired. The framework stores no token and
/// refreshes nothing by itself.
///
/// ```ignore
/// let client = OAuth2Client::new(GoogleAuth::options_for(&config))?;
/// let tokens = client.refresh(stored_refresh_token).await?;
/// ```
pub struct OAuth2Client {
    provider: Provider,
}

impl std::fmt::Debug for OAuth2Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuth2Client")
            .field("client_id", &self.provider.client_id)
            .field("token_host", &self.provider.token_host())
            .finish_non_exhaustive()
    }
}

impl OAuth2Client {
    /// Validates `options` (every rule of the strategy, except that the state storage need not be chosen) and builds
    /// the client.
    pub fn new(options: OAuth2Options) -> Result<Self, super::options::OAuth2OptionsError> {
        let mut options = options;
        if matches!(options.storage, super::options::StateStorage::Unset) {
            options.storage = super::options::StateStorage::Session;
        }
        let problems = options.problems();
        if !problems.is_empty() {
            return Err(super::options::OAuth2OptionsError(format!("invalid OAuth2 options: {}", problems.join("; "))));
        }
        let provider = Provider::new(&options).map_err(|p| super::options::OAuth2OptionsError(format!("invalid OAuth2 options: {p}")))?;
        Ok(OAuth2Client { provider })
    }

    /// `grant_type=refresh_token` with the client credentials, as `passport-oauth2-refresh` sends it.
    pub async fn refresh(&self, refresh_token: &str) -> Result<OAuth2TokenSet, OAuth2Error> {
        if refresh_token.is_empty() || refresh_token.len() > 16 * 1024 {
            return Err(OAuth2Error::InvalidResponse);
        }
        let result = self.provider.token(Grant::Refresh { refresh_token }).await;
        if let Err(error) = &result {
            Logger::new("OAuth2Client").warn(format!("refresh request to {} failed: {error}", self.provider.token_host()));
        }
        let result = result?;
        Ok(OAuth2TokenSet { access_token: result.access_token, refresh_token: result.refresh_token, params: result.response })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(status: u16, content_type: &str, body: &str) -> Answer {
        Answer { status, content_type: Some(content_type.to_string()), body: body.as_bytes().to_vec() }
    }

    #[test]
    fn codes_are_sanitized() {
        assert_eq!(sanitize_error_code("invalid_grant").as_deref(), Some("invalid_grant"));
        assert_eq!(sanitize_error_code("a.b-c_D9").as_deref(), Some("a.b-c_D9"));
        assert_eq!(sanitize_error_code(""), None);
        assert_eq!(sanitize_error_code(&"a".repeat(64)).as_deref(), Some(&*"a".repeat(64)));
        assert_eq!(sanitize_error_code(&"a".repeat(65)), None);
        assert_eq!(sanitize_error_code("<script>"), None);
        assert_eq!(sanitize_error_code("a b"), None);
        assert_eq!(sanitize_error_code("caf\u{e9}"), None);
    }

    #[test]
    fn token_answers() {
        let t = read_token(answer(
            200,
            "application/json",
            r#"{"access_token":"t","token_type":"Bearer","refresh_token":"r","id_token":"i","expires_in":3600,"scope":"a b"}"#,
        ))
        .unwrap();
        assert_eq!(t.access_token.expose(), "t");
        assert_eq!(t.refresh_token.as_ref().unwrap().expose(), "r");
        assert_eq!(t.response.expires_in(), Some(3600));
        assert_eq!(t.response.scope(), Some("a b"));
        assert_eq!(t.response.id_token().unwrap().expose(), "i");
        assert!(
            t.response.get("access_token").is_none() && t.response.get("refresh_token").is_none() && t.response.get("id_token").is_none()
        );
        let printed = format!("{:?}", t.response);
        assert!(printed.contains("expires_in") && !printed.contains("3600") && !printed.contains("\"t\""), "{printed}");
    }

    #[test]
    fn form_encoded_token_answers() {
        let t = read_token(answer(200, "application/x-www-form-urlencoded", "access_token=t&token_type=bearer&scope=a%2Cb")).unwrap();
        assert_eq!(t.access_token.expose(), "t");
        assert_eq!(t.response.scope(), Some("a,b"));
        assert!(read_token(answer(200, "text/plain; charset=utf-8", "access_token=t")).is_ok());
        assert_eq!(
            read_token(answer(200, "application/x-www-form-urlencoded", "access_token=t&access_token=u")).err(),
            Some(OAuth2Error::InvalidResponse)
        );
    }

    #[test]
    fn unusable_token_answers() {
        fn bad(status: u16, ct: &str, body: &str) -> OAuth2Error {
            read_token(answer(status, ct, body)).err().unwrap()
        }
        assert_eq!(bad(200, "application/json", "{}"), OAuth2Error::InvalidResponse);
        assert_eq!(bad(200, "application/json", r#"{"access_token":""}"#), OAuth2Error::InvalidResponse);
        assert_eq!(bad(200, "application/json", r#"{"access_token":5}"#), OAuth2Error::InvalidResponse);
        assert_eq!(bad(200, "application/json", r#"{"access_token":"t","token_type":"mac"}"#), OAuth2Error::InvalidResponse);
        assert_eq!(bad(200, "application/json", r#"{"access_token":"t","token_type":5}"#), OAuth2Error::InvalidResponse);
        let huge = "a".repeat(16 * 1024 + 1);
        assert_eq!(bad(200, "application/json", &format!(r#"{{"access_token":"{huge}"}}"#)), OAuth2Error::InvalidResponse);
        assert_eq!(
            bad(200, "application/json", &format!(r#"{{"access_token":"t","refresh_token":"{huge}"}}"#)),
            OAuth2Error::InvalidResponse
        );
        assert_eq!(bad(200, "application/json", &format!(r#"{{"access_token":"t","id_token":"{huge}"}}"#)), OAuth2Error::InvalidResponse);
        assert_eq!(bad(200, "application/json", r#"{"access_token":"t","refresh_token":7}"#), OAuth2Error::InvalidResponse);
        assert_eq!(bad(200, "application/json", "[1]"), OAuth2Error::InvalidResponse);
        assert_eq!(bad(200, "application/json", "nope"), OAuth2Error::InvalidResponse);
        assert_eq!(bad(200, "text/html", r#"{"access_token":"t"}"#), OAuth2Error::InvalidResponse);
        assert_eq!(bad(200, "", r#"{"access_token":"t"}"#), OAuth2Error::InvalidResponse);
        assert_eq!(bad(500, "application/json", r#"{"error":"x"}"#), OAuth2Error::Status(500));
        assert_eq!(bad(403, "application/json", "{}"), OAuth2Error::Status(403));
    }

    #[test]
    fn rejections_carry_only_a_sanitized_code() {
        let r = |status, body: &str| read_token(answer(status, "application/json", body)).err().unwrap();
        assert_eq!(
            r(400, r#"{"error":"invalid_grant","error_description":"secret text"}"#),
            OAuth2Error::Rejected { code: Some("invalid_grant".into()) }
        );
        assert_eq!(r(401, r#"{"error":"<script>"}"#), OAuth2Error::Rejected { code: None });
        assert_eq!(r(400, "garbage"), OAuth2Error::Rejected { code: None });
        assert_eq!(r(200, r#"{"error":"bad_verification_code"}"#), OAuth2Error::Rejected { code: Some("bad_verification_code".into()) });
        let text = OAuth2Error::Rejected { code: Some("invalid_grant".into()) }.to_string();
        assert!(text.contains("invalid_grant"));
        assert_eq!(OAuth2Error::Rejected { code: None }.to_string(), "the provider rejected the request");
    }

    fn provider(auth: ClientAuth, secret: &str) -> Provider {
        let options = OAuth2Options::github("id:1 %", secret, "https://app.example/cb").client_auth(auth).state_in_session();
        Provider::new(&options).unwrap()
    }

    #[test]
    fn the_token_request_has_the_fields_of_the_original() {
        let p = provider(ClientAuth::RequestBody, "sec ret");
        let (body, headers) = p.token_request(&Grant::AuthorizationCode { code: "c d", verifier: Some("v") });
        assert_eq!(
            body,
            "grant_type=authorization_code&redirect_uri=https%3A%2F%2Fapp.example%2Fcb&code_verifier=v&client_id=id%3A1+%25&client_secret=sec+ret&code=c+d"
        );
        assert!(headers.iter().all(|(n, _)| *n != "authorization"));
        let (body, _) = p.token_request(&Grant::Refresh { refresh_token: "r" });
        assert_eq!(body, "grant_type=refresh_token&client_id=id%3A1+%25&client_secret=sec+ret&refresh_token=r");
    }

    #[test]
    fn basic_authentication_form_encodes_before_base64() {
        use base64::Engine as _;
        let p = provider(ClientAuth::Basic, "se:cr%et");
        let (body, headers) = p.token_request(&Grant::AuthorizationCode { code: "c", verifier: None });
        assert!(!body.contains("client_secret") && !body.contains("client_id") && !body.contains("code_verifier"), "{body}");
        let (_, value) = headers.iter().find(|(n, _)| *n == "authorization").unwrap();
        let decoded = base64::engine::general_purpose::STANDARD.decode(value.strip_prefix("Basic ").unwrap()).unwrap();
        assert_eq!(String::from_utf8(decoded).unwrap(), "id%3A1+%25:se%3Acr%25et");
    }

    #[test]
    fn the_refresh_scope_is_sent_only_when_set() {
        let options = OAuth2Options::github("i", "s", "https://app.example/cb").refresh_scope(["a", "b"]).state_in_session();
        let (body, _) = Provider::new(&options).unwrap().token_request(&Grant::Refresh { refresh_token: "r" });
        assert!(body.contains("scope=a%2Cb"), "{body}");
    }

    #[test]
    fn plain_http_goes_to_loopback_addresses_only() {
        let addr = |s: &str| s.parse::<SocketAddr>().unwrap();
        for ok in ["127.0.0.1:80", "[::1]:80", "127.9.9.9:1", "[::ffff:127.0.0.1]:80"] {
            assert!(addresses_allowed(false, &[addr(ok)]), "{ok}");
        }
        for bad in ["10.0.0.5:80", "192.168.1.1:80", "8.8.8.8:80", "[2001:db8::1]:80", "[::ffff:10.0.0.1]:80", "0.0.0.0:80"] {
            assert!(!addresses_allowed(false, &[addr(bad)]), "{bad}");
        }
        assert!(!addresses_allowed(false, &[addr("127.0.0.1:80"), addr("10.0.0.5:80")]), "one stray address refuses the name");
        assert!(addresses_allowed(true, &[addr("10.0.0.5:443")]));
    }

    #[test]
    fn media_types() {
        assert_eq!(media_type(Some("Application/JSON; charset=utf-8")), "application/json");
        assert_eq!(media_type(None), "");
        assert!(is_json("application/vnd.github+json"));
        assert!(!is_json("text/json"));
    }

    #[test]
    fn a_bearer_header_is_refused_for_a_token_that_does_not_fit() {
        assert!(bearer(&OAuth2Token::new("ok-token".into())).is_ok());
        assert!(bearer(&OAuth2Token::new("a b".into())).is_err());
        assert!(bearer(&OAuth2Token::new("a\r\nb".into())).is_err());
        assert!(bearer(&OAuth2Token::new(String::new())).is_err());
    }
}
