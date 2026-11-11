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

/// An option `#[websocket_gateway]` does not know is a compile error that names the accepted ones.
///
/// ```compile_fail
/// use corvis::websocket_gateway;
///
/// #[websocket_gateway(namspace = "/chat")]
/// pub struct Chat;
/// ```
///
/// Spelled right it compiles.
///
/// ```
/// use corvis::websocket_gateway;
///
/// #[websocket_gateway(namespace = "/chat")]
/// pub struct Chat;
/// ```
pub struct GatewayUnknownOption;

/// A namespace with a comma, which no packet header could address, is a compile error.
///
/// ```compile_fail
/// use corvis::websocket_gateway;
///
/// #[websocket_gateway(namespace = "/a,b")]
/// pub struct Chat;
/// ```
///
/// Without the comma it compiles.
///
/// ```
/// use corvis::websocket_gateway;
///
/// #[websocket_gateway(namespace = "/a-b")]
/// pub struct Chat;
/// ```
pub struct GatewayInvalidNamespace;

/// `cors` takes `true`, `false`, an origin or an array of origins.
///
/// ```compile_fail
/// use corvis::websocket_gateway;
///
/// #[websocket_gateway(path = "/rt", cors = 3)]
/// pub struct Chat;
/// ```
///
/// With an accepted form it compiles.
///
/// ```
/// use corvis::websocket_gateway;
///
/// #[websocket_gateway(path = "/rt", cors = ["https://a.example", "https://b.example"])]
/// pub struct Chat;
/// ```
pub struct GatewayInvalidCors;

/// A `#[connected_socket]` of a type that is no gateway socket is a compile error, not a runtime 500.
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
///     fn who(&self, #[connected_socket] client: String) {}
/// }
/// ```
///
/// With the socket of the `ws` adapter it compiles.
///
/// ```
/// use corvis::WsClient;
/// use corvis::prelude::*;
///
/// #[injectable]
/// pub struct Gateway;
///
/// #[handlers]
/// impl Gateway {
///     #[subscribe_message("who")]
///     fn who(&self, #[connected_socket] client: WsClient) {}
/// }
/// ```
pub struct GatewaySocketOfAnotherType;

/// A `#[websocket_server]` field of a type that is no gateway server is a compile error.
///
/// ```compile_fail
/// use corvis::websocket_gateway;
///
/// #[websocket_gateway("/chat")]
/// pub struct Chat {
///     #[websocket_server]
///     server: String,
/// }
/// ```
///
/// With `WsServer` it compiles.
///
/// ```
/// use corvis::{WsServer, websocket_gateway};
///
/// #[websocket_gateway("/chat")]
/// pub struct Chat {
///     #[websocket_server]
///     server: WsServer,
/// }
/// ```
pub struct GatewayServerOfAnotherType;
