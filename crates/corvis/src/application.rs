//! `CorvisFactory` and `CorvisApplication`.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use crate::common::filter::ExceptionFilter;
use crate::common::guard::CanActivate;
use crate::common::interceptor::CorvisInterceptor;
use crate::common::lifecycle::HookKind;
use crate::common::middleware::{CorvisMiddleware, FnMiddleware, Next};
use crate::common::pipe::PipeTransform;
use crate::di::container::{Container, Overrides};
use crate::di::provider::{GlobalEnhancer, downcast};
use crate::di::{DiError, Module, ModuleImport, ModuleRef, Token};
use crate::http::{HandlerResult, Request, Response};
use crate::logger::{LogLevel, Logger, LoggerService};
use crate::router::cors::CorsOptions;
use crate::router::versioning::VersioningOptions;
use crate::router::{AppShared, RouterConfig, build_middleware, build_router, dispatch};

pub mod introspection;

/// Logger configuration for `CorvisFactory::create_with_options`.
pub enum LoggerOption {
    Enabled,
    Disabled,
    Levels(Vec<LogLevel>),
    Custom(Box<dyn LoggerService>),
}

/// Options of [`CorvisFactory::create_with_options`]: logger, CORS, body limit, abort on error.
pub struct CorvisApplicationOptions {
    pub logger: LoggerOption,
    pub cors: Option<CorsOptions>,
    /// Maximum request body size in bytes (default 1 MiB).
    pub body_limit: usize,
    /// When `false`, bootstrap errors are returned instead of exiting the process.
    pub abort_on_error: bool,
}

impl Default for CorvisApplicationOptions {
    fn default() -> Self {
        CorvisApplicationOptions { logger: LoggerOption::Enabled, cors: None, body_limit: 1024 * 1024, abort_on_error: false }
    }
}

/// Entry point to create application instances.
pub struct CorvisFactory;

impl CorvisFactory {
    /// Creates the application of the root module `M` with the default options.
    pub async fn create<M: Module>() -> crate::Result<CorvisApplication> {
        Self::create_with_options::<M>(CorvisApplicationOptions::default()).await
    }

    /// Creates the application of the root module `M` with explicit options (logger, CORS, ...).
    pub async fn create_with_options<M: Module>(options: CorvisApplicationOptions) -> crate::Result<CorvisApplication> {
        CorvisApplication::bootstrap(ModuleImport::of::<M>(), Overrides::default(), options).await
    }

    /// A standalone application context
    /// (DI container without HTTP server), handy for CLIs, workers and scripts.
    ///
    /// It registers the `#[on_event]`, `#[cron]`, `#[interval]` and `#[timeout]` handlers, runs
    /// `on_module_init`, starts the scheduled jobs and runs `on_application_bootstrap`, in the same order
    /// as [`CorvisApplication::init`]. Routes, middleware and gateways are not built. Use
    /// `ScheduleModule::for_root_with` to turn a kind of scheduled handler off.
    pub async fn create_application_context<M: Module>() -> crate::Result<CorvisApplication> {
        let mut app = Self::create::<M>().await?;
        app.call_init_hooks().await?;
        Ok(app)
    }
}

pub(crate) struct AppConfig {
    router: RouterConfig,
    global_middleware: Vec<Arc<dyn CorvisMiddleware>>,
    cors: Option<CorsOptions>,
    body_limit: usize,
    http_settings: crate::http::HttpSettings,
    server: crate::server::ServerOptions,
}

/// A Corvis application instance.
pub struct CorvisApplication {
    pub(crate) container: Arc<Container>,
    config: AppConfig,
    shared: Option<Arc<AppShared>>,
    hooks_called: bool,
    initialized: bool,
    closed: bool,
    shutdown_hooks: bool,
    log_routes: bool,
    started: Instant,
    local_addr: Option<SocketAddr>,
}

