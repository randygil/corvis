// Records what `bcryptjs` 3.0.3 answers for the password hashes of `PasswordHasher` (`tests/auth_password.rs`). Run
// once with `npm ci && node bcrypt.mjs`; the output (`bcrypt.json`) is committed and replayed without Node.
//
// Two directions:
//   fromOriginal  hashes made by bcryptjs with fixed salts; for each, `compareSync` of the password, of its first 72
//                 bytes (when they are valid UTF-8), of a one-character change and of the empty string. Corvis must
//                 answer the same for every check.
//   fromCorvis    hashes made by Corvis (`corvis_bcrypt_hashes.json`, written by the ignored test
//                 `write_corvis_hashes` of `tests/auth_password.rs`); `compareSync` of each with its password and
//                 with a wrong one. bcryptjs must accept what Corvis made.
//
// Passwords over 72 bytes: bcryptjs truncates them silently (`truncates(password)`), Corvis refuses to hash them (a
// documented divergence the Rust test asserts), but both verify the hashes of such passwords the same way.
import { readFileSync, writeFileSync, existsSync } from 'node:fs';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const bcrypt = require('bcryptjs');
const version = JSON.parse(readFileSync(new URL('./node_modules/bcryptjs/package.json', import.meta.url), 'utf8')).version;

const a = (n) => 'a'.repeat(n);
const PASSWORDS = [
  ['empty', ''],
  ['ascii', 'secret'],
  ['latin', 'pässword'],
  ['emoji', '\u{1F511}secret'],
  ['nul-middle', 'pass\u0000word'],
  ['nul-end', 'pass\u0000'],
  ['nul-start', '\u0000pass'],
  ['bytes-71', a(71)],
  ['bytes-72', a(72)],
  ['bytes-73', a(73)],
  ['bytes-73-differs-at-end', a(72) + 'b'],
  ['bytes-100', a(100)],
  ['multibyte-boundary', a(71) + 'é'],
  ['space', ' leading and trailing '],
];

const SALT = (cost, prefix = '2b') => `$${prefix}$${String(cost).padStart(2, '0')}$abcdefghijklmnopqrstuu`;

const enc = new TextEncoder();
const dec = new TextDecoder('utf-8', { fatal: true });
function first72(password) {
  const bytes = enc.encode(password);
  if (bytes.length <= 72) return null;
  try {
    return dec.decode(bytes.slice(0, 72));
  } catch {
    return null;
  }
}
function changed(password) {
  return password.length === 0 ? 'x' : password.slice(0, -1) + (password.endsWith('z') ? 'y' : 'z');
}

function checksFor(password, hash) {
  const checks = [{ what: 'password', password, expected: bcrypt.compareSync(password, hash) }];
  const prefix = first72(password);
  if (prefix !== null) checks.push({ what: 'first-72-bytes', password: prefix, expected: bcrypt.compareSync(prefix, hash) });
  const other = changed(password);
  checks.push({ what: 'changed', password: other, expected: bcrypt.compareSync(other, hash) });
  checks.push({ what: 'empty', password: '', expected: bcrypt.compareSync('', hash) });
  return checks;
}

const fromOriginal = [];
for (const [name, password] of PASSWORDS) {
  fromOriginal.push({ name, cost: 4, hash: bcrypt.hashSync(password, SALT(4)) });
}
for (const [name, password] of PASSWORDS.filter(([n]) => ['ascii', 'latin', 'bytes-73'].includes(n))) {
  fromOriginal.push({ name: `${name}-cost-10`, cost: 10, hash: bcrypt.hashSync(password, SALT(10)) });
}
for (const prefix of ['2a', '2y']) {
  fromOriginal.push({ name: `ascii-${prefix}`, cost: 4, hash: bcrypt.hashSync('secret', SALT(4, prefix)) });
}
const passwordOf = new Map(PASSWORDS);
for (const entry of fromOriginal) {
  const base = entry.name.replace(/-cost-10$/, '').replace(/-(2a|2y)$/, '');
  entry.password = passwordOf.get(base);
  entry.checks = checksFor(entry.password, entry.hash);
}

const fromCorvis = [];
const corvisFile = new URL('./corvis_bcrypt_hashes.json', import.meta.url);
if (existsSync(corvisFile)) {
  for (const { name, password, hash } of JSON.parse(readFileSync(corvisFile, 'utf8'))) {
    const wrong = changed(password);
    fromCorvis.push({
      name,
      password,
      hash,
      expected: bcrypt.compareSync(password, hash),
      wrongPassword: wrong,
      wrongExpected: bcrypt.compareSync(wrong, hash),
    });
  }
}

writeFileSync(new URL('./bcrypt.json', import.meta.url), JSON.stringify({ bcryptjs: version, fromOriginal, fromCorvis }, null, 2) + '\n');
console.log(`bcryptjs ${version}: ${fromOriginal.length} original hashes, ${fromCorvis.length} Corvis hashes`);
