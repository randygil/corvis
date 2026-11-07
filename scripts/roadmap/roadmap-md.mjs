// Generates ROADMAP.md from spec.mjs and state.json (issue numbers created by create.mjs).
import { readFileSync, writeFileSync } from 'node:fs';
import { phases, epics, dod } from './spec.mjs';

const state = JSON.parse(readFileSync(new URL('./state.json', import.meta.url), 'utf8'));
const byEpic = {};
for (const [num, t] of Object.entries(state.tasks)) (byEpic[t.epic] ??= []).push({ num: Number(num), ...t });

const out = [];
out.push('# Corvis roadmap', '');
out.push('Corvis aims to be the way to move Node.js backends built on the NestJS + Prisma model to Rust **with verified behavioral parity**: the same API, the same database effects, a fraction of the resources. This roadmap is the big plan; the work happens in small, verifiable steps.', '');
out.push('## How work is organized', '');
out.push('- **Phases** are GitHub milestones. **Epics** are parent issues; their **tasks** are sub-issues, each sized to a few sessions at most.');
out.push('- When a task starts, it becomes an **OpenSpec change** (`openspec/changes/<change>/`): `/opsx:propose` writes the proposal, spec deltas, design and task list; after review, `/opsx:apply` implements it and `/opsx:archive` merges the specs into `openspec/specs/`. The change name is shown in each issue.');
out.push('- Features with an original Node library are verified with a differential test against it (same inputs to both, compare outputs and state), like the three ports verified with apidiff.');
out.push('- Sizes: **S** about one session, **M** two or three, **L** more (split it when starting).', '');
out.push('## Definition of done', '');
for (const d of dod) out.push(`- ${d}`);
out.push('');
for (const p of phases) {
  out.push(`## ${p.title}`, '', p.description, '');
  for (const e of epics.filter((x) => x.phase === p.key)) {
    const ep = state.epics[e.key];
    out.push(`### ${e.key} · ${e.title} — #${ep.number}`, '', e.goal, '');
    out.push('| Issue | Task | Size | OpenSpec change |', '|---|---|---|---|');
    for (const t of byEpic[e.key].sort((a, b) => a.num - b.num)) {
      const kind = t.kind === 'task' ? '' : ` _(${t.kind})_`;
      out.push(`| #${t.num} | ${t.title.replace(/\|/g, '\\|')}${kind} | ${t.size} | \`${t.change}\` |`);
    }
    out.push('');
  }
}
writeFileSync(process.argv[2], out.join('\n'));
console.log('written', process.argv[2]);
