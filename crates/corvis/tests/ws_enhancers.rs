//! Global enhancers and exception filters on WebSocket gateways.
//!
//! Its own test binary: the logger is process-wide, so every test shares one capturing sink and the
//! tests are serialized. `replays_the_original_framework_fixture` replays
//! `tests/fixtures/ws_gateways/ws_gateways.json`, recorded by running the original framework.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use corvis::prelude::*;
use corvis::testing::request;
use corvis::{
    CacheInterceptor, CacheModule, CacheOptions, ContextType, LogLevel, LoggerService, LoggingInterceptor, WsClient, WsException,
    WsResponse, WsServer, websocket_gateway,
};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

const IS_PUBLIC: &str = "isPublic";

static CALLS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static LOGS: Mutex<Vec<(LogLevel, String, Option<String>)>> = Mutex::new(Vec::new());
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn record(name: &str) {
    CALLS.lock().unwrap().push(name.to_string());
}

fn calls() -> Vec<String> {
    CALLS.lock().unwrap().clone()
}

struct Capture;

impl LoggerService for Capture {
    fn log(&self, level: LogLevel, message: &str, context: Option<&str>) {
        LOGS.lock().unwrap().push((level, message.to_string(), context.map(str::to_string)));
    }
}

fn exception_logs() -> Vec<String> {
    LOGS.lock()
        .unwrap()
        .iter()
        .filter(|(level, _, ctx)| *level == LogLevel::Error && ctx.as_deref() == Some("WsExceptionsHandler"))
        .map(|(_, m, _)| m.clone())
        .collect()
}

fn warnings() -> Vec<String> {
    LOGS.lock().unwrap().iter().filter(|(level, _, _)| *level == LogLevel::Warn).map(|(_, m, _)| m.clone()).collect()
}

// ---------------------------------------------------------------------------
// Guards, pipes, interceptors and filters used by the gateways.
// ---------------------------------------------------------------------------

macro_rules! recording_guard {
    ($name:ident, $label:literal, $allow:expr) => {
        #[injectable]
        pub struct $name;

        #[async_trait]
        impl CanActivate for $name {
            async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
                record($label);
                let allow: fn(&ExecutionContext) -> bool = $allow;
                Ok(allow(ctx))
            }
        }
    };
}

recording_guard!(GuardA, "A", |_| true);
recording_guard!(GuardB, "B", |_| true);
recording_guard!(Deny, "Deny", |_| false);
recording_guard!(WsOnly, "WsOnly", |ctx| ctx.get_type() == ContextType::Ws);

/// Lets the targets marked with `IS_PUBLIC` metadata through and refuses everything else.
#[injectable]
pub struct PublicOnly {
    reflector: Arc<Reflector>,
}

#[async_trait]
impl CanActivate for PublicOnly {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        record("PublicOnly");
        Ok(self.reflector.get_all_and_override::<bool>(IS_PUBLIC, &[ctx.get_handler(), ctx.get_class()]).unwrap_or(false))
    }
}

#[injectable]
pub struct TrimPipe;

#[async_trait]
impl PipeTransform for TrimPipe {
    async fn transform(&self, value: Value, _meta: &corvis::ArgumentMetadata) -> Result<Value, HttpException> {
        record("TrimPipe");
        Ok(value.as_str().map(|s| Value::String(s.trim().to_string())).unwrap_or(value))
    }
}

#[injectable]
pub struct WrapInterceptor;

#[async_trait]
impl CorvisInterceptor for WrapInterceptor {
    async fn intercept(&self, _ctx: &ExecutionContext, next: CallHandler) -> HandlerResult {
        record("WrapInterceptor");
        // A `WsResponse` reaches interceptors as `{ event, data }`: wrap the data, keep the event.
        Ok(next.handle().await?.map_data(|d| match d.get("event") {
            Some(event) if event.is_string() => json!({ "event": event, "data": { "wrapped": d["data"] } }),
            _ => json!({ "wrapped": d }),
        }))
    }
}

macro_rules! named_filter {
    ($name:ident, $label:literal) => {
        #[injectable]
        pub struct $name;

        #[async_trait]
        impl ExceptionFilter for $name {
            async fn catch(&self, _exception: HttpException, _host: &ArgumentsHost) -> Response {
                record($label);
                Response::json(json!({ "by": $label }))
            }
        }
    };
}

named_filter!(GatewayFilter, "GatewayFilter");
named_filter!(MethodFilter, "MethodFilter");
named_filter!(First, "First");
named_filter!(Second, "Second");
named_filter!(FltA, "A");
named_filter!(FltC, "C");

/// Catches nothing and records nothing, like the fixture's `B` that only catches a `RangeError`.
#[injectable]
pub struct FltB;

#[async_trait]
impl ExceptionFilter for FltB {
    fn catches(&self, _exception: &HttpException) -> bool {
        false
    }
    async fn catch(&self, _exception: HttpException, _host: &ArgumentsHost) -> Response {
        unreachable!("catches returns false")
    }
}
named_filter!(GlobalFilter, "GlobalFilter");
named_filter!(ProviderFilter, "ProviderFilter");

