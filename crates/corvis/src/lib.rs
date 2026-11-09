//! # corvis
//!
//! A modular backend framework for Rust, built around modules,
//! controllers, providers with dependency injection, middleware, guards,
//! interceptors, pipes, exception filters, validation, configuration, testing...
//!
//! ```ignore
//! use corvis::prelude::*;
//!
//! #[injectable]
//! pub struct AppService;
//!
//! impl AppService {
//!     pub fn get_hello(&self) -> String { "Hello World!".into() }
//! }
//!
//! #[controller]
//! pub struct AppController { app_service: Arc<AppService> }
//!
//! #[routes]
//! impl AppController {
//!     #[get]
//!     async fn get_hello(&self) -> String { self.app_service.get_hello() }
//! }
//!
//! #[module(controllers = [AppController], providers = [AppService])]
//! pub struct AppModule;
//!
//! #[corvis::main]
//! async fn main() -> corvis::Result<()> {
//!     let app = CorvisFactory::create::<AppModule>().await?;
//!     app.listen(3000).await
//! }
//! ```

#![forbid(unsafe_code)]

extern crate self as corvis;

pub mod application;
pub mod common;
pub mod config;
pub mod di;
pub mod discovery;
#[cfg(test)]
mod fuzz_tests;
pub mod http;
pub mod http_adapter;
pub mod logger;
pub mod modules;
pub mod multer;
pub mod openapi;
pub mod posture;
pub mod router;
pub mod testing;
pub mod validation;
pub mod z;

#[cfg(feature = "jwt")]
pub mod jwt;
#[cfg(feature = "websockets")]
pub mod websockets;
/// Plain WebSockets (`ws` package) on top of `HttpAdapterHost` upgrade listeners.
#[cfg(feature = "websockets")]
pub mod ws;

mod server;
pub use server::ServerOptions;

#[doc(hidden)]
pub mod __private;

// Compiles the Rust blocks of the repository README with `cargo test --doc -p corvis`, so they cannot drift from the API.
#[cfg(doctest)]
#[doc = include_str!("../../../README.md")]
pub struct ReadmeDoctests;

#[cfg(doctest)]
mod doctests;

/// `Result` alias used by bootstrap code, lifecycle hooks and factories.
pub type Result<T, E = anyhow::Error> = std::result::Result<T, E>;

/// `Result` alias for route handlers.
pub type HttpResult<T> = std::result::Result<T, http::HttpException>;

pub use anyhow;
pub use async_trait::async_trait;
pub use bytes;
pub use futures;
pub use serde_json;
pub use serde_json::json;
pub use tokio;

pub use application::{CorvisApplication, CorvisApplicationOptions, CorvisFactory, InitFailedError, ListenAddr, LoggerOption};
pub use common::context::{ArgumentsHost, ContextType, ExecutionContext, Target};
pub use common::decorator::{CustomDecorator, apply_decorators};
pub use common::enhancer::{
    FilterRef, GuardRef, InterceptorRef, MiddlewareRef, PipeRef, filter_instance, filter_of, guard_instance, guard_of,
    interceptor_instance, interceptor_of, middleware_instance, middleware_of, pipe_instance, pipe_of,
};
pub use common::filter::{BaseExceptionFilter, ExceptionFilter};
pub use common::guard::CanActivate;
pub use common::interceptor::{CallHandler, CorvisInterceptor, LoggingInterceptor, TimeoutInterceptor};
pub use common::lifecycle::{BeforeApplicationShutdown, OnApplicationBootstrap, OnApplicationShutdown, OnModuleDestroy, OnModuleInit};
pub use common::middleware::{CorvisMiddleware, FnMiddleware, IntoRouteSpecs, MiddlewareConsumer, Next, RouteInfo, RouteSpec};
pub use common::param::FromContext;
pub use common::parse_file::{
    FileTypePattern, FileTypeValidator, FileValidator, MaxFileSizeValidator, ParseFilePipe, ParseFilePipeBuilder,
};
pub use common::pipe::{
    ArgumentMetadata, DefaultValuePipe, Metatype, ParamKind, ParseArrayPipe, ParseBoolPipe, ParseEnumPipe, ParseFloatPipe, ParseIntPipe,
    ParseUUIDPipe, ParseUuidPipe, PipeTransform,
};
pub use common::reflector::Reflector;
pub use config::{ConfigModule, ConfigModuleOptions, ConfigService};
pub use di::{
    CorvisModule, DiError, DynamicModule, ExportItem, Hooks, InjectField, Injectable, Module, ModuleDefinition, ModuleImport, ModuleRef,
    Provider, ResolveCtx, Scope, Token,
};
pub use http::*;
pub use logger::{ConsoleLogger, LogLevel, Logger, LoggerService};
pub use multer::{
    AnyFilesInterceptor, FileFieldsInterceptor, FileInterceptor, FilesInterceptor, MulterField, MulterModule, MulterOptions,
    NoFilesInterceptor, UploadedFile,
};
pub use router::controller::{
    Controller, ControllerBase, ControllerDef, ControllerMetadata, ParamDef, RouteDef, VERSION_NEUTRAL, VersionValue,
};
/// Metadata key of `@HttpCode()` (`__httpCode__`): a [`CustomDecorator`] that sets it changes the
/// route's default status, like `applyDecorators(HttpCode(201), ...)` in a decorator-based framework.
pub const HTTP_CODE_METADATA: &str = "__httpCode__";

