//! Task scheduling: `ScheduleModule.forRoot()`, `@Cron()`, `@Interval()`, `@Timeout()`
//! and the `SchedulerRegistry`, on top of a port of the `cron` package (`CronJob`, `CronTime`).
//!
//! ```ignore
//! #[injectable]
//! pub struct TasksService;
//!
//! #[handlers]
//! impl TasksService {
//!     #[cron(CronExpression::EVERY_30_SECONDS, name = "notifications", time_zone = "Europe/Paris")]
//!     async fn handle_cron(&self) { Logger::new("TasksService").debug("Called every 30 seconds"); }
//!
//!     #[interval(10_000)]
//!     async fn handle_interval(&self) {}
//!
//!     #[timeout("boot", 5_000)]
//!     async fn handle_timeout(&self) {}
//! }
//! ```
//!
//! Semantics: handlers are collected at `onModuleInit` (duplicate names fail the bootstrap),
//! mounted before `onApplicationBootstrap` (timeouts, intervals, then cron jobs) and cleared before
//! `beforeApplicationShutdown`. Errors thrown by a handler are logged by the `Scheduler` logger. Runs
//! may overlap (like `setInterval` and `cron` without `waitForCompletion`). Unnamed jobs get a random
//! UUID. The default time zone is the process one (`TZ`, like Node).

mod cron_job;
mod cron_time;

pub(crate) use cron_job::panic_message;
pub use cron_job::{CronErrorHandler, CronJob, CronJobParams, TickFn, TickOutput, tick_fn};
pub use cron_time::{CronError, CronSource, CronTime, CronZone};

use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, FixedOffset};
use futures::FutureExt;
use indexmap::IndexMap;

use crate::di::{DynamicModule, Module, ModuleDefinition, ModuleImport, Provider, ResolveCtx, Token};
use crate::discovery::TaskCall;
use crate::logger::Logger;

/// `CronExpression`: predefined cron patterns (`EVERY_MINUTE`, `EVERY_DAY_AT_MIDNIGHT`, ...).
pub struct CronExpression;