#[injectable]
pub struct NoCatch;

#[async_trait]
impl ExceptionFilter for NoCatch {
    fn catches(&self, _exception: &HttpException) -> bool {
        record("NoCatch");
        false
    }
    async fn catch(&self, _exception: HttpException, _host: &ArgumentsHost) -> Response {
        unreachable!("catches returns false")
    }
}

#[injectable]
pub struct PanicCatches;

#[async_trait]
impl ExceptionFilter for PanicCatches {
    fn catches(&self, _exception: &HttpException) -> bool {
        panic!("catches broke");
    }
    async fn catch(&self, _exception: HttpException, _host: &ArgumentsHost) -> Response {
        unreachable!("catches panics")
    }
}

#[injectable]
pub struct TextFilter;

#[async_trait]
impl ExceptionFilter for TextFilter {
    async fn catch(&self, _exception: HttpException, _host: &ArgumentsHost) -> Response {
        Response::text("plain words").with_status(418)
    }
}

/// Answers through the client and returns an empty response.
#[injectable]
pub struct EmitFilter;

#[async_trait]
impl ExceptionFilter for EmitFilter {
    async fn catch(&self, _exception: HttpException, host: &ArgumentsHost) -> Response {
        let client = host.switch_to_ws().get_client::<WsClient>().expect("ws client");
        client.emit("error", json!({ "own": true }));
        Response::new()
    }
}

#[injectable]
pub struct StreamFilter;

#[async_trait]
impl ExceptionFilter for StreamFilter {
    async fn catch(&self, _exception: HttpException, _host: &ArgumentsHost) -> Response {
        Response::stream(Box::pin(futures::stream::empty()), "text/plain")
    }
}

#[injectable]
pub struct BytesFilter;

#[async_trait]
impl ExceptionFilter for BytesFilter {
    async fn catch(&self, _exception: HttpException, _host: &ArgumentsHost) -> Response {
        Response::bytes(vec![1u8, 2, 3], "application/octet-stream")
    }
}

#[injectable]
pub struct PanicFilter;

#[async_trait]
impl ExceptionFilter for PanicFilter {
    async fn catch(&self, _exception: HttpException, _host: &ArgumentsHost) -> Response {
        panic!("filter broke");
    }
}

/// Reports what the host gives a filter.
#[injectable]
pub struct HostFilter;

#[async_trait]
impl ExceptionFilter for HostFilter {
    async fn catch(&self, exception: HttpException, host: &ArgumentsHost) -> Response {
        Response::json(json!({
            "ws": host.get_type() == ContextType::Ws,
            "data": host.switch_to_ws().get_data(),
            "message": exception.message(),
        }))
    }
}

// ---------------------------------------------------------------------------
// Gateways. One path each, so the scenarios of the tests can share a module.
// ---------------------------------------------------------------------------

#[websocket_gateway(path = "/plain")]
pub struct PlainGateway {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl PlainGateway {
    #[subscribe_message("join")]
    fn join(&self, #[message_body] text: String) -> WsResponse {
        record("handler");
        WsResponse::new("joined", text)
    }

    #[subscribe_message("fail")]
    fn fail(&self, #[message_body] kind: String) -> HttpResult<()> {
        record("handler");
        match kind.as_str() {
            "ws-string" => Err(WsException::new("boom")),
            "ws-object" => Err(WsException::object(json!({ "code": "ROOM_FULL" }))),
            "panic" => panic!("handler panicked"),
            _ => Err(HttpException::from_anyhow(corvis::anyhow::anyhow!("internal"))),
        }
    }
}

#[websocket_gateway(path = "/guarded")]
#[use_guards(GuardB)]
pub struct GuardedGateway {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl GuardedGateway {
    #[subscribe_message("join")]
    fn join(&self, #[message_body] text: String) -> WsResponse {
        record("handler");
        WsResponse::new("joined", text)
    }

    #[subscribe_message("fail")]
    fn fail(&self, #[message_body] kind: String) -> HttpResult<()> {
        record("handler");
        match kind.as_str() {
            "ws-string" => Err(WsException::new("boom")),
            "ws-object" => Err(WsException::object(json!({ "code": "ROOM_FULL" }))),
            "panic" => panic!("handler panicked"),
            _ => Err(HttpException::from_anyhow(corvis::anyhow::anyhow!("internal"))),
        }
    }
}

#[websocket_gateway(path = "/public")]
#[set_metadata(IS_PUBLIC, true)]
pub struct PublicGateway {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl PublicGateway {
    #[subscribe_message("join")]
    fn join(&self, #[message_body] text: String) -> WsResponse {
        record("handler");
        WsResponse::new("joined", text)
    }

