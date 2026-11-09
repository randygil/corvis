## 1. Workflows

- [x] 1.1 `ci.yml`: add `push` to `main`; cancel in progress only on pull requests; update the header comment
- [x] 1.2 `nightly.yml`: add a daily `schedule`; default `PROPTEST_CASES` to 100000 when there is no input; update the
  header comment
- [x] 1.3 `CONTRIBUTING.md`: describe the triggers and the pull-request-only flow

## 2. Prove the checks (from #140)

- [ ] 2.1 Throwaway pull requests, one failure each: an unused import, a clippy warning, a misformatted line, a failing
  Postgres assertion, an English docs page without its Spanish page; record which job failed for each, then close them
- [ ] 2.2 Run `nightly.yml` by hand on `main` once; record its duration and adjust the timeout or case count
- [ ] 2.3 Check the duration of the `deny` job's tool install over these runs; cache or pin it if it is slow again

## 3. Protection

- [ ] 3.1 Protect `main` with the required status checks, enforced for administrators; verify a direct push is rejected

## 4. Definition of done

- [ ] 4.1 This change merged through a pull request with every check green
- [ ] 4.2 Archive with /opsx:archive, tick #140, close it
