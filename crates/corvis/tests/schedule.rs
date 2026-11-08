//! Scheduling + `cron` parity: CronTime parsing/next dates, CronJob timing semantics,
//! SchedulerRegistry API and decorated handlers (`#[cron]` options, names, lifecycle).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use corvis::prelude::*;
use corvis::schedule::{CronJobParams, CronTime, ScheduleModuleOptions, TimerRef, set_interval, set_timeout};
use corvis::testing::Test;

#[tokio::test]
async fn every_second_job_ticks_and_stops() {
    let n = Arc::new(AtomicUsize::new(0));
    let c = n.clone();
    let job = CronJob::from(CronJobParams::new("* * * * * *", move || {
        let c = c.clone();
        async move {
            c.fetch_add(1, Ordering::SeqCst);
        }
    }).start(true).name("sec")).unwrap();
    assert!(job.is_active());
    let next = job.next_date().unwrap();
    assert!(next.timestamp_millis() - chrono::Utc::now().timestamp_millis() <= 1000);
    assert_eq!(job.next_dates(3).unwrap().len(), 3);
    tokio::time::sleep(Duration::from_millis(2300)).await;
    let ticks = n.load(Ordering::SeqCst);
    assert!((2..=3).contains(&ticks), "ticks = {ticks}");
    assert!(job.last_date().is_some());
    job.stop();
    assert!(!job.is_active());
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(n.load(Ordering::SeqCst), ticks);
}

// The first tick falls on the next whole second, so it comes between 0 and 1 s after `start`
// (`t0`), then every second. The job runs for 4.5 s and the observation window is 3.5 s:
//  - at least three ticks (t0, t0 + 1, t0 + 2 <= 3.0 s) fall inside the window, whatever `t0` is;
//  - the first run is still going at the end of the window (it ends at t0 + 4.5 >= 4.5 s), so with
//    `wait_for_completion` every tick after the first must have been skipped (exactly one start),
//    and without it the overlapping ticks must have started their own runs (at least three).
#[tokio::test]
async fn wait_for_completion_skips_overlapping_ticks_and_default_overlaps() {
    for wait in [true, false] {
        let started = Arc::new(AtomicUsize::new(0));
        let s = started.clone();
        let job = CronJob::from(
            CronJobParams::new("* * * * * *", move || {
                let s = s.clone();
                async move {
                    s.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(4500)).await;
                }
            })
            .wait_for_completion(wait)
            .start(true),
        )
        .unwrap();
        tokio::time::sleep(Duration::from_millis(3500)).await;
        job.stop();
        let got = started.load(Ordering::SeqCst);
        if wait { assert_eq!(got, 1, "wait") } else { assert!(got >= 3, "overlap: {got}") }
    }
}

#[tokio::test]
async fn one_shot_date_run_on_init_on_complete_and_errors() {
    let fired = Arc::new(AtomicUsize::new(0));
    let f = fired.clone();
    let at = chrono::Utc::now() + chrono::Duration::milliseconds(600);
    let job = CronJob::from(CronJobParams::new(at, move || {
        let f = f.clone();
        async move { f.fetch_add(1, Ordering::SeqCst); }
    }).start(true)).unwrap();
    assert!(job.run_once());
    assert_eq!(job.next_dates(5).unwrap().len(), 1);
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert_eq!(fired.load(Ordering::SeqCst), 1);
    assert!(!job.is_active());
    assert_eq!(job.next_date().unwrap_err().0, "WARNING: Date in past. Will never be fired.");

    let errors = Arc::new(std::sync::Mutex::new(vec![]));
    let completed = Arc::new(AtomicUsize::new(0));
    let (e2, c2) = (errors.clone(), completed.clone());
    let job = CronJob::from(
        CronJobParams::new("0 0 0 1 1 *", || async { Err::<(), _>(anyhow::anyhow!("boom")) })
            .run_on_init(true)
            .error_handler(move |e| e2.lock().unwrap().push(e.to_string()))
            .on_complete(move || {
                let c2 = c2.clone();
                async move { c2.fetch_add(1, Ordering::SeqCst); }
            }),
    )
    .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(*errors.lock().unwrap(), vec!["boom".to_string()]);
    assert!(job.last_date().is_some());
    assert!(!job.is_active());
    job.add_callback(|| async { if true { panic!("kaput") } });
    job.fire_on_tick().await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(errors.lock().unwrap().len(), 3);
    assert_eq!(errors.lock().unwrap()[2], "kaput");
    job.stop_and_wait().await;
    assert_eq!(completed.load(Ordering::SeqCst), 1);
}