impl CronExpression {
    pub const EVERY_SECOND: &'static str = "* * * * * *";
    pub const EVERY_5_SECONDS: &'static str = "*/5 * * * * *";
    pub const EVERY_10_SECONDS: &'static str = "*/10 * * * * *";
    pub const EVERY_30_SECONDS: &'static str = "*/30 * * * * *";
    pub const EVERY_MINUTE: &'static str = "*/1 * * * *";
    pub const EVERY_5_MINUTES: &'static str = "0 */5 * * * *";
    pub const EVERY_10_MINUTES: &'static str = "0 */10 * * * *";
    pub const EVERY_30_MINUTES: &'static str = "0 */30 * * * *";
    pub const EVERY_HOUR: &'static str = "0 0-23/1 * * *";
    pub const EVERY_2_HOURS: &'static str = "0 0-23/2 * * *";
    pub const EVERY_3_HOURS: &'static str = "0 0-23/3 * * *";
    pub const EVERY_4_HOURS: &'static str = "0 0-23/4 * * *";
    pub const EVERY_5_HOURS: &'static str = "0 0-23/5 * * *";
    pub const EVERY_6_HOURS: &'static str = "0 0-23/6 * * *";
    pub const EVERY_7_HOURS: &'static str = "0 0-23/7 * * *";
    pub const EVERY_8_HOURS: &'static str = "0 0-23/8 * * *";
    pub const EVERY_9_HOURS: &'static str = "0 0-23/9 * * *";
    pub const EVERY_10_HOURS: &'static str = "0 0-23/10 * * *";
    pub const EVERY_11_HOURS: &'static str = "0 0-23/11 * * *";
    pub const EVERY_12_HOURS: &'static str = "0 0-23/12 * * *";
    pub const EVERY_DAY_AT_1AM: &'static str = "0 01 * * *";
    pub const EVERY_DAY_AT_2AM: &'static str = "0 02 * * *";
    pub const EVERY_DAY_AT_3AM: &'static str = "0 03 * * *";
    pub const EVERY_DAY_AT_4AM: &'static str = "0 04 * * *";
    pub const EVERY_DAY_AT_5AM: &'static str = "0 05 * * *";
    pub const EVERY_DAY_AT_6AM: &'static str = "0 06 * * *";
    pub const EVERY_DAY_AT_7AM: &'static str = "0 07 * * *";
    pub const EVERY_DAY_AT_8AM: &'static str = "0 08 * * *";
    pub const EVERY_DAY_AT_9AM: &'static str = "0 09 * * *";
    pub const EVERY_DAY_AT_10AM: &'static str = "0 10 * * *";
    pub const EVERY_DAY_AT_11AM: &'static str = "0 11 * * *";
    pub const EVERY_DAY_AT_NOON: &'static str = "0 12 * * *";
    pub const EVERY_DAY_AT_1PM: &'static str = "0 13 * * *";
    pub const EVERY_DAY_AT_2PM: &'static str = "0 14 * * *";
    pub const EVERY_DAY_AT_3PM: &'static str = "0 15 * * *";
    pub const EVERY_DAY_AT_4PM: &'static str = "0 16 * * *";
    pub const EVERY_DAY_AT_5PM: &'static str = "0 17 * * *";
    pub const EVERY_DAY_AT_6PM: &'static str = "0 18 * * *";
    pub const EVERY_DAY_AT_7PM: &'static str = "0 19 * * *";
    pub const EVERY_DAY_AT_8PM: &'static str = "0 20 * * *";
    pub const EVERY_DAY_AT_9PM: &'static str = "0 21 * * *";
    pub const EVERY_DAY_AT_10PM: &'static str = "0 22 * * *";
    pub const EVERY_DAY_AT_11PM: &'static str = "0 23 * * *";
    pub const EVERY_DAY_AT_MIDNIGHT: &'static str = "0 0 * * *";
    pub const EVERY_WEEK: &'static str = "0 0 * * 0";
    pub const EVERY_WEEKDAY: &'static str = "0 0 * * 1-5";
    pub const EVERY_WEEKEND: &'static str = "0 0 * * 6,0";
    pub const EVERY_1ST_DAY_OF_MONTH_AT_MIDNIGHT: &'static str = "0 0 1 * *";
    pub const EVERY_1ST_DAY_OF_MONTH_AT_NOON: &'static str = "0 12 1 * *";
    pub const EVERY_2ND_HOUR: &'static str = "0 */2 * * *";
    pub const EVERY_2ND_HOUR_FROM_1AM_THROUGH_11PM: &'static str = "0 1-23/2 * * *";
    pub const EVERY_2ND_MONTH: &'static str = "0 0 1 */2 *";
    pub const EVERY_QUARTER: &'static str = "0 0 1 */3 *";
    pub const EVERY_6_MONTHS: &'static str = "0 0 1 */6 *";
    pub const EVERY_YEAR: &'static str = "0 0 1 1 *";
    pub const EVERY_30_MINUTES_BETWEEN_9AM_AND_5PM: &'static str = "0 */30 9-17 * * *";
    pub const EVERY_30_MINUTES_BETWEEN_9AM_AND_6PM: &'static str = "0 */30 9-18 * * *";
    pub const EVERY_30_MINUTES_BETWEEN_10AM_AND_7PM: &'static str = "0 */30 10-19 * * *";
    pub const MONDAY_TO_FRIDAY_AT_1AM: &'static str = "0 0 01 * * 1-5";
    pub const MONDAY_TO_FRIDAY_AT_2AM: &'static str = "0 0 02 * * 1-5";
    pub const MONDAY_TO_FRIDAY_AT_3AM: &'static str = "0 0 03 * * 1-5";
    pub const MONDAY_TO_FRIDAY_AT_4AM: &'static str = "0 0 04 * * 1-5";
    pub const MONDAY_TO_FRIDAY_AT_5AM: &'static str = "0 0 05 * * 1-5";
    pub const MONDAY_TO_FRIDAY_AT_6AM: &'static str = "0 0 06 * * 1-5";
    pub const MONDAY_TO_FRIDAY_AT_7AM: &'static str = "0 0 07 * * 1-5";
    pub const MONDAY_TO_FRIDAY_AT_8AM: &'static str = "0 0 08 * * 1-5";
    pub const MONDAY_TO_FRIDAY_AT_9AM: &'static str = "0 0 09 * * 1-5";
    pub const MONDAY_TO_FRIDAY_AT_09_30AM: &'static str = "0 30 09 * * 1-5";
    pub const MONDAY_TO_FRIDAY_AT_10AM: &'static str = "0 0 10 * * 1-5";
    pub const MONDAY_TO_FRIDAY_AT_11AM: &'static str = "0 0 11 * * 1-5";
    pub const MONDAY_TO_FRIDAY_AT_11_30AM: &'static str = "0 30 11 * * 1-5";
    pub const MONDAY_TO_FRIDAY_AT_12PM: &'static str = "0 0 12 * * 1-5";
    pub const MONDAY_TO_FRIDAY_AT_1PM: &'static str = "0 0 13 * * 1-5";
    pub const MONDAY_TO_FRIDAY_AT_2PM: &'static str = "0 0 14 * * 1-5";
    pub const MONDAY_TO_FRIDAY_AT_3PM: &'static str = "0 0 15 * * 1-5";
    pub const MONDAY_TO_FRIDAY_AT_4PM: &'static str = "0 0 16 * * 1-5";
    pub const MONDAY_TO_FRIDAY_AT_5PM: &'static str = "0 0 17 * * 1-5";
    pub const MONDAY_TO_FRIDAY_AT_6PM: &'static str = "0 0 18 * * 1-5";
    pub const MONDAY_TO_FRIDAY_AT_7PM: &'static str = "0 0 19 * * 1-5";
    pub const MONDAY_TO_FRIDAY_AT_8PM: &'static str = "0 0 20 * * 1-5";
    pub const MONDAY_TO_FRIDAY_AT_9PM: &'static str = "0 0 21 * * 1-5";
    pub const MONDAY_TO_FRIDAY_AT_10PM: &'static str = "0 0 22 * * 1-5";
    pub const MONDAY_TO_FRIDAY_AT_11PM: &'static str = "0 0 23 * * 1-5";
}

