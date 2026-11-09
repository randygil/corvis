//! The rate limit headers `ThrottlerGuard` sends. `replays_the_original_library_fixture` replays
//! `tests/fixtures/throttler/throttler.json`, recorded by running the original throttler library 6.7.1
//! (`tests/fixtures/throttler/generate.mjs`).

use corvis::prelude::*;
use corvis::testing::{Test, TestResponse, request};
use corvis::{
    CorvisApplication, SKIP_THROTTLE, THROTTLE_KEY_GENERATOR, THROTTLE_LIMIT, THROTTLE_TRACKER, THROTTLE_TTL, ThrottlerGuard,
    ThrottlerModule, ThrottlerOptions, ThrottlerStorage,
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
    #[apply_decorators(skip_throttle(["default", "short", "long"]))]
    fn skip(&self) -> &'static str {
        "ok"
    }

    #[get("bare")]
    #[set_metadata(SKIP_THROTTLE, true)]
    fn bare(&self) -> &'static str {
        "ok"
    }

    #[get("byname")]
    #[apply_decorators(skip_throttle(["short"]))]
    fn byname(&self) -> &'static str {
        "ok"
    }

    #[get("string-key")]
    #[set_metadata(skip_throttle_key("short"), true)]
    fn string_key(&self) -> &'static str {
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

/// A controller skip that its handler `on` undoes for `default` (`@SkipThrottle()` on the class and
/// `@SkipThrottle({ default: false })` on the handler).
#[controller("undo")]
#[use_guards(ThrottlerGuard)]
#[set_metadata(SKIP_THROTTLE, true)]
pub struct UndoController;

#[routes]
impl UndoController {
    #[get("on")]
    #[set_metadata(SKIP_THROTTLE, false)]
    fn on(&self) -> &'static str {
        "ok"
    }

    #[get("off")]
    fn off(&self) -> &'static str {
        "ok"
    }

    #[get("short-on")]
    #[set_metadata(skip_throttle_key("default"), false)]
    fn short_on(&self) -> &'static str {
        "ok"
    }
}

/// A controller that skips `short` while its handler has a bare skip: each name is resolved on its own.
#[controller("mixed")]
#[use_guards(ThrottlerGuard)]
#[apply_decorators(skip_throttle(["short"]))]
pub struct MixedController;

#[routes]
impl MixedController {
    #[get("h")]
    #[set_metadata(SKIP_THROTTLE, true)]
    fn h(&self) -> &'static str {
        "ok"
    }
}

/// Per-name overrides (#156): the routes of the fixture cases `overrides-named`, `overrides-default`, `module-tracker` and
/// `module-key`, plus the ones that only Corvis has (negative and text values, an unregistered tracker).
#[controller("ovr")]
#[use_guards(ThrottlerGuard)]
pub struct OvrController;

