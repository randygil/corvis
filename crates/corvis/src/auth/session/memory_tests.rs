use std::time::Duration;

use serde_json::{Value, json};

use super::*;

fn record(user: Option<&str>, pad: usize) -> Value {
    let mut r = json!({ "cookie": { "originalMaxAge": null }, "pad": "x".repeat(pad), "__corvis": { "createdAt": 1, "lastAccess": 1 } });
    if let Some(user) = user {
        r["passport"] = json!({ "user": user });
    }
    r
}

const TTL: Duration = Duration::from_secs(3600);

#[tokio::test]
async fn the_number_of_sessions_is_bounded_and_the_most_recent_are_kept() {
    let store = MemorySessionStore::new().max_sessions(100);
    for i in 0..10_000 {
        store.create(&format!("id{i}"), &record(None, 0), TTL).await.unwrap();
        assert!(store.len() <= 100);
    }
    assert_eq!(store.len(), 100);
    for i in 9_900..10_000 {
        assert!(store.load(&format!("id{i}")).await.unwrap().is_some(), "id{i}");
    }
    assert!(store.load("id9899").await.unwrap().is_none());
}

#[tokio::test]
async fn the_bytes_are_bounded() {
    let store = MemorySessionStore::new().max_total_bytes(1 << 20);
    for i in 0..500 {
        store.create(&format!("id{i}"), &record(None, 10 * 1024), TTL).await.unwrap();
        assert!(store.total_bytes() <= 1 << 20, "{}", store.total_bytes());
    }
    assert!(store.len() < 110 && store.len() > 50, "{}", store.len());
    // Removing records gives the bytes back.
    for i in 0..500 {
        store.destroy(&format!("id{i}")).await.unwrap();
    }
    assert_eq!(store.total_bytes(), 0);
    // A record that can never fit is an error, not an eviction of everything.
    store.create("keep", &record(None, 10), TTL).await.unwrap();
    let tiny = MemorySessionStore::new().max_total_bytes(500);
    assert!(tiny.create("big", &record(None, 5000), TTL).await.is_err());
    assert!(store.load("keep").await.unwrap().is_some());
}

#[tokio::test(start_paused = true)]
async fn expired_records_leave_without_being_loaded() {
    let store = MemorySessionStore::new();
    for i in 0..10 {
        store.create(&format!("old{i}"), &record(None, 0), Duration::from_secs(60)).await.unwrap();
    }
    assert_eq!(store.len(), 10);
    tokio::time::advance(Duration::from_secs(61)).await;
    store.create("fresh", &record(None, 0), TTL).await.unwrap();
    assert_eq!(store.len(), 1, "the writes purged the expired records");
    // And one that is loaded after expiry is gone as well.
    store.create("short", &record(None, 0), Duration::from_secs(1)).await.unwrap();
    tokio::time::advance(Duration::from_secs(2)).await;
    assert!(store.load("short").await.unwrap().is_none());
}

#[tokio::test(start_paused = true)]
async fn an_expired_record_cannot_be_updated_but_its_identifier_can_be_reused() {
    let store = MemorySessionStore::new();
    store.create("a", &record(None, 0), Duration::from_secs(1)).await.unwrap();
    tokio::time::advance(Duration::from_secs(2)).await;
    assert!(!store.update("a", &record(None, 0), TTL).await.unwrap());
    assert!(store.load("a").await.unwrap().is_none());
    store.create("a", &record(None, 0), TTL).await.unwrap();
}

#[tokio::test]
async fn create_never_overwrites_and_update_never_creates() {
    let store = MemorySessionStore::new();
    store.create("a", &record(Some("1"), 0), TTL).await.unwrap();
    assert!(matches!(store.create("a", &record(Some("2"), 0), TTL).await, Err(SessionStoreError::Exists)));
    assert_eq!(store.load("a").await.unwrap().unwrap()["passport"]["user"], "1");
    assert!(!store.update("missing", &record(None, 0), TTL).await.unwrap());
    assert!(!store.touch("missing", &record(None, 0), TTL).await.unwrap());
    assert_eq!(store.len(), 1, "only the record created above");
    store.destroy("a").await.unwrap();
    store.destroy("a").await.unwrap();
    assert!(!store.update("a", &record(None, 0), TTL).await.unwrap(), "a destroyed session stays destroyed");
    assert!(store.is_empty());
}