#[test]
fn cron_time_parsing_and_errors() {
    let t = CronTime::new("0 9 * * mon-fri", Some("America/Caracas"), None).unwrap();
    assert_eq!(t.to_string(), "0 0 9 * * 1,2,3,4,5");
    assert_eq!(CronTime::new("@weekends", Some("UTC"), None).unwrap().to_string(), "0 0 0 * * 0,6");
    let err = |e: &str| CronTime::new(e, None, None).unwrap_err().0;
    assert_eq!(err("* * * *"), "Too few fields");
    assert_eq!(err("* * * * * * *"), "Too many fields");
    assert_eq!(err("0 0 * * sunday"), "Unknown alias: day");
    assert_eq!(err("*/0 * * * *"), "Field (minute) has a step of zero");
    assert_eq!(err("5-1 * * * *"), "Field (minute) has an invalid range");
    assert_eq!(err("61 * * * *"), "Field value (61) is out of range");
    assert_eq!(err("1-* * * * *"), "Field (1-*) has an invalid wildcard expression");
    assert_eq!(err("0 0 L * *"), "Unknown alias: l");
    assert_eq!(CronTime::new("* * * * *", Some("Nope/Nope"), None).unwrap_err().0, "Invalid timezone.");
    assert_eq!(CronTime::new("* * * * *", Some("UTC"), Some(0)).unwrap_err().0, "You can't specify both timeZone and utcOffset");
    assert!(CronTime::validate_cron_expression("0 30 9,17 * * *").is_ok());
    // dayOfMonth OR dayOfWeek: Friday the 13th style expressions match either.
    let t = CronTime::new("0 0 12 13 * 5", Some("UTC"), None).unwrap();
    let start = chrono::DateTime::parse_from_rfc3339("2026-11-07T00:00:00Z").unwrap().timestamp_millis();
    let next = t.next_date_from(start, &t.zone().unwrap()).unwrap();
    assert_eq!(next.to_rfc3339(), "2026-11-13T12:00:00+00:00");
    // Zones: 00:00Z is 20:00 of the previous day in Caracas.
    let t = CronTime::new("0 21 * * *", Some("America/Caracas"), None).unwrap();
    let next = t.next_date_from(start, &t.zone().unwrap()).unwrap();
    assert_eq!(next.to_rfc3339(), "2026-11-06T21:00:00-04:00");
    // utcOffset in minutes.
    let t = CronTime::new("0 9 * * *", None, Some(-270)).unwrap();
    assert_eq!(t.next_date_from(start, &t.zone().unwrap()).unwrap().to_rfc3339(), "2026-11-07T09:00:00-04:30");
    // DST gap (New York, 2026-03-08 02:30 does not exist): runs at 03:00 like `cron`.
    let t = CronTime::new("0 30 2 * * *", Some("America/New_York"), None).unwrap();
    let start = chrono::DateTime::parse_from_rfc3339("2026-03-08T06:59:59Z").unwrap().timestamp_millis();
    assert_eq!(t.next_date_from(start, &t.zone().unwrap()).unwrap().to_rfc3339(), "2026-03-08T03:00:00-04:00");
}

// ---- SchedulerRegistry ----

