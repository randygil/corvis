//! `CronJob` from the `cron` package (v4): timer loop with the same early-wake correction,
//! missed-deadline threshold, `waitForCompletion`, `onComplete`, `runOnInit` and one-shot dates.

use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, FixedOffset, Utc};
use futures::FutureExt;
use futures::future::BoxFuture;

use super::cron_time::{CronError, CronSource, CronTime};

/// A callback as the job stores it.
pub type TickFn = Arc<dyn Fn() -> BoxFuture<'static, crate::Result<()>> + Send + Sync>;
/// `errorHandler(error)`.
pub type CronErrorHandler = Arc<dyn Fn(anyhow::Error) + Send + Sync>;

/// What a tick callback may return: `()` or a `Result` (an `Err` goes to the error handler).
pub trait TickOutput: Send + 'static {
    fn into_tick_result(self) -> crate::Result<()>;
}

impl TickOutput for () {
    fn into_tick_result(self) -> crate::Result<()> {
        Ok(())
    }
}

impl<T: Send + 'static, E: Into<anyhow::Error> + Send + 'static> TickOutput for Result<T, E> {
    fn into_tick_result(self) -> crate::Result<()> {
        self.map(|_| ()).map_err(Into::into)
    }
}

/// Wraps a closure into a [`TickFn`]; panics are reported like errors.
pub fn tick_fn<F, Fut>(f: F) -> TickFn
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future + Send + 'static,
    Fut::Output: TickOutput,
{
    let f = Arc::new(f);
    Arc::new(move || {
        let fut = f();
        async move {
            match AssertUnwindSafe(fut).catch_unwind().await {
                Ok(out) => out.into_tick_result(),
                Err(p) => Err(anyhow::anyhow!(panic_message(&p))),
            }
        }
        .boxed()
    })
}

pub(crate) fn panic_message(p: &Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "panic".into())
}

const MAXDELAY: i64 = 2_147_483_647;

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// `CronJob.from({ ... })` parameters.
pub struct CronJobParams {
    pub cron_time: CronSource,
    pub on_tick: TickFn,
    pub on_complete: Option<TickFn>,
    pub start: bool,
    pub time_zone: Option<String>,
    pub utc_offset: Option<i32>,
    pub run_on_init: bool,
    pub unref_timeout: bool,
    pub wait_for_completion: bool,
    pub error_handler: Option<CronErrorHandler>,
    pub name: Option<String>,
    pub threshold: Option<i64>,
}

impl CronJobParams {
    pub fn new<F, Fut>(cron_time: impl Into<CronSource>, on_tick: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future + Send + 'static,
        Fut::Output: TickOutput,
    {
        Self::from_tick_fn(cron_time, tick_fn(on_tick))
    }

    pub fn from_tick_fn(cron_time: impl Into<CronSource>, on_tick: TickFn) -> Self {
        CronJobParams {
            cron_time: cron_time.into(),
            on_tick,
            on_complete: None,
            start: false,
            time_zone: None,
            utc_offset: None,
            run_on_init: false,
            unref_timeout: false,
            wait_for_completion: false,
            error_handler: None,
            name: None,
            threshold: None,
        }
    }

    pub fn on_complete<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future + Send + 'static,
        Fut::Output: TickOutput,
    {
        self.on_complete = Some(tick_fn(f));
        self
    }
    pub fn start(mut self, start: bool) -> Self {
        self.start = start;
        self
    }
    pub fn time_zone(mut self, tz: impl Into<String>) -> Self {
        self.time_zone = Some(tz.into());
        self
    }
    /// Minutes east of UTC.
    pub fn utc_offset(mut self, minutes: i32) -> Self {
        self.utc_offset = Some(minutes);
        self
    }
    pub fn run_on_init(mut self, v: bool) -> Self {
        self.run_on_init = v;
        self
    }
    /// Accepted for parity; tokio timers never keep the process alive on their own.
    pub fn unref_timeout(mut self, v: bool) -> Self {
        self.unref_timeout = v;
        self
    }
    pub fn wait_for_completion(mut self, v: bool) -> Self {
        self.wait_for_completion = v;
        self
    }
    pub fn error_handler(mut self, f: impl Fn(anyhow::Error) + Send + Sync + 'static) -> Self {
        self.error_handler = Some(Arc::new(f));
        self
    }
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
    /// Milliseconds a missed tick may be late and still run (default 250).
    pub fn threshold(mut self, ms: i64) -> Self {
        self.threshold = Some(ms);
        self
    }
}

struct Inner {
    cron_time: Mutex<CronTime>,
    callbacks: Mutex<Vec<TickFn>>,
    on_complete: Option<TickFn>,
    wait_for_completion: bool,
    error_handler: Option<CronErrorHandler>,
    name: Option<String>,
    threshold: i64,
    run_once: AtomicBool,
    active: AtomicBool,
    callback_running: AtomicBool,
    last_execution: Mutex<Option<DateTime<Utc>>>,
    timer: Mutex<Option<tokio::task::AbortHandle>>,
}

/// A cron job (`new CronJob(...)` / `CronJob.from(...)`). Cloning gives another handle to the same job.
#[derive(Clone)]
pub struct CronJob(Arc<Inner>);

impl std::fmt::Debug for CronJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CronJob")
            .field("name", &self.0.name)
            .field("cron_time", &self.cron_time().to_string())
            .field("active", &self.is_active())
            .finish()
    }
}

