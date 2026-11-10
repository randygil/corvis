// Records what `cookie-signature` 1.0.7 and `cookie-parser` 1.4.7 (express 5.3.0) do with signed cookies, for the signed
// cookies of Corvis. Run once with `npm ci && node signed_cookies.mjs`; the output (`signed_cookies.json`) is committed
// and replayed by `tests/signed_cookies.rs`. The server binds to 127.0.0.1 only. All secrets are fake.
//
// `sign`: `'s:' + signature.sign(value, secret)` for fixed values and secrets.
// `read`: a `Cookie` header sent to `cookieParser(secrets)`; `signed` is `req.signedCookies[name]` (`false` when the
//         signature is wrong, `null` when the key is not there at all).
// `set`: the `Set-Cookie` header of `res.cookie(name, value, { signed: true, ...options })` with the first secret.
//
// A case with a `divergence` is a behavior Corvis changes on purpose; the Rust test asserts the specification instead.
import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import express from 'express';
import cookieParser from 'cookie-parser';

const require = createRequire(import.meta.url);
const signature = require('cookie-signature');
const version = (name) => JSON.parse(readFileSync(new URL(`./node_modules/${name}/package.json`, import.meta.url), 'utf8')).version;

const SECRETS = {
  a: 'corvis-fixture-secret-A-0123456789-abcdef',
  b: 'corvis-fixture-secret-B-9876543210-fedcba',
};

const VALUES = ['hello', '', 'a.b.c', 'café', 'with space', 'x'.repeat(100), 'j:{"a":1}', 's:nested', '\u{1F600}', '%41'];

const sign = [];
for (const value of VALUES) {
  for (const secret of ['a', 'b']) {
    sign.push({ value, secret, signed: 's:' + signature.sign(value, SECRETS[secret]) });
  }
}

const app = express();
function route(secrets) {
  const r = express.Router();
  r.use(cookieParser(secrets.map((s) => SECRETS[s])));
  r.get('/read', (req, res) => {
    const out = {};
    for (const name of ['prefs', 'other']) {
      out[name] = Object.hasOwn(req.signedCookies, name) ? req.signedCookies[name] : null;
    }
    res.json(out);
  });
  r.get('/set', (req, res) => {
    const options = { signed: true };
    if (req.query.httpOnly === '1') options.httpOnly = true;
    if (req.query.secure === '1') options.secure = true;
    if (req.query.sameSite) options.sameSite = req.query.sameSite;
    if (req.query.path) options.path = req.query.path;
    if (req.query.maxAge) options.maxAge = Number(req.query.maxAge);
    res.cookie(req.query.name ?? 'prefs', req.query.value ?? 'dark', options);
    res.json({});
  });
  return r;
}
app.use('/a', route(['a']));
app.use('/ba', route(['b', 'a']));
app.use('/b', route(['b']));

const server = await new Promise((resolve) => {
  const s = app.listen(0, '127.0.0.1', () => resolve(s));
});
const base = `http://127.0.0.1:${server.address().port}`;

const enc = (s) => encodeURIComponent(s);
const flip = (s, i) => s.slice(0, i) + (s[i] === 'A' ? 'B' : 'A') + s.slice(i + 1);
const signedA = 's:' + signature.sign('dark', SECRETS.a);
const signedB = 's:' + signature.sign('dark', SECRETS.b);
const bigValue = 'v'.repeat(5000);
const READ = [
  // [name, mount, cookie header, divergence]
  ['valid', 'a', `prefs=${enc(signedA)}`],
  ['valid, quoted value', 'a', `prefs="${enc(signedA)}"`],
  ['valid, raw colon', 'a', `prefs=${signedA}`],
  ['valid among other cookies', 'a', `x=1; prefs=${enc(signedA)}; y=2`],
  ['valid for the second name', 'a', `other=${enc(signedA)}`],
  ['first occurrence wins', 'a', `prefs=${enc(signedA)}; prefs=${enc('s:' + signature.sign('light', SECRETS.a))}`],
  ['first occurrence invalid, second valid', 'a', `prefs=${enc(flip(signedA, signedA.length - 1))}; prefs=${enc(signedA)}`],
  ['tampered signature', 'a', `prefs=${enc(flip(signedA, signedA.length - 1))}`],
  ['tampered value', 'a', `prefs=${enc('s:' + 'darl' + signedA.slice(6))}`],
  ['truncated signature', 'a', `prefs=${enc(signedA.slice(0, -1))}`],
  ['extra signature byte', 'a', `prefs=${enc(signedA + 'A')}`],
  ['padded signature', 'a', `prefs=${enc(signedA + '=')}`],
  ['no prefix', 'a', `prefs=${enc(signedA.slice(2))}`],
  ['prefix only', 'a', `prefs=${enc('s:')}`],
  ['no dot', 'a', `prefs=${enc('s:dark')}`],
  ['unsigned', 'a', 'prefs=dark'],
  ['empty cookie', 'a', 'prefs='],
  ['signed with the other secret', 'a', `prefs=${enc(signedB)}`],
  ['retired secret accepted by the rotated keys', 'ba', `prefs=${enc(signedA)}`],
  ['current secret accepted by the rotated keys', 'ba', `prefs=${enc(signedB)}`],
  ['removed secret refused', 'b', `prefs=${enc(signedA)}`],
  ['empty signed value', 'a', `prefs=${enc('s:' + signature.sign('', SECRETS.a))}`],
  ['utf-8 value', 'a', `prefs=${enc('s:' + signature.sign('café', SECRETS.a))}`],
  ['value with dots', 'a', `prefs=${enc('s:' + signature.sign('a.b.c', SECRETS.a))}`],
  ['invalid percent escape', 'a', `prefs=${signedA.replace(':', '%zz')}`],
  [
    'oversized value',
    'a',
    `prefs=${enc('s:' + signature.sign(bigValue, SECRETS.a))}`,
    'the original verifies any length; Corvis refuses a signed input over 4096 bytes before computing a signature',
  ],
];

const read = [];
for (const [name, mount, header, divergence] of READ) {
  const res = await fetch(`${base}/${mount}/read`, { headers: { cookie: header } });
  const body = await res.json();
  read.push({ name, secrets: { a: ['a'], ba: ['b', 'a'], b: ['b'] }[mount], header, status: res.status, signed: body.prefs, other: body.other, ...(divergence ? { divergence } : {}) });
}

const SET = [
  ['defaults', {}],
  ['value with space and colon', { value: 'a b:c' }],
  ['http only, secure, lax', { httpOnly: '1', secure: '1', sameSite: 'lax' }],
  ['path and strict', { path: '/app', sameSite: 'strict' }],
  ['utf-8 value', { value: 'café' }],
];
const set = [];
for (const [name, query] of SET) {
  const url = new URL(`${base}/a/set`);
  for (const [k, v] of Object.entries(query)) url.searchParams.set(k, v);
  const res = await fetch(url);
  set.push({ name, query, setCookie: res.headers.getSetCookie() });
}

server.close();
const out = {
  versions: { 'cookie-signature': version('cookie-signature'), 'cookie-parser': version('cookie-parser'), express: version('express'), cookie: version('cookie') },
  secrets: SECRETS,
  sign,
  read,
  set,
};
writeFileSync(new URL('./signed_cookies.json', import.meta.url), JSON.stringify(out, null, 2) + '\n');
console.log(`signed_cookies.json: ${sign.length} signatures, ${read.length} reads, ${set.length} set-cookie headers`);
