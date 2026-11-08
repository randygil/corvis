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
        self.event.as_deref().is_some_and(|e| e == "*" || e == event) && template_matches(&self.pattern, path)
    }

    fn describe(&self) -> String {
        match &self.event {
            Some(event) => format!("WS {} event \"{}\"", self.pattern, event),
            None => format!("{} {}", self.method.as_str(), self.pattern),
        }
    }

    fn matches(&self, method: RequestMethod, path: &str) -> bool {
        self.event.is_none() && (self.method == RequestMethod::All || self.method == method) && template_matches(&self.pattern, path)
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

fn is_param_segment(segment: &str) -> bool {
    segment.starts_with(':') || segment.starts_with('*') || (segment.starts_with('{') && segment.ends_with('}'))
}

/// `*` inside a segment matches any run of characters except `/` (segments never contain one); everything else
/// is compared ignoring ASCII case.
fn segment_glob(pattern: &str, text: &str) -> bool {
    let (p, t) = (pattern.as_bytes(), text.as_bytes());
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && p[pi] == b'*' {
            star = Some((pi, ti));
            pi += 1;
        } else if pi < p.len() && p[pi].eq_ignore_ascii_case(&t[ti]) {
            pi += 1;
            ti += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|b| *b == b'*')
}

/// Matches an exemption pattern against a route's path template, literally:
/// - a literal segment matches the same literal segment (ignoring ASCII case), never a `:param`;
/// - `:x` or `{x}` matches only a `:param` or `{param}` segment of the template;
/// - `*` as a whole segment matches exactly one segment, `**` matches zero or more segments;
/// - `*` inside a segment (`health*`) matches within that segment and never crosses a `/`.
fn template_matches(pattern: &str, template: &str) -> bool {
    fn rec(pat: &[&str], segs: &[&str]) -> bool {
        match pat.first() {
            None => segs.is_empty(),
            Some(&"**") => (0..=segs.len()).any(|i| rec(&pat[1..], &segs[i..])),
            Some(p) => match segs.first() {
                None => false,
                Some(s) => {
                    let ok = if *p == "*" {
                        true
                    } else if p.starts_with(':') || (p.starts_with('{') && p.ends_with('}')) {
                        is_param_segment(s)
                    } else if p.contains('*') {
                        !is_param_segment(s) && segment_glob(p, s)
                    } else {
                        p.eq_ignore_ascii_case(s)
                    };
                    ok && rec(&pat[1..], &segs[1..])
                }
            },
        }
    }
    let pat: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let segs: Vec<&str> = template.split('/').filter(|s| !s.is_empty()).collect();
    rec(&pat, &segs)
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
    /// Where a [`PostureStatus::Public`] status comes from; `None` for any other status.
    pub public_source: Option<PublicSource>,
}

/// Where the public marker of a route is set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicSource {
    /// `#[public]` (or `PUBLIC_METADATA`) on the route method.
    Handler,
    /// `#[public]` (or `PUBLIC_METADATA`) on the controller.
    Controller,
    /// `#[public]` (or `PUBLIC_METADATA`) on the gateway.
    Gateway,
}

impl PublicSource {
    fn label(self) -> &'static str {
        match self {
            PublicSource::Handler => "handler",
            PublicSource::Controller => "controller",
            PublicSource::Gateway => "gateway",
        }
    }
}

/// The level at which a guard is registered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "level", rename_all = "snake_case")]
#[non_exhaustive]
pub enum GuardScope {
    /// `use_global_guards` or `Provider::app_guard`: every route.
    Global,
    /// `#[use_guards]` below `#[controller]`.
    Controller { controller: String },
    /// `#[use_guards]` on a route method.
    Route { controller: String, handler: String },
    /// `#[use_guards]` below `#[websocket_gateway]`: every message handler of the gateway.
    Gateway { gateway: String },
    /// `#[use_guards]` on a `#[subscribe_message]` method.
    Message { gateway: String, handler: String },
}

impl fmt::Display for GuardScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GuardScope::Global => write!(f, "global"),
            GuardScope::Controller { controller } => write!(f, "controller {controller}"),
            GuardScope::Route { controller, handler } => write!(f, "route {controller}::{handler}"),
            GuardScope::Gateway { gateway } => write!(f, "gateway {gateway}"),
            GuardScope::Message { gateway, handler } => write!(f, "message handler {gateway}::{handler}"),
        }
    }
}

