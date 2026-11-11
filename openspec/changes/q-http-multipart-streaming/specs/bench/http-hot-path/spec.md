# Spec Delta

## ADDED Requirements

### Requirement: The HTTP benchmark covers uploads
`bench/http/run.sh` SHALL also measure the scenarios `upload-disk-8m` (an 8 MiB file to a `disk_storage` route) and
`upload-memory-1m` (a 1 MiB file to a `memory_storage` route), checking the status of each before measuring it and
reporting the same figures as for the other scenarios.

#### Scenario: Upload scenarios in a full run
- **WHEN** `bash bench/http/run.sh --label after` runs
- **THEN** the printed table has rows for `upload-disk-8m` and `upload-memory-1m` with requests per second and bytes
  allocated per request

#### Scenario: Upload that fails before measuring
- **WHEN** the upload route answers anything but its expected status during the check
- **THEN** the run stops with a message naming the scenario
