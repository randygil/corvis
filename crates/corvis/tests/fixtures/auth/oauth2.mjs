// Records what `passport-oauth2` 1.8.0, `passport-google-oauth20` 2.0.0, `passport-github2` 0.1.12 and
// `passport-oauth2-refresh` 2.2.0 (passport 0.7.0, express 5.3.0, express-session 1.19.0) send to a provider and make of
// its answers, for the OAuth2 strategies of Corvis. Run once with `npm ci && node oauth2.mjs`; the output
// (`oauth2.json`) is committed and replayed by `tests/auth_oauth2_original.rs` against a mock provider on 127.0.0.1,
// without Node and without a network. Everything here binds to 127.0.0.1 only and every credential is a fake string.
//
// The fake provider answers by the `code` of the token request (`ok`, `badrequest`, `down`, ...) and by the access token
// of a profile request (`ACCESS-ok`, `ACCESS-profile500`, ...), and records each request it receives. The original runs
// with `state: true` and `pkce: true` (the only way it checks a state), with an in-memory session. The port of the fake
// provider is replaced by `<provider>`, and the values of `state`, `code_challenge` and `code_verifier` by
// placeholders, so the output is the same on every run.
//
// A case with a `divergence` is a behavior Corvis changes on purpose; the Rust test asserts the specification (named in
// the text) instead of the recorded behavior for the part that differs.
process.env.NODE_ENV = 'test'; // express prints the stack of an error only outside the test environment
import http from 'node:http';
import crypto from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import express from 'express';
import session from 'express-session';
import passport from 'passport';

const require = createRequire(import.meta.url);
const OAuth2Strategy = require('passport-oauth2');
const { Strategy: GoogleStrategy } = require('passport-google-oauth20');
const { Strategy: GitHubStrategy } = require('passport-github2');
const refresh = require('passport-oauth2-refresh');
const version = (name) => JSON.parse(readFileSync(new URL(`./node_modules/${name}/package.json`, import.meta.url), 'utf8')).version;

const CLIENT_ID = 'client-id-FAKE';
const CLIENT_SECRET = 'client-secret-FAKE';
const REDIRECT = 'https://app.example/auth/callback';

// ---- the fake provider ----

const GOOGLE_DOC = {
  sub: '110169484474386276334',
  name: 'Ana Perez',
  given_name: 'Ana',
  family_name: 'Perez',
  picture: 'https://lh3.example/a.png',
  email: 'ana@example.com',
  email_verified: true,
};
const GITHUB_USER = {
  login: 'octocat',
  id: 583231,
  node_id: 'MDQ6VXNlcjU4MzIzMQ==',
  avatar_url: 'https://avatars.example/u/583231',
  html_url: 'https://github.com/octocat',
  name: 'The Octocat',
  email: null,
};
const GITHUB_EMAILS = [
  { email: 'octocat@users.noreply.github.com', primary: false, verified: true, visibility: null },
  { email: 'octocat@example.com', primary: true, verified: true, visibility: 'public' },
];

let received = [];
let providerPort = 0;

function readBody(req) {
  return new Promise((resolve) => {
    const chunks = [];
    req.on('data', (c) => chunks.push(c));
    req.on('end', () => resolve(Buffer.concat(chunks).toString('utf8')));
  });
}