/// `@Cron(expr, options)` options.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CronOptions {
    pub name: Option<String>,
    pub time_zone: Option<String>,
    /// Minutes east of UTC (exclusive with `time_zone`).
    pub utc_offset: Option<i32>,
    pub unref_timeout: Option<bool>,
    pub wait_for_completion: Option<bool>,
    pub disabled: bool,
    pub threshold: Option<i64>,
    /// Milliseconds to wait before starting the job.
    pub initial_delay: Option<u64>,
}

impl CronOptions {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
    pub fn time_zone(mut self, tz: impl Into<String>) -> Self {
        self.time_zone = Some(tz.into());
        self
    }
    pub fn utc_offset(mut self, minutes: i32) -> Self {
        self.utc_offset = Some(minutes);
        self
    }
    pub fn unref_timeout(mut self, v: bool) -> Self {
        self.unref_timeout = Some(v);
        self
    }
    pub fn wait_for_completion(mut self, v: bool) -> Self {
        self.wait_for_completion = Some(v);
        self
    }
    pub fn disabled(mut self, v: bool) -> Self {
        self.disabled = v;
        self
    }
    pub fn threshold(mut self, ms: i64) -> Self {
        self.threshold = Some(ms);
        self
    }
    pub fn initial_delay(mut self, ms: u64) -> Self {
        self.initial_delay = Some(ms);
        self
    }

