//! Route posture: which controller routes are guarded, public or exempt, an opt-in startup check and a report.
//!
//! The check reads the compiled route table, so it sees exactly the guards the router runs for each route.
//! It is off by default; see [`CorvisApplication::require_route_posture`](crate::CorvisApplication::require_route_posture).

use std::fmt;
use std::sync::Arc;

use serde::Serialize;
use serde::ser::{SerializeStruct, Serializer};

use crate::common::context::Target;
use crate::common::guard::CanActivate;
use crate::common::middleware::path_matches;
use crate::common::reflector::Reflector;
use crate::di::token::short_type_name;
use crate::http::RequestMethod;
use crate::router::RouterTable;

/// What the startup check does with unprotected routes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RoutePostureMode {
    /// No check (default). Nothing changes.
    #[default]
    Off,
    /// Log the unprotected routes once at warning level and start.
    Warn,
    /// Fail `init()` with a [`RoutePostureError`] listing the unprotected routes.
    Enforce,
}

/// A route the check accepts without a guard or a public marker, with the reason why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PostureExemption {
    method: RequestMethod,
    pattern: String,
    reason: String,
    /// `Some` for a gateway message exemption: an event name, or `*` for every event of the gateway.
    event: Option<String>,
}

impl PostureExemption {
    /// The HTTP method; [`RequestMethod::All`] matches every method. A message exemption has no method and
    /// returns [`RequestMethod::All`] here; check [`event`](Self::event).
    pub fn method(&self) -> RequestMethod {
        self.method
    }

    /// For a gateway message exemption, the event name (or `*`); `None` for a route exemption.
    pub fn event(&self) -> Option<&str> {
        self.event.as_deref()
    }

    /// The path pattern compared with the route's full path template (or the gateway path).
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// Why the route is open or protected outside the framework.
    pub fn reason(&self) -> &str {
        &self.reason
    }

    fn matches_message(&self, path: &str, event: &str) -> bool {
        self.event.as_deref().is_some_and(|e| e == "*" || e == event) && path_matches(&self.pattern, path)
    }

    fn describe(&self) -> String {
        match &self.event {
            Some(event) => format!("WS {} event \"{}\"", self.pattern, event),
            None => format!("{} {}", self.method.as_str(), self.pattern),
        }
    }

    fn matches(&self, method: RequestMethod, path: &str) -> bool {
        self.event.is_none() && (self.method == RequestMethod::All || self.method == method) && path_matches(&self.pattern, path)
    }
}

impl Serialize for PostureExemption {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("PostureExemption", if self.event.is_some() { 4 } else { 3 })?;
        s.serialize_field("method", if self.event.is_some() { "WS" } else { self.method.as_str() })?;
        s.serialize_field("pattern", &self.pattern)?;
        s.serialize_field("reason", &self.reason)?;
        if let Some(event) = &self.event {
            s.serialize_field("event", event)?;
        }
        s.end()
    }
}

/// Options of the route posture check: a mode and the exemptions.
///
/// ```
/// use corvis::{RequestMethod, RoutePostureOptions};
///
/// let options = RoutePostureOptions::enforce()
///     .exempt(RequestMethod::Get, "/metrics", "scraped from the private network only");
/// ```
#[derive(Clone, Debug, Default)]
pub struct RoutePostureOptions {
    mode: RoutePostureMode,
    exemptions: Vec<PostureExemption>,
}

impl RoutePostureOptions {
    /// No check.
    pub fn off() -> Self {
        Self::default()
    }

    /// Log the unprotected routes and start.
    pub fn warn() -> Self {
        RoutePostureOptions { mode: RoutePostureMode::Warn, exemptions: Vec::new() }
    }

    /// Refuse to start when a route is unprotected.
    pub fn enforce() -> Self {
        RoutePostureOptions { mode: RoutePostureMode::Enforce, exemptions: Vec::new() }
    }

    /// Accepts the routes matching `method` and `pattern` (compared with the full path template, global prefix
    /// and version included, with the same rules as middleware route matching) although they have no guard and
    /// are not public. `reason` is mandatory: a blank one makes `init()` fail.
    pub fn exempt(mut self, method: RequestMethod, pattern: impl Into<String>, reason: impl Into<String>) -> Self {
        self.exemptions.push(PostureExemption { method, pattern: pattern.into(), reason: reason.into(), event: None });
        self
    }

