//! Support code for the procedural macros. Not part of the public API.
//!
//! Many helpers use "autoref specialization": the macros know the concrete types
//! at expansion time, so method resolution picks the most specific implementation
//! (e.g. "does this provider implement `OnModuleInit`?").

#![allow(missing_docs)]

use std::marker::PhantomData;
use std::sync::Arc;

pub use futures::FutureExt;
pub use futures::future::BoxFuture;
pub use regex;
pub use serde;
pub use serde_json;
pub use std::sync::LazyLock;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::common::pipe::{ParamKind, ValidatorFn};
use crate::di::{DiError, InjectField, Instance, ResolveCtx, Token};
use crate::http::{BadRequestException, HandlerResult, HttpException, IntoResponse, Response};

pub struct Probe<T: ?Sized>(PhantomData<fn() -> Box<T>>);

impl<T: ?Sized> Probe<T> {
    pub const fn new() -> Self {
        Probe(PhantomData)
    }
}

impl<T: ?Sized> Default for Probe<T> {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Field injection: `Arc<T>` / `Option<Arc<T>>` / `ModuleRef` are injected,
// anything else falls back to `Default`.
// ---------------------------------------------------------------------------

pub struct InjectTag;
impl InjectTag {
    pub fn resolve<T: InjectField>(self, ctx: ResolveCtx, token: Option<Token>, index: usize) -> BoxFuture<'static, Result<T, DiError>> {
        T::inject(ctx, token, index)
    }
}

pub struct DefaultTag;
impl DefaultTag {
    pub fn resolve<T: Default + Send + 'static>(
        self,
        _ctx: ResolveCtx,
        _token: Option<Token>,
        _index: usize,
    ) -> BoxFuture<'static, Result<T, DiError>> {
        async { Ok(T::default()) }.boxed()
    }
}

pub trait InjectKind {
    fn __corvis_inject_kind(&self) -> InjectTag {
        InjectTag
    }
}
impl<T: InjectField> InjectKind for Probe<T> {}

pub trait DefaultKind {
    fn __corvis_inject_kind(&self) -> DefaultTag {
        DefaultTag
    }
}
impl<T: Default> DefaultKind for &Probe<T> {}

/// Resolves `Arc<T>` by token and clones the inner value (`#[inject("TOKEN")] port: u16`).
pub async fn inject_cloned<T: Clone + Send + Sync + 'static>(ctx: ResolveCtx, token: Token, index: usize) -> Result<T, DiError> {
    let arc = ctx.resolve_dependency::<T>(token, index).await?;
    Ok((*arc).clone())
}

// ---------------------------------------------------------------------------
// Lifecycle hooks detection.
// ---------------------------------------------------------------------------

pub mod hooks {
    use super::*;
    use crate::common::lifecycle::*;

    type Hook<T> = Option<fn(Arc<T>) -> BoxFuture<'static, crate::Result<()>>>;
    type SigHook<T> = Option<fn(Arc<T>, Option<String>) -> BoxFuture<'static, crate::Result<()>>>;

    macro_rules! detect {
        ($has:ident, $no:ident, $method:ident, $tr:ident, $call:ident) => {
            pub trait $has<T> {
                fn $method(&self) -> Hook<T>;
            }
            impl<T: $tr + Send + Sync + 'static> $has<T> for Probe<T> {
                fn $method(&self) -> Hook<T> {
                    Some(|this: Arc<T>| -> BoxFuture<'static, crate::Result<()>> { Box::pin(async move { this.$call().await }) })
                }
            }
            pub trait $no<T> {
                fn $method(&self) -> Hook<T> {
                    None
                }
            }
            impl<T> $no<T> for &Probe<T> {}
        };
    }