const provider = http.createServer(async (req, res) => {
  const body = await readBody(req);
  const url = new URL(req.url, 'http://127.0.0.1');
  received.push({
    method: req.method,
    path: url.pathname,
    query: url.search.slice(1),
    headers: req.headers,
    body,
  });
  const send = (status, headers, text) => {
    res.writeHead(status, headers);
    res.end(text);
  };
  const json = (status, value) => send(status, { 'Content-Type': 'application/json' }, JSON.stringify(value));
  if (req.method === 'POST' && url.pathname === '/token') {
    const form = new URLSearchParams(body);
    if (form.get('grant_type') === 'refresh_token') {
      return json(200, { access_token: 'ACCESS-refreshed', refresh_token: 'REFRESH-refreshed', expires_in: 1800, token_type: 'Bearer' });
    }
    const code = form.get('code');
    switch (code) {
      case 'norefresh': return json(200, { access_token: 'ACCESS-ok', token_type: 'Bearer' });
      case 'form': return send(200, { 'Content-Type': 'application/x-www-form-urlencoded' }, 'access_token=ACCESS-ok&token_type=bearer');
      case 'badrequest': return json(400, { error: 'invalid_grant', error_description: 'bad code' });
      case 'down': return json(500, { x: 'y' });
      case 'redirect': return send(302, { Location: 'http://127.0.0.1:1/elsewhere' }, '');
      case 'garbage': return send(200, { 'Content-Type': 'application/json' }, 'not json');
      case 'noaccess': return json(200, {});
      default:
        return json(200, {
          access_token: `ACCESS-${code}`,
          token_type: 'Bearer',
          refresh_token: `REFRESH-${code}`,
          expires_in: 3600,
          scope: 'email profile',
        });
    }
  }
  const token = (req.headers.authorization ?? '').replace(/^Bearer /i, '') || url.searchParams.get('access_token') || '';
  if (req.method === 'GET' && ['/userinfo', '/user', '/user/emails'].includes(url.pathname)) {
    if (token === 'ACCESS-profile500') return json(500, {});
    if (url.pathname === '/userinfo') return json(200, GOOGLE_DOC);
    if (url.pathname === '/user') return json(200, GITHUB_USER);
    return json(200, GITHUB_EMAILS);
  }
  return json(404, {});
});
await new Promise((resolve) => provider.listen(0, '127.0.0.1', resolve));
providerPort = provider.address().port;
const P = `http://127.0.0.1:${providerPort}`;

// ---- the original ----

// `validate`: hands everything to the route, or refuses for the access token `ACCESS-validatorfalse`.
function verify(accessToken, refreshToken, params, profile, done) {
  if (accessToken === 'ACCESS-validatorfalse') return done(null, false);
  done(null, { accessToken, refreshToken, params, profile });
}

const CONFIGS = {
  // name: [kind, options]
  'generic': ['generic', {}],
  'generic-scope': ['generic', { scope: ['a', 'b'] }],
  'generic-tenant': ['generic', { authorizationURL: `${P}/authorize?tenant=a` }],
  'google': ['google', { scope: ['email', 'profile'] }],
  'google-offline': ['google', { scope: ['email', 'profile'] }],
  'github': ['github', { scope: ['user:email', 'read:user'], allRawEmails: true }],
  'github-default-emails': ['github', { scope: ['user:email'] }],
  'github-no-email-scope': ['github', { scope: ['read:org'] }],
};

function strategyFor(name, withState) {
  const [kind, extra] = CONFIGS[name];
  const base = {
    clientID: CLIENT_ID,
    clientSecret: CLIENT_SECRET,
    callbackURL: REDIRECT,
    authorizationURL: `${P}/authorize`,
    tokenURL: `${P}/token`,
    ...extra,
  };
  if (withState) Object.assign(base, { state: true, pkce: true });
  if (kind === 'generic') {
    const s = new OAuth2Strategy(base, verify);
    s.userProfile = (accessToken, done) => done(null, {});
    return s;
  }
  if (kind === 'google') return new GoogleStrategy({ ...base, userProfileURL: `${P}/userinfo` }, verify);
  return new GitHubStrategy({ ...base, userProfileURL: `${P}/user`, userEmailURL: `${P}/user/emails` }, verify);
}

// One express app per configuration, with and without `state` and `pkce`.
const apps = new Map();
async function appFor(name, withState) {
  const key = `${name}:${withState}`;
  if (apps.has(key)) return apps.get(key);
  const app = express();
  app.use(session({ secret: 'corvis-oauth2-fixture-secret-0123456789abcdef', resave: false, saveUninitialized: false }));
  const p = new passport.Passport();
  const strategy = strategyFor(name, withState);
  p.use(name, strategy);
  if (name === 'generic') refresh.use('generic', strategy);
  app.get('/login', p.authenticate(name, { session: false, ...(name === 'google-offline' ? { accessType: 'offline' } : {}) }));
  app.get('/callback', p.authenticate(name, { session: false }), (req, res) => {
    const { accessToken, refreshToken, params, profile } = req.user;
    const { _raw, ...rest } = profile ?? {};
    res.json({ accessToken, refreshToken, params, profile: rest });
  });
  const server = app.listen(0, '127.0.0.1');
  await new Promise((resolve) => server.once('listening', resolve));
  const entry = { server, port: () => server.address().port };
  apps.set(key, entry);
  return entry;
}

