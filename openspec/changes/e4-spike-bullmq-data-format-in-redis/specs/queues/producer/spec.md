# Spec Delta

## ADDED Requirements

### Requirement: Adding a job stores what Node BullMQ stores
`Queue::add(name, data, options)` SHALL store the job with the same id assignment (`INCR` of the `id` key unless
`job_id` is given), the same target (`delayed` when `delay > 0`, `prioritized` when `priority > 0`, otherwise `wait`,
pushed at the tail for FIFO or the head with `lifo`), the same job hash and the same events (`added`, then `waiting`
or `delayed`) as Node BullMQ `queue.add` with the same arguments. `Queue::add_bulk` SHALL do the same for each job.

#### Scenario: Options round-trip
- **WHEN** Corvis adds a job with `delay`, `attempts: 3`, exponential backoff of 1000 ms, `priority: 5`,
  `remove_on_complete: 10` and `remove_on_fail: { age: 3600 }`
- **THEN** the job hash `opts` field, parsed, equals the one Node BullMQ writes for the same options, and a Node
  `queue.getJob(id)` returns the same `opts`

#### Scenario: Existing job id
- **WHEN** Corvis adds a job with `job_id: "order-1"` and a job with that id already exists
- **THEN** no job data is overwritten, the existing id is returned and a `duplicated` event is added, as in Node

### Requirement: Same validation as Node BullMQ, before writing
Adding a job SHALL fail without writing to Redis in every case where Node BullMQ `queue.add` throws (the validation
table of design.md: reserved and integer ids, `:` in ids, priority range, `size_limit`, exclusive parent-failure
options, deduplication rules, jitter range). The error messages SHALL say what was wrong in the same terms as Node
BullMQ.

#### Scenario: Integer custom id
- **WHEN** Corvis adds a job with `job_id: "42"`
- **THEN** it returns an error saying custom ids cannot be integers, and the queue's keys are unchanged

#### Scenario: Differential validation table
- **WHEN** the interop suite runs each invalid option set of the table through Node `queue.add` and Corvis
  `Queue::add`
- **THEN** both reject the same cases and neither writes any key

### Requirement: Default job options
Default job options given to `BullModule::for_root` or to one queue SHALL be merged with the options of each `add`
the way Node BullMQ merges `defaultJobOptions` (per-call options win, field by field at the top level).

#### Scenario: Override one field
- **WHEN** the queue defaults are `attempts: 3, backoff: fixed 500` and a job is added with `attempts: 1`
- **THEN** the stored options are `attempts: 1` with `backoff` fixed 500