    /// `CronJob.from({ ...options, cronTime, onTick, start: !disabled && !initialDelay })`.
    pub fn to_params(&self, cron_time: impl Into<CronSource>, on_tick: TickFn) -> CronJobParams {
        let mut p = CronJobParams::from_tick_fn(cron_time, on_tick)
            .start(!self.disabled && self.initial_delay.unwrap_or(0) == 0)
            .wait_for_completion(self.wait_for_completion.unwrap_or(false))
            .unref_timeout(self.unref_timeout.unwrap_or(false));
        p.name = self.name.clone();
        p.time_zone = self.time_zone.clone();
        p.utc_offset = self.utc_offset;
        p.threshold = self.threshold;
        p
    }
}

/// `ScheduleModule.forRoot(options)`: which kinds of decorated handlers are registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduleModuleOptions {
    pub cron_jobs: bool,
    pub intervals: bool,
    pub timeouts: bool,
}

impl Default for ScheduleModuleOptions {
    fn default() -> Self {
        ScheduleModuleOptions { cron_jobs: true, intervals: true, timeouts: true }
    }
}

/// A `setInterval`/`setTimeout` handle (what `getInterval`/`getTimeout` return).
#[derive(Clone)]
pub struct TimerRef(Arc<Mutex<Option<tokio::task::AbortHandle>>>);

impl TimerRef {
    /// `clearInterval(ref)` / `clearTimeout(ref)`. Callbacks already running are not cancelled.
    pub fn clear(&self) {
        if let Some(h) = self.0.lock().unwrap().take() {
            h.abort();
        }
    }
    /// `false` once cleared (or, for a timeout, once it fired).
    pub fn is_pending(&self) -> bool {
        self.0.lock().unwrap().as_ref().is_some_and(|h| !h.is_finished())
    }
}

impl PartialEq for TimerRef {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for TimerRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TimerRef").field("pending", &self.is_pending()).finish()
    }
}

/// Node clamps delays outside `1..=2^31-1` to 1 ms.
fn node_delay(ms: u64) -> Duration {
    Duration::from_millis(if (1..=2_147_483_647).contains(&ms) { ms } else { 1 })
}

/// `setInterval(fn, ms)`: every run is spawned, so slow runs overlap like in Node. Needs a tokio runtime.
pub fn set_interval<F, Fut>(ms: u64, f: F) -> TimerRef
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let delay = node_delay(ms);
    let handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + delay, delay);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            tokio::spawn(f());
        }
    });
    TimerRef(Arc::new(Mutex::new(Some(handle.abort_handle()))))
}

/// `setTimeout(fn, ms)`. Needs a tokio runtime.
pub fn set_timeout<F, Fut>(ms: u64, f: F) -> TimerRef
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let delay = node_delay(ms);
    let handle = tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        tokio::spawn(f());
    });
    TimerRef(Arc::new(Mutex::new(Some(handle.abort_handle()))))
}

/// `NO_SCHEDULER_FOUND(type, name)`.
fn no_scheduler_found(kind: &str, name: &str) -> anyhow::Error {
    anyhow::anyhow!("No {kind} was found with the given name ({name}). Check that you created one with a decorator or with the create API.")
}

/// `DUPLICATE_SCHEDULER(type, name)`.
fn duplicate_scheduler(kind: &str, name: &str) -> anyhow::Error {
    anyhow::anyhow!("{kind} with the given name ({name}) already exists. Ignored.")
}

enum Pending {
    Cron { name: String, cron_time: CronSource, options: CronOptions, call: TickFn },
    Interval { name: String, millis: u64, call: TaskCall },
    Timeout { name: String, millis: u64, call: TaskCall },
}