#[tokio::test]
async fn registry_api_and_messages() {
    let r = SchedulerRegistry::default();
    let e = r.get_cron_job("x").unwrap_err().to_string();
    assert_eq!(e, "No Cron Job was found with the given name (x). Check that you created one with a decorator or with the create API.");
    assert_eq!(r.get_interval("i").unwrap_err().to_string(), "No Interval was found with the given name (i). Check that you created one with a decorator or with the create API.");
    assert_eq!(r.get_timeout("t").unwrap_err().to_string(), "No Timeout was found with the given name (t). Check that you created one with a decorator or with the create API.");
    assert!(r.delete_cron_job("x").is_err());

    let job = CronJob::new("0 0 0 1 1 *", || async {}).unwrap();
    r.add_cron_job("yearly", job.clone()).unwrap();
    assert_eq!(r.add_cron_job("yearly", job.clone()).unwrap_err().to_string(), "Cron Job with the given name (yearly) already exists. Ignored.");
    assert!(r.does_exist("cron", "yearly"));
    assert!(!r.does_exist("cron", "nope"));
    assert!(!r.does_exist("other", "yearly"));
    assert_eq!(r.get_cron_job("yearly").unwrap(), job);
    job.start();
    r.delete_cron_job("yearly").unwrap();
    assert!(!job.is_active());
    assert!(r.get_cron_jobs().is_empty());

    let n = Arc::new(AtomicUsize::new(0));
    let c = n.clone();
    let iv: TimerRef = set_interval(20, move || {
        let c = c.clone();
        async move {
            c.fetch_add(1, Ordering::SeqCst);
        }
    });
    r.add_interval("iv", iv.clone()).unwrap();
    assert_eq!(r.add_interval("iv", iv).unwrap_err().to_string(), "Interval with the given name (iv) already exists. Ignored.");
    r.add_timeout("to", set_timeout(10_000, || async {})).unwrap();
    assert_eq!(r.get_intervals(), vec!["iv".to_string()]);
    assert_eq!(r.get_timeouts(), vec!["to".to_string()]);
    tokio::time::sleep(Duration::from_millis(110)).await;
    r.delete_interval("iv").unwrap();
    r.delete_timeout("to").unwrap();
    let ticks = n.load(Ordering::SeqCst);
    assert!(ticks >= 3, "{ticks}");
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(n.load(Ordering::SeqCst), ticks);
    assert!(r.get_intervals().is_empty() && r.get_timeouts().is_empty());
}

// ---- decorated handlers ----

static FAST: AtomicUsize = AtomicUsize::new(0);
static FAILING: AtomicUsize = AtomicUsize::new(0);
static DELAYED: AtomicUsize = AtomicUsize::new(0);
static BOOT_SAW: Mutex<Vec<String>> = Mutex::new(Vec::new());

#[injectable]
pub struct Jobs {
    registry: Arc<SchedulerRegistry>,
}

#[handlers]
impl Jobs {
    #[cron("* * * * * *", name = "fast", time_zone = "America/Caracas")]
    async fn fast(&self) {
        FAST.fetch_add(1, Ordering::SeqCst);
    }

    #[cron(CronExpression::EVERY_DAY_AT_9PM, CronOptions::new().name("night").utc_offset(-240))]
    async fn night(&self) {}

    #[cron("* * * * * *", name = "off", disabled = true)]
    async fn off(&self) {}

    #[cron("* * * * * *", name = "later", initial_delay = 1_500)]
    async fn later(&self) {
        DELAYED.fetch_add(1, Ordering::SeqCst);
    }

    #[cron("* * * * * *", wait_for_completion = true, threshold = 500)]
    async fn unnamed(&self) {}

    #[interval("failing", 30)]
    async fn failing(&self) -> corvis::Result<()> {
        FAILING.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("expected failure")
    }

    #[timeout(5_000)]
    async fn anonymous_timeout(&self) {}
}

#[corvis::async_trait]
impl OnApplicationBootstrap for Jobs {
    async fn on_application_bootstrap(&self) -> corvis::Result<()> {
        let mut seen: Vec<String> = self.registry.get_cron_jobs().keys().cloned().collect();
        seen.extend(self.registry.get_intervals());
        *BOOT_SAW.lock().unwrap() = seen;
        Ok(())
    }
}

#[module(imports = [ScheduleModule::for_root()], providers = [Jobs])]
pub struct JobsModule;