impl PartialEq for CronJob {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl CronJob {
    /// `new CronJob(cronTime, onTick)` – not started.
    pub fn new<F, Fut>(cron_time: impl Into<CronSource>, on_tick: F) -> Result<Self, CronError>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future + Send + 'static,
        Fut::Output: TickOutput,
    {
        Self::from(CronJobParams::new(cron_time, on_tick))
    }

    /// `CronJob.from(params)`. Starting (`start: true`, `runOnInit`) needs a tokio runtime.
    #[allow(clippy::should_implement_trait)]
    pub fn from(p: CronJobParams) -> Result<Self, CronError> {
        if p.time_zone.is_some() && p.utc_offset.is_some() {
            return Err(CronError::exclusive("timeZone", "utcOffset"));
        }
        let cron_time = CronTime::new(p.cron_time, p.time_zone.as_deref(), p.utc_offset)?;
        let run_once = cron_time.real_date;
        let job = CronJob(Arc::new(Inner {
            cron_time: Mutex::new(cron_time),
            callbacks: Mutex::new(vec![p.on_tick]),
            on_complete: p.on_complete,
            wait_for_completion: p.wait_for_completion,
            error_handler: p.error_handler,
            name: p.name,
            threshold: p.threshold.map(|t| t.abs()).unwrap_or(250),
            run_once: AtomicBool::new(run_once),
            active: AtomicBool::new(false),
            callback_running: AtomicBool::new(false),
            last_execution: Mutex::new(None),
            timer: Mutex::new(None),
        }));
        if p.run_on_init {
            *job.0.last_execution.lock().unwrap() = Some(Utc::now());
            let j = job.clone();
            tokio::spawn(async move { j.fire_on_tick().await });
        }
        if p.start {
            job.start();
        }
        Ok(job)
    }

    pub fn name(&self) -> Option<&str> {
        self.0.name.as_deref()
    }
    /// `job.isActive`.
    pub fn is_active(&self) -> bool {
        self.0.active.load(Ordering::SeqCst)
    }
    /// `job.isCallbackRunning`.
    pub fn is_callback_running(&self) -> bool {
        self.0.callback_running.load(Ordering::SeqCst)
    }
    /// `job.runOnce` (a job built from a date).
    pub fn run_once(&self) -> bool {
        self.0.run_once.load(Ordering::SeqCst)
    }
    pub fn wait_for_completion(&self) -> bool {
        self.0.wait_for_completion
    }
    /// `job.cronTime`.
    pub fn cron_time(&self) -> CronTime {
        self.0.cron_time.lock().unwrap().clone()
    }
    /// `job.addCallback(fn)`.
    pub fn add_callback<F, Fut>(&self, f: F)
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future + Send + 'static,
        Fut::Output: TickOutput,
    {
        self.0.callbacks.lock().unwrap().push(tick_fn(f));
    }
    /// `job.nextDate()`.
    pub fn next_date(&self) -> Result<DateTime<FixedOffset>, CronError> {
        self.0.cron_time.lock().unwrap().send_at()
    }
    /// `job.nextDates(n)`.
    pub fn next_dates(&self, n: usize) -> Result<Vec<DateTime<FixedOffset>>, CronError> {
        self.0.cron_time.lock().unwrap().send_at_n(n)
    }
    /// `job.lastDate()`.
    pub fn last_date(&self) -> Option<DateTime<Utc>> {
        *self.0.last_execution.lock().unwrap()
    }

    /// `job.setTime(cronTime)`: stops, swaps the time and restarts if it was running.
    pub fn set_time(&self, time: CronTime) {
        let was_running = self.is_active();
        self.stop();
        if time.real_date {
            self.0.run_once.store(true, Ordering::SeqCst);
        }
        *self.0.cron_time.lock().unwrap() = time;
        if was_running {
            self.start();
        }
    }