impl CorvisApplication {
    pub(crate) async fn bootstrap(
        root: ModuleImport,
        overrides: Overrides,
        options: CorvisApplicationOptions,
    ) -> crate::Result<CorvisApplication> {
        match options.logger {
            LoggerOption::Enabled => {}
            LoggerOption::Disabled => crate::logger::set_logging_enabled(false),
            LoggerOption::Levels(l) => crate::logger::set_log_levels(&l),
            LoggerOption::Custom(c) => crate::logger::set_logger(BoxedLogger(c)),
        }
        let started = Instant::now();
        Logger::new("CorvisFactory").log("Starting Corvis application...");
        let result = async {
            let container = Container::scan(root, overrides)?;
            container.instantiate_all(true).await?;
            Ok::<_, DiError>(container)
        }
        .await;
        let container = match result {
            Ok(c) => c,
            Err(e) => {
                Logger::new("ExceptionHandler").error(e.to_string());
                if options.abort_on_error {
                    std::process::exit(1);
                }
                return Err(e.into());
            }
        };
        Ok(CorvisApplication {
            container,
            config: AppConfig {
                router: RouterConfig::default(),
                global_middleware: Vec::new(),
                cors: options.cors,
                body_limit: options.body_limit,
                http_settings: crate::http::HttpSettings::default(),
                server: crate::server::ServerOptions::default(),
            },
            shared: None,
            hooks_called: false,
            initialized: false,
            closed: false,
            shutdown_hooks: false,
            log_routes: true,
            started,
            local_addr: None,
        })
    }

    /// `app.setGlobalPrefix('api')`.
    pub fn set_global_prefix(&mut self, prefix: impl Into<String>) -> &mut Self {
        self.config.router.global_prefix = prefix.into();
        self
    }

    /// `app.setGlobalPrefix('api', { exclude: ['health'] })`.
    pub fn set_global_prefix_with_exclude(&mut self, prefix: impl Into<String>, exclude: &[&str]) -> &mut Self {
        self.config.router.global_prefix = prefix.into();
        self.config.router.prefix_exclude = exclude.iter().map(|s| s.to_string()).collect();
        self
    }

    /// `app.useGlobalPipes(new ValidationPipe())`.
    pub fn use_global_pipes(&mut self, pipe: impl PipeTransform) -> &mut Self {
        self.config.router.global_pipes.push(Arc::new(pipe));
        self
    }

    /// `app.useGlobalGuards(new RolesGuard())`.
    pub fn use_global_guards(&mut self, guard: impl CanActivate) -> &mut Self {
        self.config.router.global_guards.push(Arc::new(guard));
        self
    }

    /// `app.useGlobalInterceptors(new LoggingInterceptor())`.
    pub fn use_global_interceptors(&mut self, interceptor: impl CorvisInterceptor) -> &mut Self {
        self.config.router.global_interceptors.push(Arc::new(interceptor));
        self
    }

    /// `app.useGlobalFilters(new HttpExceptionFilter())`.
    pub fn use_global_filters(&mut self, filter: impl ExceptionFilter) -> &mut Self {
        self.config.router.global_filters.push(Arc::new(filter));
        self
    }

    /// `app.use(middleware)` – global middleware instance.
    pub fn use_middleware(&mut self, middleware: impl CorvisMiddleware) -> &mut Self {
        self.config.global_middleware.push(Arc::new(middleware));
        self
    }

    /// `app.use((req, res, next) => ...)` – global functional middleware.
    pub fn use_fn<F, Fut>(&mut self, f: F) -> &mut Self
    where
        F: Fn(Request, Next) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = HandlerResult> + Send + 'static,
    {
        self.config.global_middleware.push(Arc::new(FnMiddleware(f)));
        self
    }

    /// `app.enableCors()`.
    pub fn enable_cors(&mut self, options: CorsOptions) -> &mut Self {
        self.config.cors = Some(options);
        self
    }

    /// `app.enableVersioning({ type: VersioningType.URI })`.
    pub fn enable_versioning(&mut self, options: VersioningOptions) -> &mut Self {
        self.config.router.versioning = Some(options);
        self
    }

    /// `app.enableShutdownHooks()` – listen to SIGINT/SIGTERM and run the shutdown hooks.
    pub fn enable_shutdown_hooks(&mut self) -> &mut Self {
        self.shutdown_hooks = true;
        self
    }

    /// Maximum request body size in bytes.
    pub fn set_body_limit(&mut self, bytes: usize) -> &mut Self {
        self.config.body_limit = bytes;
        self
    }

    /// Timeouts and connection limits of the HTTP server (`headersTimeout`, `requestTimeout`,
    /// `keepAliveTimeout`, `maxConnections`, ...). Call it before `listen()`.
    pub fn set_server_options(&mut self, options: crate::server::ServerOptions) -> &mut Self {
        self.config.server = options;
        self
    }

    /// `app.useLogger(logger)`.
    pub fn use_logger(&mut self, logger: impl LoggerService) -> &mut Self {
        crate::logger::set_logger(logger);
        self
    }

    /// Do not print the mapped routes during `init()`.
    pub fn silence_route_logs(&mut self) -> &mut Self {
        self.log_routes = false;
        self
    }

