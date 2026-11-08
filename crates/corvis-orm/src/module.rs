//! Corvis integration: `OrmModule::for_root(...)`, an injectable `OrmService` (the client
//! with the module lifecycle: connect on init, disconnect on destroy) and
//! `OrmExceptionFilter`, which maps ORM errors to HTTP responses.
//!
//! ```ignore
//! #[module(imports = [OrmModule::for_root(OrmModuleOptions::new("file:./dev.db").global(true))])]
//! pub struct AppModule;
//!
//! #[injectable]
//! pub struct UsersService { db: Arc<OrmService> }
//!
//! impl UsersService {
//!     pub async fn find_all(&self) -> Result<Vec<User>, OrmError> {
//!         self.db.user().find_many(json!({})).await
//!     }
//! }
//! ```

use std::collections::HashMap;
use std::future::Future;
use std::ops::Deref;
use std::sync::Arc;

use corvis::di::{DiError, Hooks, Injectable, ResolveCtx};
use corvis::{
    ArgumentsHost, BadRequestException, BaseExceptionFilter, ConflictException, DynamicModule, ExceptionFilter, HttpException, Logger,
    Module, ModuleDefinition, NotFoundException, OnModuleDestroy, OnModuleInit, Provider, Response, Token,
};
use futures::FutureExt;
use futures::future::BoxFuture;

use crate::client::{OrmClient, OrmClientOptions};
use crate::error::OrmError;
use crate::meta::{ModelMeta, ModelSet};

/// Token of the [`OrmModuleOptions`] provider.
pub const ORM_MODULE_OPTIONS: &str = "ORM_MODULE_OPTIONS";

/// Options of [`OrmModule::for_root`].
#[derive(Clone)]
pub struct OrmModuleOptions {
    /// Datasource URL (`file:./dev.db`, `sqlite::memory:`, `postgresql://...`).
    pub url: String,
    /// Register the module globally.
    pub is_global: bool,
    /// Log every query (`log: ['query']`).
    pub log_queries: bool,
    /// Pool size (`connection_limit`).
    pub max_connections: Option<u32>,
    /// Connect eagerly in `on_module_init` (default `true`).
    pub explicit_connect: bool,
    /// Models to synchronise on startup (`db push`), handy in development and tests.
    pub push_models: Vec<&'static ModelMeta>,
    /// Default `{ maxWait, timeout, isolationLevel }` of interactive transactions (`transactionOptions`).
    pub transaction_options: crate::TransactionOptions,
}

impl std::fmt::Debug for OrmModuleOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OrmModuleOptions")
            .field("is_global", &self.is_global)
            .field("log_queries", &self.log_queries)
            .field("push_models", &self.push_models.iter().map(|m| m.name).collect::<Vec<_>>())
            .finish()
    }
}

impl OrmModuleOptions {
    pub fn new(url: impl Into<String>) -> Self {
        OrmModuleOptions {
            url: url.into(),
            is_global: false,
            log_queries: false,
            max_connections: None,
            explicit_connect: true,
            push_models: Vec::new(),
            transaction_options: crate::TransactionOptions::default(),
        }
    }

    /// Reads `DATABASE_URL` from the `ConfigService`.
    pub fn from_config(config: &corvis::ConfigService) -> corvis::Result<Self> {
        Ok(Self::new(config.try_get::<String>("DATABASE_URL")?))
    }

    pub fn global(mut self, v: bool) -> Self {
        self.is_global = v;
        self
    }
    pub fn log_queries(mut self, v: bool) -> Self {
        self.log_queries = v;
        self
    }
    pub fn max_connections(mut self, n: u32) -> Self {
        self.max_connections = Some(n);
        self
    }
    pub fn explicit_connect(mut self, v: bool) -> Self {
        self.explicit_connect = v;
        self
    }
    /// Default options of interactive transactions (see [`crate::OrmClientOptions::transaction_options`]).
    pub fn transaction_options(mut self, o: impl Into<crate::TransactionOptions>) -> Self {
        self.transaction_options = o.into();
        self
    }
    /// Run `db push` for these models when the module initializes.
    pub fn push<S: ModelSet>(mut self) -> Self {
        self.push_models.extend(S::metas());
        self
    }
}