/// Metadata key set by `#[public]` (`"isPublic"`, the key apps already use for public routes). Its value is
/// `true` on a controller or a handler that is open on purpose. The route posture check reads it, and a global
/// guard can skip public routes with `Reflector::get_all_and_override::<bool>(PUBLIC_METADATA, ..)`.
///
/// `#[public]` goes below `#[controller]` or on a route method inside `#[routes]`; the route posture check
/// reads it for gateways and SSE routes too:
///
/// ```
/// use corvis::prelude::*;
///
/// #[controller("status")]
/// #[public]
/// pub struct StatusController;
///
/// #[routes]
/// impl StatusController {
///     #[get("ping")]
///     async fn ping(&self) -> &'static str {
///         "pong"
///     }
/// }
/// ```
///
/// It takes no arguments:
///
/// ```compile_fail
/// use corvis::prelude::*;
///
/// #[controller("status")]
/// pub struct StatusController;
///
/// #[routes]
/// impl StatusController {
///     #[get("ping")]
///     #[public(true)]
///     async fn ping(&self) -> &'static str {
///         "pong"
///     }
/// }
/// ```
///
/// It needs a route attribute on the same method:
///
/// ```compile_fail
/// use corvis::prelude::*;
///
/// #[controller("status")]
/// pub struct StatusController;
///
/// #[routes]
/// impl StatusController {
///     #[public]
///     fn helper(&self) {}
/// }
/// ```
///
/// And it cannot be used on other items:
///
/// ```compile_fail
/// use corvis::prelude::*;
///
/// #[public]
/// fn open() {}
/// ```
///
/// Below `#[websocket_gateway]` and on `#[subscribe_message]` methods it marks gateways and message handlers:
///
/// ```
/// use corvis::prelude::*;
/// use corvis::{WsServer, websocket_gateway};
///
/// #[websocket_gateway(path = "/status")]
/// #[public]
/// pub struct StatusGateway {
///     #[websocket_server]
///     server: WsServer,
/// }
///
/// #[handlers]
/// impl StatusGateway {
///     #[subscribe_message("ping")]
///     #[public]
///     fn ping(&self) {}
/// }
/// ```
///
/// It is rejected on the other `#[handlers]` kinds:
///
/// ```compile_fail
/// use corvis::prelude::*;
///
/// #[injectable]
/// pub struct Jobs;
///
/// #[handlers]
/// impl Jobs {
///     #[cron("0 * * * * *")]
///     #[public]
///     fn tick(&self) {}
/// }
/// ```
///
/// And it takes no arguments on a gateway either:
///
/// ```compile_fail
/// use corvis::prelude::*;
/// use corvis::{WsServer, websocket_gateway};
///
/// #[websocket_gateway(path = "/status")]
/// #[public(true)]
/// pub struct StatusGateway {
///     #[websocket_server]
///     server: WsServer,
/// }
/// ```
pub const PUBLIC_METADATA: &str = "isPublic";
pub use application::introspection::{ControllerInfo, ModuleInfo, ResolvedRoutePath};
pub use openapi::{ApiSchema, ApiSchemaDef, ApiSchemaRef};
pub use router::controller::RouteMeta;
pub use router::cors::{CorsOptions, CorsOrigin, OriginMatcher};
pub use router::versioning::{VersioningOptions, VersioningType};
pub use validation::{Validate, ValidationError, ValidationPipe, ValidationPipeOptions, ValidatorOptions};
pub use z::{ZodDto, ZodSchema, ZodValidationPipe};

pub use corvis_macros::{Validate, controller, handlers, injectable, main, module, public, routes, test, websocket_gateway};
pub use posture::{
    AccessGuardSummary, GuardInfo, GuardScope, PostureEntry, PostureExemption, PostureKind, PostureStatus, PublicSource, RoutePostureError,
    RoutePostureMode, RoutePostureOptions, RoutePostureReport, UncheckedSurface,
};

