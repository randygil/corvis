// Records what `express-session` 1.19.0 with `passport` 0.7.0 (express 5.3.0, cookie-signature 1.0.7) does for fixed
// request sequences, for the sessions of Corvis. Run once with `npm ci && node session.mjs`; the output
// (`session.json`) is committed and replayed by `tests/auth_session.rs`. The server binds to 127.0.0.1 only. All
// secrets are fake strings of at least 32 bytes.
//
// Every app below has its own `express-session` options and a store (the stock `MemoryStore`) wrapped to record each
// `get`, `set`, `touch` and `destroy`. Session identifiers are replaced by placeholders (`sid1`, `sid2`, ...) in the
// output, in the order they first show up. A step is one request: `cookie` says which cookie it sends (a placeholder
// from an earlier response, optionally mutated), `response` is what came back (status, JSON body, the parsed
// `Set-Cookie`), `ops` the store calls it caused and `store` the records the app's store holds afterwards (the
// `cookie.expires` date is replaced by `<date>`).
//
// A step with a `divergence` is a behavior Corvis changes on purpose; the Rust test asserts the specification
// (named in the text) instead of the recorded behavior for the part that differs.
import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import express from 'express';
import session from 'express-session';
import passport from 'passport';
import { Strategy as CustomStrategy } from 'passport-custom';

const require = createRequire(import.meta.url);
const signature = require('cookie-signature');
const version = (name) => JSON.parse(readFileSync(new URL(`./node_modules/${name}/package.json`, import.meta.url), 'utf8')).version;

const SECRETS = {
  new: 'corvis-session-secret-NEW-0123456789abcdef',
  old: 'corvis-session-secret-OLD-fedcba9876543210',
};
const UNKNOWN_ID = 'unknownUnknownUnknownUnknown0001';
const USERS = { 1: { id: '1', name: 'ana' }, 2: { id: '2', name: 'bob' } };
const deleted = new Set();

// The options of every app; the Rust test builds the same configuration from this table.
const APPS = {
  // The defaults of the original, given explicitly (it warns when they are not).
  original: { secrets: ['new'], resave: true, saveUninitialized: true },
  // What Corvis defaults to.
  strict: { secrets: ['new'], resave: false, saveUninitialized: false },
  rolling: { secrets: ['new'], resave: false, saveUninitialized: false, rolling: true },
  maxage: { secrets: ['new'], resave: false, saveUninitialized: false, maxAge: 3600000 },
  // Two apps that share one store: `old-only` signs with the old secret, `rotated` signs with the new and verifies both.
  'old-only': { secrets: ['old'], resave: false, saveUninitialized: false, store: 'rotation' },
  rotated: { secrets: ['new', 'old'], resave: false, saveUninitialized: false, store: 'rotation' },
  login: { secrets: ['new'], resave: false, saveUninitialized: false },
  // A fixed identifier, so that the cookie and the record the original produces are the same on every run.
  seeded: { secrets: ['new'], resave: false, saveUninitialized: false, genid: 'FIXEDseedSEEDfixedSEEDfixed00001' },
  keep: { secrets: ['new'], resave: false, saveUninitialized: false, keepSessionInfo: true },
};

const stores = {};
const ops = [];
const sids = new Map();
const ph = (id) => {
  if (id === UNKNOWN_ID) return 'unknown';
  if (!sids.has(id)) sids.set(id, `sid${sids.size + 1}`);
  return sids.get(id);
};

function recordingStore(group) {
  if (stores[group]) return stores[group];
  const inner = new session.MemoryStore();
  const wrap = (name) => {
    const original = inner[name].bind(inner);
    inner[name] = (...args) => {
      const id = args[0];
      if (name === 'destroy') {
        const existed = Object.hasOwn(inner.sessions, id);
        ops.push({ op: 'destroy', id, existed });
      } else {
        ops.push({ op: name, id });
      }
      return original(...args);
    };
  };
  for (const name of ['get', 'set', 'touch', 'destroy']) wrap(name);
  stores[group] = inner;
  return inner;
}