    /// Accepts the message handlers of the gateways whose path matches `pattern` (same rules as [`exempt`](Self::exempt))
    /// for `event`, compared exactly, or for every event of the gateway with `*`, although they have no guard
    /// and are not public. `reason` is mandatory: a blank one makes `init()` fail.
    pub fn exempt_message(mut self, pattern: impl Into<String>, event: impl Into<String>, reason: impl Into<String>) -> Self {
        self.exemptions.push(PostureExemption {
            method: RequestMethod::All,
            pattern: pattern.into(),
            reason: reason.into(),
            event: Some(event.into()),
        });
        self
    }

    /// The configured mode.
    pub fn mode(&self) -> RoutePostureMode {
        self.mode
    }

    /// The configured exemptions, in order.
    pub fn exemptions(&self) -> &[PostureExemption] {
        &self.exemptions
    }

    /// Fails when an exemption has a blank reason, naming its method and pattern.
    pub(crate) fn validate(&self) -> crate::Result<()> {
        for e in &self.exemptions {
            if e.reason.trim().is_empty() {
                anyhow::bail!(
                    "Route posture exemption {} has no reason: say why the route is open or protected outside the framework",
                    e.describe()
                );
            }
        }
        Ok(())
    }
}

/// What a posture entry is: a controller route, an SSE stream or a gateway message handler.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PostureKind {
    /// An HTTP route (`#[get]`, `#[post]`, ...).
    Http { method: RequestMethod },
    /// A route declared with `#[sse]` (a long-lived `GET` stream).
    Sse,
    /// A `#[subscribe_message]` handler of a WebSocket gateway.
    WsMessage { event: String },
}

impl Serialize for PostureKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            PostureKind::Http { method } => {
                let mut s = serializer.serialize_struct("PostureKind", 2)?;
                s.serialize_field("type", "http")?;
                s.serialize_field("method", method.as_str())?;
                s.end()
            }
            PostureKind::Sse => {
                let mut s = serializer.serialize_struct("PostureKind", 1)?;
                s.serialize_field("type", "sse")?;
                s.end()
            }
            PostureKind::WsMessage { event } => {
                let mut s = serializer.serialize_struct("PostureKind", 2)?;
                s.serialize_field("type", "ws_message")?;
                s.serialize_field("event", event)?;
                s.end()
            }
        }
    }
}

impl PostureKind {
    fn label(&self) -> &'static str {
        match self {
            PostureKind::Http { method } => method.as_str(),
            PostureKind::Sse => "SSE",
            PostureKind::WsMessage { .. } => "WS",
        }
    }

    /// `event "name"` for a message handler, empty otherwise.
    fn detail(&self) -> String {
        match self {
            PostureKind::WsMessage { event } => format!("event \"{event}\""),
            _ => String::new(),
        }
    }
}

/// Why a route is accepted, or that it is not.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PostureStatus {
    /// A guard that controls access runs for the route.
    Protected,
    /// The route is marked `#[public]` (or `PUBLIC_METADATA` is `true`).
    Public,
    /// An exemption matches the route.
    Exempt { reason: String },
    /// No guard that controls access, not public and no exemption.
    Unprotected,
}

/// A guard that runs for a route.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GuardInfo {
    /// The guard's name ([`CanActivate::guard_name`](crate::CanActivate::guard_name), without module paths).
    pub name: String,
    /// Whether it counts for posture ([`CanActivate::controls_access`](crate::CanActivate::controls_access)).
    pub controls_access: bool,
}

/// The posture of one compiled route or gateway message handler (each path alias and URI version of a route is its own entry).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct PostureEntry {
    pub kind: PostureKind,
    /// The full path template, global prefix and version included.
    pub path: String,
    /// The host the controller is restricted to, if any.
    pub host: Option<String>,
    /// The controller's name (the gateway's for a message handler).
    pub owner: String,
    /// The handler's name (the method of a message handler).
    pub handler: String,
    /// Every guard the router runs for the route, in order.
    pub guards: Vec<GuardInfo>,
    pub status: PostureStatus,
}

impl PostureEntry {
    /// Builds an entry (used by tests of code that renders reports).
    pub fn new(
        kind: PostureKind,
        path: impl Into<String>,
        owner: impl Into<String>,
        handler: impl Into<String>,
        guards: Vec<GuardInfo>,
        status: PostureStatus,
    ) -> Self {
        PostureEntry { kind, path: path.into(), host: None, owner: owner.into(), handler: handler.into(), guards, status }
    }

