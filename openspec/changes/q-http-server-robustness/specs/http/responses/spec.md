# Spec Delta

## ADDED Requirements

### Requirement: An @Res() response nobody can end is answered
When a `#[res]` handler in the handled mode has returned without ending the response, the framework SHALL keep waiting
while the application holds a handle of the response. When the last such handle is dropped and the response is still not
ended, it MUST answer with the generic internal-error `500` and log one error line, instead of waiting for the client to
leave.

#### Scenario: Handler that forgets to end the response
- **WHEN** a handled `#[res]` handler returns without calling `send`, `json`, `end` or `redirect` and keeps no handle
- **THEN** the client receives `500` with the generic internal-error body and one error line is logged

#### Scenario: Response ended later by a task
- **WHEN** the handler moves its `Res` into a spawned task and returns, and the task later calls `json`
- **THEN** the client receives that JSON response, not a `500`

#### Scenario: Task that drops its handle
- **WHEN** the handler moves its `Res` into a spawned task that drops it without ending the response
- **THEN** the client receives `500` once the task dropped the handle

#### Scenario: Passthrough mode
- **WHEN** a `#[res(passthrough)]` handler returns a value without ending the response
- **THEN** the response is built from the returned value, as before