    macro_rules! detect_sig {
        ($has:ident, $no:ident, $method:ident, $tr:ident, $call:ident) => {
            pub trait $has<T> {
                fn $method(&self) -> SigHook<T>;
            }
            impl<T: $tr + Send + Sync + 'static> $has<T> for Probe<T> {
                fn $method(&self) -> SigHook<T> {
                    Some(|this: Arc<T>, sig: Option<String>| -> BoxFuture<'static, crate::Result<()>> {
                        Box::pin(async move { this.$call(sig).await })
                    })
                }
            }
            pub trait $no<T> {
                fn $method(&self) -> SigHook<T> {
                    None
                }
            }
            impl<T> $no<T> for &Probe<T> {}
        };
    }

    detect!(HasOnModuleInit, NoOnModuleInit, __corvis_on_module_init, OnModuleInit, on_module_init);
    detect!(
        HasOnApplicationBootstrap,
        NoOnApplicationBootstrap,
        __corvis_on_application_bootstrap,
        OnApplicationBootstrap,
        on_application_bootstrap
    );
    detect!(HasOnModuleDestroy, NoOnModuleDestroy, __corvis_on_module_destroy, OnModuleDestroy, on_module_destroy);
    detect_sig!(
        HasBeforeApplicationShutdown,
        NoBeforeApplicationShutdown,
        __corvis_before_application_shutdown,
        BeforeApplicationShutdown,
        before_application_shutdown
    );
    detect_sig!(
        HasOnApplicationShutdown,
        NoOnApplicationShutdown,
        __corvis_on_application_shutdown,
        OnApplicationShutdown,
        on_application_shutdown
    );

    // Method handlers (`#[handlers]`).
    pub trait HasHandlers<T> {
        fn __corvis_handlers_fn(&self) -> Option<fn(Arc<T>) -> Vec<crate::discovery::DiscoveredHandler>>;
    }
    impl<T: crate::discovery::HandlersProvider> HasHandlers<T> for Probe<T> {
        fn __corvis_handlers_fn(&self) -> Option<fn(Arc<T>) -> Vec<crate::discovery::DiscoveredHandler>> {
            Some(<T as crate::discovery::HandlersProvider>::__corvis_handlers)
        }
    }
    pub trait NoHandlers<T> {
        fn __corvis_handlers_fn(&self) -> Option<fn(Arc<T>) -> Vec<crate::discovery::DiscoveredHandler>> {
            None
        }
    }
    impl<T> NoHandlers<T> for &Probe<T> {}

    // WebSocket gateways (`#[websocket_gateway]`).
    type GatewayFn<T> = Option<fn(Arc<T>) -> crate::discovery::GatewayInfo>;
    #[cfg(feature = "websockets")]
    pub trait HasGateway<T> {
        fn __corvis_gateway_fn(&self) -> GatewayFn<T>;
    }
    #[cfg(feature = "websockets")]
    impl<T: crate::websockets::Gateway> HasGateway<T> for Probe<T> {
        fn __corvis_gateway_fn(&self) -> GatewayFn<T> {
            Some(<T as crate::websockets::Gateway>::__corvis_gateway)
        }
    }
    pub trait NoGateway<T> {
        fn __corvis_gateway_fn(&self) -> GatewayFn<T> {
            None
        }
    }
    impl<T> NoGateway<T> for &Probe<T> {}

    // Middleware configuration (`CorvisModule`).
    pub trait HasConfigure {
        fn __corvis_configure(&self) -> Option<crate::di::ConfigureFn>;
    }
    impl<T: crate::di::CorvisModule + Send + Sync + 'static> HasConfigure for Probe<T> {
        fn __corvis_configure(&self) -> Option<crate::di::ConfigureFn> {
            Some(|inst: &Instance, consumer: &mut crate::common::middleware::MiddlewareConsumer| {
                if let Some(m) = crate::di::provider::downcast::<T>(inst) {
                    m.configure(consumer);
                }
            })
        }
    }
    pub trait NoConfigure {
        fn __corvis_configure(&self) -> Option<crate::di::ConfigureFn> {
            None
        }
    }
    impl<T> NoConfigure for &Probe<T> {}
}

// ---------------------------------------------------------------------------
// Validation detection.
// ---------------------------------------------------------------------------

pub trait HasValidator {
    fn __corvis_validator(&self) -> Option<ValidatorFn>;
}
impl<T: crate::validation::Validate> HasValidator for Probe<T> {
    fn __corvis_validator(&self) -> Option<ValidatorFn> {
        Some(T::validate_value)
    }
}
pub trait NoValidator {
    fn __corvis_validator(&self) -> Option<ValidatorFn> {
        None
    }
}
impl<T> NoValidator for &Probe<T> {}

pub trait HasZod {
    fn __corvis_zod(&self) -> Option<fn() -> &'static crate::z::Schema>;
}
impl<T: crate::z::ZodDto> HasZod for Probe<T> {
    fn __corvis_zod(&self) -> Option<fn() -> &'static crate::z::Schema> {
        Some(T::schema)
    }
}
pub trait NoZod {
    fn __corvis_zod(&self) -> Option<fn() -> &'static crate::z::Schema> {
        None
    }
}
impl<T> NoZod for &Probe<T> {}