    fn sort_key(&self) -> (&str, &'static str, String) {
        (self.path.as_str(), self.kind.label(), format!("{}::{}::{}", self.kind.detail(), self.owner, self.handler))
    }
}

/// The posture of every controller route, sorted by path, then method, then `Controller::handler`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RoutePostureReport {
    entries: Vec<PostureEntry>,
    stale_exemptions: Vec<PostureExemption>,
    unchecked: Vec<UncheckedSurface>,
}

/// A surface the check cannot enforce, listed so that a reviewer sees it exists.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum UncheckedSurface {
    /// A `ServeStaticModule` mount.
    StaticFiles { serve_root: String },
    /// The WebSocket handshake of a gateway: it runs no guard; `verify_client` is the only check.
    WsHandshake { path: String, gateway: String, verify_client: bool },
    /// Raw `upgrade` listeners registered on the HTTP server (`on_upgrade`).
    UpgradeListeners { count: usize },
}

impl fmt::Display for UncheckedSurface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UncheckedSurface::StaticFiles { serve_root } => write!(f, "static files served at {serve_root}"),
            UncheckedSurface::WsHandshake { path, gateway, verify_client } => {
                write!(f, "WebSocket handshake of {gateway} at {path} (verify_client {})", if *verify_client { "set" } else { "not set" })
            }
            UncheckedSurface::UpgradeListeners { count } => write!(f, "{count} raw upgrade listener{}", if *count == 1 { "" } else { "s" }),
        }
    }
}

/// `(event, handler target, guards in the order they run)`.
pub(crate) type GatewayHandlerPosture = (String, Arc<Target>, Vec<Arc<dyn CanActivate>>);

/// A gateway as the posture check sees it: the handlers with the guards that run for each message.
pub(crate) struct GatewayPosture {
    pub path: String,
    pub name: String,
    pub class_target: Arc<Target>,
    pub verify_client: bool,
    /// `(event, handler target, guards in the order they run)`.
    pub handlers: Vec<GatewayHandlerPosture>,
}

impl RoutePostureReport {
    /// Builds a report; the entries are sorted.
    pub fn new(mut entries: Vec<PostureEntry>, stale_exemptions: Vec<PostureExemption>) -> Self {
        entries.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
        RoutePostureReport { entries, stale_exemptions, unchecked: Vec::new() }
    }

    /// Sets the surfaces listed as not checked.
    pub fn with_unchecked(mut self, unchecked: Vec<UncheckedSurface>) -> Self {
        self.unchecked = unchecked;
        self
    }

    /// The surfaces the check cannot enforce (static mounts, WebSocket handshakes, raw upgrade listeners). Filled
    /// by `route_posture()`, which reads live state; empty in the report the startup check builds.
    pub fn unchecked(&self) -> &[UncheckedSurface] {
        &self.unchecked
    }

    /// Every route and message handler.
    pub fn entries(&self) -> &[PostureEntry] {
        &self.entries
    }

    /// The routes with status [`PostureStatus::Unprotected`].
    pub fn unprotected(&self) -> Vec<&PostureEntry> {
        self.entries.iter().filter(|e| e.status == PostureStatus::Unprotected).collect()
    }

    /// The exemptions that match no route or handler.
    pub fn stale_exemptions(&self) -> &[PostureExemption] {
        &self.stale_exemptions
    }
}

fn status_label(status: &PostureStatus) -> String {
    match status {
        PostureStatus::Protected => "protected".to_string(),
        PostureStatus::Public => "public".to_string(),
        PostureStatus::Exempt { reason } => format!("exempt ({reason})"),
        PostureStatus::Unprotected => "UNPROTECTED".to_string(),
    }
}

