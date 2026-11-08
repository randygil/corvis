# Spec Delta

## ADDED Requirements

### Requirement: Worker and global rate limits
A worker `limiter { max, duration }` and a global rate limit set on the queue (`meta` fields `max` and `duration`)
SHALL limit how many jobs start per `duration` milliseconds across all workers of the queue, using the queue's
`limiter` key as Node BullMQ does, so Node and Corvis workers on one queue share the same limit.

#### Scenario: Shared limit between languages
- **WHEN** a Node worker and a Corvis worker both use `limiter { max: 2, duration: 1000 }` on a queue with 10 waiting
  jobs
- **THEN** no window of 1000 ms sees more than 2 jobs started in total, measured from the jobs' `processedOn`

#### Scenario: Manual rate limit from a processor
- **WHEN** a Corvis processor calls the queue's rate-limit operation for 500 ms and returns `JobError::rate_limited`
- **THEN** the job goes back to `wait` without incrementing `atm` (as Node's `moveJobFromActiveToWait` script), and
  no worker of the queue starts a job for 500 ms

### Requirement: Global concurrency
A global concurrency set on the queue (`meta` field `concurrency`) SHALL cap the number of active jobs across all
workers of the queue, Node and Corvis alike.

#### Scenario: Global concurrency of 1
- **WHEN** the queue's global concurrency is 1 and a Node worker and a Corvis worker each have local concurrency 5
- **THEN** at most one job is in `active` at any time