/// The injectable client. Derefs to [`OrmClient`], so `self.db.user()` works.
pub struct OrmService {
    client: OrmClient,
    options: OrmModuleOptions,
    logger: Logger,
}

impl OrmService {
    pub fn new(options: OrmModuleOptions) -> Result<Self, OrmError> {
        let mut client_opts =
            OrmClientOptions::new(options.url.clone()).log_queries(options.log_queries).transaction_options(options.transaction_options);
        client_opts.max_connections = options.max_connections;
        Ok(OrmService { client: OrmClient::with_options(client_opts)?, options, logger: Logger::new("OrmService") })
    }

    pub fn client(&self) -> &OrmClient {
        &self.client
    }
}

impl Deref for OrmService {
    type Target = OrmClient;
    fn deref(&self) -> &OrmClient {
        &self.client
    }
}

#[corvis::async_trait]
impl OnModuleInit for OrmService {
    async fn on_module_init(&self) -> corvis::Result<()> {
        if self.options.explicit_connect || !self.options.push_models.is_empty() {
            self.logger.log(format!("Connecting to the {} database...", self.client.dialect().name()));
            self.client.connect().await?;
            self.logger.log("Database connection established");
        }
        if !self.options.push_models.is_empty() {
            self.client.db_push_models(&self.options.push_models).await?;
            let names = self.options.push_models.iter().map(|m| m.name).collect::<Vec<_>>().join(", ");
            self.logger.log(format!("Your database is now in sync with the schema ({names})"));
        }
        Ok(())
    }
}

#[corvis::async_trait]
impl OnModuleDestroy for OrmService {
    async fn on_module_destroy(&self) -> corvis::Result<()> {
        self.client.disconnect().await?;
        self.logger.log("Database connection closed");
        Ok(())
    }
}

impl Injectable for OrmService {
    fn construct(ctx: ResolveCtx) -> BoxFuture<'static, Result<Self, DiError>> {
        async move {
            let options = ctx.get_token::<OrmModuleOptions>(ORM_MODULE_OPTIONS).await?;
            OrmService::new((*options).clone())
                .map_err(|e| DiError::Factory { token: "OrmService".into(), cause: Arc::new(anyhow::Error::new(e)) })
        }
        .boxed()
    }

    fn hooks() -> Hooks<Self> {
        Hooks {
            on_module_init: Some(|this: Arc<Self>| -> BoxFuture<'static, corvis::Result<()>> {
                Box::pin(async move { this.on_module_init().await })
            }),
            on_module_destroy: Some(|this: Arc<Self>| -> BoxFuture<'static, corvis::Result<()>> {
                Box::pin(async move { this.on_module_destroy().await })
            }),
            ..Hooks::default()
        }
    }
}

/// The module that provides [`OrmService`].
#[derive(Default)]
pub struct OrmModule;
corvis::default_injectable!(OrmModule);

impl Module for OrmModule {
    fn definition() -> ModuleDefinition {
        ModuleDefinition::new::<OrmModule>("OrmModule").module_provider(Provider::class::<OrmModule>())
    }
}

impl OrmModule {
    /// Registers [`OrmService`] with static options (global when `options.is_global`).
    pub fn for_root(options: OrmModuleOptions) -> DynamicModule {
        let global = options.is_global;
        DynamicModule::new::<OrmModule>()
            .provider(Provider::use_value(ORM_MODULE_OPTIONS, options))
            .provider(Provider::class::<OrmService>())
            .export(Token::of::<OrmService>())
            .global(global)
    }

