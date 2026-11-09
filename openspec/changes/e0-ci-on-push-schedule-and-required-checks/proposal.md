# Proposal

Follow-up of [randygil/corvis#140](https://github.com/randygil/corvis/issues/140) (epic
[#1](https://github.com/randygil/corvis/issues/1)).

## Why

The CI workflows were limited to pull requests and manual runs because the repository was private and Actions
minutes were limited. The repository is now public, where GitHub-hosted runners have no minute limit. Changes were
pushed straight to `main`, so `main` itself was never checked automatically, the long fuzzing run never ran on a
schedule, and nothing stopped a commit that breaks the definition of done from landing on `main`.

## What Changes

- `ci.yml` also runs on every push to `main`; runs on `main` are not cancelled by a newer push (each commit keeps its
  result), runs on pull requests still are.
- `nightly.yml` runs on a daily schedule as well as by hand; a scheduled run uses 100,000 cases per property.
- `main` is protected: the CI jobs are required status checks, administrators included, so every change reaches
  `main` through a pull request whose checks passed.
- `CONTRIBUTING.md` describes the new triggers and the pull-request-only flow.

## Capabilities

- Modified: `infra/ci`

## Impact

- `.github/workflows/ci.yml`, `.github/workflows/nightly.yml`, `CONTRIBUTING.md`, the branch protection of `main`
  (repository setting, applied with `gh api`).
- Maintainer workflow: batches are merged through pull requests (`gh pr merge --merge`) instead of a direct push.

## Out of scope

- Required reviews (single maintainer).
- Larger or self-hosted runners.