#[tokio::test]
async fn touch_renews_the_timestamps_and_keeps_the_data() {
    let store = MemorySessionStore::new();
    store.create("a", &record(None, 0), TTL).await.unwrap();
    let mut newer = record(None, 0);
    newer["pad"] = json!("changed by a request that did not save");
    newer["__corvis"]["lastAccess"] = json!(99);
    assert!(store.touch("a", &newer, TTL).await.unwrap());
    let held = store.load("a").await.unwrap().unwrap();
    assert_eq!(held["pad"], "");
    assert_eq!(held["__corvis"]["lastAccess"], 99);
    // A record written by the original has no timestamps: touch adds them.
    let original = json!({ "cookie": { "originalMaxAge": null }, "views": 1 });
    store.create("b", &original, TTL).await.unwrap();
    assert!(store.touch("b", &newer, TTL).await.unwrap());
    assert_eq!(store.load("b").await.unwrap().unwrap()["__corvis"]["lastAccess"], 99);
}

#[tokio::test]
async fn a_flood_of_anonymous_sessions_does_not_evict_logged_in_users() {
    let store = MemorySessionStore::new().max_sessions(10);
    for i in 0..5 {
        store.create(&format!("user{i}"), &record(Some("7"), 0), TTL).await.unwrap();
    }
    for i in 0..10_000 {
        store.create(&format!("anon{i}"), &record(None, 0), TTL).await.unwrap();
    }
    for i in 0..5 {
        assert!(store.load(&format!("user{i}")).await.unwrap().is_some(), "user{i} was evicted");
    }
    assert_eq!(store.len(), 10);
    // When only users are left, a new session still gets in: the oldest user goes.
    let full = MemorySessionStore::new().max_sessions(2);
    full.create("u1", &record(Some("1"), 0), TTL).await.unwrap();
    full.create("u2", &record(Some("2"), 0), TTL).await.unwrap();
    full.create("u3", &record(Some("3"), 0), TTL).await.unwrap();
    assert!(full.load("u1").await.unwrap().is_none());
    assert!(full.load("u3").await.unwrap().is_some());
}

#[tokio::test]
async fn a_load_makes_a_session_recent() {
    let store = MemorySessionStore::new().max_sessions(3);
    for id in ["a", "b", "c"] {
        store.create(id, &record(None, 0), TTL).await.unwrap();
    }
    store.load("a").await.unwrap();
    store.create("d", &record(None, 0), TTL).await.unwrap();
    assert!(store.load("a").await.unwrap().is_some());
    assert!(store.load("b").await.unwrap().is_none(), "the least recently used one was evicted");
}

#[test]
fn bounds_of_zero_are_refused_at_startup() {
    assert!(MemorySessionStore::new().max_sessions(0).validate().is_err());
    assert!(MemorySessionStore::new().max_total_bytes(0).validate().is_err());
    assert!(MemorySessionStore::new().validate().is_ok());
    assert!(MemorySessionStore::new().max_sessions_per_user(0).validate().is_err());
    assert!(MemorySessionStore::new().max_sessions_per_user(1).validate().is_ok());
}

#[tokio::test]
async fn destroy_says_whether_there_was_a_record() {
    let store = MemorySessionStore::new();
    store.create("a", &record(None, 0), TTL).await.unwrap();
    assert!(store.destroy("a").await.unwrap());
    assert!(!store.destroy("a").await.unwrap());
    assert!(!store.destroy("never").await.unwrap());
}

#[tokio::test(start_paused = true)]
async fn destroy_does_not_count_an_expired_record_as_present() {
    let store = MemorySessionStore::new();
    store.create("a", &record(None, 0), Duration::from_secs(1)).await.unwrap();
    tokio::time::advance(Duration::from_secs(2)).await;
    assert!(!store.destroy("a").await.unwrap());
}

#[tokio::test]
async fn a_record_comes_back_as_it_was_written() {
    let store = MemorySessionStore::new();
    let original = json!({ "cookie": { "originalMaxAge": 5 }, "n": [1, 2, { "k": "v" }], "passport": { "user": { "id": "7" } } });
    store.create("a", &original, TTL).await.unwrap();
    assert_eq!(store.load("a").await.unwrap().unwrap(), original);
}

