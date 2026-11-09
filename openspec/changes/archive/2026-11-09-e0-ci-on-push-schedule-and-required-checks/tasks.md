## 1. Workflows

- [x] 1.1 `ci.yml`: add `push` to `main`; cancel in progress only on pull requests; update the header comment
- [x] 1.2 `nightly.yml`: add a daily `schedule`; default `PROPTEST_CASES` to 100000 when there is no input; update the
  header comment
- [x] 1.3 `CONTRIBUTING.md`: describe the triggers and the pull-request-only flow

## 2. Prove the checks (from #140)

- [x] 2.1 Throwaway pull requests, one failure each: an unused import, a clippy warning, a misformatted line, a failing
  Postgres assertion, an English docs page without its Spanish page; record which job failed for each, then close them
- [x] 2.2 Run `nightly.yml` by hand on `main` once; record its duration and adjust the timeout or case count
- [x] 2.3 Check the duration of the `deny` job's tool install over these runs; cache or pin it if it is slow again

## 3. Protection

- [x] 3.1 Create `dev` from `main` once this change is merged
- [x] 3.2 Protect `main` with the required status checks, enforced for administrators, no force push or deletion;
  verify a direct push is rejected
- [x] 3.3 Block force pushes and deletion of `dev`

## 4. Definition of done

- [x] 4.1 This change merged into `main` through a pull request with every check green
- [x] 4.2 Archive with /opsx:archive on `dev`, tick #140, close it

## Verification (2026-11-09)

- Pull request #164: CI green on both pushes (runs 42713243639 and 42716562835), merged into `main` as 5948300.
- Throwaway pull requests #165–#169, each with one deliberate failure, closed without merging:

  | Pull request | Failure | Jobs that failed |
  | --- | --- | --- |
  | #165 | unused import in `crates/corvis/src/config.rs` | `clippy` and `test` (Linux and Windows) |
  | #166 | `v.len() == 0` | `clippy` (Linux and Windows) |
  | #167 | misformatted function | `fmt`; also `clippy` (the item sat after the test module) and `test (windows-latest)` from a flaky test, below |
  | #168 | failing assertion after a Postgres setup | `test (ubuntu-latest)` only (Windows has no Postgres and skips) |
  | #169 | `docs/throwaway.md` without `docs/es/throwaway.md` | `docs` |

  #167 exposed a race in `an_idle_sse_stream_over_h2c_is_not_cut`: the clock started when the response head arrived,
  after the server could already be in its pause. Both SSE pause tests now start the clock before the request.
- Nightly checks run by hand on `dev` (run 42718017239): `fuzz (long)` 17 min 32 s, `audit` 19 s. The fuzz timeout
  went from 120 to 45 minutes; 100,000 cases stay.
- `deny` job (run 42701896050): `taiki-e/install-action` under 1 s (prebuilt binary, version pinned), whole job 15 s.
  Nothing to cache.
- `dev` created from `main` at 5948300. `main` protected: 10 required checks (GitHub Actions app), enforced for
  administrators, no force push or deletion. A direct push of a new commit to `main` was rejected with
  "protected branch hook declined" and "10 of 10 required status checks are expected". `dev` blocks force pushes
  and deletion.
