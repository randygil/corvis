//! Testing utilities in the style of `supertest`: `Test::create_testing_module()`, provider and
//! enhancer overrides, and an in-memory HTTP client.
//!
//! ```ignore
//! let module = Test::create_testing_module()
//!     .import::<CatsModule>()
//!     .override_provider::<CatsService>().use_value(CatsService::mock())
//!     .compile()
//!     .await?;
//! let mut app = module.create_corvis_application();
//! app.init().await?;
//! request(&app).get("/cats").send().await.expect_status(200);
//! ```

use std::sync::Arc;

use bytes::Bytes;
use http::{HeaderMap, HeaderName, HeaderValue, Method, Uri};
use http_body_util::BodyExt;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::application::{CorvisApplication, CorvisApplicationOptions, LoggerOption};
use crate::common::filter::ExceptionFilter;
use crate::common::guard::CanActivate;
use crate::common::interceptor::CorvisInterceptor;
use crate::common::pipe::PipeTransform;
use crate::di::container::Overrides;
use crate::di::{DiError, DynamicModule, Injectable, Module, ModuleDefinition, ModuleImport, Provider, Token};
use crate::router::controller::{Controller, ControllerDef};
use crate::router::{AppShared, dispatch};

/// The synthetic root module of a testing module.
#[derive(Default)]
pub struct RootTestModule;
crate::default_injectable!(RootTestModule);
impl Module for RootTestModule {
    fn definition() -> ModuleDefinition {
        ModuleDefinition::new::<RootTestModule>("RootTestModule").module_provider(Provider::class::<RootTestModule>())
    }
}

/// Entry point of the testing utilities.
pub struct Test;

impl Test {
    /// `Test.createTestingModule({ ... })`.
    pub fn create_testing_module() -> TestingModuleBuilder {
        TestingModuleBuilder {
            imports: Vec::new(),
            providers: Vec::new(),
            controllers: Vec::new(),
            overrides: Overrides::default(),
            logger: false,
        }
    }
}

pub(crate) fn enhancer_override_token<E: ?Sized + 'static>() -> Token {
    Token::named(format!("__corvis_override:{}", std::any::type_name::<E>()))
}

pub struct TestingModuleBuilder {
    imports: Vec<ModuleImport>,
    providers: Vec<Provider>,
    controllers: Vec<ControllerDef>,
    overrides: Overrides,
    logger: bool,
}

impl TestingModuleBuilder {
    pub fn import<M: Module>(mut self) -> Self {
        self.imports.push(ModuleImport::of::<M>());
        self
    }
    pub fn import_dynamic(mut self, m: DynamicModule) -> Self {
        self.imports.push(m.into());
        self
    }
    pub fn imports(mut self, imports: Vec<ModuleImport>) -> Self {
        self.imports.extend(imports);
        self
    }
    pub fn provider<T: Injectable>(mut self) -> Self {
        self.providers.push(Provider::class::<T>());
        self
    }
    pub fn providers(mut self, providers: Vec<Provider>) -> Self {
        self.providers.extend(providers);
        self
    }
    pub fn controller<C: Controller>(mut self) -> Self {
        self.controllers.push(ControllerDef::of::<C>());
        self
    }
    pub fn controllers(mut self, controllers: Vec<ControllerDef>) -> Self {
        self.controllers.extend(controllers);
        self
    }
    /// Show framework logs while testing (hidden by default, except errors).
    pub fn with_logger(mut self, enabled: bool) -> Self {
        self.logger = enabled;
        self
    }

