# Spec Delta

## ADDED Requirements

### Requirement: Compatible BullMQ version is stated and tested
Corvis queues SHALL read and write the Redis data format of Node BullMQ 6.x. The interop test harness MUST pin an
exact Node BullMQ 6.x version (6.7.2 when this requirement was written), and the queues documentation MUST name the
supported major version and the tested 5.x subset (add, process, retry, fail, delay, priority, progress, events,
flows, rate limiting). Raising the pinned version MUST be a deliberate change that reruns the interop suite.

#### Scenario: Node 6.x producer, Corvis worker
- **WHEN** Node BullMQ 6.7.2 adds a job to queue `q` and a Corvis worker for `q` is running on the same Redis and
  prefix
- **THEN** the Corvis worker processes the job and the normalized Redis state of `q` equals the state left by a Node
  6.7.2 worker for the same job

#### Scenario: Node 5.x producer, Corvis worker
- **WHEN** Node BullMQ 5.81.5 adds a delayed, a prioritized and a standard job to a queue that is not paused
- **THEN** a Corvis worker processes all three in the same order as a Node 6.7.2 worker would

### Requirement: Same key layout as Node BullMQ
Every key Corvis creates for queue `<q>` SHALL be named `<prefix>:<q>:<suffix>` with the suffixes, Redis types and
score encodings of Node BullMQ (default prefix `bull`). Corvis MUST NOT write keys under the queue prefix that Node
BullMQ does not know.

#### Scenario: State snapshot after a mixed run
- **WHEN** the same scenario runs with Node producer and Node worker on one queue, and with Corvis producer and
  Corvis worker on another
- **THEN** after normalization (placeholders for UUIDs and lock tokens, order instead of timestamps, parsed JSON
  fields, `stacktrace` compared by length, `meta.version`/`meta.library` excluded) both queues contain the same keys
  with the same types and contents

#### Scenario: Custom prefix
- **WHEN** a Corvis queue is configured with prefix `{app}` and a Node queue with the same name uses prefix `{app}`
- **THEN** both see the same jobs, and no key outside `{app}:<q>:*` is written

### Requirement: Same job hash as Node BullMQ
Job hashes SHALL use Node BullMQ's field names, including the abbreviated `atm`, `ats`, `stc`, `pb`, `rjk`, `deid`
and `defa`. `data`, `returnvalue`, `progress` and `stacktrace` MUST be JSON, and `opts` MUST be JSON with the
abbreviated option keys `de`, `fpof`, `cpof`, `idof`, `kl` and `rdof`.

#### Scenario: Node reads a job finished by Corvis
- **WHEN** a Corvis worker completes a job after one failed attempt
- **THEN** Node `queue.getJob(id)` returns the same `attemptsMade`, `attemptsStarted`, `returnvalue`, `processedOn`
  ordering and `opts` as for a job finished the same way by a Node worker

### Requirement: Atomic state changes use the upstream BullMQ scripts
Every job state change Corvis performs (add, move to active, finish, retry, delay, stalled recovery, pause,
scheduler and flow updates) SHALL run as one of the Lua scripts published with the pinned BullMQ release, unchanged.
Corvis MUST NOT change job state with separate non-atomic commands.

#### Scenario: Scripts match the pinned release
- **WHEN** the interop suite starts
- **THEN** a check compares the SHA1 of each script the Rust side loads with the scripts of the pinned Node BullMQ
  package and fails, naming the script, if one differs other than in a documented, read-only argument

### Requirement: Documented divergences only
Any difference between the Redis state written by Corvis and by Node BullMQ for the same scenario SHALL be listed in
the queues documentation with its reason. Differences in job state, ordering, retry counts or events MUST NOT be
accepted as divergences.

#### Scenario: Library version field
- **WHEN** a queue is created only by Corvis
- **THEN** the documentation explains which `meta` field records the library and what Node's
  `queue.getVersion()` returns for that queue

#### Scenario: Mixed 5.x and 6.x pause and resume
- **WHEN** a reader looks up pausing a queue shared with BullMQ 5.x processes
- **THEN** the documentation warns that 5.x `resume` can overwrite the `wait` list when 6.x processes added jobs
  while paused