function appFor(name, cfg) {
  const app = express();
  const store = recordingStore(cfg.store ?? name);
  app.use(
    session({
      secret: cfg.secrets.map((s) => SECRETS[s]),
      resave: cfg.resave,
      saveUninitialized: cfg.saveUninitialized,
      rolling: cfg.rolling ?? false,
      ...(cfg.genid ? { genid: () => cfg.genid } : {}),
      store,
      cookie: { secure: false, ...(cfg.maxAge ? { maxAge: cfg.maxAge } : {}) },
    }),
  );
  passport.use('header', new CustomStrategy((req, done) => done(null, USERS[req.headers['x-user']] ?? false)));
  app.use(passport.initialize());
  app.use(passport.session());
  app.get('/sess', (req, res) => {
    const { cookie, ...data } = JSON.parse(JSON.stringify(req.session));
    res.json({ data });
  });
  app.post('/set', (req, res) => {
    req.session[req.query.k] = req.query.v;
    res.json({ ok: true });
  });
  app.post('/set-same', (req, res) => {
    // The value it already has: the session does not count as modified.
    req.session[req.query.k] = req.session[req.query.k];
    res.json({ ok: true });
  });
  app.post('/regenerate', (req, res, next) => {
    req.session.regenerate((err) => (err ? next(err) : res.json({ ok: true })));
  });
  app.post('/destroy', (req, res, next) => {
    req.session.destroy((err) => (err ? next(err) : res.json({ ok: true })));
  });
  app.post('/login', (req, res, next) => {
    passport.authenticate('header', { session: false, failWithError: true })(req, res, (err) => {
      if (err) return next(err);
      req.logIn(req.user, { keepSessionInfo: cfg.keepSessionInfo ?? false }, (err) => (err ? next(err) : res.json({ user: req.user })));
    });
  });
  app.post('/logout', (req, res, next) => {
    req.logout({ keepSessionInfo: cfg.keepSessionInfo ?? false }, (err) => (err ? next(err) : res.json({ ok: true })));
  });
  app.get('/me', (req, res) => {
    if (!req.isAuthenticated()) return res.status(401).json({ message: 'Unauthorized', statusCode: 401 });
    res.json({ user: req.user });
  });
  app.post('/admin/delete', (req, res) => {
    deleted.add(req.query.id);
    res.json({ ok: true });
  });
  app.post('/admin/restore', (req, res) => {
    deleted.delete(req.query.id);
    res.json({ ok: true });
  });
  app.use((err, req, res, next) => {
    if (err.status === 401) return res.status(401).json({ message: 'Unauthorized', statusCode: 401 });
    res.status(500).json({ message: 'Internal server error', statusCode: 500 });
  });
  return app;
}

passport.serializeUser((user, done) => done(null, user.id));
passport.deserializeUser((id, done) => done(null, deleted.has(id) ? false : USERS[id]));

const root = express();
for (const [name, cfg] of Object.entries(APPS)) root.use(`/${name}`, appFor(name, cfg));
const server = await new Promise((resolve) => {
  const s = root.listen(0, '127.0.0.1', () => resolve(s));
});
const base = `http://127.0.0.1:${server.address().port}`;

// The jar: the raw `name=value` pair of the last Set-Cookie of each placeholder.
const jar = new Map();

function parseSetCookie(header) {
  const [pair, ...attrs] = header.split('; ');
  const value = decodeURIComponent(pair.slice(pair.indexOf('=') + 1));
  const out = { flags: [], attributes: {} };
  for (const attr of attrs) {
    const [k, v] = attr.split('=');
    if (v === undefined) out.flags.push(k);
    else out.attributes[k] = k === 'Expires' ? '<date>' : v;
  }
  if (value === '') {
    out.cleared = true;
  } else if (value.startsWith('s:')) {
    const id = value.slice(2, value.lastIndexOf('.'));
    out.sid = ph(id);
    out.signed = signature.unsign(value.slice(2), SECRETS.new) !== false ? 'new' : signature.unsign(value.slice(2), SECRETS.old) !== false ? 'old' : 'none';
  }
  return out;
}

