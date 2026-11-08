# Spec Delta

## ADDED Requirements

### Requirement: Job schedulers share Node BullMQ's format
`Queue::upsert_job_scheduler(id, repeat, template)` SHALL store a job scheduler as Node BullMQ 6 does (the `repeat`
sorted set and the `repeat:<id>` hash), with `pattern` or `every`, optional `limit`, `start_date`, `end_date`, `tz`,
and a template (name, data, options). Produced jobs MUST have ids `repeat:<id>:<millis>` and the `rjk` field. Legacy
`repeat` job options (removed in BullMQ 6) MUST NOT be offered.

#### Scenario: Scheduler created by Corvis, run by Node
- **WHEN** Corvis upserts scheduler `report` with `every: 1000` and a Node worker runs on the queue
- **THEN** the Node worker processes one `report` job per second, and Node `queue.getJobSchedulers()` lists `report`
  with the same `every` and template

#### Scenario: Upsert replaces
- **WHEN** Corvis upserts `report` with `every: 1000`, then again with `pattern: "0 * * * *"`
- **THEN** only one scheduler `report` exists, with the new pattern, as after the same calls in Node

### Requirement: Cron next runs match Node BullMQ
For a fixture of cron patterns and time zones, the next run times computed by Corvis SHALL equal those computed by
Node BullMQ (the pinned version and its `cron-parser`). A pattern for which they differ MUST be either rejected with
an error naming the unsupported construct or listed in the documentation.

#### Scenario: Recorded fixture
- **WHEN** the test computes the next 10 runs of each fixture pattern from a fixed start instant, including seconds
  fields, `L`, `#`, and daylight-saving transitions in `Europe/Madrid` and `America/New_York`
- **THEN** each list equals the one recorded from Node BullMQ, or the pattern is in the documented exceptions list
