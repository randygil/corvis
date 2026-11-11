# Spec Delta

## ADDED Requirements

### Requirement: The CLI signals only its own process group
On Unix, `corvis posture` SHALL learn that the application's process ended without reaping it, send its signal to the
process group while the ended leader still holds the group id, and reap the leader only afterwards, so the signal never
reaches a group that reused the id. On Windows the Job Object keeps doing this.

#### Scenario: Application that exits on its own
- **WHEN** the application prints its report and exits, and `corvis posture` then stops what is left of the run
- **THEN** the group signal is sent before the leader is reaped, and the exit status of the run is the leader's

#### Scenario: Helpers left behind
- **WHEN** the application started a helper process that is still running when the application exits
- **THEN** `corvis posture` ends the helper before returning, as before
