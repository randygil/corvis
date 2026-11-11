---
status: documented
---

# Route posture

Forgetting `#[use_guards(...)]` is silent: the route works and anyone can call it. The route posture check turns that
mistake into a startup error. When you enable it, the application refuses to start while any route has no guard and
has not been opened on purpose, and it can print the posture of every route for review.

The check is opt-in. An application that does not enable it behaves exactly as before.

## The rule

Every controller route (each HTTP method, path alias and URI version), every `#[sse]` stream and every
`#[subscribe_message]` handler of a WebSocket gateway must be one of:

- **protected**: a guard that controls access runs for it, registered at any level the router runs guards (see
  [Guards](/guards#order)): `Provider::app_guard`, `use_global_guards`, the controller or the route;
- **public**: it is marked `#[public]`, on the route or on its controller (or gateway);
- **exempt**: an exemption you configured matches it, with a reason.

Anything else is **unprotected**.

## Enabling it

Call `require_route_posture()` before `init()` or `listen()`; calls made afterwards have no effect:

```rust
let mut app = CorvisFactory::create::<AppModule>().await?;
app.require_route_posture();
app.listen("127.0.0.1:3000").await?;
```

`init()` (so `listen()` and `listen_in_background()` too) then fails with a `RoutePostureError` before the server
binds and before any module or gateway hook runs. Projects created with [`corvis new`](/cli/usages#corvis-new) call it
already, and `examples/tertulia` and `examples/cats-app` enforce it in `create_app()`.

To adopt it step by step, use warn mode. It logs the same list once, at warning level, and the application starts
normally. Routes that are protected outside the framework can be exempted, always with a reason:

```rust
use corvis::{RequestMethod, RoutePostureOptions};

app.set_route_posture(
    RoutePostureOptions::warn()
        .exempt(RequestMethod::Get, "/metrics", "scraped from the private network only")
        .exempt(RequestMethod::All, "/internal/**", "authenticated by the reverse proxy before the request arrives"),
);
```

`RoutePostureOptions::enforce()` selects enforce mode in the same way, and `off()` the default.

- The pattern is compared literally with the route's full path template (global prefix and version included), segment
  by segment: a literal segment matches the same literal (ignoring ASCII case), `:id` or `{id}` matches only a
  `:param` segment (so `/files/:id` does not exempt `/files/export-all`), `*` as a whole segment is exactly one
  segment (`/api/*/internal/*`), `**` is zero or more segments (`/metrics/**`), and `*` inside a segment (`/health*`)
  never crosses a `/`.
- `RequestMethod::All` matches any method. A route declared with `#[all]` is matched only by an `All` exemption.
- The reason is required. A blank reason makes `init()` fail, in every mode except off.
- An exemption that matches no route logs a warning, and so does one that matches only routes that already have a
  guard or are public (it would silently accept them if the guard were removed).
- Authentication done in middleware or by a reverse proxy is invisible to the check, because middleware runs before
  routing and has no declaration to read. Exempt those routes with a reason that says where the protection lives, as in
  the `/internal/**` example above. The reason shows up in the report, so a reviewer can challenge it.
- For gateway messages use `exempt_message(path, event, reason)`; see [WebSocket guards](/websockets/guards#route-posture-for-gateways).

## Marking routes public

`#[public]` goes on a route method inside `#[routes]`, or below `#[controller]` to open the whole controller. It takes
no arguments and does not change how requests are handled: it stores the metadata entry `PUBLIC_METADATA` (`"isPublic"`)
with the value `true`.

```rust
#[controller("auth")]
pub struct AuthController { /* ... */ }

#[routes]
impl AuthController {
    #[post("login")]
    #[public]
    fn login(&self, #[body] dto: LoginDto) -> HttpResult<LoginResponse> { /* ... */ }
}

// The whole controller, including its SSE stream:
#[controller("stats")]
#[public]
pub struct StatsController { /* ... */ }
```

A route's value wins over the controller's, so `#[set_metadata(PUBLIC_METADATA, false)]` on a route makes it private
again inside a public controller. `#[set_metadata(PUBLIC_METADATA, true)]`, the same literal key written by hand, and a
[custom decorator](/custom-decorators#composing-metadata-customdecorator) that sets it have the same effect.

Because the key is the one applications already use, a global guard that skips routes whose `"isPublic"` metadata is
`true` keeps working with `#[public]` unchanged; see [Public routes with a global guard](/guards#public-routes-with-a-global-guard)
and [Authentication](/security/authentication#protecting-everything-by-default). That is also the catch: `#[public]`
is a declaration for the check and for your guards. A guard that does not read the metadata will still reject the route.

## Which guards count

A guard counts only when it declares that it controls access. Guards declare it by default. `ThrottlerGuard` and
`ZodGuard` limit or validate requests without deciding who may call a route, so they do not count: a route whose only
guard is a global rate limiter is unprotected.

Override `controls_access()` to return `false` in your own guards that let requests through unless something is
configured (a roles guard that returns `Ok(true)` when the route declares no roles, tenant, maintenance-mode or
feature-flag guards). Registered globally without that override, such a guard makes every route look protected. Counting a
guard is a declaration, not proof that it authenticates. See [Pass-through global guards](/guards#pass-through-global-guards),
which also describes the line logged in warn and enforce mode with the guards that counted and how many routes each
covers. `corvis g guard` generates a guard that denies every request until you implement it, so a fresh guard never
counts as protection while letting everyone in.

## WebSockets and SSE

`#[sse]` routes follow the same rule as any other route and are labelled `SSE` in the error and in the report.

Every `#[subscribe_message]` handler is checked too. The guards that count for a message are the ones that run for it:
global guards (only while global enhancers apply to gateways), then the gateway's, then the handler's. `#[public]` goes
below `#[websocket_gateway]` or on a handler, and `exempt_message` exempts a handler by gateway path and event. The details are in
[Route posture for gateways](/websockets/guards#route-posture-for-gateways).

Prefer `#[public]` on a gateway to a global guard that returns `Ok(true)` for WebSocket contexts: the guard would make
the check accept every handler while nothing authenticates.

## What is not checked

The check covers controller routes and gateway message handlers. These surfaces are outside it, so protect them on
their own (the full list is in [What the check does not see](/guards#what-the-check-does-not-see)):

- static files (`ServeStaticModule` and `serve_static`); a static file with the same path as a protected endpoint is
  served before the endpoint;
- raw `upgrade` listeners (`on_upgrade`, `corvis::ws`);
- the Swagger and Scalar UIs and their JSON document;
- any middleware that answers without calling `next`;
- the WebSocket handshake of a gateway, which runs no guard (`verify_client` and `allowed_origins` are the checks that
  exist there; see [Authenticating on connect](/websockets/guards#authenticating-on-connect)).

`app.route_posture()` lists `ServeStaticModule` mounts, gateway handshakes (with whether `verify_client` is set) and the
number of raw `upgrade` listeners under "Not checked" so that a reviewer sees them. They never make `init()` fail.
`serve_static` and other middleware are not visible to the report at all.

## The error

In enforce mode `init()` returns an error that lists every unprotected entry, not only the first, sorted by path and
method, and explains the three fixes. The same text is logged once at error level:

```text
Route posture check failed: 2 routes have no guard and are not marked #[public].
  GET   /api/rooms                 RoomsController::list
  POST  /api/rooms/:room/messages  RoomsController::post
Add #[use_guards(...)] to the route or its controller (or register a global guard), mark it #[public] if it is meant to be open, or add an exemption with a reason. Guards that do not control access (ThrottlerGuard, ZodGuard) do not count.
```

Each line has the method (`SSE` for a stream, `WS` for a gateway message), the full path and `Controller::handler`.
Fix each one by adding a guard to the route or its controller, marking it `#[public]` if it is meant to be open, or
adding an exemption with a reason. See [Route posture check failed](/faq/common-errors#route-posture-check-failed) in the
common errors. The error type is `RoutePostureError`, which you can downcast from the `anyhow::Error` that `listen()`
returns. The message never includes request data or configuration values.

## Reviewing the posture

After `init()`, and in every mode (also off), `app.route_posture()` returns the report: every entry with its method or
event, full path, controller and handler, guards (and whether each controls access) and status (protected, public with
`public_source`, exempt with its reason, or unprotected), the surfaces that are not checked, the guards that counted
(`access_guards()`) and the stale or redundant exemptions. It implements `Display` (a table) and `serde::Serialize`.
Use it in a test, so a new route without a guard fails CI even if the application does not enforce the check:

```rust
let mut app = CorvisFactory::create::<AppModule>().await?;
app.init().await?;
let report = app.route_posture().expect("the report exists after init()");
assert!(report.unprotected().is_empty(), "{report}");
```

`examples/tertulia/tests/chat.rs` (`route_posture_matches_the_routes`) goes further and pins which routes are public and
protected.

`corvis posture` prints the same report from the command line, without listening on any port, whatever mode the
application uses. It exits with `0` when nothing is unprotected, `1` when something is, and `2` when it could not get a
trustworthy report (the build or startup failed, or the timeout passed). Use `--json` in CI. Its options, the
output format and the limits are in [`corvis posture`](/cli/usages#corvis-posture).

::: warning Report mode
`corvis posture` works by setting `CORVIS_ROUTE_POSTURE_REPORT`, which makes any binary that reads it print its routes
and stop at `init()` with exit code 40 or 41. Never set it yourself in a deployment or in a CI job that runs your
tests; see [Report mode](/guards#report-mode).
:::

## Migrating an existing application

1. Run `corvis posture` to see the current state without changing the application. Every route marked `UNPROTECTED` is
   a decision to make.
2. Call `app.set_route_posture(RoutePostureOptions::warn())`. The application starts as before and logs the list once.
3. For each route, add a guard, mark it `#[public]` or, if something outside the framework protects it, add an exemption
   with a reason. If you already mark open routes with `#[set_metadata(IS_PUBLIC_KEY, true)]` and the value `"isPublic"`,
   they count as public already; you can switch them to `#[public]` whenever you like.
4. Read the "access guards counted" line that warn mode logs. A global guard that is not your authentication guard and
   covers every route is the one to review: give it `controls_access()` returning `false` if it lets requests through
   when it has nothing to check.
5. Replace the warn options with `app.require_route_posture()` (or keep your exemptions with
   `RoutePostureOptions::enforce()`), and add a test that reads `app.route_posture()` or a `corvis posture` step to CI.