#[derive(Default)]
struct Orchestrator {
    crons: Vec<String>,
    intervals: Vec<String>,
    timeouts: Vec<String>,
    pending: Vec<Pending>,
    initial_delays: Vec<TimerRef>,
}

/// `SchedulerRegistry`: the cron jobs, intervals and timeouts by name.
#[derive(Default)]
pub struct SchedulerRegistry {
    cron_jobs: Mutex<IndexMap<String, CronJob>>,
    intervals: Mutex<IndexMap<String, TimerRef>>,
    timeouts: Mutex<IndexMap<String, TimerRef>>,
    orchestrator: Mutex<Orchestrator>,
}

impl SchedulerRegistry {
    /// `doesExist('cron' | 'interval' | 'timeout', name)`.
    pub fn does_exist(&self, kind: &str, name: &str) -> bool {
        match kind {
            "cron" => self.cron_jobs.lock().unwrap().contains_key(name),
            "interval" => self.intervals.lock().unwrap().contains_key(name),
            "timeout" => self.timeouts.lock().unwrap().contains_key(name),
            _ => false,
        }
    }

    /// `getCronJob(name)`.
    pub fn get_cron_job(&self, name: &str) -> crate::Result<CronJob> {
        self.cron_jobs.lock().unwrap().get(name).cloned().ok_or_else(|| no_scheduler_found("Cron Job", name))
    }
    /// `getInterval(name)`.
    pub fn get_interval(&self, name: &str) -> crate::Result<TimerRef> {
        self.intervals.lock().unwrap().get(name).cloned().ok_or_else(|| no_scheduler_found("Interval", name))
    }
    /// `getTimeout(name)`.
    pub fn get_timeout(&self, name: &str) -> crate::Result<TimerRef> {
        self.timeouts.lock().unwrap().get(name).cloned().ok_or_else(|| no_scheduler_found("Timeout", name))
    }

    /// `addCronJob(name, job)` (the job keeps its own started/stopped state).
    pub fn add_cron_job(&self, name: &str, job: CronJob) -> crate::Result<()> {
        let mut jobs = self.cron_jobs.lock().unwrap();
        if jobs.contains_key(name) {
            return Err(duplicate_scheduler("Cron Job", name));
        }
        jobs.insert(name.to_string(), job);
        Ok(())
    }
    /// `addInterval(name, setInterval(...))`.
    pub fn add_interval(&self, name: &str, interval: TimerRef) -> crate::Result<()> {
        let mut map = self.intervals.lock().unwrap();
        if map.contains_key(name) {
            return Err(duplicate_scheduler("Interval", name));
        }
        map.insert(name.to_string(), interval);
        Ok(())
    }
    /// `addTimeout(name, setTimeout(...))`.
    pub fn add_timeout(&self, name: &str, timeout: TimerRef) -> crate::Result<()> {
        let mut map = self.timeouts.lock().unwrap();
        if map.contains_key(name) {
            return Err(duplicate_scheduler("Timeout", name));
        }
        map.insert(name.to_string(), timeout);
        Ok(())
    }

    /// `getCronJobs()` (insertion order).
    pub fn get_cron_jobs(&self) -> IndexMap<String, CronJob> {
        self.cron_jobs.lock().unwrap().clone()
    }
    /// `getIntervals()`: the names.
    pub fn get_intervals(&self) -> Vec<String> {
        self.intervals.lock().unwrap().keys().cloned().collect()
    }
    /// `getTimeouts()`: the names.
    pub fn get_timeouts(&self) -> Vec<String> {
        self.timeouts.lock().unwrap().keys().cloned().collect()
    }