const PLACEHOLDERS = [];
function normalize(text) {
  return text.replaceAll(`127.0.0.1:${providerPort}`, '<provider>');
}

function pairs(search) {
  return [...new URLSearchParams(search).entries()];
}

function placeholders(list) {
  return list.map(([k, v]) => {
    if (k === 'state') return [k, '<state>'];
    if (k === 'code_challenge') return [k, '<challenge>'];
    if (k === 'code_verifier') return [k, '<verifier>'];
    return [k, normalize(v)];
  });
}

async function call(entry, path, cookie) {
  const res = await fetch(`http://127.0.0.1:${entry.port()}${path}`, { redirect: 'manual', headers: cookie ? { cookie } : {} });
  const text = await res.text();
  let body;
  try { body = JSON.parse(text); } catch { body = text; }
  return { res, body, cookie: (res.headers.getSetCookie?.() ?? []).map((c) => c.split(';')[0]).join('; ') || cookie };
}

async function login(name, withState, path = '/login') {
  const entry = await appFor(name, withState);
  const r = await call(entry, path);
  const location = new URL(r.res.headers.get('location'));
  return { entry, cookie: r.cookie, status: r.res.status, location, state: location.searchParams.get('state') };
}

const out = {
  meta: {
    'passport-oauth2': version('passport-oauth2'),
    'passport-google-oauth20': version('passport-google-oauth20'),
    'passport-github2': version('passport-github2'),
    'passport-oauth2-refresh': version('passport-oauth2-refresh'),
    note: 'values of state, code_challenge and code_verifier are placeholders; <provider> is the fake provider',
  },
};

// ---- the authorization redirect ----

out.authorization = [];
for (const name of Object.keys(CONFIGS)) {
  for (const withState of [false, true]) {
    if (name === 'github-default-emails' || name === 'github-no-email-scope') continue;
    const l = await login(name, withState);
    const entry = {
      config: name,
      state_and_pkce: withState,
      status: l.status,
      location_path: l.location.pathname,
      params: placeholders(pairs(l.location.search)),
    };
    if (!withState) {
      entry.divergence = 'Corvis always sends state and a PKCE challenge: the redirect has code_challenge, code_challenge_method=S256 and state where this one has none';
    }
    out.authorization.push(entry);
  }
}

// ---- the token request, the profile requests and the profiles ----

async function complete(name, code = 'ok', callbackQuery = (state) => `code=${code}&state=${state}`) {
  const l = await login(name, true);
  received = [];
  const r = await call(l.entry, `/callback?${callbackQuery(l.state)}`, l.cookie);
  return { r, requests: received.slice(), l };
}

const requestRecord = (rq) => ({
  method: rq.method,
  path: rq.path,
  query: placeholders(pairs(rq.query)),
  content_type: rq.headers['content-type'] ?? null,
  authorization: rq.headers.authorization ? rq.headers.authorization.split(' ')[0] : null,
  user_agent: rq.headers['user-agent'] ?? null,
  form: rq.method === 'POST' ? placeholders(pairs(rq.body)) : null,
});

out.token_request = {};
out.profile_requests = {};
out.profiles = {};
for (const name of ['generic', 'google', 'github', 'github-default-emails', 'github-no-email-scope']) {
  const { r, requests } = await complete(name);
  const token = requests.find((x) => x.path === '/token');
  if (['generic', 'google', 'github'].includes(name)) out.token_request[name] = requestRecord(token);
  const profileRequests = requests.filter((x) => x.path !== '/token').map(requestRecord);
  if (profileRequests.length) out.profile_requests[name] = profileRequests;
  out.profiles[name] = { status: r.res.status, profile: r.body.profile, access_token: r.body.accessToken, refresh_token: r.body.refreshToken, params: r.body.params };
}
out.token_request.divergence =
  'Corvis also sends Accept: application/json and its own User-Agent; the compared parts are the method, the path, the content type and the form fields in order';
