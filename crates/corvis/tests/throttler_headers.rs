//! The rate limit headers `ThrottlerGuard` sends. `replays_the_original_library_fixture` replays
//! `tests/fixtures/throttler/throttler.json`, recorded by running the original throttler library 6.7.1
//! (`tests/fixtures/throttler/generate.mjs`).

use corvis::prelude::*;
use corvis::testing::{Test, TestResponse, request};
use corvis::{
    CorvisApplication, SKIP_THROTTLE, THROTTLE_LIMIT, THROTTLE_TTL, ThrottlerGuard, ThrottlerModule, ThrottlerOptions, ThrottlerStorage,
};

#[controller]
#[use_guards(ThrottlerGuard)]
pub struct LimitedController;

#[routes]
impl LimitedController {
    #[get("plain")]
    fn plain(&self) -> &'static str {
        "ok"
    }

    #[get("skip")]
    #[set_metadata(SKIP_THROTTLE, true)]
    fn skip(&self) -> &'static str {
        "ok"
    }

    #[get("override")]
    #[set_metadata(THROTTLE_LIMIT, 1)]
    fn limit_override(&self) -> &'static str {
        "ok"
    }

    #[get("ttl5")]
    #[set_metadata(THROTTLE_TTL, 5_000)]
    #[set_metadata(THROTTLE_LIMIT, 1)]
    fn ttl5(&self) -> &'static str {
        "ok"
    }

    #[get("block1")]
    #[set_metadata(THROTTLE_LIMIT, 1)]
    #[set_metadata(THROTTLE_BLOCK_DURATION, 1_000)]
    fn block1(&self) -> &'static str {
        "ok"
    }

    #[get("block10")]
    #[set_metadata(THROTTLE_LIMIT, 1)]
    #[set_metadata(THROTTLE_BLOCK_DURATION, 10_000)]
    fn block10(&self) -> &'static str {
        "ok"
    }

    #[get("noblock")]
    #[set_metadata(THROTTLE_LIMIT, 2)]
    #[set_metadata(THROTTLE_BLOCK_DURATION, 0)]
    fn noblock(&self) -> &'static str {
        "ok"
    }

    #[get("closed")]
    #[set_metadata(THROTTLE_LIMIT, 0)]
    #[set_metadata(THROTTLE_BLOCK_DURATION, 1_000)]
    fn closed(&self) -> &'static str {
        "ok"
    }

    #[get("three")]
    #[set_metadata(THROTTLE_LIMIT, 3)]
    fn three(&self) -> &'static str {
        "ok"
    }
}

fn throttler(name: &str, ttl: u64, limit: u32) -> ThrottlerOptions {
    ThrottlerOptions { name: name.into(), ttl, limit }
}

async fn app_with(module: corvis::DynamicModule) -> CorvisApplication {
    let module = Test::create_testing_module().import_dynamic(module).controller::<LimitedController>().compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    app
}

async fn app(throttlers: Vec<ThrottlerOptions>) -> CorvisApplication {
    app_with(ThrottlerModule::for_root(throttlers)).await
}

fn names(r: &TestResponse) -> Vec<String> {
    let mut v: Vec<String> =
        r.headers.keys().map(|k| k.as_str().to_string()).filter(|k| k.starts_with("x-ratelimit") || k.starts_with("retry-after")).collect();
    v.sort();
    v
}

#[corvis::test]
async fn first_request_with_the_default_throttler() {
    let app = app(vec![ThrottlerOptions::new(60_000, 10)]).await;
    let r = request(&app).get("/plain").send().await;
    r.expect_status(200).expect_header("x-ratelimit-limit", "10").expect_header("x-ratelimit-remaining", "9");
    r.expect_header("x-ratelimit-reset", "60");
}

#[corvis::test]
async fn limit_overridden_by_metadata() {
    let app = app(vec![ThrottlerOptions::new(60_000, 10)]).await;
    let r = request(&app).get("/three").send().await;
    r.expect_status(200).expect_header("x-ratelimit-limit", "3").expect_header("x-ratelimit-remaining", "2");
}