/// A guard that counted as access control, where it is registered and how many routes it covers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AccessGuardSummary {
    pub name: String,
    pub scope: GuardScope,
    /// Compiled routes (each path alias and URI version counts) and gateway message handlers the guard runs for.
    pub routes: usize,
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
        PostureEntry {
            kind,
            path: path.into(),
            host: None,
            owner: owner.into(),
            handler: handler.into(),
            guards,
            status,
            public_source: None,
        }
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
    access_guards: Vec<AccessGuardSummary>,
    redundant_exemptions: Vec<PostureExemption>,
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

/// One message handler: the guards that run for it, in order, and how many of them are global and gateway-level.
pub(crate) struct GatewayHandlerPosture {
    pub event: String,
    pub target: Arc<Target>,
    pub guards: Vec<Arc<dyn CanActivate>>,
    /// The leading guards that are global (0 when global enhancers are kept HTTP-only).
    pub global_guard_count: usize,
    /// The guards after the global ones that come from the gateway.
    pub gateway_guard_count: usize,
}

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
        RoutePostureReport { entries, stale_exemptions, unchecked: Vec::new(), access_guards: Vec::new(), redundant_exemptions: Vec::new() }
    }

    fn with_details(mut self, access_guards: Vec<AccessGuardSummary>, redundant_exemptions: Vec<PostureExemption>) -> Self {
        self.access_guards = access_guards;
        self.redundant_exemptions = redundant_exemptions;
        self
    }

    /// The guards that counted as access control, with their scope and the number of routes they cover. A
    /// guard that lets every request through shows up here with all the routes it makes look protected.
    pub fn access_guards(&self) -> &[AccessGuardSummary] {
        &self.access_guards
    }

    /// The exemptions that match only routes that are already protected or public: they accept nothing today
    /// but would silently accept those routes if their guard were removed.
    pub fn redundant_exemptions(&self) -> &[PostureExemption] {
        &self.redundant_exemptions
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

fn status_label(entry: &PostureEntry) -> String {
    match &entry.status {
        PostureStatus::Protected => "protected".to_string(),
        PostureStatus::Public => match entry.public_source {
            Some(source) => format!("public ({})", source.label()),
            None => "public".to_string(),
        },
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
                status_label(e),
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
    let mut needed = vec![false; options.exemptions.len()];
    let mut summaries: Vec<AccessGuardSummary> = Vec::new();
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
        let public =
            reflector.get_all_and_override::<bool>(crate::PUBLIC_METADATA, &[&route.handler_target, &route.class_target]) == Some(true);
        let guarded = guards.iter().any(|g| g.controls_access);
        let mut exemption = None;
        for (i, e) in options.exemptions.iter().enumerate() {
            if e.matches(route.method, &route.path) {
                matched[i] = true;
                needed[i] |= !public && !guarded;
                exemption.get_or_insert(e);
            }
        }
        let public_source = public.then(|| {
            if reflector.get::<bool>(crate::PUBLIC_METADATA, &route.handler_target) == Some(true) {
                PublicSource::Handler
            } else {
                PublicSource::Controller
            }
        });
        for (i, g) in route.guards.iter().enumerate() {
            if !g.controls_access() {
                continue;
            }
            let scope = if i < route.global_guard_count {
                GuardScope::Global
            } else if i < route.global_guard_count + route.controller_guard_count {
                GuardScope::Controller { controller: route.class_target.name.clone() }
            } else {
                GuardScope::Route { controller: route.class_target.name.clone(), handler: route.handler_target.name.clone() }
            };
            let name = short_type_name(g.guard_name());
            match summaries.iter_mut().find(|s| s.name == name && s.scope == scope) {
                Some(s) => s.routes += 1,
                None => summaries.push(AccessGuardSummary { name, scope, routes: 1 }),
            }
        }
        let status = status_of(&guards, public, exemption);
        entries.push(PostureEntry {
            kind: if route.sse { PostureKind::Sse } else { PostureKind::Http { method: route.method } },
            path: route.path.clone(),
            host: route.host.clone(),
            owner: route.class_target.name.clone(),
            handler: route.handler_target.name.clone(),
            guards,
            status,
            public_source,
        });
    }
    for gateway in gateways {
        for h in &gateway.handlers {
            let (event, target) = (&h.event, &h.target);
            let guards = guard_infos(&h.guards);
            let guarded = guards.iter().any(|g| g.controls_access);
            let public = reflector.get_all_and_override::<bool>(crate::PUBLIC_METADATA, &[target, &gateway.class_target]) == Some(true);
            let mut exemption = None;
            for (i, e) in options.exemptions.iter().enumerate() {
                if e.matches_message(&gateway.path, event) {
                    matched[i] = true;
                    needed[i] |= !public && !guarded;
                    exemption.get_or_insert(e);
                }
            }
            let public_source = public.then(|| {
                if reflector.get::<bool>(crate::PUBLIC_METADATA, target) == Some(true) {
                    PublicSource::Handler
                } else {
                    PublicSource::Gateway
                }
            });
            for (i, g) in h.guards.iter().enumerate() {
                if !g.controls_access() {
                    continue;
                }
                let scope = if i < h.global_guard_count {
                    GuardScope::Global
                } else if i < h.global_guard_count + h.gateway_guard_count {
                    GuardScope::Gateway { gateway: gateway.name.clone() }
                } else {
                    GuardScope::Message { gateway: gateway.name.clone(), handler: target.name.clone() }
                };
                let name = short_type_name(g.guard_name());
                match summaries.iter_mut().find(|s| s.name == name && s.scope == scope) {
                    Some(s) => s.routes += 1,
                    None => summaries.push(AccessGuardSummary { name, scope, routes: 1 }),
                }
            }
            let status = status_of(&guards, public, exemption);
            entries.push(PostureEntry {
                kind: PostureKind::WsMessage { event: event.clone() },
                path: gateway.path.clone(),
                host: None,
                owner: gateway.name.clone(),
                handler: target.name.clone(),
                guards,
                status,
                public_source,
            });
        }
    }
    let stale = options.exemptions.iter().zip(&matched).filter(|(_, m)| !**m).map(|(e, _)| e.clone()).collect();
    let redundant = options.exemptions.iter().enumerate().filter(|(i, _)| matched[*i] && !needed[*i]).map(|(_, e)| e.clone()).collect();
    summaries.sort_by_key(|s| (s.scope != GuardScope::Global, s.name.clone(), s.scope.to_string()));
    RoutePostureReport::new(entries, stale).with_details(summaries, redundant)
}

/// `Route posture: access guards counted: RolesGuard (global, 42 routes), AuthGuard (controller CatsController, 5 routes)`
fn access_guards_line(guards: &[AccessGuardSummary]) -> String {
    if guards.is_empty() {
        return "Route posture: access guards counted: none".to_string();
    }
    let list: Vec<String> =
        guards.iter().map(|g| format!("{} ({}, {} route{})", g.name, g.scope, g.routes, if g.routes == 1 { "" } else { "s" })).collect();
    format!("Route posture: access guards counted: {}", list.join(", "))
}

/// The line that opens the report in report mode.
pub const REPORT_BEGIN: &str = "----- BEGIN CORVIS ROUTE POSTURE REPORT -----";
/// The line that closes the report in report mode.
pub const REPORT_END: &str = "----- END CORVIS ROUTE POSTURE REPORT -----";
/// The environment variable that turns report mode on (`text` or `json`).
pub const REPORT_ENV: &str = "CORVIS_ROUTE_POSTURE_REPORT";

/// How report mode prints the report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportFormat {
    /// The aligned table of [`RoutePostureReport`]'s `Display`.
    Text,
    /// The pretty-printed JSON of the serialized report.
    Json,
}