#[routes]
impl OvrController {
    #[get("short1")]
    #[apply_decorators(throttle("short").limit(1))]
    fn short1(&self) -> &'static str {
        "ok"
    }

    #[get("both")]
    #[apply_decorators(throttle("short").limit(2).ttl(10_000), throttle("long").limit(1))]
    fn both(&self) -> &'static str {
        "ok"
    }

    #[get("block")]
    #[apply_decorators(throttle("short").limit(1).block_duration(5_000))]
    fn block(&self) -> &'static str {
        "ok"
    }

    #[get("unknown")]
    #[apply_decorators(throttle("shrot").limit(1))]
    fn unknown(&self) -> &'static str {
        "ok"
    }

    /// The bare key is the `default` throttler's entry.
    #[get("default")]
    #[set_metadata(THROTTLE_LIMIT, 1)]
    fn default_limit(&self) -> &'static str {
        "ok"
    }

    #[get("invalid")]
    #[set_metadata(throttle_key(THROTTLE_LIMIT, "short"), "abc")]
    fn invalid(&self) -> &'static str {
        "ok"
    }

    #[get("negative")]
    #[set_metadata(THROTTLE_LIMIT, -1)]
    fn negative(&self) -> &'static str {
        "ok"
    }

    #[get("fraction")]
    #[set_metadata(THROTTLE_TTL, 1.5)]
    fn fraction(&self) -> &'static str {
        "ok"
    }

    #[get("fixed")]
    #[apply_decorators(throttle("default").tracker("fixed"))]
    fn fixed(&self) -> &'static str {
        "ok"
    }

    #[get("own-key")]
    #[apply_decorators(throttle("default").key_generator("own"))]
    fn own_key(&self) -> &'static str {
        "ok"
    }

    #[get("missing-tracker")]
    #[apply_decorators(throttle("default").tracker("missing"))]
    fn missing_tracker(&self) -> &'static str {
        "ok"
    }

    #[get("number-tracker")]
    #[set_metadata(throttle_key(THROTTLE_TRACKER, "default"), 5)]
    fn number_tracker(&self) -> &'static str {
        "ok"
    }

    #[get("bad-key-generator")]
    #[set_metadata(throttle_key(THROTTLE_KEY_GENERATOR, "default"), "nope")]
    fn bad_key_generator(&self) -> &'static str {
        "ok"
    }

    /// A per-name entry beats the bare key on the same target.
    #[get("bare-and-named")]
    #[set_metadata(THROTTLE_LIMIT, 1)]
    #[set_metadata(throttle_key(THROTTLE_LIMIT, "default"), 2)]
    fn bare_and_named(&self) -> &'static str {
        "ok"
    }
}

/// The class overrides `short`; `h` overrides only the limit; `sibling` has no override; `nulled` sets `null`.
#[controller("ovr-class")]
#[use_guards(ThrottlerGuard)]
#[apply_decorators(throttle("short").limit(2).ttl(20_000))]
pub struct OvrClassController;

#[routes]
impl OvrClassController {
    #[get("h")]
    #[apply_decorators(throttle("short").limit(1))]
    fn h(&self) -> &'static str {
        "ok"
    }

    #[get("sibling")]
    fn sibling(&self) -> &'static str {
        "ok"
    }

    #[get("nulled")]
    #[set_metadata(throttle_key(THROTTLE_LIMIT, "short"), Option::<u32>::None)]
    fn nulled(&self) -> &'static str {
        "ok"
    }
}

fn throttler(name: &str, ttl: u64, limit: u32) -> ThrottlerOptions {
    ThrottlerOptions { name: name.into(), ttl, limit }
}

async fn app_with(module: corvis::DynamicModule) -> CorvisApplication {
    let module = Test::create_testing_module()
        .import_dynamic(module)
        .controller::<LimitedController>()
        .controller::<UndoController>()
        .controller::<MixedController>()
        .controller::<OvrController>()
        .controller::<OvrClassController>()
        .compile()
        .await
        .unwrap();
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    app
}

async fn app(throttlers: Vec<ThrottlerOptions>) -> CorvisApplication {
    app_with(ThrottlerModule::for_root(throttlers)).await
}

/// The named hooks every override test registers: only routes that select them use them.
fn hooks(storage: ThrottlerStorage) -> ThrottlerStorage {
    storage
        .with_named_tracker("fixed", |_| async { Ok("fixed".to_string()) })
        .with_named_key_generator("own", |_, tracker, _| format!("own:{tracker}"))
}

/// The tracker of the fixture case `module-tracker`: the `x-client` header, else the IP.
async fn client_header(ctx: ExecutionContext) -> Result<String, HttpException> {
    let request = ctx.request();
    Ok(request.header("x-client").map(str::to_string).unwrap_or_else(|| request.ip()))
}