out.profile_requests.divergence =
  'the original sends the Google access token as the access_token query parameter (and GitHub in an Authorization: Bearer header); Corvis sends a bearer header for both and never a token in a URL';

// A GitHub profile whose /user document has a public address, without the e-mail scope.
out.profiles.github_public_email = null;
{
  const saved = GITHUB_USER.email;
  GITHUB_USER.email = 'public@example.com';
  const { r } = await complete('github-no-email-scope');
  out.profiles.github_public_email = { status: r.res.status, profile: r.body.profile };
  GITHUB_USER.email = saved;
}
out.profiles.divergence =
  'GitHub e-mails: with user:email Corvis keeps every address of /user/emails as { value, primary, verified } (the original with allRawEmails: true, which adds visibility; without it only the primary address as { value }); every e-mail has a verified flag, which the original omits for GitHub';

// ---- callback outcomes ----

out.outcomes = [];
async function outcome(name, config, query, extra = {}) {
  const l = await login(config, true);
  received = [];
  const q = typeof query === 'function' ? query(l.state) : query.replace('<state>', l.state);
  const r = await call(l.entry, `/callback?${q}`, l.cookie);
  const entry = { name, config, query: q.replace(l.state, '<state>'), status: r.res.status, token_requests: received.filter((x) => x.path === '/token').length, ...extra };
  out.outcomes.push(entry);
}
await outcome('success', 'google', 'code=ok&state=<state>');
await outcome('access_denied', 'google', 'error=access_denied&error_description=denied&state=<state>');
await outcome('other_provider_error', 'google', 'error=server_error&error_description=boom&state=<state>', {
  divergence: 'Corvis answers 401 for a provider error other than access_denied: it is caused by the request, not by the server (auth/oauth2: Request-caused callback failures are answered 401)',
});
await outcome('state_mismatch', 'google', 'code=ok&state=forged', {
  note: 'passport answers the status the strategy gave (403); the AuthGuard of Corvis answers 401 for every failed attempt',
});
await outcome('no_state', 'google', 'code=ok');
await outcome('token_400', 'google', 'code=badrequest&state=<state>', {
  divergence: 'Corvis answers 401 when the token endpoint rejects the code (400 or 401): the request caused it (a reused or forged code)',
});
await outcome('token_500', 'google', 'code=down&state=<state>');
await outcome('token_redirect', 'google', 'code=redirect&state=<state>', {
  note: 'node-oauth does not follow the redirect; the empty answer has no access token',
});
await outcome('token_garbage', 'google', 'code=garbage&state=<state>');
await outcome('token_without_access_token', 'google', 'code=noaccess&state=<state>');
await outcome('token_form_encoded', 'google', 'code=form&state=<state>');
await outcome('profile_500', 'google', 'code=profile500&state=<state>');
await outcome('validator_false', 'google', 'code=validatorfalse&state=<state>');
await outcome('no_refresh_token', 'google', 'code=norefresh&state=<state>');

// ---- refresh ----

{
  received = [];
  await new Promise((resolve, reject) => {
    appFor('generic', true).then(() => refresh.requestNewAccessToken('generic', 'OLD-REFRESH', (err, accessToken, refreshToken, results) => {
      if (err) return reject(err);
      out.refresh = {
        request: requestRecord(received.find((x) => x.path === '/token')),
        access_token: accessToken,
        refresh_token: refreshToken,
        params: results,
      };
      resolve();
    }));
  });
}

for (const { server } of apps.values()) server.close();
provider.close();
const text = normalize(JSON.stringify(out, null, 2)) + '\n';
writeFileSync(new URL('./oauth2.json', import.meta.url), text);
console.log(`wrote oauth2.json (${text.length} bytes)`);
process.exit(0);
