// Moves an issue's card on the Corvis Roadmap board (GitHub Projects v2, randygil/projects/1).
// Usage: node board-status.mjs <issue-number> "<Todo|In Progress|Done>"
import { execFileSync } from 'node:child_process';

const [num, status] = process.argv.slice(2);
if (!num || !status) {
  console.error('usage: node board-status.mjs <issue-number> "<Todo|In Progress|Done>"');
  process.exit(2);
}
const gh = (...a) => execFileSync('gh', a, { encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] });
const OWNER = 'randygil';
const PROJECT = '1';

const projectId = JSON.parse(gh('project', 'view', PROJECT, '--owner', OWNER, '--format', 'json')).id;
const field = JSON.parse(gh('project', 'field-list', PROJECT, '--owner', OWNER, '--format', 'json')).fields.find((f) => f.name === 'Status');
const option = field.options.find((o) => o.name.toLowerCase() === status.toLowerCase());
if (!option) {
  console.error(`unknown status "${status}"; options: ${field.options.map((o) => o.name).join(', ')}`);
  process.exit(2);
}
// Adding an issue that is already on the board returns its existing item.
const url = `https://github.com/${OWNER}/corvis/issues/${num}`;
const itemId = JSON.parse(gh('project', 'item-add', PROJECT, '--owner', OWNER, '--url', url, '--format', 'json')).id;
gh('project', 'item-edit', '--id', itemId, '--project-id', projectId, '--field-id', field.id, '--single-select-option-id', option.id);
console.log(`#${num} → ${option.name}`);