/// Reads the value of [`REPORT_ENV`]: unset or blank means no report mode; `text` and `json` (surrounding
/// whitespace ignored, case-sensitive) select the format; anything else is an error naming the variable.
pub fn report_format_from_env(value: Option<&str>) -> crate::Result<Option<ReportFormat>> {
    match value.map(str::trim) {
        None | Some("") => Ok(None),
        Some("text") => Ok(Some(ReportFormat::Text)),
        Some("json") => Ok(Some(ReportFormat::Json)),
        Some(other) => anyhow::bail!("{REPORT_ENV} must be \"text\" or \"json\" (got \"{other}\")"),
    }
}

/// What report mode writes to stdout and the exit code it ends the process with (0 when nothing is
/// unprotected, 1 otherwise).
pub fn render_report_mode(report: &RoutePostureReport, format: ReportFormat) -> (String, i32) {
    let mut out = String::new();
    out.push_str(REPORT_BEGIN);
    out.push('\n');
    match format {
        ReportFormat::Text => {
            let table = report.to_string();
            if table.is_empty() {
                out.push_str(
                    "(no routes)
",
                );
            } else {
                out.push_str(&table);
            }
            let notes = access_guards_line(report.access_guards());
            out.push_str(&format!(
                "
{}
",
                notes.trim_start_matches("Route posture: ")
            ));
            for e in report.stale_exemptions() {
                let what = if e.event.is_some() { "handler" } else { "route" };
                out.push_str(&format!(
                    "stale exemption: {} ({}) matches no {what}
",
                    e.describe(),
                    e.reason
                ));
            }
            for e in report.redundant_exemptions() {
                out.push_str(&format!(
                    "redundant exemption: {} ({}) matches only guarded or public {}
",
                    e.describe(),
                    e.reason,
                    if e.event.is_some() { "message handlers" } else { "routes" }
                ));
            }
        }
        ReportFormat::Json => {
            out.push_str(&serde_json::to_string_pretty(report).expect("the report serializes"));
            out.push('\n');
        }
    }
    out.push_str(REPORT_END);
    out.push('\n');
    (out, if report.unprotected().is_empty() { 0 } else { 1 })
}