async fn app_hooks(throttlers: Vec<ThrottlerOptions>) -> CorvisApplication {
    app_with(ThrottlerModule::for_root_storage(hooks(ThrottlerStorage::new(throttlers)))).await
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

#[corvis::test]
async fn bare_skip_with_the_default_throttler_only() {
    let app = app(vec![ThrottlerOptions::new(60_000, 10)]).await;
    let r = request(&app).get("/bare").send().await;
    r.expect_status(200);
    assert_eq!(names(&r), Vec::<String>::new());
}

#[corvis::test]
async fn bare_skip_next_to_a_named_throttler_only_skips_default() {
    let app = app(vec![ThrottlerOptions::new(60_000, 10), throttler("short", 1_000, 3)]).await;
    let r = request(&app).get("/bare").send().await;
    r.expect_status(200).expect_header("x-ratelimit-limit-short", "3").expect_header("x-ratelimit-remaining-short", "2");
    assert!(r.header("x-ratelimit-limit").is_none());
}

#[corvis::test]
async fn bare_skip_with_named_throttlers_only_still_limits() {
    let app = app(vec![throttler("short", 30_000, 3), throttler("long", 60_000, 5)]).await;
    let c = request(&app);
    for _ in 0..3 {
        let r = c.get("/bare").send().await;
        r.expect_status(200).expect_header("x-ratelimit-limit-short", "3").expect_header("x-ratelimit-limit-long", "5");
    }
    let r = c.get("/bare").send().await;
    r.expect_status(429);
    assert!(r.header("retry-after-short").is_some());
}

#[corvis::test]
async fn skip_by_name_counts_only_the_other_throttlers() {
    let app = app(vec![throttler("short", 30_000, 3), throttler("long", 60_000, 5)]).await;
    let c = request(&app);
    for i in 0..5 {
        let r = c.get("/byname").send().await;
        r.expect_status(200).expect_header("x-ratelimit-limit-long", "5").expect_header("x-ratelimit-remaining-long", &(4 - i).to_string());
        assert!(r.header("x-ratelimit-limit-short").is_none());
    }
    c.get("/byname").send().await.expect_status(429);
}

#[corvis::test]
async fn a_string_key_works_on_a_route() {
    let app = app(vec![throttler("short", 60_000, 3), throttler("long", 60_000, 5)]).await;
    let r = request(&app).get("/string-key").send().await;
    r.expect_status(200).expect_header("x-ratelimit-limit-long", "5");
    assert!(r.header("x-ratelimit-limit-short").is_none());
}

#[corvis::test]
async fn a_handler_undoes_the_controllers_skip() {
    let app = app(vec![ThrottlerOptions::new(60_000, 10)]).await;
    let c = request(&app);
    c.get("/undo/on").send().await.expect_status(200).expect_header("x-ratelimit-limit", "10");
    let r = c.get("/undo/off").send().await;
    r.expect_status(200);
    assert_eq!(names(&r), Vec::<String>::new());
}

#[corvis::test]
async fn a_per_name_false_undoes_only_that_name() {
    let app = app(vec![ThrottlerOptions::new(60_000, 10), throttler("short", 60_000, 3)]).await;
    let r = request(&app).get("/undo/short-on").send().await;
    r.expect_status(200).expect_header("x-ratelimit-limit", "10").expect_header("x-ratelimit-limit-short", "3");
}

#[corvis::test]
async fn controller_and_handler_skip_different_throttlers() {
    let app = app(vec![ThrottlerOptions::new(60_000, 1), throttler("short", 60_000, 1)]).await;
    let c = request(&app);
    for _ in 0..3 {
        let r = c.get("/mixed/h").send().await;
        r.expect_status(200);
        assert_eq!(names(&r), Vec::<String>::new());
    }
}

#[test]
fn skip_helpers_use_the_original_key_spelling() {
    assert_eq!(skip_throttle_key("short"), "THROTTLER:SKIPshort");
    let d = skip_throttle(["short", "long"]);
    assert_eq!(d.entries(), [("THROTTLER:SKIPshort".to_string(), json!(true)), ("THROTTLER:SKIPlong".to_string(), json!(true))].as_slice());
    assert!(skip_throttle(Vec::<String>::new()).is_empty());
}

/// The new constant and record type come with the prelude.
#[test]
fn prelude_exports_the_block_duration_items() {
    assert_eq!(THROTTLE_BLOCK_DURATION, "THROTTLER:BLOCK_DURATION");
    let _ = std::any::type_name::<ThrottlerRecord>();
}

fn short_long() -> Vec<ThrottlerOptions> {
    vec![throttler("short", 30_000, 3), throttler("long", 60_000, 5)]
}

#[corvis::test]
async fn an_override_changes_one_throttler_only() {
    let app = app_hooks(short_long()).await;
    let c = request(&app);
    let r = c.get("/ovr/short1").send().await;
    r.expect_status(200).expect_header("x-ratelimit-limit-short", "1").expect_header("x-ratelimit-limit-long", "5");
    let r = c.get("/ovr/short1").send().await;
    r.expect_status(429).expect_header("retry-after-short", "30");
    assert!(r.header("retry-after-long").is_none());
}

#[corvis::test]
async fn two_throttlers_overridden_on_one_route() {
    let app = app_hooks(short_long()).await;
    let c = request(&app);
    let r = c.get("/ovr/both").send().await;
    r.expect_status(200)
        .expect_header("x-ratelimit-limit-short", "2")
        .expect_header("x-ratelimit-reset-short", "10")
        .expect_header("x-ratelimit-limit-long", "1");
    c.get("/ovr/both").send().await.expect_status(429).expect_header("retry-after-long", "60");
}

#[corvis::test]
async fn a_partial_handler_override_keeps_the_controllers_other_options() {
    let app = app_hooks(short_long()).await;
    let c = request(&app);
    let r = c.get("/ovr-class/h").send().await;
    r.expect_status(200).expect_header("x-ratelimit-limit-short", "1").expect_header("x-ratelimit-reset-short", "20");
    let r = c.get("/ovr-class/sibling").send().await;
    r.expect_status(200).expect_header("x-ratelimit-limit-short", "2").expect_header("x-ratelimit-reset-short", "20");
    assert_eq!(r.header("x-ratelimit-limit-long"), Some("5"), "long is not overridden");
}

#[corvis::test]
async fn null_counts_as_absent() {
    let app = app_hooks(short_long()).await;
    let r = request(&app).get("/ovr-class/nulled").send().await;
    r.expect_status(200).expect_header("x-ratelimit-limit-short", "2");
}

#[corvis::test]
async fn a_block_duration_override_applies_to_one_throttler() {
    let app = app_hooks(short_long()).await;
    let c = request(&app);
    c.get("/ovr/block").send().await.expect_status(200);
    c.get("/ovr/block").send().await.expect_status(429).expect_header("retry-after-short", "5");
}

#[corvis::test]
async fn a_bare_limit_next_to_a_named_throttler_only_changes_default() {
    let app = app(vec![ThrottlerOptions::new(60_000, 3), throttler("short", 30_000, 3)]).await;
    let c = request(&app);
    let r = c.get("/override").send().await;
    r.expect_status(200).expect_header("x-ratelimit-limit", "1").expect_header("x-ratelimit-limit-short", "3");
    c.get("/override").send().await.expect_status(429).expect_header("retry-after", "60");
}

#[corvis::test]
async fn a_bare_limit_with_named_throttlers_only_overrides_nothing() {
    let app = app(short_long()).await;
    let r = request(&app).get("/ovr/default").send().await;
    r.expect_status(200).expect_header("x-ratelimit-limit-short", "3").expect_header("x-ratelimit-limit-long", "5");
}

#[corvis::test]
async fn a_per_name_entry_wins_over_the_bare_key_on_the_same_target() {
    let app = app(vec![ThrottlerOptions::new(60_000, 10)]).await;
    request(&app).get("/ovr/bare-and-named").send().await.expect_status(200).expect_header("x-ratelimit-limit", "2");
}

#[corvis::test]
async fn invalid_override_values_fail_closed() {
    let paths = ["/ovr/invalid", "/ovr/negative", "/ovr/fraction", "/ovr/number-tracker", "/ovr/bad-key-generator", "/ovr/missing-tracker"];
    for path in paths {
        let app = app_hooks(vec![ThrottlerOptions::new(60_000, 10), throttler("short", 30_000, 3)]).await;
        let r = request(&app).get(path).send().await;
        r.expect_status(500).expect_json(json!({ "statusCode": 500, "message": "Internal server error" }));
        assert!(r.header("x-ratelimit-limit").is_none(), "{path}: no counted hit is reported");
    }
}

#[corvis::test]
async fn a_tracker_from_a_header_separates_clients() {
    let storage = hooks(ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 1)])).with_tracker(client_header);
    let app = app_with(ThrottlerModule::for_root_storage(storage)).await;
    let c = request(&app);
    c.get("/plain").header("x-client", "a").send().await.expect_status(200);
    c.get("/plain").header("x-client", "a").send().await.expect_status(429);
    c.get("/plain").header("x-client", "b").send().await.expect_status(200);
}