    /// `deleteCronJob(name)`: stops it and removes it.
    pub fn delete_cron_job(&self, name: &str) -> crate::Result<()> {
        let job = self.get_cron_job(name)?;
        job.stop();
        self.cron_jobs.lock().unwrap().shift_remove(name);
        Ok(())
    }
    /// `deleteInterval(name)`.
    pub fn delete_interval(&self, name: &str) -> crate::Result<()> {
        self.get_interval(name)?.clear();
        self.intervals.lock().unwrap().shift_remove(name);
        Ok(())
    }
    /// `deleteTimeout(name)`.
    pub fn delete_timeout(&self, name: &str) -> crate::Result<()> {
        self.get_timeout(name)?.clear();
        self.timeouts.lock().unwrap().shift_remove(name);
        Ok(())
    }

    // ---- orchestrator (decorated handlers) ----

    pub(crate) fn add_cron(&self, cron_time: CronSource, options: CronOptions, call: TaskCall) -> crate::Result<()> {
        let mut o = self.orchestrator.lock().unwrap();
        let name = options.name.clone().filter(|n| !n.is_empty()).unwrap_or_else(random_uuid);
        if o.crons.contains(&name) {
            return Err(duplicate_scheduler("Cron Job", &name));
        }
        o.crons.push(name.clone());
        o.pending.push(Pending::Cron { name, cron_time, options, call: wrap_task(call) });
        Ok(())
    }

    pub(crate) fn add_interval_handler(&self, name: Option<String>, millis: u64, call: TaskCall) -> crate::Result<()> {
        let mut o = self.orchestrator.lock().unwrap();
        let name = name.unwrap_or_else(random_uuid);
        if o.intervals.contains(&name) {
            return Err(duplicate_scheduler("Interval", &name));
        }
        o.intervals.push(name.clone());
        o.pending.push(Pending::Interval { name, millis, call });
        Ok(())
    }

    pub(crate) fn add_timeout_handler(&self, name: Option<String>, millis: u64, call: TaskCall) -> crate::Result<()> {
        let mut o = self.orchestrator.lock().unwrap();
        let name = name.unwrap_or_else(random_uuid);
        if o.timeouts.contains(&name) {
            return Err(duplicate_scheduler("Timeout", &name));
        }
        o.timeouts.push(name.clone());
        o.pending.push(Pending::Timeout { name, millis, call });
        Ok(())
    }

    /// `onApplicationBootstrap`: mounts timeouts, then intervals, then cron jobs.
    pub(crate) fn mount(self: &Arc<Self>) -> crate::Result<()> {
        let pending = std::mem::take(&mut self.orchestrator.lock().unwrap().pending);
        let (mut timeouts, mut intervals, mut crons) = (vec![], vec![], vec![]);
        for p in pending {
            match p {
                Pending::Timeout { .. } => timeouts.push(p),
                Pending::Interval { .. } => intervals.push(p),
                Pending::Cron { .. } => crons.push(p),
            }
        }
        for p in timeouts.into_iter().chain(intervals).chain(crons) {
            match p {
                Pending::Timeout { name, millis, call } => {
                    let t = set_timeout(millis, move || run_logged(call));
                    self.add_timeout(&name, t)?;
                }
                Pending::Interval { name, millis, call } => {
                    let t = set_interval(millis, move || run_logged(call.clone()));
                    self.add_interval(&name, t)?;
                }
                Pending::Cron { name, cron_time, options, call } => {
                    let job = CronJob::from(options.to_params(cron_time, call)).map_err(anyhow::Error::new)?;
                    self.add_cron_job(&name, job.clone())?;
                    if let Some(delay) = options.initial_delay.filter(|d| *d > 0)
                        && !options.disabled
                    {
                        let registry = Arc::downgrade(self);
                        let t = set_timeout(delay, move || async move {
                            // Only if it is still the job registered under this name.
                            if let Some(r) = registry.upgrade()
                                && r.get_cron_job(&name).ok().as_ref() == Some(&job)
                            {
                                job.start();
                            }
                        });
                        self.orchestrator.lock().unwrap().initial_delays.push(t);
                    }
                }
            }
        }
        Ok(())
    }