impl fmt::Display for RoutePostureReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let method_w = self.entries.iter().map(|e| e.kind.label().len()).max().unwrap_or(0);
        let path_w = self.entries.iter().map(|e| e.path.len()).max().unwrap_or(0);
        let detail_w = self.entries.iter().map(|e| e.kind.detail().len()).max().unwrap_or(0);
        let route_w = self.entries.iter().map(|e| e.owner.len() + 2 + e.handler.len()).max().unwrap_or(0);
        for e in &self.entries {
            let route = format!("{}::{}", e.owner, e.handler);
            let guards = if e.guards.is_empty() {
                "no guards".to_string()
            } else {
                e.guards
                    .iter()
                    .map(|g| if g.controls_access { g.name.clone() } else { format!("{} (does not control access)", g.name) })
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let detail = if detail_w == 0 { String::new() } else { format!("{:<detail_w$}  ", e.kind.detail()) };
            writeln!(
                f,
                "{:<method_w$}  {:<path_w$}  {detail}{:<route_w$}  {}  [{}]",
                e.kind.label(),
                e.path,
                route,
                status_label(&e.status),
                guards
            )?;
        }
        if !self.unchecked.is_empty() {
            writeln!(f)?;
            writeln!(f, "Not checked:")?;
            for u in &self.unchecked {
                writeln!(f, "  {u}")?;
            }
        }
        Ok(())
    }
}

/// Returned by `init()` (and `listen()`, `listen_in_background()`) in enforce mode when a route is unprotected.
/// Downcast it from the `anyhow::Error`: `err.downcast_ref::<RoutePostureError>()`.
#[derive(Clone, Debug)]
pub struct RoutePostureError {
    unprotected: Vec<PostureEntry>,
}

impl RoutePostureError {
    pub(crate) fn new(unprotected: Vec<PostureEntry>) -> Self {
        RoutePostureError { unprotected }
    }

    /// Every unprotected route, sorted by path then method.
    pub fn unprotected(&self) -> &[PostureEntry] {
        &self.unprotected
    }
}

impl fmt::Display for RoutePostureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let n = self.unprotected.len();
        let messages = self.unprotected.iter().filter(|e| matches!(e.kind, PostureKind::WsMessage { .. })).count();
        let (noun, verb) = match (n, messages) {
            (1, 0) => ("route", "has"),
            (1, _) => ("message handler", "has"),
            (_, 0) => ("routes", "have"),
            _ if messages == n => ("message handlers", "have"),
            _ => ("routes or message handlers", "have"),
        };
        writeln!(
            f,
            "Route posture check failed: {n} {noun} {verb} no guard and {} not marked #[public].",
            if n == 1 { "is" } else { "are" }
        )?;
        let method_w = self.unprotected.iter().map(|e| e.kind.label().len()).max().unwrap_or(0);
        let path_w = self.unprotected.iter().map(|e| e.path.len()).max().unwrap_or(0);
        let detail_w = self.unprotected.iter().map(|e| e.kind.detail().len()).max().unwrap_or(0);
        for e in &self.unprotected {
            let detail = if detail_w == 0 { String::new() } else { format!("{:<detail_w$}  ", e.kind.detail()) };
            writeln!(f, "  {:<method_w$}  {:<path_w$}  {detail}{}::{}", e.kind.label(), e.path, e.owner, e.handler)?;
        }
        let place =
            if messages > 0 { "the route, the message handler, its controller or the gateway" } else { "the route or its controller" };
        write!(
            f,
            "Add #[use_guards(...)] to {place} (or register a global guard), mark it #[public] if it is \
             meant to be open, or add an exemption with a reason. Guards that do not control access (ThrottlerGuard, ZodGuard) \
             do not count."
        )
    }
}

impl std::error::Error for RoutePostureError {}

