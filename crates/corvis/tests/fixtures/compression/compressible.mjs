// Writes crates/corvis/src/http/compression/compressible.rs from the installed mime-db: every type
// that has a `compressible` flag, sorted, as `(type, bool)` pairs.
// Usage: npm install && node compressible.mjs
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);
const db = require('mime-db');
const version = JSON.parse(readFileSync(path.join(here, 'node_modules', 'mime-db', 'package.json'), 'utf8')).version;

const entries = Object.keys(db)
  .filter((k) => typeof db[k].compressible === 'boolean')
  .sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));

const lines = entries.map((k) => `    (${JSON.stringify(k)}, ${db[k].compressible}),`);
const src = `// @generated from mime-db ${version} by crates/corvis/tests/fixtures/compression/compressible.mjs
// (run \`node compressible.mjs\` in that folder). Do not edit by hand.
//
// Every media type of mime-db that has a \`compressible\` flag, sorted by byte order for a binary search.
// ${entries.length} entries.

pub(super) const COMPRESSIBLE: &[(&str, bool)] = &[
${lines.join('\n')}
];
`;
const target = path.join(here, '..', '..', '..', 'src', 'http', 'compression', 'compressible.rs');
writeFileSync(target, src);
console.log('wrote', entries.length, 'entries to', target);