/// Report mode of `init()`: when [`REPORT_ENV`] asks for it, prints the report between the marker lines,
/// flushes stdout and ends the process. Returns `Ok` (and does nothing) when the variable is unset or blank.
pub(crate) fn run_report_mode(table: &RouterTable, gateways: &[GatewayPosture], options: &RoutePostureOptions) -> crate::Result<()> {
    let value = std::env::var(REPORT_ENV).ok();
    let Some(format) = report_format_from_env(value.as_deref())? else {
        return Ok(());
    };
    let report = report_for(table, gateways, options);
    let (text, code) = render_report_mode(&report, format);
    {
        use std::io::Write;
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }
    std::process::exit(code);
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
    for e in report.redundant_exemptions() {
        log.warn(format!(
            "Route posture exemption {} matches only guarded or public {}: it accepts nothing today, but it would accept them if their guard were removed",
            e.describe(),
            if e.event.is_some() { "message handlers" } else { "routes" }
        ));
    }
    log.log(access_guards_line(report.access_guards()));
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
                    "public_source": null,
                }],
                "stale_exemptions": [{"method": "GET", "pattern": "/gone", "reason": "old"}],
                "unchecked": [],
                "access_guards": [],
                "redundant_exemptions": [],
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

    fn m(pattern: &str, template: &str) -> bool {
        template_matches(pattern, template)
    }

    #[test]
    fn exemption_literals_ignore_case_and_never_match_a_param() {
        assert!(m("/Api/V1/CATS", "/api/v1/cats"));
        assert!(m("api/v1/cats/", "/api/v1/cats"), "slashes at the ends are ignored");
        assert!(!m("/files/export-all", "/files/:id"), "a literal pattern segment does not match a :param segment");
        assert!(!m("/files/:id", "/files/export-all"), "a :param pattern segment does not match a literal template segment");
    }

    #[test]
    fn exemption_params_match_only_params() {
        assert!(m("/files/:id", "/files/:id"));
        assert!(m("/files/:id", "/files/:file"));
        assert!(m("/files/{id}", "/files/:id"));
        assert!(m("/files/:id", "/files/{id}"));
        assert!(!m("/files/:id", "/files/export-all"));
        assert!(!m("/files/:id", "/files"));
    }

    #[test]
    fn exemption_star_is_one_segment_and_double_star_is_the_rest() {
        assert!(m("/api/*/internal/*", "/api/v1/internal/jobs"));
        assert!(m("/api/*/cats/*", "/api/v1/cats/:id"));
        assert!(!m("/api/*/cats/*", "/api/v1/cats/:id/toys"), "a trailing * is one segment");
        assert!(!m("/metrics/*", "/metrics"), "* needs a segment");
        assert!(m("/metrics/**", "/metrics"));
        assert!(m("/metrics/**", "/metrics/a/b/:c"));
        assert!(m("**/cats/*", "/api/v1/cats/:id"));
        assert!(m("/**", "/anything/at/all"));
        assert!(!m("*", "/a/b"));
    }

    #[test]
    fn exemption_star_inside_a_segment_does_not_cross_a_slash() {
        assert!(m("/health*", "/healthz"));
        assert!(m("/health*", "/health"));
        assert!(!m("/health*", "/healthcare/records/:id"));
        assert!(m("/a*c/x", "/abc/x"));
        assert!(!m("/health*", "/:id"), "a glob never matches a :param segment");
    }

    #[test]
    fn report_format_reads_the_variable() {
        assert_eq!(report_format_from_env(None).unwrap(), None);
        assert_eq!(report_format_from_env(Some("")).unwrap(), None);
        assert_eq!(report_format_from_env(Some("  ")).unwrap(), None);
        assert_eq!(report_format_from_env(Some("text")).unwrap(), Some(ReportFormat::Text));
        assert_eq!(report_format_from_env(Some(" json\n")).unwrap(), Some(ReportFormat::Json));
        for bad in ["yaml", "TEXT", "Json", "1"] {
            let err = report_format_from_env(Some(bad)).unwrap_err().to_string();
            assert!(err.contains("CORVIS_ROUTE_POSTURE_REPORT") && err.contains("\"text\"") && err.contains("\"json\""), "{err}");
            assert!(err.contains(bad), "{err}");
        }
    }

    #[test]
    fn the_markers_have_a_fixed_text() {
        assert_eq!(REPORT_BEGIN, "----- BEGIN CORVIS ROUTE POSTURE REPORT -----");
        assert_eq!(REPORT_END, "----- END CORVIS ROUTE POSTURE REPORT -----");
    }

    #[test]
    fn report_mode_renders_both_formats_and_picks_the_exit_code() {
        let open = entry(RequestMethod::Get, "/open", "C", "open", PostureStatus::Unprotected);
        let fine = entry(RequestMethod::Get, "/fine", "C", "fine", PostureStatus::Public);
        let bad = RoutePostureReport::new(vec![open, fine.clone()], Vec::new());
        let good = RoutePostureReport::new(vec![fine], Vec::new());

        let (text, code) = render_report_mode(&bad, ReportFormat::Text);
        assert_eq!(code, 1);
        assert!(text.starts_with(&format!("{REPORT_BEGIN}\n")) && text.ends_with(&format!("{REPORT_END}\n")), "{text}");
        assert!(text.contains("UNPROTECTED") && text.contains("C::open"), "{text}");

        let (text, code) = render_report_mode(&good, ReportFormat::Text);
        assert_eq!(code, 0);
        assert!(!text.contains("UNPROTECTED"), "{text}");

        let (text, code) = render_report_mode(&bad, ReportFormat::Json);
        assert_eq!(code, 1);
        let body = text.strip_prefix(&format!("{REPORT_BEGIN}\n")).unwrap().strip_suffix(&format!("{REPORT_END}\n")).unwrap();
        let json: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(json["entries"].as_array().unwrap().len(), 2);
        assert_eq!(render_report_mode(&good, ReportFormat::Json).1, 0);

        let (text, _) = render_report_mode(&RoutePostureReport::new(Vec::new(), Vec::new()), ReportFormat::Text);
        assert!(text.contains("(no routes)"), "{text}");
    }

    #[test]
    fn a_blank_reason_is_rejected_with_the_method_and_pattern() {
        let options = RoutePostureOptions::enforce().exempt(RequestMethod::Get, "/metrics", "  ");
        let err = options.validate().unwrap_err().to_string();
        assert!(err.contains("GET /metrics"), "{err}");
        assert!(RoutePostureOptions::enforce().exempt(RequestMethod::Get, "/metrics", "private").validate().is_ok());
    }
}