    #[subscribe_message("fail")]
    fn fail(&self, #[message_body] kind: String) -> HttpResult<()> {
        record("handler");
        match kind.as_str() {
            "ws-string" => Err(WsException::new("boom")),
            "ws-object" => Err(WsException::object(json!({ "code": "ROOM_FULL" }))),
            "panic" => panic!("handler panicked"),
            _ => Err(HttpException::from_anyhow(corvis::anyhow::anyhow!("internal"))),
        }
    }
}

#[websocket_gateway(path = "/filtered")]
#[use_filters(GatewayFilter)]
pub struct FilteredGateway {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl FilteredGateway {
    #[subscribe_message("join")]
    fn join(&self, #[message_body] text: String) -> WsResponse {
        record("handler");
        WsResponse::new("joined", text)
    }

    #[subscribe_message("fail")]
    fn fail(&self, #[message_body] kind: String) -> HttpResult<()> {
        record("handler");
        match kind.as_str() {
            "ws-string" => Err(WsException::new("boom")),
            "ws-object" => Err(WsException::object(json!({ "code": "ROOM_FULL" }))),
            "panic" => panic!("handler panicked"),
            _ => Err(HttpException::from_anyhow(corvis::anyhow::anyhow!("internal"))),
        }
    }

    #[subscribe_message("own")]
    #[use_filters(MethodFilter)]
    fn own(&self) -> HttpResult<()> {
        record("handler");
        Err(WsException::new("boom"))
    }

    #[subscribe_message("refused")]
    #[use_guards(Deny)]
    fn refused(&self) -> HttpResult<()> {
        record("handler");
        Ok(())
    }

    #[subscribe_message("order")]
    #[use_filters(First, Second)]
    fn order(&self) -> HttpResult<()> {
        record("handler");
        Err(WsException::new("boom"))
    }

    #[subscribe_message("skipped")]
    #[use_filters(NoCatch)]
    fn skipped(&self) -> HttpResult<()> {
        record("handler");
        Err(WsException::new("boom"))
    }

    #[subscribe_message("host")]
    #[use_filters(HostFilter)]
    fn host(&self, #[message_body] _text: String) -> HttpResult<()> {
        Err(WsException::new("boom"))
    }
}

/// No gateway-level filter: the answer depends on the handler's filters alone.
#[websocket_gateway(path = "/shapes")]
pub struct ShapesGateway {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl ShapesGateway {
    #[subscribe_message("text")]
    #[use_filters(TextFilter)]
    fn text(&self) -> HttpResult<()> {
        Err(WsException::new("boom"))
    }

    #[subscribe_message("empty")]
    #[use_filters(EmitFilter)]
    fn empty(&self) -> HttpResult<()> {
        Err(WsException::new("boom"))
    }

    #[subscribe_message("stream")]
    #[use_filters(StreamFilter)]
    fn stream(&self) -> HttpResult<()> {
        Err(WsException::new("boom"))
    }

    #[subscribe_message("bytes")]
    #[use_filters(BytesFilter)]
    fn bytes(&self) -> HttpResult<()> {
        Err(WsException::new("boom"))
    }

    #[subscribe_message("filter_panics")]
    #[use_filters(PanicFilter)]
    fn filter_panics(&self) -> HttpResult<()> {
        Err(WsException::new("boom"))
    }

    #[subscribe_message("panic")]
    #[use_filters(MethodFilter)]
    fn panics(&self) {
        panic!("handler panicked");
    }

    #[subscribe_message("internal")]
    #[use_filters(MethodFilter)]
    fn internal(&self) -> HttpResult<()> {
        Err(HttpException::from_anyhow(corvis::anyhow::anyhow!("database is on fire")))
    }

    #[subscribe_message("catches_panics")]
    #[use_filters(PanicCatches)]
    fn catches_panics(&self) -> HttpResult<()> {
        Err(WsException::new("boom"))
    }

    #[subscribe_message("internal_builtin")]
    fn internal_builtin(&self) -> HttpResult<()> {
        Err(HttpException::from_anyhow(corvis::anyhow::anyhow!("database is on fire")))
    }
}

/// Replies depend on who asks (`who`) or are constant (`ping`); both would collide if cached by the
/// handshake URL.
#[websocket_gateway(path = "/cached")]
pub struct CachedGateway {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl CachedGateway {
    #[subscribe_message("who")]
    fn who(&self, #[connected_socket] client: WsClient) -> String {
        record("who-handler");
        client.handshake().query_param("user").unwrap_or_default().to_string()
    }

