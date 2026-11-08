# Spec Delta

## ADDED Requirements

### Requirement: Events stream entries match Node BullMQ
Every state change performed by Corvis SHALL add to `<prefix>:<q>:events` the same entries, with the same field names
and values, as Node BullMQ does for the same change (for example `completed` with `jobId`, `returnvalue` and
`prev: active`; `failed` with `failedReason`; `progress` with `data`), trimmed with the `opts.maxLenEvents` value of
the queue `meta` hash (default 10000).

#### Scenario: Differential event sequence
- **WHEN** the same scenario (add, process, progress, retry, fail) runs with Node on one queue and with Corvis on
  another
- **THEN** the ordered lists of `(event, jobId, fields other than ids and times)` are equal

### Requirement: Cross-process listeners
A provider marked `#[queue_events_listener("q")]` with `#[on_queue_event("completed")]` (and the other event names
of Node BullMQ `QueueEvents`) SHALL receive events produced by any process on the same Redis, Node or Corvis, by
reading the events stream from the time it starts, and stop on application close.

#### Scenario: Node worker, Corvis listener
- **WHEN** a Node worker completes a job of `q` while a Corvis listener for `q` is running
- **THEN** the listener's `completed` method receives the job id and the return value as JSON

#### Scenario: Corvis worker, Node listener
- **WHEN** a Corvis worker fails a job of `q` while a Node `QueueEvents` for `q` is running
- **THEN** the Node listener receives `failed` with the job id and the `failedReason`

### Requirement: Local worker events
Methods marked `#[on_worker_event("completed" | "failed" | "progress" | "active" | ...)]` on a processor SHALL run in
the same process for the jobs that processor's worker handles, with the job and the result or error, like
`@OnWorkerEvent` listeners. An error in such a method MUST be logged and MUST NOT change the job's outcome.

#### Scenario: Failing listener
- **WHEN** an `#[on_worker_event("completed")]` method returns an error
- **THEN** the error is logged with the processor and method names and the job stays `completed`
