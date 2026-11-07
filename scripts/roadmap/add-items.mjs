import { execFileSync } from 'node:child_process';
const gh = (...a) => execFileSync('gh', a, { encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] });
const pid = JSON.parse(gh('project', 'view', '1', '--owner', 'randygil', '--format', 'json')).id;
const fields = JSON.parse(gh('project', 'field-list', '1', '--owner', 'randygil', '--format', 'json')).fields;
const opt = (f, pred) => { const x = fields.find((y) => y.name === f); return [x.id, x.options.find(pred).id]; };
for (const [num, size] of process.argv.slice(2).map((a) => a.split(':'))) {
  const id = JSON.parse(gh('project', 'item-add', '1', '--owner', 'randygil', '--url', `https://github.com/randygil/corvis/issues/${num}`, '--format', 'json')).id;
  for (const [fid, oid] of [opt('Status', (o) => o.name === 'Todo'), opt('Size', (o) => o.name === size), opt('Priority', (o) => o.name === 'High'), opt('Epic', (o) => o.name.startsWith('E0 '))])
    gh('project', 'item-edit', '--id', id, '--project-id', pid, '--field-id', fid, '--single-select-option-id', oid);
  console.log(`#${num} on the board`);
}
