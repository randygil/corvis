# Spec Delta

## ADDED Requirements

### Requirement: A value that cannot be serialized is never answered as null
When a handler result, `Response::try_json`, `Res::json` or `MessageEvent::try_new` is given a value whose `Serialize`
implementation fails, the framework MUST report the error: an `Err` to the caller, or the generic internal-error `500`
for a handler result. It MUST NOT answer `200` with the body `null`. `Response::json` and `MessageEvent::new` SHALL only
accept values that convert into JSON without failing.

#### Scenario: Typed handler result that fails to serialize
- **WHEN** a handler returns a map whose keys are not strings, which `serde_json` cannot serialize
- **THEN** the client receives `500` with the generic internal-error body and the error is logged

#### Scenario: Res::json with such a value
- **WHEN** a `#[res]` handler calls `res.json(&value)?` with the same map
- **THEN** the call returns `Err` and the client receives `500`

#### Scenario: A Value is not copied
- **WHEN** `Response::json(value)` is given a `serde_json::Value`
- **THEN** the response holds that value without serializing it again