#[corvis::test]
async fn a_key_generator_can_share_a_counter_between_routes() {
    let storage =
        ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 1)]).with_key_generator(|_, tracker, name| format!("{name}:{tracker}"));
    let app = app_with(ThrottlerModule::for_root_storage(storage)).await;
    let c = request(&app);
    c.get("/plain").send().await.expect_status(200);
    c.get("/byname").send().await.expect_status(429);
}

#[corvis::test]
async fn a_tracker_error_is_returned_unchanged_and_counts_no_hit() {
    let storage = ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 1)]).with_tracker(|ctx| async move {
        ctx.request().header("x-client").map(str::to_string).ok_or_else(|| HttpException::new("no client id", 400))
    });
    let app = app_with(ThrottlerModule::for_root_storage(storage)).await;
    let c = request(&app);
    for _ in 0..3 {
        c.get("/plain").send().await.expect_status(400);
    }
    c.get("/plain").header("x-client", "a").send().await.expect_status(200);
}

#[corvis::test]
async fn a_route_tracker_beats_the_module_tracker() {
    let storage = hooks(ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 1)])).with_tracker(client_header);
    let app = app_with(ThrottlerModule::for_root_storage(storage)).await;
    let c = request(&app);
    c.get("/ovr/fixed").header("x-client", "a").send().await.expect_status(200);
    c.get("/ovr/fixed").header("x-client", "b").send().await.expect_status(429);
}