pub trait HasApiSchema {
    fn __corvis_api_schema(&self) -> Option<crate::openapi::ApiSchemaRef>;
}
impl<T: crate::openapi::ApiSchema> HasApiSchema for Probe<T> {
    fn __corvis_api_schema(&self) -> Option<crate::openapi::ApiSchemaRef> {
        Some(crate::openapi::ApiSchemaRef::of::<T>())
    }
}
pub trait NoApiSchema {
    fn __corvis_api_schema(&self) -> Option<crate::openapi::ApiSchemaRef> {
        None
    }
}
impl<T> NoApiSchema for &Probe<T> {}

// ---------------------------------------------------------------------------
// Handler return values: `IntoResponse` first, then any `Serialize` as JSON.
// ---------------------------------------------------------------------------

pub struct ReplyTag;
impl ReplyTag {
    pub fn convert<T: IntoResponse>(self, v: T) -> HandlerResult {
        v.into_response()
    }
}
pub struct SerializeTag;
impl SerializeTag {
    pub fn convert<T: Serialize>(self, v: T) -> HandlerResult {
        let value = serde_json::to_value(&v).map_err(HttpException::from)?;
        Ok(Response::json(value))
    }
}
pub trait ReplyKind {
    fn __corvis_reply_kind(&self) -> ReplyTag {
        ReplyTag
    }
}
impl<T: IntoResponse> ReplyKind for T {}
pub trait SerializeKind {
    fn __corvis_reply_kind(&self) -> SerializeTag {
        SerializeTag
    }
}
impl<T: Serialize> SerializeKind for &T {}

// Errors: `Into<HttpException>` first, then anything convertible to `anyhow::Error`.
pub struct ExcTag;
impl ExcTag {
    pub fn convert<E: Into<HttpException>>(self, e: E) -> HttpException {
        e.into()
    }
}
pub struct AnyhowTag;
impl AnyhowTag {
    pub fn convert<E: Into<anyhow::Error>>(self, e: E) -> HttpException {
        HttpException::from_anyhow(e.into())
    }
}
pub trait ExcKind {
    fn __corvis_exc_kind(&self) -> ExcTag {
        ExcTag
    }
}
impl<E: Into<HttpException>> ExcKind for E {}
pub trait AnyhowKind {
    fn __corvis_exc_kind(&self) -> AnyhowTag {
        AnyhowTag
    }
}
impl<E: Into<anyhow::Error>> AnyhowKind for &E {}

// ---------------------------------------------------------------------------
// Handler arguments.
// ---------------------------------------------------------------------------

/// Deserializes the (already piped) argument `i` into the declared parameter type.
pub fn extract_arg<T: DeserializeOwned>(args: &mut [Value], i: usize, kind: ParamKind, name: Option<&str>) -> Result<T, HttpException> {
    let v = std::mem::take(&mut args[i]);
    let res = match kind {
        ParamKind::Body | ParamKind::File | ParamKind::Files => serde_json::from_value::<T>(v),
        _ => crate::common::lenient::from_value::<T>(v),
    };
    res.map_err(|e| {
        let msg = match name {
            Some(n) => format!("{n}: {e}"),
            None => e.to_string(),
        };
        BadRequestException::new(msg)
    })
}

/// Serializes metadata values for `#[set_metadata]`.
pub fn to_value<T: Serialize>(v: T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

/// Provider for `#[injectable]` types listed in `providers: [...]`.
pub trait ProviderItem {
    fn __corvis_provider() -> crate::di::Provider;
}

/// Export entry for types listed in `exports: [...]`.
pub trait Exportable {
    fn __corvis_export() -> crate::di::ExportItem;
}

pub fn module_export<M: 'static>(name: &'static str) -> crate::di::ExportItem {
    crate::di::ExportItem::Module(std::any::TypeId::of::<M>(), name)
}

pub fn token_export<T: 'static>() -> crate::di::ExportItem {
    crate::di::ExportItem::Token(Token::of::<T>())
}

/// `true` when type `T` is a module (used to give better error messages).
pub fn type_name_of<T: ?Sized>() -> &'static str {
    std::any::type_name::<T>()
}

#[doc(hidden)]
#[macro_export]
macro_rules! default_injectable {
    ($($t:ty),* $(,)?) => {
        $(
            impl $crate::di::Injectable for $t {
                fn construct(_ctx: $crate::di::ResolveCtx) -> $crate::__private::BoxFuture<'static, ::std::result::Result<Self, $crate::di::DiError>> {
                    ::std::boxed::Box::pin(async { Ok(<$t as ::std::default::Default>::default()) })
                }
            }
        )*
    };
}
