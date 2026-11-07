# Roadmap scripts

`spec.mjs` is the source of the roadmap (phases, epics, tasks). The GitHub side lives in `randygil/corvis`
(milestones, epic issues with task sub-issues) and on the board https://github.com/users/randygil/projects/1.

- `node create.mjs` creates labels, milestones and issues from `spec.mjs`. Idempotent: issues are matched by title and
  the issue numbers are kept in `state.json`.
- `node add-items.mjs <issue>:<S|M|L> ...` puts new task issues on the board (Todo, size, priority, epic E0 by default;
  adjust the epic in the board if needed).
- `node roadmap-md.mjs ../../ROADMAP.md` regenerates `ROADMAP.md` from `spec.mjs` and `state.json`.

To add a task: append it to its epic in `spec.mjs`, run `create.mjs`, `add-items.mjs` for the new issue and
`roadmap-md.mjs`. Moving cards during work: `.claude/skills/corvis-task/board-status.mjs`.