    /// `.overrideProvider(CatsService)`.
    pub fn override_provider<T: ?Sized + 'static>(self) -> OverrideBy {
        OverrideBy { builder: self, token: Token::of::<T>() }
    }

    /// `.overrideProvider('TOKEN')`.
    pub fn override_provider_token(self, token: impl Into<Token>) -> OverrideBy {
        OverrideBy { builder: self, token: token.into() }
    }

    /// `.overrideModule(ProdModule).useModule(MockModule)`.
    pub fn override_module<M: Module, R: Module>(mut self) -> Self {
        self.overrides.modules.insert(std::any::TypeId::of::<M>(), R::definition);
        self
    }

    /// `.overrideGuard(AuthGuard).useValue(mock)`.
    pub fn override_guard<G: 'static>(self, mock: impl CanActivate) -> Self {
        self.override_enhancer(Provider::use_value_arc(enhancer_override_token::<G>(), Arc::new(mock) as Arc<dyn CanActivate>))
    }
    /// `.overrideInterceptor(LoggingInterceptor).useValue(mock)`.
    pub fn override_interceptor<I: 'static>(self, mock: impl CorvisInterceptor) -> Self {
        self.override_enhancer(Provider::use_value_arc(enhancer_override_token::<I>(), Arc::new(mock) as Arc<dyn CorvisInterceptor>))
    }
    /// `.overridePipe(ValidationPipe).useValue(mock)`.
    pub fn override_pipe<P: 'static>(self, mock: impl PipeTransform) -> Self {
        self.override_enhancer(Provider::use_value_arc(enhancer_override_token::<P>(), Arc::new(mock) as Arc<dyn PipeTransform>))
    }
    /// `.overrideFilter(HttpExceptionFilter).useValue(mock)`.
    pub fn override_filter<F: 'static>(self, mock: impl ExceptionFilter) -> Self {
        self.override_enhancer(Provider::use_value_arc(enhancer_override_token::<F>(), Arc::new(mock) as Arc<dyn ExceptionFilter>))
    }

    fn override_enhancer(mut self, provider: Provider) -> Self {
        self.overrides.enhancers.push(provider);
        self
    }

    /// `.compile()` – bootstraps the container.
    pub async fn compile(self) -> crate::Result<TestingModule> {
        let mut root = DynamicModule::new::<RootTestModule>();
        root.imports = self.imports;
        root.providers = self.providers;
        root.controllers = self.controllers;
        let options = CorvisApplicationOptions {
            logger: if self.logger {
                LoggerOption::Enabled
            } else {
                LoggerOption::Levels(vec![crate::logger::LogLevel::Error, crate::logger::LogLevel::Fatal])
            },
            ..Default::default()
        };
        let mut app = CorvisApplication::bootstrap(ModuleImport::Dynamic(root), self.overrides, options).await?;
        app.silence_route_logs();
        Ok(TestingModule { app })
    }
}

pub struct OverrideBy {
    builder: TestingModuleBuilder,
    token: Token,
}

impl OverrideBy {
    /// `.useValue(value)` – the value must be of the type consumers request.
    pub fn use_value<T: Send + Sync + 'static>(mut self, value: T) -> TestingModuleBuilder {
        let p = Provider::use_value(self.token.clone(), value);
        self.builder.overrides.providers.insert(self.token, p);
        self.builder
    }
    /// `.useValue(Arc<dyn Trait>)`.
    pub fn use_value_arc<T: ?Sized + Send + Sync + 'static>(mut self, value: Arc<T>) -> TestingModuleBuilder {
        let p = Provider::use_value_arc(self.token.clone(), value);
        self.builder.overrides.providers.insert(self.token, p);
        self.builder
    }
    /// `.useClass(MockClass)`.
    pub fn use_class<T: Injectable>(mut self) -> TestingModuleBuilder {
        let p = Provider::use_class::<T>(self.token.clone());
        self.builder.overrides.providers.insert(self.token, p);
        self.builder
    }
    /// `.useFactory({ factory })`.
    pub fn use_factory<T, F, Fut>(mut self, factory: F) -> TestingModuleBuilder
    where
        T: Send + Sync + 'static,
        F: Fn(crate::di::ResolveCtx) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = crate::Result<T>> + Send + 'static,
    {
        let p = Provider::use_factory(self.token.clone(), factory);
        self.builder.overrides.providers.insert(self.token, p);
        self.builder
    }
}

/// A compiled testing module.
pub struct TestingModule {
    app: CorvisApplication,
}

