// Dev helper: prints the modules, controllers and routes the extension finds in a folder.
// Usage: node scripts/scan.mjs <dir> [--routes]
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const p = require('../out/parser.js');

const root = process.argv[2];
const showRoutes = process.argv.includes('--routes');
const files = [];
(function walk(dir) {
  for (const e of readdirSync(dir)) {
    if (e === 'target' || e === 'node_modules' || e.startsWith('.')) continue;
    const f = join(dir, e);
    if (statSync(f).isDirectory()) walk(f);
    else if (f.endsWith('.rs')) files.push(f);
  }
})(root);

const all = { controllers: [], modules: [], providers: [], routeImpls: [], handlers: [], app: [] };
for (const f of files) {
  const r = p.parseRust(readFileSync(f, 'utf8'));
  for (const k of ['controllers', 'modules', 'providers', 'routeImpls', 'handlers']) all[k].push(...r[k].map((x) => ({ ...x, file: f })));
  if (r.app) all.app.push({ ...r.app, file: f });
}
const ctx = all.app.find((a) => a.rootModule) ?? {};
console.log(`files=${files.length} modules=${all.modules.length} controllers=${all.controllers.length} providers=${all.providers.length} routes=${all.routeImpls.reduce((n, i) => n + i.routes.length, 0)} handlers=${all.handlers.length}`);
console.log('app:', JSON.stringify(all.app.map(({ file, ...a }) => a)));
if (showRoutes) {
  for (const impl of all.routeImpls) {
    const c = all.controllers.find((c) => c.name === impl.owner);
    for (const r of impl.routes) console.log(`${r.method.padEnd(7)} ${p.fullRoutePath(r, c, ctx)}  ${impl.owner}.${r.handler}`);
  }
  for (const m of all.modules) console.log(`module ${m.name}${m.global ? ' (global)' : ''}: imports=[${m.imports.map(p.displayItem).join(', ')}] controllers=[${m.controllers.join(', ')}] providers=[${m.providers.map(p.displayItem).join(', ')}]`);
}
