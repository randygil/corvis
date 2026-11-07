//! # corvis
//!
//! A progressive Rust framework for building efficient and scalable server-side
//! applications, built around modules,
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
pub mod http;
pub mod http_adapter;
pub mod logger;
pub mod modules;
pub mod multer;
pub mod openapi;
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

pub use application::{ListenAddr, LoggerOption, CorvisApplication, CorvisApplicationOptions, CorvisFactory};
pub use common::context::{ArgumentsHost, ContextType, ExecutionContext, Target};
pub use common::enhancer::{
    FilterRef, GuardRef, InterceptorRef, MiddlewareRef, PipeRef, filter_instance, filter_of, guard_instance, guard_of,
    interceptor_instance, interceptor_of, middleware_instance, middleware_of, pipe_instance, pipe_of,
};
pub use common::filter::{BaseExceptionFilter, ExceptionFilter};
pub use common::guard::CanActivate;
pub use common::interceptor::{CallHandler, LoggingInterceptor, CorvisInterceptor, TimeoutInterceptor};
pub use common::lifecycle::{BeforeApplicationShutdown, OnApplicationBootstrap, OnApplicationShutdown, OnModuleDestroy, OnModuleInit};
pub use common::middleware::{FnMiddleware, IntoRouteSpecs, MiddlewareConsumer, CorvisMiddleware, Next, RouteInfo, RouteSpec};
pub use common::param::FromContext;
pub use common::pipe::{
    ArgumentMetadata, DefaultValuePipe, Metatype, ParamKind, ParseArrayPipe, ParseBoolPipe, ParseEnumPipe, ParseFloatPipe,
    ParseIntPipe, ParseUUIDPipe, ParseUuidPipe, PipeTransform,
};
pub use common::reflector::Reflector;
pub use common::parse_file::{
    FileTypePattern, FileTypeValidator, FileValidator, MaxFileSizeValidator, ParseFilePipe, ParseFilePipeBuilder,
};
pub use multer::{
    AnyFilesInterceptor, FileFieldsInterceptor, FileInterceptor, FilesInterceptor, MulterField, MulterModule,
    MulterOptions, NoFilesInterceptor, UploadedFile,
};
pub use config::{ConfigModule, ConfigModuleOptions, ConfigService};
pub use di::{
    DiError, DynamicModule, ExportItem, Hooks, InjectField, Injectable, Module, ModuleDefinition, ModuleImport, ModuleRef,
    CorvisModule, Provider, ResolveCtx, Scope, Token,
};
pub use http::*;
pub use logger::{ConsoleLogger, LogLevel, Logger, LoggerService};
pub use router::controller::{
    Controller, ControllerBase, ControllerDef, ControllerMetadata, ParamDef, RouteDef, VERSION_NEUTRAL, VersionValue,
};
pub use common::decorator::{CustomDecorator, apply_decorators};
/// Metadata key of `@HttpCode()` (`__httpCode__`): a [`CustomDecorator`] that sets it changes the
/// route's default status, like `applyDecorators(HttpCode(201), ...)` in a decorator-based framework.
pub const HTTP_CODE_METADATA: &str = "__httpCode__";
pub use openapi::{ApiSchema, ApiSchemaDef, ApiSchemaRef};
pub use router::controller::RouteMeta;
pub use application::introspection::{ModuleInfo, ControllerInfo, ResolvedRoutePath};
pub use router::cors::{CorsOptions, CorsOrigin, OriginMatcher};
pub use router::versioning::{VersioningOptions, VersioningType};
pub use validation::{Validate, ValidationError, ValidationPipe, ValidationPipeOptions, ValidatorOptions};
pub use z::{ZodDto, ZodSchema, ZodValidationPipe};

pub use corvis_macros::{
    Validate, controller, handlers, injectable, main, module, routes, test, websocket_gateway,
};