#[cfg(feature = "compression")]
pub use http::compression::{CompressionContext, CompressionOptions, should_compress};
pub use http_adapter::{HttpAdapter, HttpAdapterHost, HttpServer, UpgradeRequest};
#[cfg(feature = "jwt")]
pub use jwt::{JwtModule, JwtModuleOptions, JwtService, JwtSignOptions, extract_bearer_token};
pub use modules::cache::{
    CACHE_KEY_METADATA, CACHE_MANAGER, CACHE_TTL_METADATA, CacheInterceptor, CacheManager, CacheModule, CacheOptions,
};
pub use modules::event_emitter::{EventEmitter, EventEmitterModule, EventEmitterOptions};
pub use modules::health::{HealthCheckService, HealthIndicator, HttpHealthIndicator, MemoryHealthIndicator, PingOptions, TerminusModule};
pub use modules::schedule;
pub use modules::schedule::{CronExpression, CronJob, CronOptions, CronTime, ScheduleModule, ScheduleModuleOptions, SchedulerRegistry};
pub use modules::serve_static::{ServeStaticModule, ServeStaticOptions};
pub use modules::throttler::{
    SKIP_THROTTLE, THROTTLE_BLOCK_DURATION, THROTTLE_KEY_GENERATOR, THROTTLE_LIMIT, THROTTLE_TRACKER, THROTTLE_TTL, ThrottleOverride,
    ThrottlerGuard, ThrottlerKeyGeneratorFn, ThrottlerModule, ThrottlerOptions, ThrottlerRecord, ThrottlerStorage, ThrottlerTrackerFn,
    skip_throttle, skip_throttle_key, throttle, throttle_key,
};
#[cfg(feature = "websockets")]
pub use websockets::{
    GatewayOptions, OnGatewayConnection, OnGatewayDisconnect, OnGatewayInit, WsClient, WsException, WsResponse, WsServer,
};

/// Everything you need in a typical application: `use corvis::prelude::*;`.
pub mod prelude {
    pub use crate::MiddlewareConsumer;
    pub use crate::z::{self, ZodSchema, ZodValidationPipe};
    pub use crate::{
        ArgumentMetadata, ArgumentsHost, BadRequestException, BaseExceptionFilter, BeforeApplicationShutdown, Body, CACHE_KEY_METADATA,
        CACHE_TTL_METADATA, CacheInterceptor, CacheManager, CacheModule, CacheOptions, CallHandler, CanActivate, ConfigModule,
        ConfigModuleOptions, ConfigService, ConflictException, Controller, ControllerDef, Cookie, CorsOptions, CorvisApplication,
        CorvisFactory, CorvisInterceptor, CorvisMiddleware, CorvisModule, CronExpression, CronJob, CronOptions, DefaultValuePipe,
        DynamicModule, EventEmitter, EventEmitterModule, EventEmitterOptions, ExceptionFilter, ExecutionContext, ForbiddenException,
        FromContext, HandlerResult, HealthCheckService, HealthIndicator, HttpError, HttpException, HttpResult, HttpStatus, Injectable,
        InternalServerErrorException, IntoResponse, Json, Logger, MessageEvent, Middleware, ModuleImport, ModuleRef, Next,
        NotFoundException, OnApplicationBootstrap, OnApplicationShutdown, OnModuleDestroy, OnModuleInit, PUBLIC_METADATA, ParseArrayPipe,
        ParseBoolPipe, ParseEnumPipe, ParseFloatPipe, ParseIntPipe, ParseUUIDPipe, PipeTransform, Provider, Redirect, Reflector, Request,
        RequestMethod, Response, ResultHttpExt, RouteInfo, RouteSpec, SKIP_THROTTLE, ScheduleModule, SchedulerRegistry, Scope,
        ServeStaticModule, ServeStaticOptions, Sse, StreamableFile, THROTTLE_BLOCK_DURATION, THROTTLE_KEY_GENERATOR, THROTTLE_LIMIT,
        THROTTLE_TRACKER, THROTTLE_TTL, TerminusModule, ThrottleOverride, ThrottlerGuard, ThrottlerModule, ThrottlerOptions,
        ThrottlerRecord, Token, UnauthorizedException, UnprocessableEntityException, VERSION_NEUTRAL, Validate, ValidationPipe,
        VersionValue, VersioningOptions, VersioningType, controller, handlers, injectable, json, module, public, routes, skip_throttle,
        skip_throttle_key, throttle, throttle_key,
    };
    #[cfg(feature = "compression")]
    pub use crate::{CompressionOptions, should_compress};
    pub use crate::{HttpAdapterHost, StreamableFileOptions};
    pub use async_trait::async_trait;
    pub use serde_json::Value;
    pub use std::sync::Arc;
}

#[doc(hidden)]
pub use common::middleware::CorvisMiddleware as Middleware;
