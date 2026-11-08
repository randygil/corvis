# Spec Delta

## ADDED Requirements

### Requirement: Processors are declared with attributes
A provider marked `#[processor("queue")]` SHALL start one worker for that queue when the application (or a
standalone application context) bootstraps, and stop it on close. A `#[process("welcome")]` method handles jobs named
`welcome`; a `#[process]` method without a name handles the others.

#### Scenario: Dispatch by job name
- **WHEN** `EmailsProcessor` has `#[process("welcome")]` and `#[process]` methods and jobs `welcome` and `reset` are
  added
- **THEN** `welcome` runs the first method and `reset` the second

#### Scenario: Standalone worker process
- **WHEN** a process starts with `CorvisFactory::create_application_context::<WorkerModule>()` and `WorkerModule`
  declares a processor
- **THEN** the worker processes jobs without an HTTP server and stops when the context closes

### Requirement: Worker defaults match Node BullMQ
Worker options not given in `#[processor]` SHALL default to Node BullMQ's worker defaults: `concurrency: 1`,
`lockDuration: 30000`, `lockRenewTime: lockDuration / 2`, `stalledInterval: 30000`, `maxStalledCount: 1`.

#### Scenario: No options
- **WHEN** a processor is declared as `#[processor("emails")]`
- **THEN** its worker runs one job at a time and uses a 30 000 ms lock renewed every 15 000 ms

### Requirement: Job outcomes match Node BullMQ
For the same job and the same outcome of the processing function, the Redis state and events after processing SHALL
equal a Node BullMQ worker's: a returned value becomes `returnvalue` in `completed`; an error becomes `failedReason`
and is retried while `attemptsMade + 1 < attempts` (via `delayed`, or `wait` when the delay is 0), else `failed` with
`retries-exhausted`; an unrecoverable error fails without retry. A panic MUST fail the job, not the worker.

#### Scenario: Retries then failure, Node producer
- **WHEN** Node adds a job with `attempts: 3` and fixed backoff 100 ms, and the Corvis processor always returns an
  error `"boom"`
- **THEN** the job ends in `failed` with `atm` 3 and `failedReason` `"boom"`, and the ordered events stream entries
  for the job (including `retries-exhausted`) equal those a Node worker produces for the same job

#### Scenario: Unrecoverable error
- **WHEN** the processor returns `JobError::unrecoverable("bad input")` for a job with `attempts: 5`
- **THEN** the job is in `failed` after one attempt with `failedReason` `"bad input"`

#### Scenario: Payload of the wrong shape
- **WHEN** a Node producer adds a job whose `data` does not deserialize into the processor's `Job<T>` type
- **THEN** the job fails without retry and `failedReason` names the expected type and the deserialization error

#### Scenario: Panic in a processor
- **WHEN** a processing method panics
- **THEN** the job fails with a `failedReason` built from the panic message and the worker keeps processing the next
  job

### Requirement: Backoff delays match Node BullMQ
Fixed and exponential backoff SHALL compute the same delays as Node BullMQ for the same attempt: `delay` for fixed,
`round(2^(attemptsMade-1) * delay)` for exponential, and with `jitter` a value within the same bounds.

#### Scenario: Exponential backoff table
- **WHEN** the backoff of `exponential` 1000 ms is computed for attempts 1 to 5 without jitter
- **THEN** the delays are 1000, 2000, 4000, 8000 and 16000 ms, as recorded from Node BullMQ

### Requirement: Locks and stalled jobs
A Corvis worker SHALL hold the job lock (`<job>:lock` with its token) while it processes the job and renew it before
it expires. A job whose worker died SHALL be recovered by the stalled check as Node BullMQ does (back to `wait` with
`stc + 1` and a `stalled` event, or failed with `"job stalled more than allowable limit"` past `maxStalledCount`),
whichever side runs the check.

#### Scenario: Corvis worker killed mid-job, Node worker recovers
- **WHEN** a Corvis worker with `lockDuration: 1000` is killed while processing a job and a Node worker with
  `stalledInterval: 500` runs on the same queue
- **THEN** the Node worker moves the job back to `wait`, emits `stalled` and processes it, with `stc` 1

### Requirement: Graceful worker shutdown
On application close a worker MUST stop taking new jobs, wait for its active jobs up to a configurable timeout, and
leave jobs still unfinished at the timeout to be recovered as stalled.

#### Scenario: Close waits for the active job
- **WHEN** `app.close()` runs while a job takes 300 ms and the close timeout is 5 s
- **THEN** the job completes, then the worker stops and `close()` returns

### Requirement: Progress and logs
`Job::update_progress(value)` and `Job::log(line)` SHALL write the `progress` field (JSON number or object) and the
`<job>:logs` list, and emit the `progress` event, as Node BullMQ does.

#### Scenario: Progress seen by Node
- **WHEN** a Corvis processor reports progress `50` then `{ "step": "upload" }`
- **THEN** a Node `QueueEvents` listener receives two `progress` events with those values, and Node `getJob` shows the
  last one