    /// `job.fireOnTick()`: runs the callbacks now. With `waitForCompletion` they are awaited (and a tick
    /// that arrives while they run is skipped); otherwise they run detached and may overlap.
    pub async fn fire_on_tick(&self) {
        let inner = &self.0;
        if inner.wait_for_completion && inner.callback_running.load(Ordering::SeqCst) {
            return;
        }
        inner.callback_running.store(true, Ordering::SeqCst);
        let callbacks = inner.callbacks.lock().unwrap().clone();
        for cb in callbacks {
            let fut = cb();
            if inner.wait_for_completion {
                if let Err(e) = fut.await {
                    self.report(e);
                }
            } else {
                let job = self.clone();
                tokio::spawn(async move {
                    if let Err(e) = fut.await {
                        job.report(e);
                    }
                });
            }
        }
        inner.callback_running.store(false, Ordering::SeqCst);
    }

    fn report(&self, e: anyhow::Error) {
        match &self.0.error_handler {
            Some(h) => h(e),
            None => eprintln!("[Cron] error in callback {e}"),
        }
    }

    /// `job.start()`. Needs a tokio runtime.
    pub fn start(&self) {
        let mut timer = self.0.timer.lock().unwrap();
        if self.0.active.swap(true, Ordering::SeqCst) {
            return;
        }
        let job = self.clone();
        let handle = tokio::spawn(async move { job.run_timer().await });
        *timer = Some(handle.abort_handle());
    }

    fn timeout_ms(&self) -> Result<i64, CronError> {
        self.0.cron_time.lock().unwrap().get_timeout()
    }

    async fn run_timer(self) {
        'start: loop {
            let mut timeout = match self.timeout_ms() {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("{e}");
                    self.0.active.store(false, Ordering::SeqCst);
                    return;
                }
            };
            let mut remaining: i64 = 0;
            if timeout >= 0 {
                if timeout > MAXDELAY {
                    remaining = timeout - MAXDELAY;
                    timeout = MAXDELAY;
                }
            } else {
                let absolute = timeout.abs();
                let message = format!(
                    "[Cron] Missed execution deadline by {absolute}ms for job{} with cron expression '{}'",
                    self.0.name.as_ref().map(|n| format!(" \"{n}\"")).unwrap_or_default(),
                    self.0.cron_time.lock().unwrap().source
                );
                if absolute <= self.0.threshold {
                    eprintln!("{message}. Executing immediately.");
                    *self.0.last_execution.lock().unwrap() = Some(Utc::now());
                    let j = self.clone();
                    tokio::spawn(async move { j.fire_on_tick().await });
                } else {
                    eprintln!("{message}. Skipping execution as it exceeds threshold ({}ms).", self.0.threshold);
                }
                timeout = self.timeout_ms().unwrap_or(0);
            }
            loop {
                let start_time = now_ms();
                tokio::time::sleep(Duration::from_millis(timeout.max(1) as u64)).await;
                // Woke up early (the wall clock moved less than the timer): wait for the rest.
                let diff = start_time + timeout - now_ms();
                if diff > 0 {
                    let mut new_timeout = self.timeout_ms().unwrap_or(diff);
                    if new_timeout > diff {
                        new_timeout = diff;
                    }
                    remaining += new_timeout;
                }
                if remaining != 0 {
                    if remaining > MAXDELAY {
                        remaining -= MAXDELAY;
                        timeout = MAXDELAY;
                    } else {
                        timeout = remaining;
                        remaining = 0;
                    }
                    continue;
                }
                *self.0.last_execution.lock().unwrap() = Some(Utc::now());
                let j = self.clone();
                tokio::spawn(async move { j.fire_on_tick().await });
                if self.run_once() {
                    self.0.active.store(false, Ordering::SeqCst);
                    *self.0.timer.lock().unwrap() = None;
                    return;
                }
                continue 'start;
            }
        }
    }

    fn halt(&self) {
        if let Some(t) = self.0.timer.lock().unwrap().take() {
            t.abort();
        }
        self.0.active.store(false, Ordering::SeqCst);
    }

    /// `job.stop()`: cancels the timer (running callbacks finish) and runs `onComplete` in the background
    /// (after the running callback when `waitForCompletion` is set).
    pub fn stop(&self) {
        self.halt();
        if self.0.on_complete.is_some()
            && let Ok(rt) = tokio::runtime::Handle::try_current()
        {
            let job = self.clone();
            rt.spawn(async move { job.complete().await });
        }
    }

    /// Like [`CronJob::stop`], awaiting the running callback (with `waitForCompletion`) and `onComplete`.
    pub async fn stop_and_wait(&self) {
        self.halt();
        self.complete().await;
    }

    async fn complete(&self) {
        if self.0.wait_for_completion {
            while self.is_callback_running() {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        if let Some(f) = &self.0.on_complete
            && let Err(e) = f().await
        {
            eprintln!("[Cron] error in onComplete callback: {e}");
        }
    }
}
