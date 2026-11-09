---
name: corvis-task
description: The life cycle of one Corvis roadmap task, from GitHub issue to OpenSpec change to closed issue, keeping the project board in sync. Use when starting, finishing or reporting the status of a roadmap task (issues in randygil/corvis), or when the user names an issue number to work on.
---

# Working one roadmap task

Planning lives in GitHub (`randygil/corvis`: milestones = phases, epics = parent issues, tasks = sub-issues) and on
the board https://github.com/users/randygil/projects/1. Each task becomes one OpenSpec change; its name is written in
the issue body (`**OpenSpec change:** \`<name>\``). `ROADMAP.md` is the overview.

Board helper (moves the issue's card): `node .claude/skills/corvis-task/board-status.mjs <issue> "<Todo|In Progress|Done>"`.

## 1. Start

1. `gh issue view <n> --repo randygil/corvis` and read its epic (`**Epic:** #m`) for context.
2. `node .claude/skills/corvis-task/board-status.mjs <n> "In Progress"`.
3. Create a branch named after the change: `git switch -c <change-name>` (from an up-to-date `dev`). Work lands on
   `dev`, the integration branch; `main` is protected and receives `dev` only through a pull request whose required
   checks passed.

## 2. Propose (planning only)

Run the OpenSpec propose workflow (`/opsx:propose <change-name>`) with the issue as input. proposal.md links the issue
and states what is out of scope. **Stop and wait for the user's review**; do not implement in the same step.

## 3. Implement (after the user approves)

- Do it directly, or delegate to the `rust-implementer` agent with the change name.
- Security-relevant areas (HTTP input, parsers, auth, WebSockets, file serving, ORM queries): run the
  `security-reviewer` agent on the diff and fix confirmed findings.
- Framework changes: run the `parity-verifier` agent (or the `corvis-parity` skill). The ports must stay at 0 diffs.
- User-facing features: the `docs-writer` agent updates `docs/` in English and Spanish.
- Definition of done is in `openspec/config.yaml` and `ROADMAP.md`.

## 4. Finish

1. Archive the change (`/opsx:archive <change-name>`), which merges its specs into `openspec/specs/`.
2. Commit on the branch and open a PR only when the user asks; no Claude or AI attribution in commits or PRs. Put
   `Closes #<n>` in the PR body.
3. When the work is merged (or the user says it is done):
   - tick the task in the epic: edit the epic body, replacing `- [ ] #<n>` with `- [x] #<n>`
     (`gh issue view <m> --json body` → edit → `gh issue edit <m> --body-file ...`);
   - close the issue with a one-paragraph summary: `gh issue close <n> --repo randygil/corvis --comment "..."`;
   - `node .claude/skills/corvis-task/board-status.mjs <n> Done`;
   - if it was the epic's last task, close the epic and move it to Done too.

## Rules

- One task per change, one change per branch. If a task turns out larger than its size, split it into new issues
  (sub-issues of the same epic, added to the board) instead of growing the change.
- New work discovered along the way becomes a new issue in the right epic, not an unplanned edit.