    /// `app.disable('x-powered-by')` / `app.disable('etag')`. Unknown names are ignored.
    /// Call it before `init()` / `listen()`.
    pub fn disable(&mut self, setting: &str) -> &mut Self {
        match setting.to_ascii_lowercase().as_str() {
            "x-powered-by" => self.config.http_settings.x_powered_by = None,
            "etag" => self.config.http_settings.etag = crate::http::EtagMode::Disabled,
            _ => {}
        }
        self
    }

    /// `app.enable('x-powered-by')` / `app.enable('etag')` (weak ETags).
    pub fn enable(&mut self, setting: &str) -> &mut Self {
        match setting.to_ascii_lowercase().as_str() {
            "x-powered-by" => {
                if self.config.http_settings.x_powered_by.is_none() {
                    self.config.http_settings.x_powered_by = crate::http::HttpSettings::default().x_powered_by;
                }
            }
            "etag" => self.config.http_settings.etag = crate::http::EtagMode::Weak,
            _ => {}
        }
        self
    }

    /// `app.set('etag', 'weak' | 'strong' | false)`.
    pub fn set_etag(&mut self, mode: crate::http::EtagMode) -> &mut Self {
        self.config.http_settings.etag = mode;
        self
    }

    /// `app.useBodyParser('json' | 'urlencoded', { limit })`: changes the limit (in bytes) of
    /// one of the body parsers (default 100kb). Other names are ignored.
    pub fn use_body_parser(&mut self, parser: &str, limit: usize) -> &mut Self {
        let options = &mut self.config.http_settings.body_parser;
        match parser {
            "json" => options.json_limit = limit,
            "urlencoded" => options.urlencoded_limit = limit,
            _ => return self,
        }
        // The transport limit must not cut a body the parser accepts.
        self.config.body_limit = self.config.body_limit.max(limit);
        self
    }

    /// Disables the built-in body parsers: request bodies are not parsed.
    pub fn disable_body_parser(&mut self) -> &mut Self {
        self.config.http_settings.body_parser.enabled = false;
        self
    }

    /// `app.set('query parser', 'simple' | 'extended' | false)` (Express 5 default: simple).
    pub fn set_query_parser(&mut self, parser: crate::http::body_parser::QueryParser) -> &mut Self {
        self.config.http_settings.body_parser.query_parser = parser;
        self
    }

    /// Maximum nesting depth of JSON bodies (default 256; deeper bodies are answered with 400).
    /// It also caps the depth of urlencoded bodies (32 by default, like body-parser).
    pub fn set_body_parser_max_depth(&mut self, depth: usize) -> &mut Self {
        self.config.http_settings.body_parser.max_depth = depth;
        self
    }

    /// `app.set('trust proxy', ...)`: `false` (default, `req.ip` is the socket address), `true`,
    /// a hop count (`1` behind one reverse proxy) or a list of proxy addresses/subnets
    /// ([`TrustProxy::parse`](crate::http::TrustProxy::parse)`("loopback, 10.0.0.0/8")`).
    pub fn set_trust_proxy(&mut self, trust: impl Into<crate::http::TrustProxy>) -> &mut Self {
        self.config.http_settings.body_parser.trust_proxy = trust.into();
        self
    }

    /// Value of the `X-Powered-By` header (default `corvis`; `"Express"` reproduces
    /// the Express platform byte for byte).
    pub fn set_x_powered_by(&mut self, value: impl Into<String>) -> &mut Self {
        self.config.http_settings.x_powered_by = Some(value.into());
        self
    }

    /// `app.getHttpAdapter()`.
    pub fn get_http_adapter(&self) -> crate::http_adapter::HttpAdapter {
        self.container.instances_of::<crate::http_adapter::HttpAdapterHost>().first().map(|h| h.http_adapter().clone()).unwrap_or_default()
    }

    /// `app.getHttpServer()`.
    pub fn get_http_server(&self) -> crate::http_adapter::HttpServer {
        self.get_http_adapter().get_http_server()
    }