function cookieHeader(spec) {
  if (!spec) return undefined;
  const entry = jar.get(spec.ref);
  const raw = entry.pair; // connect.sid=s%3A<id>.<sig>
  const value = decodeURIComponent(raw.slice(raw.indexOf('=') + 1));
  const id = value.slice(2, value.lastIndexOf('.'));
  switch (spec.mutate) {
    case undefined:
      return raw;
    case 'flip-signature': {
      const last = raw.slice(-1);
      return raw.slice(0, -1) + (last === 'A' ? 'B' : 'A');
    }
    case 'unsigned':
      return `connect.sid=${id}`;
    case 'retired-secret':
      return 'connect.sid=' + encodeURIComponent('s:' + signature.sign(id, SECRETS.old));
    default:
      throw new Error(`unknown mutation ${spec.mutate}`);
  }
}

function normalize(record) {
  const copy = JSON.parse(JSON.stringify(record));
  if (copy.cookie && copy.cookie.expires) copy.cookie.expires = '<date>';
  return copy;
}

function snapshot(group) {
  const store = stores[group];
  const out = {};
  for (const [id, text] of Object.entries(store.sessions)) out[ph(id)] = normalize(JSON.parse(text));
  return out;
}

const steps = [];
async function run(name, appName, spec, divergence) {
  ops.length = 0;
  const headers = { ...(spec.headers ?? {}) };
  let cookie;
  if (spec.cookie === 'unknown') cookie = 'connect.sid=' + encodeURIComponent('s:' + signature.sign(UNKNOWN_ID, SECRETS.new));
  else cookie = cookieHeader(spec.cookie);
  if (cookie) headers.cookie = cookie;
  const res = await fetch(`${base}/${appName}${spec.path}`, { method: spec.method ?? 'GET', headers });
  const text = await res.text();
  let body;
  try {
    body = JSON.parse(text);
  } catch {
    body = text;
  }
  const setCookies = res.headers.getSetCookie().map((h) => {
    const parsed = parseSetCookie(h);
    if (parsed.sid && spec.as) jar.set(spec.as, { pair: h.split('; ')[0] });
    return parsed;
  });
  const group = APPS[appName].store ?? appName;
  steps.push({
    name,
    app: appName,
    request: { method: spec.method ?? 'GET', path: spec.path, headers: spec.headers ?? {}, cookie: spec.cookie ?? null, as: spec.as ?? null },
    response: { status: res.status, body, setCookie: setCookies },
    ops: ops.map((o) => ({ op: o.op, sid: ph(o.id), ...(o.op === 'destroy' ? { existed: o.existed } : {}) })),
    store: snapshot(group),
    ...(divergence ? { divergence } : {}),
  });
  return res;
}

const ME = { path: '/me' };
const cookie = (ref, mutate) => ({ ref, ...(mutate ? { mutate } : {}) });
const post = (path, ref, extra = {}) => ({ method: 'POST', path, cookie: ref ? cookie(ref) : null, ...extra });

// 1. The defaults of the original (resave and saveUninitialized true).
await run('original: anonymous request creates a session', 'original', { path: '/sess', as: 'o1' });
await run('original: second request saves again', 'original', { path: '/sess', cookie: cookie('o1') });
await run('original: a write', 'original', post('/set?k=views&v=1', 'o1'));
await run('original: read back', 'original', { path: '/sess', cookie: cookie('o1') });

// 2. What Corvis defaults to (resave and saveUninitialized false).
await run('strict: anonymous request creates nothing', 'strict', { path: '/sess' });
await run('strict: a write creates the session', 'strict', post('/set?k=views&v=1', null, { as: 's1' }));
await run('strict: unmodified request touches', 'strict', { path: '/sess', cookie: cookie('s1') });
await run('strict: a write to an existing session', 'strict', post('/set?k=views&v=2', 's1'));
await run('strict: the value it already has', 'strict', post('/set-same?k=views', 's1'));
await run('strict: tampered signature', 'strict', { path: '/sess', cookie: cookie('s1', 'flip-signature') });
await run('strict: unsigned identifier', 'strict', { path: '/sess', cookie: cookie('s1', 'unsigned') });
await run('strict: signed but unknown identifier, then a write', 'strict', post('/set?k=a&v=b', null, { cookie: 'unknown', as: 's-unknown' }));
await run('strict: regenerate', 'strict', post('/regenerate', 's1', { as: 's1b' }));
await run(
  'strict: destroy',
  'strict',
  post('/destroy', 's1b'),
  'express-session sends no Set-Cookie after destroy(); Corvis clears the cookie (the browser drops it) and still destroys the record',
);

