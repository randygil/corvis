# Spec Delta

## ADDED Requirements

### Requirement: A generated project is built and checked
CI SHALL run, in a job named `generated-project`, a project created by `corvis new` with the workspace's framework crates
as path dependencies and the workspace lock file: every schematic and `g res` generated into it, then `cargo build
--all-targets` and `cargo clippy` with warnings denied, `cargo test`, and `corvis posture`. The job SHALL be a required
status check of `main`, in addition to the checks listed in "Changes reach main only through checked pull requests".

#### Scenario: A template that no longer compiles
- **WHEN** a pull request changes a framework API that a template uses
- **THEN** the `generated-project` job fails at the build step and names the generated file

#### Scenario: Posture of the generated project
- **WHEN** the job runs `corvis posture --json` in the generated project
- **THEN** the unprotected entries are exactly the generated controllers' routes and the gateway handler, `GET /` is
  public, and a second project generated with `--public` gives status 0

#### Scenario: No new downloads
- **WHEN** the job builds the generated project
- **THEN** it resolves the dependency versions recorded in the workspace lock file and needs no network access beyond
  the crates.io access the other jobs use
