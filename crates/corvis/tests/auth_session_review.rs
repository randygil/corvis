//! Regression tests of the first security review of sessions: a logout or `destroy()` racing a slow request, a login
//! the guard makes and a later check refuses, the per-user cap of the memory store, and records larger than the limit
//! in a shared store. Every race is held with a gate (a notification), never with a sleep.
#![cfg(feature = "auth-session")]

mod session_common;

use std::sync::Arc;

use corvis::auth::session::{MemorySessionStore, SessionStore};
use corvis::prelude::*;
use corvis::testing::{TestResponse, request};
use session_common::*;

async fn send(app: &CorvisApplication, method: &str, path: &str, cookie: Option<&str>, headers: &[(&str, &str)]) -> TestResponse {
    let client = request(app);
    let mut call = match method {
        "POST" => client.post(path),
        _ => client.get(path),
    };
    if let Some(cookie) = cookie {
        call = call.header("cookie", cookie);
    }
    for (k, v) in headers {
        call = call.header(k, v);
    }
    call.send().await
}

async fn get(app: &CorvisApplication, path: &str, cookie: Option<&str>) -> TestResponse {
    send(app, "GET", path, cookie, &[]).await
}

async fn post(app: &CorvisApplication, path: &str, cookie: Option<&str>) -> TestResponse {
    send(app, "POST", path, cookie, &[]).await
}

