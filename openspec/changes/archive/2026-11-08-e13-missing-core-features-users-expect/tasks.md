# Tasks

## 1. Review the spike

- [x] 1.1 Read `design.md` and settle its Open questions (request scope shape, gateway defaults, gateway filter
  contract, TLS entry point, compression reference version and default features, Config async load factories,
  throttler header default, phase placement); record each answer under the question in `design.md`; verify no
  question is left without an answer or an explicit "decide in the follow-up change"
- [x] 1.2 Update the affected issue bodies in `new-issues.md` to match those answers (scope bullets, size, priority);
  verify each issue still matches its section in `design.md`
- [x] 1.3 Run `openspec validate e13-missing-core-features-users-expect` and verify it reports the change as valid
  (`.openspec.yaml` sets `skip_specs: true` because a spike has no spec delta)

## 2. File the new issues

- [x] 2.1 Create the six issues from `new-issues.md` in `randygil/corvis` with their labels and the
  "Phase 0 — Foundations" milestone (or the milestone chosen in 1.1), each as a sub-issue of epic #13; verify with
  `gh issue view 13 --repo randygil/corvis` that the six appear as sub-issues
- [x] 2.2 Add each issue to the "Corvis Roadmap" project in Todo; verify on the board
- [x] 2.3 Add a "Discovered in" or "Follow-up" note on #23 listing the six new issue numbers; verify the comment
  links resolve

## 3. Update the plan

- [x] 3.1 Add the six issues to the E13 table in `ROADMAP.md` (number, task, size, OpenSpec change name); verify the
  table renders and the change names match `new-issues.md`
- [x] 3.2 Add the six issues to the phases of #111 as decided in 1.1 (suggested: gateway enhancers in Phase 0.4 before
  route posture; the rest in a new phase after Phase 0.6); verify #111 shows every new issue exactly once
- [x] 3.3 If 1.1 decided that some "Out of scope" items of `proposal.md` become issues (request decompression, global
  pipes and interceptors on gateways, `req.protocol()` / `req.secure()`, throttler extras), file them the same way;
  otherwise record that they were not filed

## 4. Definition of done

- [ ] 4.1 No code changed in this change: `git diff --stat main -- crates examples` is empty, so there are no new
  warnings to check; record it
- [ ] 4.2 `cargo clippy --workspace --all-targets -- -D warnings`: not needed (no code changed); record it (the
  workspace state is tracked in #108)
- [ ] 4.3 `cargo test --workspace`: not needed (no code changed); record it
- [ ] 4.4 Regression tests: none, a spike adds no behavior; each new issue lists its own
- [ ] 4.5 Docs: no page changes; the existing pages (`docs/` and `docs/es/`) already describe the six features as
  missing and are updated by the follow-up changes; confirm `npm run docs:check` in `docs/` still passes if anything
  under `docs/` was touched (it should not be)
- [ ] 4.6 corvis-parity check: not needed, no framework runtime code changed; record it

## Workflow follow-up

- Archive the change with `/opsx:archive` (without spec updates: `openspec archive
  e13-missing-core-features-users-expect --skip-specs`).
- Close issue #23, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).