#[corvis::test]
async fn named_throttlers_carry_their_name() {
    let app = app(vec![throttler("short", 1_000, 3), throttler("long", 60_000, 100)]).await;
    let r = request(&app).get("/plain").send().await;
    r.expect_status(200)
        .expect_header("x-ratelimit-limit-short", "3")
        .expect_header("x-ratelimit-remaining-short", "2")
        .expect_header("x-ratelimit-reset-short", "1")
        .expect_header("x-ratelimit-limit-long", "100")
        .expect_header("x-ratelimit-remaining-long", "99")
        .expect_header("x-ratelimit-reset-long", "60");
    assert!(r.header("x-ratelimit-limit").is_none());
}

#[corvis::test]
async fn skipped_routes_carry_no_throttler_headers() {
    let app = app(vec![ThrottlerOptions::new(60_000, 10)]).await;
    let r = request(&app).get("/skip").send().await;
    r.expect_status(200);
    assert_eq!(names(&r), Vec::<String>::new());
}

#[corvis::test]
async fn a_blocked_request_carries_retry_after_and_no_rate_limit_headers() {
    let app = app(vec![ThrottlerOptions::new(60_000, 2)]).await;
    let c = request(&app);
    c.get("/plain").send().await.expect_status(200);
    c.get("/plain").send().await.expect_status(200);
    let r = c.get("/plain").send().await;
    r.expect_status(429).expect_json(json!({ "statusCode": 429, "message": "ThrottlerException: Too Many Requests" }));
    let retry: u64 = r.header("retry-after").expect("Retry-After").parse().unwrap();
    assert!((59..=60).contains(&retry), "{retry}");
    assert_eq!(names(&r), ["retry-after"]);
}

#[corvis::test]
async fn headers_of_an_earlier_throttler_stay_on_the_429() {
    let app = app(vec![throttler("short", 60_000, 100), throttler("long", 60_000, 1)]).await;
    let c = request(&app);
    c.get("/plain").send().await.expect_status(200);
    let r = c.get("/plain").send().await;
    r.expect_status(429).expect_header("x-ratelimit-limit-short", "100").expect_header("x-ratelimit-remaining-short", "98");
    assert!(r.header("retry-after-long").is_some());
    assert!(r.header("x-ratelimit-limit-long").is_none());
}

#[corvis::test]
async fn headers_can_be_turned_off_without_changing_the_limit() {
    let storage = ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 1)]).with_headers(false);
    let app = app_with(ThrottlerModule::for_root_storage(storage)).await;
    let c = request(&app);
    let r = c.get("/plain").send().await;
    r.expect_status(200);
    assert_eq!(names(&r), Vec::<String>::new());
    let r = c.get("/plain").send().await;
    r.expect_status(429);
    assert_eq!(names(&r), Vec::<String>::new());
}

#[corvis::test]
async fn throttler_names_that_are_not_header_names_are_dropped() {
    let app = app(vec![throttler("bad name\r\nX-Evil: 1", 60_000, 5)]).await;
    let r = request(&app).get("/plain").send().await;
    r.expect_status(200);
    assert_eq!(names(&r), Vec::<String>::new());
    assert!(r.header("x-evil").is_none());
}

async fn sleep(ms: u64) {
    tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
}

#[corvis::test]
async fn block_default_follows_the_ttl_override() {
    let app = app(vec![ThrottlerOptions::new(60_000, 10)]).await;
    let c = request(&app);
    c.get("/ttl5").send().await.expect_status(200);
    c.get("/ttl5").send().await.expect_status(429).expect_header("retry-after", "5");
}

#[corvis::test]
async fn retry_during_the_block_counts_down_and_does_not_extend_it() {
    let app = app(vec![ThrottlerOptions::new(60_000, 1)]).await;
    let c = request(&app);
    c.get("/block1").send().await.expect_status(200);
    c.get("/block1").send().await.expect_status(429).expect_header("retry-after", "1");
    sleep(1_100).await;
    let r = c.get("/block1").send().await;
    r.expect_status(200).expect_header("x-ratelimit-remaining", "0").expect_header("x-ratelimit-limit", "1");
}

#[corvis::test]
async fn metadata_overrides_the_storage_block_duration() {
    let storage = ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 5)]).with_block_duration("default", 300_000);
    let app = app_with(ThrottlerModule::for_root_storage(storage)).await;
    let c = request(&app);
    c.get("/block10").send().await.expect_status(200);
    c.get("/block10").send().await.expect_status(429).expect_header("retry-after", "10");
}