/// The posture of every route of `table` and every handler of `gateways`, with `options`' exemptions (none matter in
/// off mode, but they are applied anyway so that a report read in off mode is the same as one read in warn mode).
pub(crate) fn report_for(table: &RouterTable, gateways: &[GatewayPosture], options: &RoutePostureOptions) -> RoutePostureReport {
    let reflector = Reflector;
    let mut matched = vec![false; options.exemptions.len()];
    let mut entries = Vec::with_capacity(table.routes.len());
    let guard_infos = |guards: &[Arc<dyn CanActivate>]| -> Vec<GuardInfo> {
        guards.iter().map(|g| GuardInfo { name: short_type_name(g.guard_name()), controls_access: g.controls_access() }).collect()
    };
    let status_of = |guards: &[GuardInfo], public: bool, exemption: Option<&PostureExemption>| {
        if public {
            PostureStatus::Public
        } else if guards.iter().any(|g| g.controls_access) {
            PostureStatus::Protected
        } else if let Some(e) = exemption {
            PostureStatus::Exempt { reason: e.reason.clone() }
        } else {
            PostureStatus::Unprotected
        }
    };
    for route in &table.routes {
        let guards = guard_infos(&route.guards);
        let mut exemption = None;
        for (i, e) in options.exemptions.iter().enumerate() {
            if e.matches(route.method, &route.path) {
                matched[i] = true;
                exemption.get_or_insert(e);
            }
        }
        let public =
            reflector.get_all_and_override::<bool>(crate::PUBLIC_METADATA, &[&route.handler_target, &route.class_target]) == Some(true);
        let status = status_of(&guards, public, exemption);
        entries.push(PostureEntry {
            kind: if route.sse { PostureKind::Sse } else { PostureKind::Http { method: route.method } },
            path: route.path.clone(),
            host: route.host.clone(),
            owner: route.class_target.name.clone(),
            handler: route.handler_target.name.clone(),
            guards,
            status,
        });
    }
    for gateway in gateways {
        for (event, target, handler_guards) in &gateway.handlers {
            let guards = guard_infos(handler_guards);
            let mut exemption = None;
            for (i, e) in options.exemptions.iter().enumerate() {
                if e.matches_message(&gateway.path, event) {
                    matched[i] = true;
                    exemption.get_or_insert(e);
                }
            }
            let public = reflector.get_all_and_override::<bool>(crate::PUBLIC_METADATA, &[target, &gateway.class_target]) == Some(true);
            let status = status_of(&guards, public, exemption);
            entries.push(PostureEntry {
                kind: PostureKind::WsMessage { event: event.clone() },
                path: gateway.path.clone(),
                host: None,
                owner: gateway.name.clone(),
                handler: target.name.clone(),
                guards,
                status,
            });
        }
    }
    let stale = options.exemptions.iter().zip(&matched).filter(|(_, m)| !**m).map(|(e, _)| e.clone()).collect();
    RoutePostureReport::new(entries, stale)
}

