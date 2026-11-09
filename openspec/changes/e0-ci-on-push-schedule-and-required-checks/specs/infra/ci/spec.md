# Spec Delta

## REMOVED Requirements

### Requirement: CI runs only on pull requests and manual dispatch

**Reason**: the repository is public, so Actions minutes are no longer limited.
**Migration**: replaced by "CI runs on pull requests, pushes to main and dev, and a nightly schedule".

## ADDED Requirements

### Requirement: CI runs on pull requests, pushes to main and dev, and a nightly schedule

The CI workflow SHALL be triggered by `pull_request` (targeting `main` or `dev`), by `push` to `main` or `dev` and by
`workflow_dispatch`. A newer push to the same pull request SHALL cancel the run in progress; runs on `main` and `dev`
MUST NOT be cancelled by a newer push.

#### Scenario: Workflow triggers
- **WHEN** a maintainer inspects `.github/workflows/ci.yml`
- **THEN** it has the `pull_request` and `push` triggers (branches `main` and `dev`) and `workflow_dispatch`

#### Scenario: A push to dev
- **WHEN** a commit is pushed to `dev` or a pull request is merged into `main`
- **THEN** the CI workflow runs on that commit and is not cancelled by a later push

### Requirement: The long checks run every day on dev

The long checks (100,000-case fuzzing and `cargo audit` against the latest advisory database) SHALL run in a separate
workflow triggered by a daily `schedule` and by `workflow_dispatch`. A scheduled run SHALL check out `dev` and use
100,000 cases per property.

#### Scenario: A scheduled run
- **WHEN** the daily schedule of `nightly.yml` fires
- **THEN** both jobs check out `dev`, and the fuzzing suites run with `PROPTEST_CASES=100000`

### Requirement: Changes reach main only through checked pull requests

Work SHALL land on `dev`, the integration branch, and reach `main` through a pull request. The `main` branch SHALL be
protected so that the CI jobs (`fmt`, `clippy` on Linux and Windows, `test` on Linux and Windows, `fuzz`, `deny`,
`audit`, `docs`, `packaging`) are required status checks, enforced for administrators too, with force pushes and
deletion blocked. A direct push to `main` MUST be rejected. `dev` SHALL block force pushes and deletion.

#### Scenario: A direct push
- **WHEN** a maintainer pushes a commit straight to `main`
- **THEN** GitHub rejects the push because the required status checks have not passed on it

#### Scenario: A pull request with a failing check
- **WHEN** a required job fails on a pull request
- **THEN** the pull request cannot be merged until the job passes
