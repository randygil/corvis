//! Route posture of `create_app()`: it enforces the check, and the report matches the routes.

use corvis::prelude::*;
use corvis::{PostureEntry, PostureKind, PostureStatus, PublicSource, RoutePostureReport};

fn find<'a>(report: &'a RoutePostureReport, kind: &PostureKind, path: &str) -> &'a PostureEntry {
    report.entries().iter().find(|e| &e.kind == kind && e.path == path).unwrap_or_else(|| panic!("no entry for {kind:?} {path}:\n{report}"))
}

fn http(method: RequestMethod) -> PostureKind {
    PostureKind::Http { method }
}

#[corvis::test]
async fn every_route_is_guarded_or_public() {
    // SAFETY: set before the application reads the environment.
    unsafe {
        std::env::set_var("JWT_SECRET", "cats-test-secret-0123456789abcdef");
    }
    let mut app = cats_app::create_app().await.unwrap();
    app.silence_route_logs();
    // Enforce mode: `init()` fails if a route has no guard that controls access and is not `#[public]`.
    app.init().await.unwrap();
    let report = &app.route_posture().expect("the report exists after init()");
    assert!(report.unprotected().is_empty(), "{report}");

    // Route-level `#[public]`.
    let list = find(report, &http(RequestMethod::Get), "/api/v1/cats");
    assert_eq!((&list.status, list.public_source), (&PostureStatus::Public, Some(PublicSource::Handler)));
    let login = find(report, &http(RequestMethod::Post), "/api/v1/auth/login");
    assert_eq!(login.status, PostureStatus::Public);
    assert_eq!(find(report, &PostureKind::Sse, "/api/v1/cats/stream").status, PostureStatus::Public);

    // Controller-level `#[public]`.
    let hello = find(report, &http(RequestMethod::Get), "/api/v1");
    assert_eq!((&hello.status, hello.public_source), (&PostureStatus::Public, Some(PublicSource::Controller)));

    // The global `AuthGuard` counts; `ThrottlerGuard` and the pass-through `RolesGuard` do not.
    let create = find(report, &http(RequestMethod::Post), "/api/v1/cats");
    assert_eq!(create.status, PostureStatus::Protected);
    let access: Vec<&str> = create.guards.iter().filter(|g| g.controls_access).map(|g| g.name.as_str()).collect();
    assert_eq!(access, ["AuthGuard"]);

    // The chat gateway is public as a whole and its handshake is listed as not checked.
    let join =
        report.entries().iter().find(|e| matches!(&e.kind, PostureKind::WsMessage { event } if event == "join")).expect("join handler");
    assert_eq!((&join.status, join.public_source), (&PostureStatus::Public, Some(PublicSource::Gateway)));
    assert!(report.unchecked().iter().any(|s| s.to_string().contains("WebSocket handshake of ChatGateway")));
}
