# Spec Delta

## ADDED Requirements

### Requirement: Run-time status codes outside the valid range are an internal error
A status given as a number through `IntoStatus` (`u16` or `i32`) SHALL be used as is when it is between 100 and 999,
and MUST become `500 Internal Server Error` otherwise. A number MUST NOT be truncated or wrapped into another valid
status.

#### Scenario: An i32 above the u16 range
- **WHEN** a handler builds a response with the status `65736_i32`
- **THEN** the response status is `500`, not `200`

#### Scenario: Negative and too small codes
- **WHEN** a handler builds a response with the status `-1_i32` or `99_u16`
- **THEN** the response status is `500`

#### Scenario: A valid code
- **WHEN** a handler builds a response with the status `404_i32`
- **THEN** the response status is `404`
