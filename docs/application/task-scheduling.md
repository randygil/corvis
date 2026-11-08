---
status: documented
---

# Task scheduling

`ScheduleModule` runs methods of your providers at fixed times: with a cron expression, at a regular interval or once after a delay. Date calculation follows Node.js's `cron` 4 package: same syntax, same time zones and same error messages.

## Registering the module

```rust
#[module(imports = [ScheduleModule::for_root()])]
pub struct AppModule;
```

The module is global and registers `SchedulerRegistry`. Variants:

- `ScheduleModule::for_root_with(ScheduleModuleOptions { cron_jobs, intervals, timeouts })` disables types of handlers (all are enabled by default).
- `ScheduleModule::for_root_async(imports, |ctx| async move { ... })` computes those options with an async factory that can read other providers.

## Declaring tasks

Handlers are declared in a `#[handlers]` block of a registered provider:

```rust
#[injectable]
pub struct TasksService {
    #[init(Logger::new("TasksService"))]
    logger: Logger,
}

#[handlers]
impl TasksService {
    #[cron(CronExpression::EVERY_MINUTE, name = "heartbeat")]
    async fn heartbeat(&self) {
        self.logger.debug("Called every minute");
    }

    #[timeout(1_000)]
    async fn warmup(&self) {
        self.logger.log("Warm-up finished (1s after bootstrap)");
    }
}
```

(From `examples/cats-app/src/tasks.rs`.)

| Attribute | When it runs |
|---|---|
| `#[cron(expr, ...)]` | According to the cron expression |
| `#[interval(ms)]` / `#[interval("name", ms)]` | Every `ms` milliseconds, starting after the first interval |
| `#[timeout(ms)]` / `#[timeout("name", ms)]` | Once, `ms` milliseconds after startup |

The method can be `async` or synchronous and return `()` or a `Result`. An `Err` or a `panic` is logged with the `Scheduler` logger and the task stays scheduled.

### Cron expressions

Expressions with 5 or 6 fields are accepted (`[second] minute hour day-of-month month day-of-week`), with ranges, lists, steps, aliases (`jan`..`dec`, `sun`..`sat`), day of week `0` to `7` (`0` and `7` are Sunday) and presets such as `@daily` or `@weekdays`. If you restrict both the day of the month and the day of the week, it is enough for one of the two to match.

`CronExpression` provides ready-to-use constants: `EVERY_SECOND`, `EVERY_30_SECONDS`, `EVERY_MINUTE`, `EVERY_HOUR`, `EVERY_DAY_AT_MIDNIGHT`, `EVERY_WEEKDAY`, `MONDAY_TO_FRIDAY_AT_9AM`, among others.

### `#[cron]` options

```rust
#[handlers]
impl ReportsService {
    #[cron("0 30 9,17 * * *", name = "fx", time_zone = "America/Caracas")]
    async fn refresh(&self) -> corvis::Result<()> {
        Ok(())
    }
}
```

| Option | Effect |
|---|---|
| `name = "..."` | Name in the registry (without a name it gets a UUID v4) |
| `time_zone = "..."` | IANA zone (`"Europe/Madrid"`, case-insensitive) or `"UTC±H[:MM]"` |
| `utc_offset = -240` | Fixed offset in minutes (mutually exclusive with `time_zone`) |
| `disabled = true` | Registers the task without starting it |
| `wait_for_completion = true` | If the previous run is still in progress, the tick is skipped |
| `threshold = ms` | Warns with `[Cron] Missed execution deadline` if a tick arrives more than `ms` late |
| `initial_delay = ms` | Delays the start of the task |
| `unref_timeout = bool` | Accepted for compatibility |

You can also pass a `CronOptions` expression as the second argument: `#[cron(EXPR, CronOptions::new().name("x").wait_for_completion(true))]`.

Without `time_zone` or `utc_offset` the process zone is used: the `TZ` variable if it exists, or the system's. Daylight saving time jumps and repetitions are resolved as in the original package.

### Overlap

Runs can overlap: an `#[interval]` launches each run even if the previous one has not finished, just like `setInterval`, and a `#[cron]` does the same except with `wait_for_completion = true`.

## Lifecycle

- Handlers are collected when the application initializes. Two tasks of the same type with the same name make startup fail with `Interval with the given name (x) already exists. Ignored.` (or `Cron Job`, `Timeout`).
- They are started right before the `on_application_bootstrap` hooks (timeouts first, then intervals and finally cron jobs).
- They are stopped when the application shuts down, before `before_application_shutdown`.

See [Lifecycle events](/fundamentals/lifecycle-events).

## `SchedulerRegistry`: tasks at runtime

Inject `Arc<SchedulerRegistry>` to query, stop or create tasks by name:

```rust
use corvis::schedule::{set_interval, set_timeout};

#[injectable]
pub struct ReportsService {
    registry: Arc<SchedulerRegistry>,
}

impl ReportsService {
    pub fn pause(&self) -> corvis::Result<()> {
        self.registry.get_cron_job("fx")?.stop();
        Ok(())
    }

    pub fn schedule_cleanup(&self) -> corvis::Result<()> {
        let job = CronJob::new("0 0 3 * * *", || async {
            Logger::new("Cleanup").log("running");
        })?;
        self.registry.add_cron_job("cleanup", job.clone())?;
        job.start();

        self.registry.add_interval("sync", set_interval(120_000, || async { /* ... */ }))?;
        self.registry.add_timeout("once", set_timeout(5_000, || async { /* ... */ }))?;
        Ok(())
    }
}
```

| Method | Description |
|---|---|
| `get_cron_job(n)`, `get_interval(n)`, `get_timeout(n)` | Error `No Cron Job was found with the given name (n)...` if it does not exist |
| `add_cron_job(n, job)`, `add_interval(n, timer)`, `add_timeout(n, timer)` | Error if the name already exists |
| `delete_cron_job(n)`, `delete_interval(n)`, `delete_timeout(n)` | Stops and removes |
| `get_cron_jobs()` | Map of name → `CronJob` in insertion order |
| `get_intervals()`, `get_timeouts()` | Registered names |
| `does_exist("cron" \| "interval" \| "timeout", n)` | Checks whether it exists |

`add_cron_job` does not start the job: call `start()` when you want it to begin. `set_interval` and `set_timeout` return a `TimerRef` with `clear()` and `is_pending()`.

### `CronJob`

`CronJob` is a port of the object of the same name in the `cron` package. It is created with `CronJob::new(expr, closure)` or, with options, with `CronJob::from(CronJobParams::new(expr, closure).time_zone("UTC").start(true))`. Main methods: `start`, `stop`, `stop_and_wait`, `is_active`, `is_callback_running`, `next_date`, `next_dates(n)`, `last_date`, `fire_on_tick`, `add_callback` and `set_time(CronTime)`. `CronJobParams` supports `on_complete`, `run_on_init`, `wait_for_completion`, `error_handler` and `threshold`, and accepts a date instead of an expression for a job that runs only once.

The functions `corvis::schedule::send_at`, `timeout` and `validate_cron_expression` compute the next date, the milliseconds remaining, or validate an expression without creating a job.

## Limits

- Tasks live in the process: if you run several instances of the application, each one fires its own tasks. For a task to run only once in a cluster you need external coordination (see [Distributed locks](/reliability/locks)).
- There is no persistence: a tick that falls while the application is stopped is not recovered.