impl TestingModule {
    /// `moduleRef.get(CatsService)`.
    pub fn get<T: ?Sized + Send + Sync + 'static>(&self) -> Result<Arc<T>, DiError> {
        self.app.get::<T>()
    }
    pub fn get_token<T: ?Sized + Send + Sync + 'static>(&self, token: impl Into<Token>) -> Result<Arc<T>, DiError> {
        self.app.get_token::<T>(token)
    }
    /// `moduleRef.resolve(TransientService)`.
    pub async fn resolve<T: ?Sized + Send + Sync + 'static>(&self) -> Result<Arc<T>, DiError> {
        self.app.module_ref().resolve::<T>().await
    }
    /// The application of the compiled testing module (call `init()` before sending requests).
    pub fn create_corvis_application(self) -> CorvisApplication {
        self.app
    }
    /// Runs init hooks without an HTTP layer (`moduleRef.init()`).
    pub async fn init(&mut self) -> crate::Result<()> {
        self.app.init().await
    }
    pub async fn close(&mut self) -> crate::Result<()> {
        self.app.close().await
    }
}

/// `request(app.getHttpServer())` – an in-memory HTTP client (no sockets).
pub fn request(app: &CorvisApplication) -> TestClient {
    TestClient::new(app)
}

#[derive(Clone)]
pub struct TestClient {
    shared: Arc<AppShared>,
}

impl TestClient {
    /// Panics if the application was not initialized with `app.init().await`.
    pub fn new(app: &CorvisApplication) -> Self {
        let shared = app.shared().expect("call `app.init().await` before creating a TestClient");
        TestClient { shared }
    }
    pub fn get(&self, path: &str) -> TestRequest {
        self.method(Method::GET, path)
    }
    pub fn post(&self, path: &str) -> TestRequest {
        self.method(Method::POST, path)
    }
    pub fn put(&self, path: &str) -> TestRequest {
        self.method(Method::PUT, path)
    }
    pub fn patch(&self, path: &str) -> TestRequest {
        self.method(Method::PATCH, path)
    }
    pub fn delete(&self, path: &str) -> TestRequest {
        self.method(Method::DELETE, path)
    }
    pub fn options(&self, path: &str) -> TestRequest {
        self.method(Method::OPTIONS, path)
    }
    pub fn head(&self, path: &str) -> TestRequest {
        self.method(Method::HEAD, path)
    }
    pub fn method(&self, method: Method, path: &str) -> TestRequest {
        TestRequest {
            shared: self.shared.clone(),
            method,
            path: path.to_string(),
            headers: HeaderMap::new(),
            body: Bytes::new(),
            multipart: Vec::new(),
        }
    }
}

type MultipartPart = (String, Option<(String, String)>, Bytes);

pub struct TestRequest {
    shared: Arc<AppShared>,
    method: Method,
    path: String,
    headers: HeaderMap,
    body: Bytes,
    /// `.field()` / `.attach()` parts: (name, Some((filename, content type)), data).
    multipart: Vec<MultipartPart>,
}

impl TestRequest {
    /// supertest's `.field(name, value)`: a text part of a `multipart/form-data` body.
    pub fn field(mut self, name: &str, value: &str) -> Self {
        self.multipart.push((name.to_string(), None, Bytes::from(value.to_string())));
        self
    }
    /// supertest's `.attach(field, buffer, filename)`: a file part (content type from the extension).
    pub fn attach(self, field: &str, data: impl Into<Bytes>, filename: &str) -> Self {
        let ct = crate::common::mime::from_path(std::path::Path::new(filename));
        let ct = ct.split(';').next().unwrap_or(ct).trim().to_string();
        self.attach_with_type(field, data, filename, &ct)
    }
    /// `.attach(field, buffer, { filename, contentType })`.
    pub fn attach_with_type(mut self, field: &str, data: impl Into<Bytes>, filename: &str, content_type: &str) -> Self {
        self.multipart.push((field.to_string(), Some((filename.to_string(), content_type.to_string())), data.into()));
        self
    }