    /// `beforeApplicationShutdown`: clears timeouts, intervals and cron jobs.
    pub(crate) fn shutdown(&self) {
        for name in self.get_timeouts() {
            let _ = self.delete_timeout(&name);
        }
        for name in self.get_intervals() {
            let _ = self.delete_interval(&name);
        }
        for t in std::mem::take(&mut self.orchestrator.lock().unwrap().initial_delays) {
            t.clear();
        }
        let names: Vec<String> = self.cron_jobs.lock().unwrap().keys().cloned().collect();
        for name in names {
            let _ = self.delete_cron_job(&name);
        }
    }

    /// Stops everything (alias of the shutdown step).
    pub fn stop_all(&self) {
        self.shutdown();
    }
}

/// The explorer's `wrapFunctionInTryCatchBlocks`: errors (and panics) go to the `Scheduler` logger.
fn wrap_task(call: TaskCall) -> TickFn {
    Arc::new(move || {
        let call = call.clone();
        async move {
            run_logged(call).await;
            Ok(())
        }
        .boxed()
    })
}

async fn run_logged(call: TaskCall) {
    match AssertUnwindSafe(call()).catch_unwind().await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => Logger::new("Scheduler").error(format!("{e}")),
        Err(p) => Logger::new("Scheduler").error(cron_job::panic_message(&p)),
    }
}

/// `crypto.randomUUID()`.
fn random_uuid() -> String {
    let mut b: [u8; 16] = rand::random();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// `sendAt(cronTime)` from the `cron` package.
pub fn send_at(cron_time: impl Into<CronSource>) -> Result<DateTime<FixedOffset>, CronError> {
    CronTime::new(cron_time, None, None)?.send_at()
}

/// `timeout(cronTime)` from the `cron` package: milliseconds until the next date.
pub fn timeout(cron_time: impl Into<CronSource>) -> Result<i64, CronError> {
    CronTime::new(cron_time, None, None)?.get_timeout()
}

/// `validateCronExpression(expr)` from the `cron` package.
pub fn validate_cron_expression(expr: &str) -> Result<(), CronError> {
    CronTime::validate_cron_expression(expr)
}

/// `ScheduleModule`.
#[derive(Default)]
pub struct ScheduleModule;
crate::default_injectable!(ScheduleModule);

impl Module for ScheduleModule {
    fn definition() -> ModuleDefinition {
        ModuleDefinition::new::<ScheduleModule>("ScheduleModule").module_provider(Provider::class::<ScheduleModule>())
    }
}

impl ScheduleModule {
    /// `ScheduleModule.forRoot()` – global.
    pub fn for_root() -> DynamicModule {
        Self::for_root_with(ScheduleModuleOptions::default())
    }

    /// `ScheduleModule.forRoot({ cronJobs, intervals, timeouts })`.
    pub fn for_root_with(options: ScheduleModuleOptions) -> DynamicModule {
        DynamicModule::new::<ScheduleModule>()
            .provider(Provider::use_value(Token::of::<SchedulerRegistry>(), SchedulerRegistry::default()))
            .provider(Provider::use_value(Token::of::<ScheduleModuleOptions>(), options))
            .export(Token::of::<SchedulerRegistry>())
            .global(true)
    }

    /// `ScheduleModule.forRootAsync({ imports, useFactory, inject })`.
    pub fn for_root_async<F, Fut>(imports: Vec<ModuleImport>, factory: F) -> DynamicModule
    where
        F: Fn(ResolveCtx) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = crate::Result<ScheduleModuleOptions>> + Send + 'static,
    {
        DynamicModule::new::<ScheduleModule>()
            .imports(imports)
            .provider(Provider::use_value(Token::of::<SchedulerRegistry>(), SchedulerRegistry::default()))
            .provider(Provider::use_factory(Token::of::<ScheduleModuleOptions>(), factory))
            .export(Token::of::<SchedulerRegistry>())
            .global(true)
    }
}