    #[subscribe_message("ping")]
    fn ping(&self) -> &'static str {
        record("ping-handler");
        "pong"
    }
}

#[controller("h")]
pub struct HealthController;

#[routes]
impl HealthController {
    #[get]
    fn get(&self) -> Value {
        json!({ "ok": true })
    }
}

#[module(
    controllers = [HealthController],
    providers = [PlainGateway, GuardedGateway, PublicGateway, FilteredGateway, ShapesGateway, CachedGateway]
)]
pub struct AppModule;

#[module(imports = [AppModule], providers = [Provider::app_guard::<Deny>()])]
pub struct ProviderDenyModule;

#[module(
    imports = [AppModule, CacheModule::register(CacheOptions::default())],
    providers = [Provider::app_interceptor::<CacheInterceptor>()]
)]
pub struct ProviderCacheModule;

#[module(imports = [AppModule], providers = [Provider::app_guard::<PublicOnly>()])]
pub struct ProviderPublicModule;

#[module(imports = [AppModule], providers = [Provider::app_filter::<ProviderFilter>()])]
pub struct ProviderFilterModule;

#[module(imports = [AppModule], providers = [Provider::app_guard::<GuardA>(), Provider::app_guard::<GuardB>()])]
pub struct ProviderGuardsModule;

// ---------------------------------------------------------------------------
// Helpers.
// ---------------------------------------------------------------------------

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn start<M: corvis::Module>(setup: impl FnOnce(&mut CorvisApplication)) -> (CorvisApplication, std::net::SocketAddr) {
    CALLS.lock().unwrap().clear();
    LOGS.lock().unwrap().clear();
    let mut app = CorvisFactory::create::<M>().await.unwrap();
    app.use_logger(Capture);
    app.silence_route_logs();
    setup(&mut app);
    app.init().await.unwrap();
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    (app, addr)
}

async fn connect(addr: std::net::SocketAddr, path: &str) -> Ws {
    tokio_tungstenite::connect_async(format!("ws://{addr}{path}")).await.unwrap().0
}

async fn next_frame(ws: &mut Ws) -> Option<Value> {
    loop {
        match tokio::time::timeout(Duration::from_millis(600), ws.next()).await {
            Err(_) => return None,
            Ok(Some(Ok(Message::Text(t)))) => return Some(serde_json::from_str(&t).unwrap()),
            Ok(Some(Ok(_))) => continue,
            Ok(_) => return None,
        }
    }
}

/// Sends one frame and returns the next one (`None` when nothing arrives).
async fn send(ws: &mut Ws, frame: Value) -> Option<Value> {
    ws.send(Message::Text(frame.to_string())).await.unwrap();
    next_frame(ws).await
}

async fn ask(addr: std::net::SocketAddr, path: &str, frame: Value) -> Value {
    let mut ws = connect(addr, path).await;
    send(&mut ws, frame).await.expect("a frame")
}

fn forbidden() -> Value {
    json!({ "event": "exception", "data": { "status": "error", "message": "Forbidden resource" } })
}

fn built_in(message: &str) -> Value {
    json!({ "event": "exception", "data": { "status": "error", "message": message } })
}

// ---------------------------------------------------------------------------
// Global enhancers.
// ---------------------------------------------------------------------------

#[corvis::test]
async fn use_global_guards_refuse_gateway_messages() {
    let _s = SERIAL.lock().await;
    let (_app, addr) = start::<AppModule>(|app| {
        app.use_global_guards(Deny);
    })
    .await;
    assert_eq!(ask(addr, "/plain", json!({ "event": "join", "data": "lobby" })).await, forbidden());
    assert_eq!(calls(), ["Deny"]);
}

#[corvis::test]
async fn app_guard_providers_refuse_gateway_messages() {
    let _s = SERIAL.lock().await;
    let (app, addr) = start::<ProviderDenyModule>(|_| {}).await;
    assert_eq!(ask(addr, "/plain", json!({ "event": "join", "data": "lobby" })).await, forbidden());
    assert_eq!(calls(), ["Deny"]);
    // The same guard answers HTTP routes with 403.
    request(&app).get("/h").send().await.expect_status(403);
}

#[corvis::test]
async fn global_guards_run_before_the_gateway_guards() {
    let _s = SERIAL.lock().await;
    let (_app, addr) = start::<AppModule>(|app| {
        app.use_global_guards(GuardA);
    })
    .await;
    let reply = ask(addr, "/guarded", json!({ "event": "join", "data": "x" })).await;
    assert_eq!(reply, json!({ "event": "joined", "data": "x" }));
    assert_eq!(calls(), ["A", "B", "handler"]);
}

#[corvis::test]
async fn provider_guards_keep_registration_order_ahead_of_use_global_guards() {
    let _s = SERIAL.lock().await;
    let (_app, addr) = start::<ProviderGuardsModule>(|app| {
        app.use_global_guards(WsOnly);
    })
    .await;
    ask(addr, "/plain", json!({ "event": "join", "data": "x" })).await;
    assert_eq!(calls(), ["A", "B", "WsOnly", "handler"]);
}

#[corvis::test]
async fn a_guard_can_tell_the_transports_apart() {
    let _s = SERIAL.lock().await;
    let (app, addr) = start::<AppModule>(|app| {
        app.use_global_guards(WsOnly);
    })
    .await;
    assert_eq!(ask(addr, "/plain", json!({ "event": "join", "data": "x" })).await, json!({ "event": "joined", "data": "x" }));
    request(&app).get("/h").send().await.expect_status(403);
}

#[corvis::test]
async fn gateway_metadata_marks_it_public() {
    let _s = SERIAL.lock().await;
    let (_app, addr) = start::<ProviderPublicModule>(|_| {}).await;
    assert_eq!(ask(addr, "/public", json!({ "event": "join", "data": "x" })).await, json!({ "event": "joined", "data": "x" }));
    assert_eq!(ask(addr, "/plain", json!({ "event": "join", "data": "x" })).await, forbidden());
}

#[corvis::test]
async fn global_pipes_and_interceptors_apply_to_message_bodies() {
    let _s = SERIAL.lock().await;
    let (_app, addr) = start::<AppModule>(|app| {
        app.use_global_pipes(TrimPipe);
        app.use_global_interceptors(WrapInterceptor);
    })
    .await;
    let reply = ask(addr, "/plain", json!({ "event": "join", "data": "  lobby  " })).await;
    assert_eq!(reply, json!({ "event": "joined", "data": { "wrapped": "lobby" } }));
    assert_eq!(calls(), ["WrapInterceptor", "TrimPipe", "handler"]);
}

#[corvis::test]
async fn global_filters_do_not_apply_to_gateways() {
    let _s = SERIAL.lock().await;
    let (_app, addr) = start::<ProviderFilterModule>(|app| {
        app.use_global_filters(GlobalFilter);
    })
    .await;
    assert_eq!(ask(addr, "/plain", json!({ "event": "fail", "data": "ws-string" })).await, built_in("boom"));
    assert_eq!(calls(), ["handler"]);
}

#[corvis::test]
async fn the_switch_keeps_global_enhancers_http_only() {
    let _s = SERIAL.lock().await;
    let (app, addr) = start::<ProviderDenyModule>(|app| {
        app.apply_global_enhancers_to_gateways(false);
    })
    .await;
    assert_eq!(ask(addr, "/plain", json!({ "event": "join", "data": "x" })).await, json!({ "event": "joined", "data": "x" }));
    assert_eq!(calls(), ["handler"]);
    request(&app).get("/h").send().await.expect_status(403);
}

#[corvis::test]
async fn the_switch_called_after_init_warns_and_changes_nothing() {
    let _s = SERIAL.lock().await;
    let (mut app, addr) = start::<AppModule>(|app| {
        app.use_global_guards(Deny);
    })
    .await;
    assert!(warnings().is_empty());
    app.apply_global_enhancers_to_gateways(false);
    let warned = warnings();
    assert_eq!(warned.len(), 1, "{warned:?}");
    assert!(warned[0].contains("no effect"), "{warned:?}");
    assert_eq!(ask(addr, "/plain", json!({ "event": "join", "data": "x" })).await, forbidden());
}

// ---------------------------------------------------------------------------
// Gateway and handler filters.
// ---------------------------------------------------------------------------

#[corvis::test]
async fn gateway_filter_answers_for_handlers_without_filters() {
    let _s = SERIAL.lock().await;
    let (_app, addr) = start::<AppModule>(|_| {}).await;
    let reply = ask(addr, "/filtered", json!({ "event": "fail", "data": "ws-string" })).await;
    assert_eq!(reply, json!({ "event": "exception", "data": { "by": "GatewayFilter" } }));
}

#[corvis::test]
async fn handler_filter_wins_over_gateway_filter() {
    let _s = SERIAL.lock().await;
    let (_app, addr) = start::<AppModule>(|_| {}).await;
    let reply = ask(addr, "/filtered", json!({ "event": "own" })).await;
    assert_eq!(reply, json!({ "event": "exception", "data": { "by": "MethodFilter" } }));
    assert_eq!(calls(), ["handler", "MethodFilter"]);
}

#[corvis::test]
async fn filters_of_one_attribute_are_tried_last_listed_first() {
    let _s = SERIAL.lock().await;
    let (_app, addr) = start::<AppModule>(|_| {}).await;
    let reply = ask(addr, "/filtered", json!({ "event": "order" })).await;
    assert_eq!(reply["data"], json!({ "by": "Second" }));
    assert_eq!(calls(), ["handler", "Second"]);
}

#[corvis::test]
async fn filters_of_one_gateway_attribute_are_tried_last_listed_first() {
    let _s = SERIAL.lock().await;
    let (_app, addr) = start::<FxModule>(|_| {}).await;
    let reply = ask(addr, "/fx_gateway_order", json!({ "event": "failFiltered" })).await;
    assert_eq!(reply["data"], json!({ "by": "Second" }));
    assert_eq!(calls(), ["handler", "Second"]);
}

#[corvis::test]
async fn stacked_handler_attributes_keep_the_upper_one_first() {
    let _s = SERIAL.lock().await;
    let (_app, addr) = start::<FxModule>(|_| {}).await;
    let reply = ask(addr, "/fx_stacked", json!({ "event": "failFiltered" })).await;
    assert_eq!(reply["data"], json!({ "by": "A" }));
    assert_eq!(calls(), ["handler", "A"]);
}

#[corvis::test]
async fn guard_refusal_goes_through_the_filters() {
    let _s = SERIAL.lock().await;
    let (_app, addr) = start::<AppModule>(|_| {}).await;
    let reply = ask(addr, "/filtered", json!({ "event": "refused" })).await;
    assert_eq!(reply, json!({ "event": "exception", "data": { "by": "GatewayFilter" } }));
    assert_eq!(calls(), ["Deny", "GatewayFilter"]);
}

#[corvis::test]
async fn a_filter_that_does_not_catch_leaves_the_built_in_frame() {
    let _s = SERIAL.lock().await;
    let (_app, addr) = start::<AppModule>(|_| {}).await;
    // The handler's filter declines, so the gateway's filter takes over.
    let reply = ask(addr, "/filtered", json!({ "event": "skipped" })).await;
    assert_eq!(reply["data"], json!({ "by": "GatewayFilter" }));
    assert_eq!(calls(), ["handler", "NoCatch", "GatewayFilter"]);
    // No filter at all: the built-in mapping.
    assert_eq!(ask(addr, "/shapes", json!({ "event": "internal_builtin" })).await, built_in("Internal server error"));
}

#[corvis::test]
async fn filter_bodies_become_the_frame_data() {
    let _s = SERIAL.lock().await;
    let (_app, addr) = start::<AppModule>(|_| {}).await;
    let mut ws = connect(addr, "/shapes").await;
    // Text body: a JSON string, status ignored; the incoming `id` is echoed.
    let reply = send(&mut ws, json!({ "event": "text", "id": "r1" })).await.unwrap();
    assert_eq!(reply, json!({ "event": "exception", "data": "plain words", "id": "r1" }));
    let reply = send(&mut ws, json!({ "event": "internal", "id": 7 })).await.unwrap();
    assert_eq!(reply, json!({ "event": "exception", "data": { "by": "MethodFilter" }, "id": 7 }));
}

#[corvis::test]
async fn an_empty_filter_response_sends_no_exception_frame() {
    let _s = SERIAL.lock().await;
    let (_app, addr) = start::<AppModule>(|_| {}).await;
    let mut ws = connect(addr, "/shapes").await;
    let own = send(&mut ws, json!({ "event": "empty" })).await.unwrap();
    assert_eq!(own, json!({ "event": "error", "data": { "own": true } }));
    assert_eq!(next_frame(&mut ws).await, None);
}

#[corvis::test]
async fn unusable_filter_responses_fall_back_to_the_built_in_frame() {
    let _s = SERIAL.lock().await;
    let (_app, addr) = start::<AppModule>(|_| {}).await;
    let reply = ask(addr, "/shapes", json!({ "event": "stream" })).await;
    assert_eq!(reply, built_in("boom"));
    let logs = exception_logs();
    assert_eq!(logs.len(), 1, "{logs:?}");
    assert!(logs[0].contains("StreamFilter"), "{logs:?}");

    LOGS.lock().unwrap().clear();
    assert_eq!(ask(addr, "/shapes", json!({ "event": "bytes" })).await, built_in("boom"));
    assert!(exception_logs()[0].contains("BytesFilter"));

    LOGS.lock().unwrap().clear();
    assert_eq!(ask(addr, "/shapes", json!({ "event": "filter_panics" })).await, built_in("Internal server error"));
    let logs = exception_logs();
    assert!(logs.iter().any(|l| l.contains("PanicFilter") && l.contains("panicked")), "{logs:?}");
}

#[corvis::test]
async fn a_handler_panic_reaches_the_filters() {
    let _s = SERIAL.lock().await;
    let (_app, addr) = start::<AppModule>(|_| {}).await;
    let reply = ask(addr, "/shapes", json!({ "event": "panic" })).await;
    assert_eq!(reply, json!({ "event": "exception", "data": { "by": "MethodFilter" } }));
    assert!(exception_logs().is_empty());
}

#[corvis::test]
async fn the_filter_host_is_the_message_context() {
    let _s = SERIAL.lock().await;
    let (_app, addr) = start::<AppModule>(|_| {}).await;
    let reply = ask(addr, "/filtered", json!({ "event": "host", "data": "lobby" })).await;
    assert_eq!(reply["data"], json!({ "ws": true, "data": "lobby", "message": "boom" }));
}

#[corvis::test]
async fn internal_errors_are_logged_only_when_the_built_in_mapping_answers() {
    let _s = SERIAL.lock().await;
    let (_app, addr) = start::<AppModule>(|_| {}).await;
    // Caught by a filter: nothing logged.
    let reply = ask(addr, "/shapes", json!({ "event": "internal" })).await;
    assert_eq!(reply["data"], json!({ "by": "MethodFilter" }));
    assert!(exception_logs().is_empty(), "{:?}", exception_logs());
    // Built-in mapping: one error line.
    assert_eq!(ask(addr, "/shapes", json!({ "event": "internal_builtin" })).await, built_in("Internal server error"));
    let logs = exception_logs();
    assert_eq!(logs.len(), 1, "{logs:?}");
    assert!(logs[0].contains("database is on fire"));
}

// ---------------------------------------------------------------------------
// Differential replay of the original framework (`tests/fixtures/ws_gateways`).
// ---------------------------------------------------------------------------

#[websocket_gateway(path = "/fx")]
pub struct FxPlain {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl FxPlain {
    #[subscribe_message("join")]
    fn join(&self, #[message_body] text: String) -> WsResponse {
        record("handler");
        WsResponse::new("joined", text)
    }

