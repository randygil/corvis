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
use crate::di::provider::{GlobalEnhancer, Instance, downcast};
use crate::di::{DiError, Module, ModuleImport, ModuleRef, Token};
use crate::http::{HandlerResult, Request, Response};
use crate::logger::{LogLevel, Logger, LoggerService};
use crate::router::cors::CorsOptions;
use crate::router::versioning::VersioningOptions;
use crate::router::{AppShared, RouterConfig, build_middleware, build_router, dispatch};
use crate::server::Transport;

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
    route_posture: crate::posture::RoutePostureOptions,
}

/// Returned by [`CorvisApplication::init`] (and by `listen`, `listen_in_background`, `handle` and
/// `TestingModule::init`) when `init()` already failed once on that application.
#[derive(Debug, Clone)]
pub struct InitFailedError {
    first_error: String,
}

impl InitFailedError {
    fn new(first_error: String) -> Self {
        InitFailedError { first_error }
    }

    /// The message of the error that made the first `init()` fail.
    pub fn first_error(&self) -> &str {
        &self.first_error
    }
}

impl std::fmt::Display for InitFailedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the application failed to initialize earlier and cannot be initialized again; create a new application (first error: {})",
            self.first_error
        )
    }
}

impl std::error::Error for InitFailedError {}

/// A Corvis application instance.
pub struct CorvisApplication {
    pub(crate) container: Arc<Container>,
    config: AppConfig,
    shared: Option<Arc<AppShared>>,
    hooks_called: bool,
    initialized: bool,
    /// The first `init()` error, once `init()` failed: the application cannot be initialized again.
    init_error: Option<String>,
    closed: bool,
    shutdown_hooks: bool,
    log_routes: bool,
    started: Instant,
    /// `false` for applications created by the testing module: they ignore `CORVIS_ROUTE_POSTURE_REPORT`.
    pub(crate) report_mode_allowed: bool,
    local_addr: Option<SocketAddr>,
    /// The last listener started serves TLS (`get_url` says `https`).
    local_secure: bool,
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
                route_posture: crate::posture::RoutePostureOptions::default(),
            },
            shared: None,
            hooks_called: false,
            initialized: false,
            init_error: None,
            closed: false,
            shutdown_hooks: false,
            log_routes: true,
            started,
            report_mode_allowed: true,
            local_addr: None,
            local_secure: false,
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

    /// Whether global guards, interceptors and pipes (`use_global_*` and the `Provider::app_*`
    /// ones) also run on WebSocket gateway handlers. `true` by default; `false` keeps them
    /// HTTP-only. Global exception filters never apply to gateways. Must be called before
    /// `init()`: gateways are built once.
    pub fn apply_global_enhancers_to_gateways(&mut self, apply: bool) -> &mut Self {
        if self.initialized {
            Logger::new("CorvisApplication")
                .warn("apply_global_enhancers_to_gateways() has no effect after the application was initialized");
        } else {
            self.config.router.gateway_global_enhancers = apply;
        }
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

    /// Refuses to start when a controller route has no guard that controls access, is not marked
    /// `#[public]` and matches no exemption: `init()` (so `listen()` too) fails with a
    /// [`RoutePostureError`](crate::RoutePostureError) before module middleware is configured and before any
    /// hook runs. Same as `set_route_posture(RoutePostureOptions::enforce())`. Off by default. Call it before
    /// `init()` / `listen()`; it has no effect afterwards.
    pub fn require_route_posture(&mut self) -> &mut Self {
        self.set_route_posture(crate::posture::RoutePostureOptions::enforce())
    }

    /// Sets the route posture mode (off, warn or enforce) and its exemptions. Call it before `init()` /
    /// `listen()`; it has no effect afterwards.
    pub fn set_route_posture(&mut self, options: crate::posture::RoutePostureOptions) -> &mut Self {
        self.config.route_posture = options;
        self
    }

    /// The posture of every controller route and gateway message handler (guards, public marker, exemptions) and
    /// the surfaces the check cannot enforce, in every mode. `None` before `init()`.
    pub fn route_posture(&self) -> Option<crate::posture::RoutePostureReport> {
        let shared = self.shared.as_ref()?;
        let options = match self.config.route_posture.mode() {
            crate::posture::RoutePostureMode::Off => crate::posture::RoutePostureOptions::off(),
            _ => self.config.route_posture.clone(),
        };
        #[cfg(feature = "websockets")]
        let gateways = crate::websockets::posture_gateways(shared.gateways.values().map(|g| g.as_ref()));
        #[cfg(not(feature = "websockets"))]
        let gateways: Vec<crate::posture::GatewayPosture> = Vec::new();
        let mut unchecked: Vec<crate::posture::UncheckedSurface> =
            shared.static_dirs.iter().map(|d| crate::posture::UncheckedSurface::StaticFiles { serve_root: d.serve_root.clone() }).collect();
        unchecked.extend(gateways.iter().map(|g| crate::posture::UncheckedSurface::WsHandshake {
            path: g.path.clone(),
            gateway: g.name.clone(),
            verify_client: g.verify_client,
        }));
        let listeners = shared.http_server.upgrade_listener_count();
        if listeners > 0 {
            unchecked.push(crate::posture::UncheckedSurface::UpgradeListeners { count: listeners });
        }
        Some(crate::posture::report_for(&shared.router, &gateways, &options).with_unchecked(unchecked))
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

    /// `app.use(compression(options))`: compresses responses of compressible types with `br`, `gzip`
    /// or `deflate` as the client's `Accept-Encoding` allows. Off until this is called; see
    /// [`crate::http::compression`] for the rules and the differences from the original middleware.
    #[cfg(feature = "compression")]
    pub fn enable_compression(&mut self, options: crate::http::compression::CompressionOptions) -> &mut Self {
        self.config.http_settings.compression = Some(std::sync::Arc::new(options));
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
    ///
    /// After a successful `init()`, later calls do nothing. After a failed one the application is single-use:
    /// the failing call returns the original error and every later call (also through `listen`,
    /// `listen_in_background`, `handle` and `TestingModule::init`) returns [`InitFailedError`] without running
    /// any step, because hooks that already ran cannot be undone. Create a new application instead.
    pub async fn init(&mut self) -> crate::Result<()> {
        if let Some(first) = &self.init_error {
            return Err(InitFailedError::new(first.clone()).into());
        }
        if self.initialized {
            return Ok(());
        }
        match self.init_steps().await {
            Ok(()) => {
                self.initialized = true;
                Ok(())
            }
            Err(e) => {
                self.init_error = Some(format!("{e:#}"));
                self.shared = None;
                Err(e)
            }
        }
    }

    pub(crate) fn init_failed(&self) -> bool {
        self.init_error.is_some()
    }

    async fn init_steps(&mut self) -> crate::Result<()> {
        if self.config.route_posture.mode() != crate::posture::RoutePostureMode::Off {
            self.config.route_posture.validate()?;
        }
        // Global enhancers registered as providers (APP_GUARD, APP_PIPE, ...) are applied while
        // the application is created, before any `use_global_*()` call of the bootstrap code,
        // so they run first (outermost): insert them ahead of the ones already registered.
        let (mut guard_at, mut interceptor_at, mut pipe_at) = (0, 0, 0);
        let mut provider_filters: Vec<Arc<dyn ExceptionFilter>> = Vec::new();
        for (module, provider) in self.container.all_providers() {
            let Some(enh) = &provider.enhancer else { continue };
            let instance = self.container.instance(module, &provider.token);
            // A testing module's `override_guard::<T>(mock)` (and siblings) replaces `T` here too.
            let source = self.container.find_instance(enh.override_token()).or(instance);
            let (name, kind) = (provider.token.display_name(), enh.kind());
            let unusable = |source: &Option<Instance>| match source {
                Some(_) => anyhow::anyhow!("global {kind} {name} could not be applied: the provider value is not a {kind}"),
                None => anyhow::anyhow!("global {kind} {name} could not be applied: the provider has no instance"),
            };
            macro_rules! apply {
                ($cast:expr) => {
                    match source.as_ref().and_then($cast) {
                        Some(x) => x,
                        None => return Err(unusable(&source)),
                    }
                };
            }
            match enh {
                GlobalEnhancer::Guard { cast, .. } => {
                    self.config.router.global_guards.insert(guard_at, apply!(cast));
                    guard_at += 1;
                }
                GlobalEnhancer::Interceptor { cast, .. } => {
                    self.config.router.global_interceptors.insert(interceptor_at, apply!(cast));
                    interceptor_at += 1;
                }
                GlobalEnhancer::Pipe { cast, .. } => {
                    self.config.router.global_pipes.insert(pipe_at, apply!(cast));
                    pipe_at += 1;
                }
                GlobalEnhancer::Filter { cast, .. } => provider_filters.push(apply!(cast)),
            }
        }
        // Exception filters are tried last registered first, the order of the original framework
        // (verified against it): the filters of `use_global_filters` from the last call to the
        // first, then the ones registered as providers from the last to the first. The route's and
        // the controller's own filters are tried before all of these.
        self.config.router.global_filters.reverse();
        self.config.router.global_filters.extend(provider_filters.into_iter().rev());
        crate::auth::build_registry(&self.container)?;
        let router = build_router(&self.container, &self.config.router, self.log_routes).await?;
        if let Some(warning) = self.config.cors.as_ref().and_then(CorsOptions::startup_warning) {
            Logger::new("CorsOptions").warn(warning);
        }
        #[cfg(feature = "websockets")]
        let compiled_gateways = crate::websockets::compile_gateways(&self.container, &self.config.router).await?;
        // Before module middleware `configure`, gateway `after_init` and the init hooks: a failing check must not have
        // run user code.
        #[cfg(feature = "websockets")]
        let gateway_posture = crate::websockets::posture_gateways(&compiled_gateways);
        #[cfg(not(feature = "websockets"))]
        let gateway_posture = Vec::new();
        // Strategy names are checked with the route posture, before middleware `configure` and the init hooks.
        crate::auth::check_routes(&self.container, &router)?;
        #[cfg(feature = "websockets")]
        crate::auth::warn_gateways(&gateway_posture);
        // Report mode (CORVIS_ROUTE_POSTURE_REPORT) prints the report, gateways included, and ends the process here,
        // whatever the mode and before the check, so that enforce mode does not swallow the report.
        let static_roots = self
            .container
            .instances_of::<crate::modules::serve_static::ServeStaticOptions>()
            .iter()
            .map(|d| d.serve_root.clone())
            .collect();
        crate::posture::run_report_mode(self.report_mode_allowed, &router, &gateway_posture, static_roots, &self.config.route_posture)?;
        crate::posture::check_routes(&router, &gateway_posture, &self.config.route_posture)?;
        let module_middleware = build_middleware(&self.container, &router, &self.config.router).await?;
        #[cfg(feature = "websockets")]
        let gateways = crate::websockets::start_gateways(compiled_gateways).await;
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
            request_scope: self.container.has_request_scoped(),
            #[cfg(feature = "websockets")]
            gateways,
            http_server: self.get_http_server(),
            http_settings: self.config.http_settings.clone(),
        }));
        self.call_init_hooks().await?;
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
    pub async fn listen(self, addr: impl ListenAddr) -> crate::Result<()> {
        self.listen_with(addr, Transport::Plain).await
    }

    /// Like [`listen`](Self::listen), over HTTPS (`tls` feature). The application is initialized, and the route
    /// posture checked, before the port opens.
    #[cfg(feature = "tls")]
    pub async fn listen_tls(self, addr: impl ListenAddr, tls: crate::tls::TlsOptions) -> crate::Result<()> {
        self.listen_with(addr, Transport::Tls(tls.settings())).await
    }

    async fn listen_with(mut self, addr: impl ListenAddr, transport: Transport) -> crate::Result<()> {
        let listener = self.bind(addr, &transport).await?;
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
        crate::server::serve(listener, shared, self.config.server.clone(), transport, async move {
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
        self.listen_in_background_with(addr, Transport::Plain).await
    }

    /// Like [`listen_in_background`](Self::listen_in_background), over HTTPS (`tls` feature).
    #[cfg(feature = "tls")]
    pub async fn listen_tls_in_background(&mut self, addr: impl ListenAddr, tls: crate::tls::TlsOptions) -> crate::Result<SocketAddr> {
        self.listen_in_background_with(addr, Transport::Tls(tls.settings())).await
    }

    /// [`listen`](Self::listen) with the stop signal supplied by the caller instead of SIGINT/SIGTERM, running in
    /// the background: when `shutdown` resolves the server stops accepting, drains its connections and the
    /// application closes exactly as `listen` does (Engine.IO sessions closed, lifecycle hooks run, with no
    /// signal name). The application is consumed; the handle ends with the result once all of that has finished.
    /// Hidden: it exists so tests can drive the graceful shutdown without sending a process signal.
    #[doc(hidden)]
    pub async fn listen_until_in_background(
        mut self,
        addr: impl ListenAddr,
        shutdown: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> crate::Result<(SocketAddr, tokio::task::JoinHandle<crate::Result<()>>)> {
        let listener = self.bind(addr, &Transport::Plain).await?;
        let shared = self.shared.clone().expect("initialized");
        let local = listener.local_addr()?;
        let server = self.config.server.clone();
        let handle = tokio::spawn(async move {
            crate::server::serve(listener, shared, server, Transport::Plain, shutdown).await?;
            self.close_with_signal(None).await
        });
        Ok((local, handle))
    }

    async fn listen_in_background_with(&mut self, addr: impl ListenAddr, transport: Transport) -> crate::Result<SocketAddr> {
        let listener = self.bind(addr, &transport).await?;
        let shared = self.shared.clone().expect("initialized");
        let local = listener.local_addr()?;
        let server = self.config.server.clone();
        tokio::spawn(async move {
            let _ = crate::server::serve(listener, shared, server, transport, futures::future::pending()).await;
        });
        Ok(local)
    }

    async fn bind(&mut self, addr: impl ListenAddr, transport: &Transport) -> crate::Result<tokio::net::TcpListener> {
        self.init().await?;
        // At listen time, not in `init`: the options may be set after `init()` (`set_server_options`).
        if self.config.server.insecure_http_parser {
            Logger::new("ServerOptions").warn(
                "insecure_http_parser(true): HTTP/1 framing checks are off (bare LF line endings, whitespace after a chunk size, Content-Length with Transfer-Encoding and bare LF in trailers are accepted). See https://corvis.randygil.com/security/http-parsing",
            );
        }
        let addr = addr.to_socket_addr()?;
        let listener = match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                Logger::new("CorvisApplication").error(format!("listen EADDRINUSE / bind error on {addr}: {e}"));
                return Err(e.into());
            }
        };
        self.local_addr = listener.local_addr().ok();
        self.local_secure = transport.is_secure();
        self.get_http_server().set_address(self.local_addr);
        Logger::new("CorvisApplication").log(if transport.is_secure() {
            "Corvis application successfully started (HTTPS)"
        } else {
            "Corvis application successfully started"
        });
        let _ = self.started;
        Ok(listener)
    }

    /// `await app.getUrl()`.
    pub fn get_url(&self) -> Option<String> {
        self.local_addr.map(|a| {
            let host = if a.ip().is_unspecified() { "127.0.0.1".to_string() } else { a.ip().to_string() };
            let scheme = if self.local_secure { "https" } else { "http" };
            format!("{scheme}://{host}:{}", a.port())
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
        #[cfg(feature = "socket-io")]
        self.get_http_server().close_engines();
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

#[cfg(test)]
mod tests {
    use crate::prelude::*;
    use crate::testing::Test;

    use super::InitFailedError;

    #[injectable]
    pub struct G;
    #[async_trait]
    impl CanActivate for G {
        async fn can_activate(&self, _ctx: &ExecutionContext) -> Result<bool, HttpException> {
            Ok(true)
        }
    }

    #[injectable]
    pub struct I;
    #[async_trait]
    impl CorvisInterceptor for I {
        async fn intercept(&self, _ctx: &ExecutionContext, next: CallHandler) -> HandlerResult {
            next.handle().await
        }
    }

    #[injectable]
    pub struct P;
    #[async_trait]
    impl PipeTransform for P {
        async fn transform(&self, value: Value, _metadata: &ArgumentMetadata) -> Result<Value, HttpException> {
            Ok(value)
        }
    }

    #[injectable]
    pub struct F;
    #[async_trait]
    impl ExceptionFilter for F {
        async fn catch(&self, exception: HttpException, _host: &ArgumentsHost) -> Response {
            BaseExceptionFilter::handle(&exception)
        }
    }

    #[injectable]
    pub struct Failing;
    #[async_trait]
    impl OnModuleInit for Failing {
        async fn on_module_init(&self) -> crate::Result<()> {
            anyhow::bail!("database unavailable")
        }
    }

    #[module(providers = [
        Failing,
        Provider::app_guard::<G>(),
        Provider::app_interceptor::<I>(),
        Provider::app_pipe::<P>(),
        Provider::app_filter::<F>(),
    ])]
    pub struct AppModule;

    #[tokio::test]
    async fn second_init_after_a_failure_does_not_register_enhancers_again() {
        let mut app = Test::create_testing_module().import::<AppModule>().compile().await.unwrap().create_corvis_application();
        let first = app.init().await.unwrap_err();
        assert!(first.to_string().contains("database unavailable"));
        let second = app.init().await.unwrap_err();
        assert!(second.downcast_ref::<InitFailedError>().is_some());
        assert!(second.to_string().contains("database unavailable"));
        let router = &app.config.router;
        assert_eq!(router.global_guards.len(), 1);
        assert_eq!(router.global_interceptors.len(), 1);
        assert_eq!(router.global_pipes.len(), 1);
        assert_eq!(router.global_filters.len(), 1);
    }
}