#[corvis::test]
async fn a_route_key_generator_beats_the_module_generator() {
    let storage = hooks(ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 1)]))
        .with_key_generator(|_, tracker, name| format!("{name}:{tracker}"));
    let app = app_with(ThrottlerModule::for_root_storage(storage)).await;
    let c = request(&app);
    c.get("/plain").send().await.expect_status(200);
    c.get("/byname").send().await.expect_status(429);
    c.get("/ovr/own-key").send().await.expect_status(200);
}

/// The divergence of design.md, Decision 8: a key generator that returns one key for two throttlers does not make them
/// share a window or a block.
#[corvis::test]
async fn counters_stay_separate_per_throttler() {
    let storage = ThrottlerStorage::new(vec![throttler("short", 30_000, 100), throttler("long", 60_000, 2)])
        .with_key_generator(|_, tracker, _| tracker.to_string());
    let app = app_with(ThrottlerModule::for_root_storage(storage)).await;
    let c = request(&app);
    for remaining in [99, 98] {
        let r = c.get("/plain").send().await;
        r.expect_status(200)
            .expect_header("x-ratelimit-reset-short", "30")
            .expect_header("x-ratelimit-remaining-short", &remaining.to_string());
    }
    for remaining in [97, 96] {
        let r = c.get("/plain").send().await;
        r.expect_status(429)
            .expect_header("x-ratelimit-reset-short", "30")
            .expect_header("x-ratelimit-remaining-short", &remaining.to_string());
        assert!(r.header("retry-after-long").is_some(), "long blocks");
        assert!(r.header("retry-after-short").is_none(), "short does not");
    }
}

