# Spec Delta

## ADDED Requirements

### Requirement: Redis tests cannot pass by skipping in CI
The Linux `test` job SHALL run a Redis service container, point `CORVIS_REDIS_URL` at it, set
`CORVIS_REDIS_REQUIRED=1` and run the tests of the `socket-io-redis` feature. With `CORVIS_REDIS_REQUIRED=1` the
tests that need Redis MUST fail, instead of skipping, when the server is not reachable. Without the variable they
MUST skip with a message, on developer machines and in the Windows job, which has no service containers.

#### Scenario: The Redis service is down in CI
- **WHEN** the Linux `test` job runs with `CORVIS_REDIS_REQUIRED=1` and the Redis service is not reachable
- **THEN** the tests of `socket_io_redis` and `socket_io_redis_security` fail with a message naming
  `CORVIS_REDIS_URL`, instead of returning early

#### Scenario: A local run without Redis
- **WHEN** a developer runs `cargo test -p corvis --features socket-io-redis` without `CORVIS_REDIS_URL`
- **THEN** the tests that need Redis print a skip notice and pass

#### Scenario: Feature alone and fuzzing
- **WHEN** CI runs the per-feature clippy check and the `fuzz` job
- **THEN** `socket-io-redis` is one of the features checked alone, and the codec properties of the Redis adapter run
  with the other fuzzing suites

#### Scenario: Runs do not interfere
- **WHEN** two test runs use the same Redis server at the same time
- **THEN** each test uses a key of its own and cuts only connections that carry its own client name, and no test
  flushes the server or kills connections by type