    fn build_multipart(&mut self) {
        if self.multipart.is_empty() {
            return;
        }
        let boundary = "--------------------------corvistestboundary0001";
        let mut body = Vec::new();
        for (name, file, data) in std::mem::take(&mut self.multipart) {
            body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
            match file {
                Some((filename, ct)) => body.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\nContent-Type: {ct}\r\n\r\n")
                        .as_bytes(),
                ),
                None => body.extend_from_slice(format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes()),
            }
            body.extend_from_slice(&data);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        self.body = Bytes::from(body);
        let ct = format!("multipart/form-data; boundary={boundary}");
        if let Ok(v) = HeaderValue::from_str(&ct) {
            self.headers.insert(http::header::CONTENT_TYPE, v);
        }
    }

    /// `.set('Authorization', 'Bearer x')`.
    pub fn header(mut self, name: &str, value: &str) -> Self {
        if let (Ok(n), Ok(v)) = (HeaderName::from_bytes(name.as_bytes()), HeaderValue::from_str(value)) {
            self.headers.insert(n, v);
        }
        self
    }
    pub fn set(self, name: &str, value: &str) -> Self {
        self.header(name, value)
    }
    pub fn bearer(self, token: &str) -> Self {
        self.header("authorization", &format!("Bearer {token}"))
    }
    /// `.send({ ... })` with a JSON body.
    pub fn json(mut self, body: &impl Serialize) -> Self {
        self.body = Bytes::from(serde_json::to_vec(body).unwrap_or_default());
        self.header("content-type", "application/json")
    }
    pub fn form(mut self, fields: &[(&str, &str)]) -> Self {
        let s = form_urlencoded::Serializer::new(String::new()).extend_pairs(fields).finish();
        self.body = Bytes::from(s);
        self.header("content-type", "application/x-www-form-urlencoded")
    }
    pub fn body(mut self, body: impl Into<Bytes>) -> Self {
        self.body = body.into();
        self
    }
    pub fn query(mut self, key: &str, value: &str) -> Self {
        let sep = if self.path.contains('?') { '&' } else { '?' };
        let enc = form_urlencoded::Serializer::new(String::new()).append_pair(key, value).finish();
        self.path = format!("{}{sep}{enc}", self.path);
        self
    }

    pub async fn send(mut self) -> TestResponse {
        self.build_multipart();
        if !self.body.is_empty() && !self.headers.contains_key(http::header::CONTENT_LENGTH) {
            self.headers.insert(http::header::CONTENT_LENGTH, HeaderValue::from(self.body.len()));
        }
        let head = self.method == Method::HEAD;
        let uri: Uri = self.path.parse().unwrap_or_else(|_| Uri::from_static("/"));
        let info = crate::http::express::RequestInfo::new(&self.method, &self.headers);
        let req = crate::http::Request::with_parser(self.method, uri, self.headers, self.body, &self.shared.http_settings.body_parser);
        let mut resp = dispatch(self.shared.clone(), req).await;
        crate::http::express::finalize(&mut resp, &info, &self.shared.http_settings);
        let hyper_resp = crate::server::into_hyper(resp, head);
        let (parts, body) = hyper_resp.into_parts();
        let body = body.collect().await.map(|c| c.to_bytes()).unwrap_or_default();
        TestResponse { status: parts.status.as_u16(), headers: parts.headers, body }
    }
}

/// The response of a [`TestRequest`].
#[derive(Debug, Clone)]
pub struct TestResponse {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Bytes,
}

impl TestResponse {
    pub fn status(&self) -> u16 {
        self.status
    }
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    pub fn json<T: DeserializeOwned>(&self) -> T {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| panic!("invalid JSON body ({e}): {}", self.text()))
    }
    pub fn json_value(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
    /// `.expect(200)`.
    pub fn expect_status(&self, status: u16) -> &Self {
        assert_eq!(self.status, status, "unexpected status, body: {}", self.text());
        self
    }
    /// `.expect({ ... })` – exact JSON body.
    pub fn expect_json(&self, expected: Value) -> &Self {
        assert_eq!(self.json_value(), expected);
        self
    }
    /// `.expect('Hello World!')` – exact text body.
    pub fn expect_text(&self, expected: &str) -> &Self {
        assert_eq!(self.text(), expected);
        self
    }
    pub fn expect_header(&self, name: &str, value: &str) -> &Self {
        assert_eq!(self.header(name), Some(value), "header {name}");
        self
    }
}