    /// `app.get(CatsService)` – retrieves an instance from the container.
    pub fn get<T: ?Sized + Send + Sync + 'static>(&self) -> Result<Arc<T>, DiError> {
        self.module_ref().get::<T>()
    }

    /// `app.get('TOKEN')`.
    pub fn get_token<T: ?Sized + Send + Sync + 'static>(&self, token: impl Into<Token>) -> Result<Arc<T>, DiError> {
        self.module_ref().get_token::<T>(token)
    }

    /// A `ModuleRef` for the root module.
    pub fn module_ref(&self) -> ModuleRef {
        ModuleRef { container: self.container.clone(), module: self.container.root }
    }

    pub(crate) async fn call_init_hooks(&mut self) -> crate::Result<()> {
        if self.hooks_called {
            return Ok(());
        }
        self.hooks_called = true;
        // Handlers are registered here, once per application, so a standalone context runs them too and
        // `init()` after `create_application_context` does not register them a second time.
        let discovered = crate::discovery::discover(&self.container);
        crate::modules::register_handlers(&self.container, &discovered)?;
        self.container.call_hook(HookKind::OnModuleInit, None).await?;
        crate::modules::mount_scheduler(&self.container)?;
        self.container.call_hook(HookKind::OnApplicationBootstrap, None).await?;
        Ok(())
    }

    /// `app.init()` – registers routes, middleware and calls the init lifecycle hooks. Event and scheduled
    /// handlers are registered once per application, so calling it after `create_application_context` does
    /// not register them again.
    pub async fn init(&mut self) -> crate::Result<()> {
        if self.initialized {
            return Ok(());
        }
        // Global enhancers registered as providers (APP_GUARD, APP_PIPE, ...) are applied while
        // the application is created, before any `use_global_*()` call of the bootstrap code,
        // so they run first (outermost): insert them ahead of the ones already registered.
        let (mut guard_at, mut interceptor_at, mut pipe_at) = (0, 0, 0);
        for (module, provider) in self.container.all_providers() {
            let Some(enh) = &provider.enhancer else { continue };
            let Some(instance) = self.container.instance(module, &provider.token) else { continue };
            match enh {
                GlobalEnhancer::Guard(f) => {
                    if let Some(g) = f(&instance) {
                        self.config.router.global_guards.insert(guard_at, g);
                        guard_at += 1;
                    }
                }
                GlobalEnhancer::Interceptor(f) => {
                    if let Some(i) = f(&instance) {
                        self.config.router.global_interceptors.insert(interceptor_at, i);
                        interceptor_at += 1;
                    }
                }
                GlobalEnhancer::Pipe(f) => {
                    if let Some(p) = f(&instance) {
                        self.config.router.global_pipes.insert(pipe_at, p);
                        pipe_at += 1;
                    }
                }
                GlobalEnhancer::Filter(f) => {
                    if let Some(x) = f(&instance) {
                        // Filters registered through DI have priority over `useGlobalFilters`.
                        self.config.router.global_filters.insert(0, x);
                    }
                }
            }
        }
        let router = build_router(&self.container, &self.config.router, self.log_routes).await?;
        let module_middleware = build_middleware(&self.container, &router, &self.config.router).await?;
        #[cfg(feature = "websockets")]
        let gateways = crate::websockets::build_gateways(&self.container, &self.config.router).await?;
        self.shared = Some(Arc::new(AppShared {
            router,
            global_middleware: self.config.global_middleware.clone(),
            module_middleware,
            global_filters: self.config.router.global_filters.clone(),
            cors: self.config.cors.clone(),
            versioning: self.config.router.versioning.clone(),
            body_limit: self.config.body_limit,
            static_dirs: self.container.instances_of::<crate::modules::serve_static::ServeStaticOptions>(),
            container: self.container.clone(),
            #[cfg(feature = "websockets")]
            gateways,
            http_server: self.get_http_server(),
            http_settings: self.config.http_settings.clone(),
        }));
        self.call_init_hooks().await?;
        self.initialized = true;
        Ok(())
    }

    /// Processes a request in memory (no network). Used by the testing utilities.
    pub async fn handle(&mut self, req: Request) -> crate::Result<Response> {
        self.init().await?;
        let shared = self.shared.clone().expect("initialized");
        Ok(dispatch(shared, req).await)
    }

    pub(crate) fn shared(&self) -> Option<Arc<AppShared>> {
        self.shared.clone()
    }

    /// `app.listen(3000)`. Runs until the process is stopped (or, with
    /// `enable_shutdown_hooks()`, until SIGINT/SIGTERM, then closes gracefully).
    pub async fn listen(mut self, addr: impl ListenAddr) -> crate::Result<()> {
        let listener = self.bind(addr).await?;
        let shared = self.shared.clone().expect("initialized");
        let hooks = self.shutdown_hooks;
        let (tx, rx) = tokio::sync::oneshot::channel::<String>();
        if hooks {
            tokio::spawn(async move {
                let sig = wait_for_signal().await;
                let _ = tx.send(sig);
            });
        } else {
            std::mem::forget(tx);
        }
        let signal = Arc::new(std::sync::Mutex::new(None::<String>));
        let signal2 = signal.clone();
        crate::server::serve(listener, shared, self.config.server.clone(), async move {
            if let Ok(s) = rx.await {
                *signal2.lock().unwrap() = Some(s);
            }
        })
        .await?;
        let sig = signal.lock().unwrap().clone();
        self.close_with_signal(sig).await
    }

    /// Starts listening in the background and returns the bound address.
    /// Useful for e2e tests against a real socket. Use `port 0` for a random port.
    pub async fn listen_in_background(&mut self, addr: impl ListenAddr) -> crate::Result<SocketAddr> {
        let listener = self.bind(addr).await?;
        let shared = self.shared.clone().expect("initialized");
        let local = listener.local_addr()?;
        let server = self.config.server.clone();
        tokio::spawn(async move {
            let _ = crate::server::serve(listener, shared, server, futures::future::pending()).await;
        });
        Ok(local)
    }

    async fn bind(&mut self, addr: impl ListenAddr) -> crate::Result<tokio::net::TcpListener> {
        self.init().await?;
        let addr = addr.to_socket_addr()?;
        let listener = match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                Logger::new("CorvisApplication").error(format!("listen EADDRINUSE / bind error on {addr}: {e}"));
                return Err(e.into());
            }
        };
        self.local_addr = listener.local_addr().ok();
        self.get_http_server().set_address(self.local_addr);
        Logger::new("CorvisApplication").log("Corvis application successfully started");
        let _ = self.started;
        Ok(listener)
    }

    /// `await app.getUrl()`.
    pub fn get_url(&self) -> Option<String> {
        self.local_addr.map(|a| {
            let host = if a.ip().is_unspecified() { "127.0.0.1".to_string() } else { a.ip().to_string() };
            format!("http://{host}:{}", a.port())
        })
    }

    /// `app.close()` – calls `on_module_destroy`, `before_application_shutdown`
    /// and `on_application_shutdown`.
    pub async fn close(&mut self) -> crate::Result<()> {
        self.close_with_signal(None).await
    }

    async fn close_with_signal(&mut self, signal: Option<String>) -> crate::Result<()> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        self.container.call_hook(HookKind::OnModuleDestroy, None).await?;
        crate::modules::stop_scheduler(&self.container);
        self.container.call_hook(HookKind::BeforeApplicationShutdown, signal.clone()).await?;
        self.container.call_hook(HookKind::OnApplicationShutdown, signal).await?;
        Ok(())
    }

    /// Typed access to a provider registered with a token and a concrete type.
    pub fn get_instance<T: Send + Sync + 'static>(&self, token: impl Into<Token>) -> Option<Arc<T>> {
        let token = token.into();
        self.container.find_instance(&token).and_then(|i| downcast::<T>(&i))
    }
}

