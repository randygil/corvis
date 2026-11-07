// Creates labels, milestones, epic issues and task sub-issues in GitHub from spec.mjs.
// Idempotent: issues are matched by title, and the result is saved to state.json.
import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync, existsSync } from 'node:fs';
import { phases, areas, epics } from './spec.mjs';

const REPO = 'randygil/corvis';
const STATE = new URL('./state.json', import.meta.url);
const state = existsSync(STATE) ? JSON.parse(readFileSync(STATE, 'utf8')) : { milestones: {}, epics: {}, tasks: {} };
const save = () => writeFileSync(STATE, JSON.stringify(state, null, 2));
const sleep = (ms) => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);

function gh(args, input) {
  return execFileSync('gh', args, { encoding: 'utf8', input, stdio: ['pipe', 'pipe', 'pipe'] });
}
function api(method, path, body) {
  const out = gh(['api', '-X', method, path, '--input', '-'], JSON.stringify(body ?? {}));
  return out ? JSON.parse(out) : null;
}
export function slug(s) {
  return s.normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '').replace(/^(.{0,60})-.*$/, (m, head) => (m.length > 60 ? head : m));
}

// Labels
const labels = [
  ['type:epic', '5319e7', 'Group of tasks'], ['type:task', '0e8a16', 'Implementable task'],
  ['type:spike', 'fbca04', 'Research before deciding'], ['type:decision', 'd93f0b', 'Needs an explicit decision'],
  ['priority:high', 'b60205', ''], ['priority:medium', 'e99695', ''], ['priority:low', 'f9d0c4', ''],
  ['size:S', 'c2e0c6', 'About one session'], ['size:M', '7fd3a8', 'Two or three sessions'], ['size:L', '2e8b57', 'More; split when starting'],
  ...Object.entries(areas).map(([k, d]) => [`area:${k}`, '1d76db', d]),
];
for (const [name, color, description] of labels) {
  gh(['label', 'create', name, '--repo', REPO, '--color', color, '--description', description, '--force']);
}
console.log(`labels: ${labels.length}`);

// Milestones
const existingMs = JSON.parse(gh(['api', `repos/${REPO}/milestones?state=all&per_page=100`]));
for (const p of phases) {
  const found = existingMs.find((m) => m.title === p.title);
  state.milestones[p.key] = found ? found.number : api('POST', `repos/${REPO}/milestones`, { title: p.title, description: p.description }).number;
}
save();
console.log('milestones:', JSON.stringify(state.milestones));

// Existing issues by title (to skip on reruns)
const existing = new Map();
for (let page = 1; ; page++) {
  const batch = JSON.parse(gh(['api', `repos/${REPO}/issues?state=all&per_page=100&page=${page}`]));
  for (const i of batch) if (!i.pull_request) existing.set(i.title, i);
  if (batch.length < 100) break;
}

function createIssue(title, body, labels, milestone) {
  if (existing.has(title)) return existing.get(title);
  const issue = api('POST', `repos/${REPO}/issues`, { title, body, labels, milestone });
  existing.set(title, issue);
  sleep(1200);
  return issue;
}

for (const e of epics) {
  const epicTitle = `[${e.key}] ${e.title}`;
  const epic = createIssue(epicTitle, `${e.goal}\n\n_Task list below (filled in after the tasks are created)._`, ['type:epic', `area:${e.area}`, `priority:${e.priority}`], state.milestones[e.phase]);
  state.epics[e.key] = { number: epic.number, id: epic.id, title: epicTitle };
  save();
  const children = [];
  for (const task of e.tasks) {
    const kind = task.kind ?? 'task';
    const change = slug(`${e.key}-${task.title}`);
    const body = [
      task.body || task.title,
      '',
      `**Epic:** #${epic.number}`,
      `**OpenSpec change:** \`${change}\` (create it with \`/opsx:propose\` when starting; it links back to this issue)`,
      task.depends ? `**Depends on:** epic ${task.depends}` : null,
      '**Definition of done:** see [ROADMAP.md](../blob/main/ROADMAP.md#definition-of-done).',
    ].filter((l) => l !== null).join('\n');
    const issue = createIssue(task.title, body, [`type:${kind}`, `area:${task.area ?? e.area}`, `priority:${e.priority}`, `size:${task.size}`], state.milestones[e.phase]);
    state.tasks[issue.number] = { epic: e.key, title: task.title, change, size: task.size, kind };
    children.push(issue);
    save();
    try {
      api('POST', `repos/${REPO}/issues/${epic.number}/sub_issues`, { sub_issue_id: issue.id });
    } catch (err) {
      const msg = String(err.stderr || err.message);
      if (!/already|exists|duplicate/i.test(msg)) console.log(`sub-issue link failed for #${issue.number}: ${msg.split('\n')[0]}`);
    }
  }
  const list = children.map((c) => `- [ ] #${c.number}`).join('\n');
  api('PATCH', `repos/${REPO}/issues/${epic.number}`, { body: `${e.goal}\n\n### Tasks\n${list}\n\nEach task becomes an OpenSpec change when work starts. Definition of done: see [ROADMAP.md](../blob/main/ROADMAP.md#definition-of-done).` });
  console.log(`${e.key}: epic #${epic.number}, ${children.length} tasks`);
}
console.log('done');
