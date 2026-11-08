---
status: unavailable
---

# Transactional outbox

Corvis does not implement the outbox pattern (writing the messages to publish into a table within the same transaction as the data, and publishing them later from a separate process).

## Alternative

The pattern can be built with what already exists:

- Write the business record and the pending message in the same interactive `corvis-orm` transaction (`db._transaction(|tx| async move { ... })`, see [corvis-orm](/data/corvis-orm)).
- Publish the pending messages from an `#[interval]` or `#[cron]` task (see [Task scheduling](/application/task-scheduling)) and mark them as sent.

Keep in mind that `EventEmitter` is in-memory only: an event emitted right before the process stops is lost.
