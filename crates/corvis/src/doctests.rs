//! `compile_fail` doctests for enhancer attributes on methods without a route or handler attribute.
//! Each failing block has a passing twin that differs only by the route or handler attribute.

/// A guard on a `#[routes]` method without a route attribute does not compile.
///
/// ```compile_fail
/// use corvis::prelude::*;
///
/// #[injectable]
/// pub struct AdminGuard;
///
/// #[async_trait]
/// impl CanActivate for AdminGuard {
///     async fn can_activate(&self, _ctx: &ExecutionContext) -> Result<bool, HttpException> {
///         Ok(true)
///     }
/// }
///
/// #[controller("admin")]
/// pub struct AdminController;
///
/// #[routes]
/// impl AdminController {
///     #[use_guards(AdminGuard)]
///     fn purge(&self) -> &'static str {
///         "purged"
///     }
/// }
/// ```
///
/// The same code with a route attribute compiles.
///
/// ```
/// use corvis::prelude::*;
///
/// #[injectable]
/// pub struct AdminGuard;
///
/// #[async_trait]
/// impl CanActivate for AdminGuard {
///     async fn can_activate(&self, _ctx: &ExecutionContext) -> Result<bool, HttpException> {
///         Ok(true)
///     }
/// }
///
/// #[controller("admin")]
/// pub struct AdminController;
///
/// #[routes]
/// impl AdminController {
///     #[get("purge")]
///     #[use_guards(AdminGuard)]
///     fn purge(&self) -> &'static str {
///         "purged"
///     }
/// }
/// ```
pub struct RoutesHelperWithGuard;

/// A guard on a `#[handlers]` method without `#[subscribe_message]` does not compile.
///
/// ```compile_fail
/// use corvis::prelude::*;
///
/// #[injectable]
/// pub struct KickGuard;
///
/// #[async_trait]
/// impl CanActivate for KickGuard {
///     async fn can_activate(&self, _ctx: &ExecutionContext) -> Result<bool, HttpException> {
///         Ok(true)
///     }
/// }
///
/// #[injectable]
/// pub struct Gateway;
///
/// #[handlers]
/// impl Gateway {
///     #[use_guards(KickGuard)]
///     fn kick(&self) {}
/// }
/// ```
///
/// The same code with `#[subscribe_message]` compiles.
///
/// ```
/// use corvis::prelude::*;
///
/// #[injectable]
/// pub struct KickGuard;
///
/// #[async_trait]
/// impl CanActivate for KickGuard {
///     async fn can_activate(&self, _ctx: &ExecutionContext) -> Result<bool, HttpException> {
///         Ok(true)
///     }
/// }
///
/// #[injectable]
/// pub struct Gateway;
///
/// #[handlers]
/// impl Gateway {
///     #[subscribe_message("kick")]
///     #[use_guards(KickGuard)]
///     fn kick(&self) {}
/// }
/// ```
pub struct HandlersHelperWithGuard;

/// `#[ip]` on an `#[on_event]` method of `#[handlers]` does not compile: there is no client.
///
/// ```compile_fail
/// use corvis::prelude::*;
///
/// #[injectable]
/// pub struct Orders;
///
/// #[handlers]
/// impl Orders {
///     #[on_event("order.created")]
///     fn on_order(&self, #[ip] ip: String) {}
/// }
/// ```
///
/// The same parameter on a `#[subscribe_message]` method compiles.
///
/// ```
/// use corvis::prelude::*;
///
/// #[injectable]
/// pub struct Orders;
///
/// #[handlers]
/// impl Orders {
///     #[subscribe_message("order")]
///     fn on_order(&self, #[ip] ip: String) {}
/// }
/// ```
pub struct HandlersIpOnEvent;

/// `#[ip]` takes no arguments in gateway handlers.
///
/// ```compile_fail
/// use corvis::prelude::*;
///
/// #[injectable]
/// pub struct Gateway;
///
/// #[handlers]
/// impl Gateway {
///     #[subscribe_message("who")]
///     fn who(&self, #[ip(ParseIntPipe)] ip: String) {}
/// }
/// ```
///
/// Without the pipe it compiles.
///
/// ```
/// use corvis::prelude::*;
///
/// #[injectable]
/// pub struct Gateway;
///
/// #[handlers]
/// impl Gateway {
///     #[subscribe_message("who")]
///     fn who(&self, #[ip] ip: String) {}
/// }
/// ```
pub struct HandlersIpWithArguments;