    /// Registers [`OrmService`] with options built by an async factory that can resolve
    /// other providers (for example `ConfigService`).
    ///
    /// ```ignore
    /// OrmModule::for_root_async(true, |ctx| async move {
    ///     let config = ctx.get::<ConfigService>().await?;
    ///     Ok(OrmModuleOptions::from_config(&config)?.log_queries(true))
    /// })
    /// ```
    pub fn for_root_async<F, Fut>(is_global: bool, factory: F) -> DynamicModule
    where
        F: Fn(ResolveCtx) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = corvis::Result<OrmModuleOptions>> + Send + 'static,
    {
        DynamicModule::new::<OrmModule>()
            .provider(Provider::use_factory(ORM_MODULE_OPTIONS, factory))
            .provider(Provider::class::<OrmService>())
            .export(Token::of::<OrmService>())
            .global(is_global)
    }

    /// `for_root_async` reading `DATABASE_URL` from `ConfigService` (import `ConfigModule` globally).
    pub fn for_root_from_config(is_global: bool) -> DynamicModule {
        Self::for_root_async(is_global, |ctx| async move {
            let config = ctx.get::<corvis::ConfigService>().await?;
            OrmModuleOptions::from_config(&config)
        })
    }
}

impl OrmError {
    /// Default HTTP mapping (the one [`OrmExceptionFilter`] uses):
    /// `P2002` → 409, `P2025` → 404, `P2000`/validation → 400, everything else → 500.
    pub fn to_http_exception(&self) -> HttpException {
        let message = self.message().replace('\n', " ").trim().to_string();
        match self.code() {
            Some("P2002") => ConflictException::new(message),
            Some("P2025") => NotFoundException::new(message),
            Some("P2000") | Some("P2003") | Some("P2011") | Some("P2014") => BadRequestException::new(message),
            _ if self.is_validation() => BadRequestException::new(message),
            _ => HttpException::internal(self.clone()),
        }
    }
}

/// Converts `Result<T, OrmError>` into `HttpResult<T>` with the default status mapping.
///
/// A direct `impl From<OrmError> for HttpException` is impossible because corvis
/// already provides `impl<E: std::error::Error> From<E> for HttpException` (500): use
/// `.http()?` in handlers, or register [`OrmExceptionFilter`].
pub trait OrmResultExt<T> {
    fn http(self) -> corvis::HttpResult<T>;
}

impl<T> OrmResultExt<T> for Result<T, OrmError> {
    fn http(self) -> corvis::HttpResult<T> {
        self.map_err(|e| e.to_http_exception())
    }
}

/// Exception filter mapping ORM errors to HTTP responses:
///
/// ```ignore
/// app.use_global_filters(OrmExceptionFilter::new());
/// // or with a custom map: OrmExceptionFilter::new().map_code("P2000", 400)
/// ```
#[derive(Clone, Default)]
pub struct OrmExceptionFilter {
    codes: HashMap<String, u16>,
}
corvis::default_injectable!(OrmExceptionFilter);

impl OrmExceptionFilter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Overrides the HTTP status of an error code (`map_code("P2000", 400)`).
    pub fn map_code(mut self, code: &str, status: u16) -> Self {
        self.codes.insert(code.to_string(), status);
        self
    }

    fn orm_error(exception: &HttpException) -> Option<&OrmError> {
        exception.downcast_cause::<OrmError>()
    }

    /// The `HttpException` that would be produced for `error`.
    pub fn map(&self, error: &OrmError) -> HttpException {
        if let Some(status) = error.code().and_then(|c| self.codes.get(c)) {
            let message = error.message().replace('\n', " ").trim().to_string();
            return HttpException::new(message, *status);
        }
        error.to_http_exception()
    }
}

#[corvis::async_trait]
impl ExceptionFilter for OrmExceptionFilter {
    fn catches(&self, exception: &HttpException) -> bool {
        Self::orm_error(exception).is_some()
    }

    async fn catch(&self, exception: HttpException, _host: &ArgumentsHost) -> Response {
        match Self::orm_error(&exception) {
            Some(e) => BaseExceptionFilter::handle(&self.map(e)),
            None => BaseExceptionFilter::handle(&exception),
        }
    }
}