#[test]
fn override_helpers_come_with_the_prelude() {
    assert_eq!(throttle_key(THROTTLE_LIMIT, "short"), "THROTTLER:LIMITshort");
    assert_eq!(throttle_key(THROTTLE_KEY_GENERATOR, "short"), "THROTTLER:KEY_GENERATORshort");
    let d: corvis::common::decorator::CustomDecorator = throttle("short").limit(1).into();
    assert_eq!(d.entries(), [("THROTTLER:LIMITshort".to_string(), json!(1))].as_slice());
}

/// The storage of a fixture case: the functions of its module options cannot be recorded, so each case name selects them.
fn storage_for_case(case: &str, throttlers: Vec<ThrottlerOptions>) -> ThrottlerStorage {
    let storage = hooks(ThrottlerStorage::new(throttlers));
    match case {
        "module-tracker" => storage.with_tracker(client_header),
        "module-key" => storage.with_key_generator(|_, tracker, name| format!("{name}:{tracker}")),
        "shared-key" => storage.with_key_generator(|_, tracker, _| tracker.to_string()),
        _ => storage,
    }
}

/// Recorded cases where Corvis differs on purpose: a key generator that returns one key for two throttlers makes the
/// original share the reporting window and the block between them; Corvis keeps the counters separate (design.md,
/// Decision 8). The replay asserts that these cases do differ from the recording; Corvis's own values are checked by
/// `counters_stay_separate_per_throttler`.
const KNOWN_DIVERGENCES: [&str; 1] = ["shared-key"];

/// Whether a response has the status and the throttler headers of a recorded one (time values within one second).
fn matches_recording(r: &TestResponse, expected: &Value) -> bool {
    let mut want: Vec<String> = expected["headers"].as_object().unwrap().keys().cloned().collect();
    want.sort();
    r.status as u64 == expected["status"].as_u64().unwrap()
        && names(r) == want
        && expected["headers"].as_object().unwrap().iter().all(|(name, value)| {
            let (got, value) = (r.header(name).unwrap(), value.as_str().unwrap());
            if name.starts_with("retry-after") || name.starts_with("x-ratelimit-reset") {
                (got.parse::<i64>().unwrap() - value.parse::<i64>().unwrap()).abs() <= 1
            } else {
                got == value
            }
        })
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
        let app = app_with(ThrottlerModule::for_root_storage(storage_for_case(case_name, throttlers))).await;
        let c = request(&app);
        let start = std::time::Instant::now();
        let mut differences = 0;
        for (i, expected) in case["responses"].as_array().unwrap().iter().enumerate() {
            let path = expected["path"].as_str().unwrap();
            if let Some(at) = expected["at"].as_u64() {
                sleep((at as u128).saturating_sub(start.elapsed().as_millis()) as u64).await;
            }
            let mut req = c.get(path);
            for (name, value) in expected["send"].as_object().into_iter().flatten() {
                req = req.header(name, value.as_str().unwrap());
            }
            let r = req.send().await;
            let at = format!("{case_name} #{i} {path}");
            if KNOWN_DIVERGENCES.contains(&case_name) {
                differences += usize::from(!matches_recording(&r, expected));
                continue;
            }
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
        if KNOWN_DIVERGENCES.contains(&case_name) {
            assert!(differences > 0, "{case_name} is a known divergence but every response matched the recording");
        }
    }
}