#[corvis::test]
async fn storage_block_duration_applies_without_metadata() {
    let storage = ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 1)]).with_block_duration("default", 300_000);
    let app = app_with(ThrottlerModule::for_root_storage(storage)).await;
    let c = request(&app);
    c.get("/plain").send().await.expect_status(200);
    c.get("/plain").send().await.expect_status(429).expect_header("retry-after", "300");
}

#[corvis::test]
async fn a_block_duration_of_zero_is_a_sliding_limit() {
    let app = app(vec![ThrottlerOptions::new(2_000, 100)]).await;
    let c = request(&app);
    c.get("/noblock").send().await.expect_status(200);
    c.get("/noblock").send().await.expect_status(200);
    let r = c.get("/noblock").send().await;
    r.expect_status(429);
    let retry: u64 = r.header("retry-after").unwrap().parse().unwrap();
    assert!((1..=2).contains(&retry), "{retry}");
    sleep(2_200).await;
    c.get("/noblock").send().await.expect_status(200);
}

#[corvis::test]
async fn a_limit_of_zero_rejects_every_request() {
    let app = app(vec![ThrottlerOptions::new(60_000, 10)]).await;
    let c = request(&app);
    c.get("/closed").send().await.expect_status(429);
    sleep(1_500).await;
    c.get("/closed").send().await.expect_status(429);
}

#[corvis::test]
async fn throttlers_configured_longest_first_are_checked_shortest_first() {
    let app = app(vec![throttler("long", 60_000, 100), throttler("short", 1_000, 1)]).await;
    let c = request(&app);
    c.get("/plain").send().await.expect_status(200);
    sleep(100).await;
    let r = c.get("/plain").send().await;
    r.expect_status(429);
    assert!(r.header("retry-after-short").is_some());
    assert!(r.header("x-ratelimit-limit-long").is_none(), "{:?}", names(&r));
    sleep(1_200).await;
    let r = c.get("/plain").send().await;
    r.expect_status(200).expect_header("x-ratelimit-remaining-long", "98");
}

/// The new constant and record type come with the prelude.
#[test]
fn prelude_exports_the_block_duration_items() {
    assert_eq!(THROTTLE_BLOCK_DURATION, "THROTTLER:BLOCK_DURATION");
    let _ = std::any::type_name::<ThrottlerRecord>();
}

/// Header names and values of the original throttler library. `Reset` and `Retry-After` are compared within one
/// second; entries with `at` are sent that many milliseconds after the first request of their sequence.
#[corvis::test]
async fn replays_the_original_library_fixture() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/throttler/throttler.json")).unwrap();
    for case in fixture.as_array().unwrap() {
        let case_name = case["name"].as_str().unwrap();
        let throttlers = case["throttlers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| throttler(t["name"].as_str().unwrap(), t["ttl"].as_u64().unwrap(), t["limit"].as_u64().unwrap() as u32))
            .collect();
        let app = app(throttlers).await;
        let c = request(&app);
        let start = std::time::Instant::now();
        for (i, expected) in case["responses"].as_array().unwrap().iter().enumerate() {
            let path = expected["path"].as_str().unwrap();
            if let Some(at) = expected["at"].as_u64() {
                sleep((at as u128).saturating_sub(start.elapsed().as_millis()) as u64).await;
            }
            let r = c.get(path).send().await;
            let at = format!("{case_name} #{i} {path}");
            assert_eq!(r.status as u64, expected["status"].as_u64().unwrap(), "{at}");
            let mut want: Vec<String> = expected["headers"].as_object().unwrap().keys().cloned().collect();
            want.sort();
            assert_eq!(names(&r), want, "{at}: header names");
            for (name, value) in expected["headers"].as_object().unwrap() {
                let got = r.header(name).unwrap();
                let value = value.as_str().unwrap();
                if name.starts_with("retry-after") || name.starts_with("x-ratelimit-reset") {
                    let (got, value): (i64, i64) = (got.parse().unwrap(), value.parse().unwrap());
                    assert!((got - value).abs() <= 1, "{at}: {name} {got} vs {value}");
                } else {
                    assert_eq!(got, value, "{at}: {name}");
                }
            }
        }
    }
}