async fn login(app: &CorvisApplication, id: &str) -> String {
    let r = send(app, "POST", "/login", None, &[("x-user", id)]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    pair(&set_cookies(&r)[0])
}

/// `true` when every cookie the response sets clears the session cookie.
fn only_clears(r: &TestResponse) -> bool {
    set_cookies(r).iter().all(|c| c.starts_with("connect.sid=;"))
}

// ---- item 1: a logout or destroy() racing a slow request ----

/// Request A (the logged-in cookie) is held in its handler; request B ends the session with `ending`; A then finishes
/// having done `mode`. Whatever A did, the session stays ended.
async fn ended_while_a_request_runs(ending: &str, mode: &str) {
    let store = RecordingStore::new();
    let app = Arc::new(app(options().store_arc(store.clone())).await);
    let c0 = login(&app, "2").await;
    let old = id_of(&c0);
    let name = format!("{ending}-{mode}");
    let gate = named_gate(&name);
    let held = {
        let (app, c0, name, mode) = (app.clone(), c0.clone(), name.clone(), mode.to_string());
        tokio::spawn(async move { post(&app, &format!("/held?g={name}&mode={mode}"), Some(&c0)).await })
    };
    gate.started.notified().await;
    let out = post(&app, &format!("/{ending}"), Some(&c0)).await;
    assert_eq!(out.status, 200, "{ending}");
    assert!(store.inner.load(&old).await.unwrap().is_none());
    let held_after_ending = store.records().await.len();
    gate.release.notify_one();
    let late = held.await.unwrap();
    assert_eq!(late.status, 200, "{ending} / {mode}");
    assert!(only_clears(&late), "{ending} / {mode}: the late request issued a session cookie: {:?}", set_cookies(&late));
    assert_eq!(store.records().await.len(), held_after_ending, "{ending} / {mode}: the late request wrote a record");
    assert_eq!(get(&app, "/me", Some(&c0)).await.status, 401, "{ending} / {mode}");
    // Whatever cookie the late request carried back, none of them logs in.
    for cookie in set_cookies(&late) {
        assert_eq!(get(&app, "/me", Some(&pair(&cookie))).await.status, 401, "{ending} / {mode}");
    }
    // The identifiers of the late request are not in the store either.
    for (id, record) in store.records().await {
        assert!(record["passport"]["user"].is_null(), "{ending} / {mode}: {id} holds a user");
    }
}

#[tokio::test]
async fn a_logout_cannot_be_undone_by_a_request_that_regenerates_keeping_data() {
    ended_while_a_request_runs("logout", "regenerate_keeping_data").await;
}

#[tokio::test]
async fn a_logout_cannot_be_undone_by_a_request_that_regenerates() {
    ended_while_a_request_runs("logout", "regenerate").await;
}

#[tokio::test]
async fn a_logout_cannot_be_undone_by_a_request_that_changes_data() {
    ended_while_a_request_runs("logout", "set").await;
}

#[tokio::test]
async fn a_logout_cannot_be_undone_by_a_request_that_only_touches() {
    ended_while_a_request_runs("logout", "touch").await;
}

#[tokio::test]
async fn destroy_cannot_be_undone_by_a_request_that_regenerates_keeping_data() {
    ended_while_a_request_runs("destroy", "regenerate_keeping_data").await;
}

#[tokio::test]
async fn destroy_cannot_be_undone_by_a_request_that_regenerates() {
    ended_while_a_request_runs("destroy", "regenerate").await;
}

#[tokio::test]
async fn destroy_cannot_be_undone_by_a_request_that_changes_data() {
    ended_while_a_request_runs("destroy", "set").await;
}

#[tokio::test]
async fn destroy_cannot_be_undone_by_a_request_that_only_touches() {
    ended_while_a_request_runs("destroy", "touch").await;
}

#[tokio::test]
async fn regenerating_a_live_session_keeps_the_data_and_logs_nobody_out() {
    let store = RecordingStore::new();
    let app = Arc::new(app(options().store_arc(store.clone())).await);
    let c0 = login(&app, "2").await;
    let gate = named_gate("control");
    let held = {
        let (app, c0) = (app.clone(), c0.clone());
        tokio::spawn(async move { post(&app, "/held?g=control&mode=regenerate_keeping_data", Some(&c0)).await })
    };
    gate.started.notified().await;
    gate.release.notify_one();
    let late = held.await.unwrap();
    let c1 = pair(&set_cookies(&late)[0]);
    assert_ne!(id_of(&c1), id_of(&c0));
    assert_eq!(get(&app, "/me", Some(&c1)).await.status, 200, "the session carries on under its new identifier");
    assert_eq!(get(&app, "/me", Some(&c0)).await.status, 401, "the old identifier is gone");
}

// ---- item 3: a login through the guard is kept only when the response is not a rejection ----

#[tokio::test]
async fn a_login_guard_followed_by_a_refusing_guard_leaves_no_login() {
    let store = RecordingStore::new();
    let app = app(options().store_arc(store.clone())).await;
    let r = send(&app, "POST", "/login-then-deny", None, &[("x-user", "2")]).await;
    assert_eq!(r.status, 403);
    assert!(set_cookies(&r).is_empty(), "{:?}", set_cookies(&r));
    assert_eq!(store.len(), 0, "no session was created");
}

#[tokio::test]
async fn a_login_guard_followed_by_a_refusing_handler_leaves_no_login() {
    let store = RecordingStore::new();
    let app = app(options().store_arc(store.clone())).await;
    let r = send(&app, "POST", "/login-then-403", None, &[("x-user", "2")]).await;
    assert_eq!(r.status, 403);
    assert!(set_cookies(&r).is_empty());
    assert_eq!(store.len(), 0);
}

#[tokio::test]
async fn a_refused_login_leaves_the_session_the_request_arrived_with() {
    let store = RecordingStore::new();
    let app = app(options().store_arc(store.clone())).await;
    let anon = pair(&set_cookies(&post(&app, "/set?k=cart&v=milk", None).await)[0]);
    // The timestamps move with every request (a touch); the data and the identifier are what must stay.
    let stable = |mut records: std::collections::HashMap<String, Value>| {
        for record in records.values_mut() {
            record.as_object_mut().unwrap().remove("__corvis");
        }
        records
    };
    let before = stable(store.records().await);
    for path in ["/login-then-deny", "/login-then-403"] {
        let r = send(&app, "POST", path, Some(&anon), &[("x-user", "2")]).await;
        assert_eq!(r.status, 403, "{path}");
        assert!(set_cookies(&r).is_empty(), "{path}: {:?}", set_cookies(&r));
        assert_eq!(stable(store.records().await), before, "{path}: the record is as it was");
        let me = get(&app, "/sess", Some(&anon)).await;
        assert_eq!(me.json_value()["data"], json!({ "cart": "milk" }), "{path}: the old cookie still works, with its data");
        assert_eq!(get(&app, "/me", Some(&anon)).await.status, 401, "{path}: and it is not logged in");
    }
}

#[tokio::test]
async fn a_login_a_handler_makes_stands_whatever_the_status() {
    let app = app(options()).await;
    let r = send(&app, "POST", "/handler-login-then-403", None, &[]).await;
    assert_eq!(r.status, 403);
    let cookie = pair(&set_cookies(&r)[0]);
    assert_eq!(get(&app, "/me", Some(&cookie)).await.status, 200);
}

#[tokio::test]
async fn an_accepted_guard_login_still_logs_in() {
    let app = app(options()).await;
    let cookie = login(&app, "2").await;
    assert_eq!(get(&app, "/me", Some(&cookie)).await.status, 200);
}

// ---- item 4: one account cannot evict the sessions of the others ----

#[tokio::test]
async fn an_account_that_logs_in_again_and_again_evicts_only_its_own_sessions() {
    let store = Arc::new(MemorySessionStore::new().max_sessions_per_user(4));
    let app = app(options().store_arc(store.clone())).await;
    let victim = login(&app, "3").await;
    let mut cookies = Vec::new();
    for _ in 0..40 {
        cookies.push(login(&app, "2").await);
    }
    assert_eq!(get(&app, "/me", Some(&victim)).await.status, 200, "another user's session is untouched");
    assert_eq!(store.len(), 5, "four sessions of the flooder and the victim's");
    for old in &cookies[..36] {
        assert_eq!(get(&app, "/me", Some(old)).await.status, 401);
    }
    for recent in &cookies[36..] {
        assert_eq!(get(&app, "/me", Some(recent)).await.status, 200);
    }
}

// ---- item 8: a record over max_session_bytes in a shared store ----

#[tokio::test]
async fn a_stored_record_over_the_limit_is_no_session_and_is_not_deleted() {
    let store = RecordingStore::new();
    let app = app(options().store_arc(store.clone())).await;
    let cookie = login(&app, "2").await;
    let id = id_of(&cookie);
    // Another writer of a shared store saved 200 KB under this identifier.
    let mut record = store.inner.load(&id).await.unwrap().unwrap();
    record["blob"] = json!("x".repeat(200 * 1024));
    assert!(store.inner.update(&id, &record, std::time::Duration::from_secs(3600)).await.unwrap());
    store.take_ops();

    let r = get(&app, "/me", Some(&cookie)).await;
    assert_eq!(r.status, 401, "the record is not a session");
    // Writing with that cookie works and does not adopt the identifier.
    let r = post(&app, "/set?k=a&v=1", Some(&cookie)).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let fresh = pair(&set_cookies(&r)[0]);
    assert_ne!(id_of(&fresh), id);
    assert!(store.inner.load(&id).await.unwrap().is_some(), "the record was not deleted");
    assert!(store.take_ops().iter().all(|op| !(op.op == "destroy" && op.id == id)), "and no destroy was sent for it");
}