/// Runs the startup check of `init()`: enforce returns the error, warn logs the list, off does nothing.
pub(crate) fn check_routes(table: &RouterTable, gateways: &[GatewayPosture], options: &RoutePostureOptions) -> crate::Result<()> {
    if options.mode == RoutePostureMode::Off {
        return Ok(());
    }
    let report = report_for(table, gateways, options);
    let log = crate::logger::Logger::new("RoutePosture");
    for e in report.stale_exemptions() {
        let what = if e.event.is_some() { "handler" } else { "route" };
        log.warn(format!("Route posture exemption {} matches no {what}", e.describe()));
    }
    let unprotected: Vec<PostureEntry> = report.unprotected().into_iter().cloned().collect();
    if unprotected.is_empty() {
        return Ok(());
    }
    let error = RoutePostureError::new(unprotected);
    match options.mode {
        RoutePostureMode::Enforce => {
            log.error(error.to_string());
            Err(anyhow::Error::new(error))
        }
        _ => {
            log.warn(error.to_string());
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(method: RequestMethod, path: &str, owner: &str, handler: &str, status: PostureStatus) -> PostureEntry {
        PostureEntry::new(PostureKind::Http { method }, path, owner, handler, Vec::new(), status)
    }

    #[test]
    fn entries_are_sorted_by_path_then_method_then_handler() {
        let report = RoutePostureReport::new(
            vec![
                entry(RequestMethod::Post, "/b", "C", "z", PostureStatus::Protected),
                entry(RequestMethod::Get, "/b", "C", "y", PostureStatus::Protected),
                entry(RequestMethod::Post, "/a", "C", "x", PostureStatus::Protected),
                entry(RequestMethod::Get, "/b", "C", "a", PostureStatus::Protected),
            ],
            Vec::new(),
        );
        let order: Vec<String> = report.entries().iter().map(|e| format!("{} {} {}", e.kind.label(), e.path, e.handler)).collect();
        assert_eq!(order, ["POST /a x", "GET /b a", "GET /b y", "POST /b z"]);
    }

    #[test]
    fn the_error_names_one_route_in_the_singular() {
        let err =
            RoutePostureError::new(vec![entry(RequestMethod::Get, "/api/rooms", "RoomsController", "list", PostureStatus::Unprotected)]);
        let text = err.to_string();
        assert!(text.starts_with("Route posture check failed: 1 route has no guard and is not marked #[public]."), "{text}");
        assert!(text.contains("  GET  /api/rooms  RoomsController::list"), "{text}");
        assert!(text.contains("exemption with a reason"), "{text}");
        assert!(text.contains("ThrottlerGuard, ZodGuard"), "{text}");
    }

    #[test]
    fn the_error_lists_every_route_and_aligns_the_columns() {
        let err = RoutePostureError::new(vec![
            entry(RequestMethod::Get, "/api/rooms", "RoomsController", "list", PostureStatus::Unprotected),
            entry(RequestMethod::Post, "/api/rooms/:room/messages", "RoomsController", "post", PostureStatus::Unprotected),
        ]);
        let text = err.to_string();
        assert!(text.starts_with("Route posture check failed: 2 routes have no guard and are not marked #[public]."), "{text}");
        assert!(text.contains("  GET   /api/rooms                 RoomsController::list"), "{text}");
        assert!(text.contains("  POST  /api/rooms/:room/messages  RoomsController::post"), "{text}");
    }

    #[test]
    fn the_report_serializes_with_stable_field_names() {
        let mut e =
            entry(RequestMethod::Get, "/metrics", "AppController", "metrics", PostureStatus::Exempt { reason: "private network".into() });
        e.guards.push(GuardInfo { name: "ThrottlerGuard".into(), controls_access: false });
        let report = RoutePostureReport::new(vec![e], vec![PostureOptionsForTest::exemption(RequestMethod::Get, "/gone", "old")]);
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "entries": [{
                    "kind": {"type": "http", "method": "GET"},
                    "path": "/metrics",
                    "host": null,
                    "owner": "AppController",
                    "handler": "metrics",
                    "guards": [{"name": "ThrottlerGuard", "controls_access": false}],
                    "status": {"state": "exempt", "reason": "private network"},
                }],
                "stale_exemptions": [{"method": "GET", "pattern": "/gone", "reason": "old"}],
                "unchecked": [],
            })
        );
    }

    struct PostureOptionsForTest;
    impl PostureOptionsForTest {
        fn exemption(method: RequestMethod, pattern: &str, reason: &str) -> PostureExemption {
            RoutePostureOptions::warn().exempt(method, pattern, reason).exemptions[0].clone()
        }
    }

    #[test]
    fn exemption_methods_match_exactly_unless_all() {
        let all = PostureExemption { method: RequestMethod::All, pattern: "/x".into(), reason: "r".into(), event: None };
        let get = PostureExemption { method: RequestMethod::Get, pattern: "/x".into(), reason: "r".into(), event: None };
        assert!(all.matches(RequestMethod::Post, "/x"));
        assert!(all.matches(RequestMethod::All, "/x"));
        assert!(get.matches(RequestMethod::Get, "/x"));
        assert!(!get.matches(RequestMethod::Post, "/x"));
        assert!(!get.matches(RequestMethod::All, "/x"), "a route declared with #[all] needs an All exemption");
    }

    #[test]
    fn exemption_paths_use_the_middleware_pattern_rules() {
        let e = PostureExemption { method: RequestMethod::Get, pattern: "cats/*".into(), reason: "r".into(), event: None };
        assert!(!e.matches(RequestMethod::Get, "/api/v1/cats/:id"), "the pattern is compared with the full path");
        for pattern in ["/api/*/cats/*", "*/cats/*"] {
            let e = PostureExemption { method: RequestMethod::Get, pattern: pattern.into(), reason: "r".into(), event: None };
            assert!(e.matches(RequestMethod::Get, "/api/v1/cats/:id"), "{pattern}");
        }
        let e = PostureExemption { method: RequestMethod::Get, pattern: "/Api/V1/CATS/:id".into(), reason: "r".into(), event: None };
        assert!(e.matches(RequestMethod::Get, "/api/v1/cats/:id"), "literals ignore ASCII case");
    }

    #[test]
    fn a_blank_reason_is_rejected_with_the_method_and_pattern() {
        let options = RoutePostureOptions::enforce().exempt(RequestMethod::Get, "/metrics", "  ");
        let err = options.validate().unwrap_err().to_string();
        assert!(err.contains("GET /metrics"), "{err}");
        assert!(RoutePostureOptions::enforce().exempt(RequestMethod::Get, "/metrics", "private").validate().is_ok());
    }
}