#[corvis::test]
async fn decorated_handlers_options_and_lifecycle() {
    let module = Test::create_testing_module().import::<JobsModule>().compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    let registry = app.get::<SchedulerRegistry>().unwrap();

    // Mounted before onApplicationBootstrap.
    let seen = BOOT_SAW.lock().unwrap().clone();
    assert!(seen.contains(&"fast".to_string()) && seen.contains(&"failing".to_string()), "{seen:?}");

    let jobs = registry.get_cron_jobs();
    let names: Vec<String> = jobs.keys().cloned().collect();
    assert_eq!(names.len(), 5);
    assert_eq!(&names[..4], &["fast", "night", "off", "later"]);
    // Unnamed jobs get a UUID v4.
    assert_eq!(names[4].len(), 36);
    assert_eq!(names[4].as_bytes()[14], b'4');
    assert_eq!(registry.get_timeouts().len(), 1);
    assert_eq!(registry.get_timeouts()[0].len(), 36);

    assert!(jobs["fast"].is_active());
    assert!(!jobs["off"].is_active());
    assert!(!jobs["later"].is_active());
    assert!(jobs[names[4].as_str()].wait_for_completion());
    assert_eq!(jobs["fast"].cron_time().time_zone, Some(corvis::schedule::CronZone::parse("America/Caracas").unwrap()));
    let night = jobs["night"].next_date().unwrap();
    assert_eq!(night.offset().local_minus_utc(), -240 * 60);
    assert_eq!(night.format("%H:%M:%S").to_string(), "21:00:00");

    tokio::time::sleep(Duration::from_millis(2300)).await;
    assert!(FAST.load(Ordering::SeqCst) >= 2);
    assert!(jobs["later"].is_active());
    assert!(DELAYED.load(Ordering::SeqCst) <= 1);
    // Errors are logged by the `Scheduler` logger; the interval keeps running.
    assert!(FAILING.load(Ordering::SeqCst) >= 10);

    app.close().await.unwrap();
    assert!(registry.get_cron_jobs().is_empty());
    assert!(registry.get_intervals().is_empty());
    assert!(registry.get_timeouts().is_empty());
    assert!(!jobs["fast"].is_active());
    let ticks = FAST.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(FAST.load(Ordering::SeqCst), ticks);
}

#[injectable]
pub struct Dup;

#[handlers]
impl Dup {
    #[interval("same", 1000)]
    async fn a(&self) {}
    #[interval("same", 1000)]
    async fn b(&self) {}
}

#[module(imports = [ScheduleModule::for_root()], providers = [Dup])]
pub struct DupModule;

#[corvis::test]
async fn duplicate_names_fail_the_bootstrap() {
    let module = Test::create_testing_module().import::<DupModule>().compile().await.unwrap();
    let mut app = module.create_corvis_application();
    let err = app.init().await.unwrap_err().to_string();
    assert_eq!(err, "Interval with the given name (same) already exists. Ignored.");
}

#[injectable]
pub struct Skipped;

#[handlers]
impl Skipped {
    #[interval("only-interval", 20)]
    async fn tick(&self) {}
    #[cron("* * * * * *", name = "skipped-cron")]
    async fn cron(&self) {}
}

#[module(
    imports = [ScheduleModule::for_root_with(ScheduleModuleOptions { cron_jobs: false, ..Default::default() })],
    providers = [Skipped],
)]
pub struct OptionsModule;

#[corvis::test]
async fn module_options_disable_kinds() {
    let module = Test::create_testing_module().import::<OptionsModule>().compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    let registry = app.get::<SchedulerRegistry>().unwrap();
    assert!(registry.get_cron_jobs().is_empty());
    assert_eq!(registry.get_intervals(), vec!["only-interval".to_string()]);
    app.close().await.unwrap();
}

#[test]
fn invalid_expression_is_a_cron_error() {
    let e = CronJob::from(CronJobParams::new("* * *", || async {})).unwrap_err();
    assert_eq!(e.to_string(), "Too few fields");
    assert!(corvis::schedule::validate_cron_expression("0 0 31 2 *").is_ok());
    let msg = CronTime::new("0 0 31 2 *", Some("UTC"), None).unwrap().send_at().unwrap_err().0;
    assert!(msg.starts_with("Something went wrong. No execution date was found in the next 8 years."), "{msg}");
}
