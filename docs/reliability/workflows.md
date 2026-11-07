---
status: unavailable
---

# Durable workflows

Corvis has no durable workflow engine (multi-step processes that persist their state and resume after a restart or a failure).

## Alternative

For short processes, chain the steps in a service and store the progress in the database with [corvis-orm](/data/corvis-orm). To resume pending steps, check that state periodically with a `#[cron]` task (see [Task scheduling](/application/task-scheduling)). For long flows or flows with compensations, use an external workflow engine and call it from a provider.