    #[subscribe_message("fail")]
    fn fail(&self, #[message_body] kind: String) -> HttpResult<()> {
        record("handler");
        match kind.as_str() {
            "ws-string" => Err(WsException::new("boom")),
            "ws-object" => Err(WsException::object(json!({ "code": "ROOM_FULL" }))),
            "panic" => panic!("handler panicked"),
            _ => Err(HttpException::from_anyhow(corvis::anyhow::anyhow!("internal"))),
        }
    }
}

#[websocket_gateway(path = "/fx_gg")]
#[use_guards(GuardB)]
pub struct FxGatewayGuard {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl FxGatewayGuard {
    #[subscribe_message("join")]
    fn join(&self, #[message_body] text: String) -> WsResponse {
        record("handler");
        WsResponse::new("joined", text)
    }

    #[subscribe_message("fail")]
    fn fail(&self, #[message_body] kind: String) -> HttpResult<()> {
        record("handler");
        match kind.as_str() {
            "ws-string" => Err(WsException::new("boom")),
            "ws-object" => Err(WsException::object(json!({ "code": "ROOM_FULL" }))),
            "panic" => panic!("handler panicked"),
            _ => Err(HttpException::from_anyhow(corvis::anyhow::anyhow!("internal"))),
        }
    }
}

#[websocket_gateway(path = "/fx_filters")]
#[use_filters(GatewayFilter)]
pub struct FxFilters {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl FxFilters {
    #[subscribe_message("failFiltered")]
    #[use_filters(MethodFilter)]
    fn fail_filtered(&self) -> HttpResult<()> {
        record("handler");
        Err(WsException::new("boom"))
    }