#[tokio::test]
async fn a_user_over_the_cap_evicts_its_own_oldest_session() {
    let store = MemorySessionStore::new().max_sessions_per_user(3);
    store.create("other", &record(Some("9"), 0), TTL).await.unwrap();
    for i in 0..10 {
        store.create(&format!("u{i}"), &record(Some("1"), 0), TTL).await.unwrap();
    }
    assert!(store.load("other").await.unwrap().is_some(), "another user's session is never evicted by the cap");
    for i in 0..7 {
        assert!(store.load(&format!("u{i}")).await.unwrap().is_none(), "u{i}");
    }
    for i in 7..10 {
        assert!(store.load(&format!("u{i}")).await.unwrap().is_some(), "u{i}");
    }
    assert_eq!(store.len(), 4);
}

#[tokio::test]
async fn the_cap_follows_use_not_creation() {
    let store = MemorySessionStore::new().max_sessions_per_user(2);
    store.create("a", &record(Some("1"), 0), TTL).await.unwrap();
    store.create("b", &record(Some("1"), 0), TTL).await.unwrap();
    store.load("a").await.unwrap();
    store.create("c", &record(Some("1"), 0), TTL).await.unwrap();
    assert!(store.load("b").await.unwrap().is_none(), "b was the least recently used");
    assert!(store.load("a").await.unwrap().is_some());
}

#[tokio::test]
async fn an_update_does_not_count_twice_against_the_cap() {
    let store = MemorySessionStore::new().max_sessions_per_user(2);
    store.create("a", &record(Some("1"), 0), TTL).await.unwrap();
    store.create("b", &record(Some("1"), 0), TTL).await.unwrap();
    for _ in 0..5 {
        assert!(store.update("a", &record(Some("1"), 5), TTL).await.unwrap());
        assert!(store.touch("b", &record(Some("1"), 0), TTL).await.unwrap());
    }
    assert_eq!(store.len(), 2);
}

#[tokio::test]
async fn a_full_store_evicts_anonymous_sessions_then_the_user_with_the_most_sessions() {
    let store = MemorySessionStore::new().max_sessions(7);
    store.create("anon", &record(None, 0), TTL).await.unwrap();
    for (id, user) in [("a1", "1"), ("a2", "1"), ("a3", "1"), ("b1", "2"), ("b2", "2"), ("c1", "3")] {
        store.create(id, &record(Some(user), 0), TTL).await.unwrap();
    }
    assert_eq!(store.len(), 7);
    // Full: the anonymous session goes first.
    store.create("c2", &record(Some("3"), 0), TTL).await.unwrap();
    assert!(store.load("anon").await.unwrap().is_none());
    assert_eq!(store.len(), 7);
    // Then the oldest session of the user with the most sessions (user 1 has three).
    store.create("d1", &record(Some("4"), 0), TTL).await.unwrap();
    assert!(store.load("a1").await.unwrap().is_none());
    for id in ["a2", "a3", "b1", "b2", "c1", "c2", "d1"] {
        assert!(store.load(id).await.unwrap().is_some(), "{id}");
    }
    store.create("e1", &record(Some("5"), 0), TTL).await.unwrap();
    assert_eq!(store.len(), 7);
    assert!(store.load("e1").await.unwrap().is_some());
}

#[tokio::test]
async fn destroying_every_session_of_a_user_leaves_no_trace() {
    let store = MemorySessionStore::new().max_sessions_per_user(2);
    for i in 0..50 {
        let id = format!("s{i}");
        store.create(&id, &record(Some(&format!("{i}")), 0), TTL).await.unwrap();
        assert!(store.destroy(&id).await.unwrap());
    }
    assert!(store.is_empty());
    assert_eq!(store.total_bytes(), 0);
    let inner = store.lock();
    assert!(inner.by_user.is_empty() && inner.ranking.is_empty() && inner.anonymous.is_empty() && inner.expiry.is_empty());
}

#[tokio::test]
async fn touch_changes_the_timestamps_and_the_bytes_stay_accounted() {
    let store = MemorySessionStore::new();
    store.create("a", &record(Some("1"), 100), TTL).await.unwrap();
    let before = store.total_bytes();
    let mut newer = record(Some("1"), 100);
    newer["__corvis"]["lastAccess"] = json!(1_794_312_552_408_i64);
    assert!(store.touch("a", &newer, TTL).await.unwrap());
    assert!(store.total_bytes() >= before && store.total_bytes() <= before + 16);
    assert_eq!(store.len(), 1);
}
