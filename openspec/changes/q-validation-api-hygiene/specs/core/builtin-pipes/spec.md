## ADDED Requirements

### Requirement: Every failing built-in pipe takes a status code
`ParseIntPipe`, `ParseFloatPipe`, `ParseBoolPipe`, `ParseUUIDPipe`, `ParseArrayPipe` and `ParseEnumPipe` SHALL accept
`error_http_status_code(u16)`, answer their validation failures with that status, and fall back to 400 for a value that
is not a valid status code.

#### Scenario: Enum pipe with a custom status
- **WHEN** `ParseEnumPipe::new(["a", "b"]).error_http_status_code(422)` transforms `"c"`
- **THEN** the answer has status 422 and the body recorded for the original pipe with the same option

#### Scenario: Array pipe with a custom status
- **WHEN** `ParseArrayPipe::new().numbers().error_http_status_code(422)` transforms `"1,x"`
- **THEN** the answer has status 422 and the body recorded for the original pipe with the same option