async fn wait_for_signal() -> String {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = signal(SignalKind::terminate()).expect("signal");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => "SIGINT".to_string(),
            _ = term.recv() => "SIGTERM".to_string(),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        "SIGINT".to_string()
    }
}

/// Things accepted by `listen()`: a port (`3000`), `"0.0.0.0:3000"`, or a `SocketAddr`.
pub trait ListenAddr {
    fn to_socket_addr(self) -> std::io::Result<SocketAddr>;
}
impl ListenAddr for u16 {
    fn to_socket_addr(self) -> std::io::Result<SocketAddr> {
        Ok(SocketAddr::from(([0, 0, 0, 0], self)))
    }
}
impl ListenAddr for i32 {
    fn to_socket_addr(self) -> std::io::Result<SocketAddr> {
        Ok(SocketAddr::from(([0, 0, 0, 0], self as u16)))
    }
}
impl ListenAddr for SocketAddr {
    fn to_socket_addr(self) -> std::io::Result<SocketAddr> {
        Ok(self)
    }
}
impl ListenAddr for &str {
    fn to_socket_addr(self) -> std::io::Result<SocketAddr> {
        if let Ok(p) = self.parse::<u16>() {
            return p.to_socket_addr();
        }
        use std::net::ToSocketAddrs;
        self.to_socket_addrs()?.next().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid address"))
    }
}
impl ListenAddr for String {
    fn to_socket_addr(self) -> std::io::Result<SocketAddr> {
        self.as_str().to_socket_addr()
    }
}

struct BoxedLogger(Box<dyn LoggerService>);
impl LoggerService for BoxedLogger {
    fn log(&self, level: LogLevel, message: &str, context: Option<&str>) {
        self.0.log(level, message, context)
    }
}
