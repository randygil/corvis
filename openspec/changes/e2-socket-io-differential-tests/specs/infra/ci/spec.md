# Spec Delta

## ADDED Requirements

### Requirement: The Socket.IO differential job runs on every pull request
The CI workflow SHALL have a Linux job `socket-io-diff` that installs Node 22 and the harness packages with `npm ci`
(install scripts disabled), runs a Redis service container, and runs the coverage check, the differential harness in
strict mode and both protocol conformance suites with `CORVIS_NODE_DIFF=1`, `CORVIS_NODE_DIFF_STRICT=1`,
`CORVIS_REDIS_URL` and `CORVIS_REDIS_REQUIRED=1`. On failure it MUST upload the transcripts as an artifact.

#### Scenario: A parity regression fails the pull request
- **WHEN** a pull request makes Corvis answer an acknowledgement with different arguments than Node
- **THEN** the `socket-io-diff` job fails, its log names the scenario and the differing path, and its artifact holds
  the transcripts of both targets

#### Scenario: Nothing is skipped in CI
- **WHEN** the Redis service of the job is not reachable, or the gateway reference package was not installed
- **THEN** the job fails instead of reporting the affected scenarios as skipped

#### Scenario: Node is missing
- **WHEN** the job runs without Node on `PATH`
- **THEN** the differential test fails instead of printing its skip notice

#### Scenario: Suites at their pinned commits
- **WHEN** the job fetches the protocol suites
- **THEN** it uses `scripts/fetch-suites.mjs` with the commits of `suites.json`, and a cached copy is reused only
  while `suites.json` and the script are unchanged

#### Scenario: Least privilege
- **WHEN** a maintainer inspects the job
- **THEN** it uses no secret, runs with the workflow's read-only token, and uses only actions already allowed by the
  workflow's pinning rule

## MODIFIED Requirements

### Requirement: Changes reach main only through checked pull requests

Work SHALL land on `dev`, the integration branch, and reach `main` through a pull request. `main` SHALL be protected
so that the CI jobs (`fmt`, `clippy` and `test` on Linux and Windows, `fuzz`, `deny`, `audit`, `docs`, `packaging`,
`socket-io-diff`) are required status checks, enforced for administrators too, with force pushes and deletion
blocked. A direct push to `main` MUST be rejected. `dev` SHALL block force pushes and deletion.

#### Scenario: A direct push
- **WHEN** a maintainer pushes a commit straight to `main`
- **THEN** GitHub rejects the push because the required status checks have not passed on it

#### Scenario: A pull request with a failing check
- **WHEN** a required job fails on a pull request
- **THEN** the pull request cannot be merged until the job passes

#### Scenario: Socket.IO parity is required
- **WHEN** the `socket-io-diff` job fails on a pull request to `main`
- **THEN** the pull request cannot be merged

### Requirement: The long checks run every day on dev

The long checks (100,000-case fuzzing, `cargo audit` against the latest advisory database, and ten consecutive
strict runs of the Socket.IO differential harness) SHALL run in a separate workflow triggered by a daily `schedule`
and by `workflow_dispatch`. A scheduled run SHALL check out `dev` and use 100,000 cases per property.

#### Scenario: A scheduled run
- **WHEN** the daily schedule of `nightly.yml` fires
- **THEN** every job checks out `dev`, and the fuzzing suites run with `PROPTEST_CASES=100000`

#### Scenario: An intermittent difference
- **WHEN** one of the ten harness runs of the nightly job reports a difference
- **THEN** the job fails without running the scenario again, and uploads the transcripts of that run