    #[subscribe_message("join")]
    #[use_guards(Deny)]
    fn join(&self) -> HttpResult<()> {
        record("handler");
        Ok(())
    }
}

#[websocket_gateway(path = "/fx_order")]
pub struct FxOrder {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl FxOrder {
    #[subscribe_message("failFiltered")]
    #[use_filters(First, Second)]
    fn fail_filtered(&self) -> HttpResult<()> {
        record("handler");
        Err(WsException::new("boom"))
    }
}

#[websocket_gateway(path = "/fx_gateway_order")]
#[use_filters(First, Second)]
pub struct FxGatewayOrder {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl FxGatewayOrder {
    #[subscribe_message("failFiltered")]
    fn fail_filtered(&self) -> HttpResult<()> {
        record("handler");
        Err(WsException::new("boom"))
    }
}

#[websocket_gateway(path = "/fx_stacked")]
pub struct FxStacked {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl FxStacked {
    #[subscribe_message("failFiltered")]
    #[use_filters(FltA, FltB)]
    #[use_filters(FltC)]
    fn fail_filtered(&self) -> HttpResult<()> {
        record("handler");
        Err(WsException::new("boom"))
    }
}

#[module(providers = [FxPlain, FxGatewayGuard, FxFilters, FxOrder, FxGatewayOrder, FxStacked])]
pub struct FxModule;

#[module(imports = [FxModule], providers = [Provider::app_guard::<Deny>()])]
pub struct FxProviderGuard;

#[module(imports = [FxModule], providers = [Provider::app_pipe::<TrimPipe>(), Provider::app_interceptor::<WrapInterceptor>()])]
pub struct FxProviderPipe;

#[module(imports = [FxModule], providers = [Provider::app_filter::<ProviderFilter>()])]
pub struct FxProviderFilter;

#[corvis::test]
async fn replays_the_original_framework_fixture() {
    let _s = SERIAL.lock().await;
    let raw = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/ws_gateways/ws_gateways.json")).unwrap();
    let fixture: Value = serde_json::from_str(&raw).unwrap();
    let scenarios = fixture.as_object().unwrap();
    assert_eq!(scenarios.len(), 14);

    for (name, runs) in scenarios {
        let path = match name.as_str() {
            "global_guard_before_gateway_guard" => "/fx_gg",
            "method_filter_wins" | "gateway_filter_catches_guard_refusal" => "/fx_filters",
            "filters_in_one_use_filters" => "/fx_order",
            "filters_in_one_gateway_use_filters" => "/fx_gateway_order",
            "stacked_handler_use_filters" => "/fx_stacked",
            _ => "/fx",
        };
        let (_app, addr) = match name.as_str() {
            "global_guard_use_global" => {
                start::<FxModule>(|a| {
                    a.use_global_guards(Deny);
                })
                .await
            }
            "global_guard_provider" => start::<FxProviderGuard>(|_| {}).await,
            "global_guard_before_gateway_guard" => {
                start::<FxModule>(|a| {
                    a.use_global_guards(GuardA);
                })
                .await
            }
            "global_guard_ws_only" => {
                start::<FxModule>(|a| {
                    a.use_global_guards(WsOnly);
                })
                .await
            }
            "global_pipe_and_interceptor" => {
                start::<FxModule>(|a| {
                    a.use_global_pipes(TrimPipe);
                    a.use_global_interceptors(WrapInterceptor);
                })
                .await
            }
            "global_pipe_provider" => start::<FxProviderPipe>(|_| {}).await,
            "global_filter_use_global" => {
                start::<FxModule>(|a| {
                    a.use_global_filters(GlobalFilter);
                })
                .await
            }
            "global_filter_provider" => start::<FxProviderFilter>(|_| {}).await,
            _ => start::<FxModule>(|_| {}).await,
        };
        for run in runs.as_array().unwrap() {
            CALLS.lock().unwrap().clear();
            let message = run["message"].clone();
            let event = message["event"].as_str().unwrap();
            let data = message["data"].clone();
            let mut ws = connect(addr, path).await;
            let got = send(&mut ws, json!({ "event": event, "data": data })).await;
            let frames: Vec<Value> = got.into_iter().collect();
            let expected_frames = run["frames"].as_array().unwrap().clone();
            let expected_calls: Vec<String> = run["calls"].as_array().unwrap().iter().map(|c| c.as_str().unwrap().to_string()).collect();
            assert_eq!(frames, expected_frames, "{name}: frames for {message}");
            assert_eq!(calls(), expected_calls, "{name}: calls for {message}");
        }
    }
}

// ---------------------------------------------------------------------------
// Built-in interceptors that read the request now run on gateways.
// ---------------------------------------------------------------------------

#[corvis::test]
async fn the_cache_interceptor_never_caches_gateway_messages() {
    let _s = SERIAL.lock().await;
    let (_app, addr) = start::<ProviderCacheModule>(|_| {}).await;
    let mut a = connect(addr, "/cached?user=a").await;
    let mut b = connect(addr, "/cached?user=b").await;
    assert_eq!(send(&mut a, json!({ "event": "who" })).await.unwrap(), json!({ "event": "who", "data": "a" }));
    // A different event and client must reach their own handler, not A's cached reply.
    assert_eq!(send(&mut b, json!({ "event": "ping" })).await.unwrap(), json!({ "event": "ping", "data": "pong" }));
    assert_eq!(send(&mut b, json!({ "event": "who" })).await.unwrap(), json!({ "event": "who", "data": "b" }));
    assert_eq!(send(&mut a, json!({ "event": "ping" })).await.unwrap(), json!({ "event": "ping", "data": "pong" }));
    assert_eq!(send(&mut a, json!({ "event": "who" })).await.unwrap(), json!({ "event": "who", "data": "a" }));
    assert_eq!(calls(), ["who-handler", "ping-handler", "who-handler", "ping-handler", "who-handler"]);
}

#[corvis::test]
async fn the_logging_interceptor_logs_the_handler_not_the_handshake_url() {
    let _s = SERIAL.lock().await;
    let (_app, addr) = start::<AppModule>(|app| {
        app.use_global_interceptors(LoggingInterceptor);
    })
    .await;
    let mut ws = connect(addr, "/cached?user=a&token=s3cret").await;
    send(&mut ws, json!({ "event": "who" })).await.unwrap();
    let lines: Vec<String> =
        LOGS.lock().unwrap().iter().filter(|(_, _, c)| c.as_deref() == Some("LoggingInterceptor")).map(|(_, m, _)| m.clone()).collect();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].starts_with("WS CachedGateway.who 200 - "), "{lines:?}");
    assert!(!lines[0].contains("s3cret"));
}

#[corvis::test]
async fn a_filter_whose_catches_panics_does_not_catch() {
    let _s = SERIAL.lock().await;
    let (_app, addr) = start::<AppModule>(|_| {}).await;
    let mut ws = connect(addr, "/shapes").await;
    assert_eq!(send(&mut ws, json!({ "event": "catches_panics" })).await.unwrap(), built_in("boom"));
    let logs = exception_logs();
    assert!(logs.iter().any(|l| l.contains("PanicCatches") && l.contains("catches")), "{logs:?}");
    // The connection is still alive.
    assert_eq!(send(&mut ws, json!({ "event": "catches_panics" })).await.unwrap(), built_in("boom"));
}