// 3. Rolling and expiry.
await run('rolling: anonymous request', 'rolling', { path: '/sess' });
await run('rolling: a write', 'rolling', post('/set?k=a&v=1', null, { as: 'r1' }));
await run('rolling: unmodified request sets the cookie again', 'rolling', { path: '/sess', cookie: cookie('r1') });
await run('maxage: a write sets an expiry', 'maxage', post('/set?k=a&v=1', null, { as: 'm1' }));
await run('maxage: unmodified request sends no cookie', 'maxage', { path: '/sess', cookie: cookie('m1') });
await run('maxage: a write sends the cookie again', 'maxage', post('/set?k=a&v=2', 'm1'));

// 4. Two secrets.
await run('rotation: a session created with the old secret', 'old-only', post('/set?k=a&v=1', null, { as: 'old1' }));
await run(
  'rotation: the rotated app accepts it and signs again',
  'rotated',
  { path: '/sess', cookie: cookie('old1') },
  'express-session sends no Set-Cookie for an unmodified session; Corvis sends the cookie signed with the current secret',
);

// 5. Login and logout through passport.
await run('login: anonymous request to a protected route', 'login', ME);
await run('login: bad credentials', 'login', post('/login', null, { headers: { 'x-user': '9' } }));
await run(
  'login: without a previous session',
  'login',
  post('/login', null, { headers: { 'x-user': '1' }, as: 'l1' }),
  'express-session calls store.destroy for the identifier of the discarded session even when it was never stored; Corvis does not',
);
await run('login: request with the session user', 'login', { path: '/me', cookie: cookie('l1') });
await run('login: user deleted after login', 'login', post('/admin/delete?id=1'));
await run('login: the deleted user is anonymous', 'login', { path: '/me', cookie: cookie('l1') });
await run('login: the session lost the user', 'login', { path: '/sess', cookie: cookie('l1') });
await run('login: restore the user', 'login', post('/admin/restore?id=1'));
await run('login: anonymous session with data', 'login', post('/set?k=cart&v=milk', null, { as: 'anon' }));
await run('login: fixation attempt, login while presenting a pre-login cookie', 'login', post('/login', 'anon', { headers: { 'x-user': '2' }, as: 'l2' }));
await run('login: the old cookie is anonymous', 'login', { path: '/me', cookie: cookie('anon') });
await run('login: the data of before the login is gone', 'login', { path: '/sess', cookie: cookie('l2') });
await run('login: failed login keeps the session', 'login', post('/login', 'l2', { headers: { 'x-user': '9' } }));
await run(
  'logout',
  'login',
  post('/logout', 'l2', { as: 'l3' }),
  'passport saves the session without the user before it regenerates, so the original calls set before destroy; Corvis destroys the old record and creates the new one',
);
await run('login: the old cookie after logout', 'login', { path: '/me', cookie: cookie('l2') });

// 6. keepSessionInfo.
await run('keep: anonymous session with data', 'keep', post('/set?k=cart&v=milk', null, { as: 'k1' }));
await run('keep: login keeps the data', 'keep', post('/login', 'k1', { headers: { 'x-user': '1' }, as: 'k2' }));
await run('keep: read back', 'keep', { path: '/sess', cookie: cookie('k2') });

// A cookie and a record exactly as the original produced them, for the "cookie created by the original" scenario.
const seedRes = await fetch(`${base}/seeded/set?k=views&v=7`, { method: 'POST' });
const seedCookie = seedRes.headers.getSetCookie()[0].split('; ')[0];
const seedId = APPS.seeded.genid;
const seedRecord = JSON.parse(stores.seeded.sessions[seedId]);

server.close();

const out = {
  versions: {
    'express-session': version('express-session'),
    passport: version('passport'),
    express: version('express'),
    'cookie-signature': version('cookie-signature'),
  },
  secrets: SECRETS,
  unknownId: UNKNOWN_ID,
  users: USERS,
  apps: APPS,
  original: { app: 'seeded', id: seedId, cookie: seedCookie, record: seedRecord },
  steps,
};
writeFileSync(new URL('./session.json', import.meta.url), JSON.stringify(out, null, 2) + '\n');
console.log(`session.json: ${steps.length} steps, ${steps.filter((s) => s.divergence).length} with a divergence`);