#[cfg(feature = "websockets")]
pub use websockets::{GatewayOptions, OnGatewayConnection, OnGatewayDisconnect, OnGatewayInit, WsClient, WsException, WsResponse, WsServer};
pub use http_adapter::{HttpAdapter, HttpAdapterHost, HttpServer, UpgradeRequest};
#[cfg(feature = "jwt")]
pub use jwt::{JwtModule, JwtModuleOptions, JwtService, JwtSignOptions, extract_bearer_token};
pub use modules::cache::{CACHE_KEY_METADATA, CACHE_MANAGER, CACHE_TTL_METADATA, CacheInterceptor, CacheManager, CacheModule, CacheOptions};
pub use modules::event_emitter::{EventEmitter, EventEmitterModule, EventEmitterOptions};
pub use modules::health::{HealthCheckService, HealthIndicator, HttpHealthIndicator, MemoryHealthIndicator, TerminusModule};
pub use modules::schedule;
pub use modules::schedule::{CronExpression, CronJob, CronOptions, CronTime, ScheduleModule, ScheduleModuleOptions, SchedulerRegistry};
pub use modules::serve_static::{ServeStaticModule, ServeStaticOptions};
pub use modules::throttler::{SKIP_THROTTLE, THROTTLE_LIMIT, THROTTLE_TTL, ThrottlerGuard, ThrottlerModule, ThrottlerOptions, ThrottlerStorage};

/// Everything you need in a typical application: `use corvis::prelude::*;`.
pub mod prelude {
    pub use crate::{
        ArgumentMetadata, ArgumentsHost, BadRequestException, BaseExceptionFilter, Body, CallHandler, CanActivate,
        ConfigModule, ConfigModuleOptions, ConfigService, ConflictException, Controller, ControllerDef, Cookie,
        CorsOptions, DefaultValuePipe, DynamicModule, ExceptionFilter, ExecutionContext, ForbiddenException, FromContext,
        HandlerResult, HttpException, HttpResult, HttpStatus, Injectable, InternalServerErrorException, IntoResponse,
        Json, Logger, Middleware, ModuleImport, ModuleRef, CorvisApplication, CorvisFactory, CorvisInterceptor, CorvisMiddleware,
        CorvisModule, Next, NotFoundException, OnApplicationBootstrap, OnApplicationShutdown, OnModuleDestroy,
        OnModuleInit, BeforeApplicationShutdown, ParseArrayPipe, ParseBoolPipe, ParseEnumPipe, ParseFloatPipe,
        ParseIntPipe, ParseUUIDPipe, PipeTransform, Provider, Redirect, Reflector, Request, RequestMethod, Response,
        RouteInfo, RouteSpec, Scope, StreamableFile, Token, UnauthorizedException, UnprocessableEntityException,
        Validate, ValidationPipe, VersionValue, VersioningOptions, VersioningType, VERSION_NEUTRAL,
        controller, handlers, injectable, json, module, routes, Sse, MessageEvent, EventEmitter,
        EventEmitterModule, EventEmitterOptions, ScheduleModule, SchedulerRegistry, CronExpression, CronJob, CronOptions, CacheModule,
        CacheManager, CacheInterceptor, CacheOptions, CACHE_KEY_METADATA, CACHE_TTL_METADATA, ThrottlerModule, ThrottlerGuard, ThrottlerOptions,
        ServeStaticModule, ServeStaticOptions, TerminusModule, HealthCheckService, HealthIndicator, SKIP_THROTTLE,
        THROTTLE_LIMIT, THROTTLE_TTL,
    };
    pub use crate::MiddlewareConsumer;
    pub use crate::{HttpAdapterHost, StreamableFileOptions};
    pub use crate::z::{self, ZodSchema, ZodValidationPipe};
    pub use async_trait::async_trait;
    pub use serde_json::Value;
    pub use std::sync::Arc;
}

#[doc(hidden)]
pub use common::middleware::CorvisMiddleware as Middleware;
