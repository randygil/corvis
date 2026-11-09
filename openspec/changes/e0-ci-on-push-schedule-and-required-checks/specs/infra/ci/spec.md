# Spec Delta

## REMOVED Requirements

### Requirement: CI runs only on pull requests and manual dispatch

**Reason**: the repository is public, so Actions minutes are no longer limited.
**Migration**: replaced by "CI runs on pull requests, pushes to main and a nightly schedule".

## ADDED Requirements

### Requirement: CI runs on pull requests, pushes to main and a nightly schedule

The CI workflow SHALL be triggered by `pull_request` (targeting `main`), by `push` to `main` and by
`workflow_dispatch`. A newer push to the same pull request SHALL cancel the run in progress; runs on `main` MUST NOT
be cancelled by a newer push. The long checks (100,000-case fuzzing and `cargo audit` against the latest advisory
database) SHALL run in a separate workflow triggered by a daily `schedule` and by `workflow_dispatch`; a scheduled run
SHALL use 100,000 cases per property.

#### Scenario: Workflow triggers
- **WHEN** a maintainer inspects the files under `.github/workflows/`
- **THEN** `ci.yml` has the `pull_request`, `push` (branch `main`) and `workflow_dispatch` triggers, and `nightly.yml`
  has the `schedule` and `workflow_dispatch` triggers

#### Scenario: A merge to main
- **WHEN** a pull request is merged into `main`
- **THEN** the CI workflow runs on the merge commit and is not cancelled by a later push

### Requirement: Changes reach main only through checked pull requests

The `main` branch SHALL be protected so that the CI jobs (`fmt`, `clippy` on Linux and Windows, `test` on Linux and
Windows, `fuzz`, `deny`, `audit`, `docs`, `packaging`) are required status checks, enforced for administrators too.
A direct push to `main` MUST be rejected.

#### Scenario: A direct push
- **WHEN** a maintainer pushes a commit straight to `main`
- **THEN** GitHub rejects the push because the required status checks have not passed on it

#### Scenario: A pull request with a failing check
- **WHEN** a required job fails on a pull request
- **THEN** the pull request cannot be merged until the job passes
