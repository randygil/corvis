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
