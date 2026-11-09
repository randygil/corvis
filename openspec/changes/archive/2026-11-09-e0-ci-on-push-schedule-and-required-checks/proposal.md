# Proposal

Follow-up of [randygil/corvis#140](https://github.com/randygil/corvis/issues/140) (epic
[#1](https://github.com/randygil/corvis/issues/1)).

## Why

The CI workflows were limited to pull requests and manual runs because the repository was private and Actions
minutes were limited. The repository is now public, where GitHub-hosted runners have no minute limit. Changes were
pushed straight to `main`, so `main` itself was never checked automatically, the long fuzzing run never ran on a
schedule, and nothing stopped a commit that breaks the definition of done from landing on `main`.

## What Changes

- Work lands on a new `dev` integration branch; `main` stays as it is until the roadmap phases and open issues are
  done, and then receives `dev` through a pull request.
- `ci.yml` also runs on pull requests to `dev` and on every push to `main` or `dev`; those runs are not cancelled by
  a newer push (each commit keeps its result), runs on pull requests still are.
- `nightly.yml` runs on a daily schedule as well as by hand; a scheduled run checks out `dev` and uses 100,000 cases
  per property.
- `main` is protected: the CI jobs are required status checks, administrators included, so every change reaches
  `main` through a pull request whose checks passed; force pushes and deletion are blocked on `main` and `dev`.
- `CONTRIBUTING.md` describes the branches, the triggers and the pull-request-only flow to `main`.

## Capabilities

- Modified: `infra/ci`

## Impact

- `.github/workflows/ci.yml`, `.github/workflows/nightly.yml`, `CONTRIBUTING.md`, the `dev` branch and the branch
  protection of `main` and `dev` (repository settings, applied with `gh api`).
- Maintainer workflow: batches are pushed to `dev` instead of `main`; `main` receives `dev` through one pull request.

## Review notes

- `main` stays the default branch, so the repository page keeps showing the last released state. Scheduled workflows
  always start on the default branch, so the nightly checkout names `dev` for scheduled runs; that `ref` is dropped
  when `dev` is retired.
- `dev` gets no required checks: CI runs on every push to it, and a red run is fixed by the next push. Requiring
  checks there would force a pull request per batch for one maintainer.

## Out of scope

- Required reviews (single maintainer).
- Larger or self-hosted runners.
