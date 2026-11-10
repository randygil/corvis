// Records what `AuthGuard('jwt')` answers with `passport-jwt` 4.0.1 (over `jsonwebtoken` 9.0.x), driven through the
// `PassportStrategy` mixin and the guard of @nestjs/passport 12.0.1. Run once with `npm ci && node jwt.mjs`; the output
// (`jwt.json`) is committed and replayed by `tests/auth_jwt.rs`, which signs the same tokens again from the recorded
// token specs. The server binds to 127.0.0.1 only.
//
// One app has one route per strategy (`GET /<name>`, `POST /body`). A case is:
//   { name, request: { method, path, headers, body }, token: <spec>, response: { status, body }, divergence }
// `request` holds the text `{{token}}` where the token goes. A token spec is
//   { alg, key, claims }        a token signed with the named key; a claim value `{"$now": n}` is `now + n` seconds
//   { alg, key, payloadText }   the payload segment as given (a JSON string that is not an object, for instance)
//   { pad }                     a claim `pad` of that many characters, to reach a size
//   { raw }                     the token text as it is (unsigned tokens)
// Keys: s1, s2, sa, sb, other (HMAC secrets), rsa and ec (private keys; the verifying strategies hold the public
// ones), rsa-public (the public PEM used as an HMAC secret). A case with a `divergence` is a behavior Corvis changes
// on purpose: `divergence.reason` says why and `divergence.status` is what Corvis answers.
import 'reflect-metadata';
import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { Module, Controller, Get, Post, Req, UseGuards } from '@nestjs/common';
import { NestFactory } from '@nestjs/core';
import { AuthGuard, PassportStrategy } from '@nestjs/passport';

const require = createRequire(import.meta.url);
const { Strategy: JwtStrategy, ExtractJwt } = require('passport-jwt');
const cookieParser = require('cookie-parser');
const jws = require('jws');
const jsonwebtoken = require('jsonwebtoken');

const read = (name) => readFileSync(new URL(`../${name}`, import.meta.url), 'utf8');
const RSA_PRIVATE = read('jwt_rsa_private.pem');
const RSA_PUBLIC = read('jwt_rsa_public.pem');
const EC_PRIVATE = read('jwt_ec_private.pem');
const EC_PUBLIC = read('jwt_ec_public.pem');

const SECRETS = {
  s1: 'corvis-jwt-secret-one-0123456789abcdef',
  s2: 'corvis-jwt-secret-two-0123456789abcdef',
  sa: 'corvis-jwt-secret-a-0123456789abcdef!!',
  sb: 'corvis-jwt-secret-b-0123456789abcdef!!',
  other: 'corvis-jwt-secret-other-0123456789ab',
  'rsa-public': RSA_PUBLIC,
};

// ---------------------------------------------------------------------------------------------------------------
// The strategies. Each one validates like the Rust test's validators: `sub` "nobody" is refused, anything else is the
// user `{ id: sub, role }`.
// ---------------------------------------------------------------------------------------------------------------
const bearer = ExtractJwt.fromAuthHeaderAsBearerToken();
const cookie = (req) => req.cookies?.jwt;

const STRATEGIES = {
  bearer: { jwtFromRequest: bearer, secretOrKey: SECRETS.s1 },
  refresh: { jwtFromRequest: bearer, secretOrKey: SECRETS.s2 },
  scheme: { jwtFromRequest: ExtractJwt.fromAuthHeaderWithScheme('JWT'), secretOrKey: SECRETS.s1 },
  header: { jwtFromRequest: ExtractJwt.fromHeader('x-token'), secretOrKey: SECRETS.s1 },
  'header-upper': { jwtFromRequest: ExtractJwt.fromHeader('X-Token'), secretOrKey: SECRETS.s1 },
  cookie: { jwtFromRequest: cookie, secretOrKey: SECRETS.s1 },
  query: { jwtFromRequest: ExtractJwt.fromUrlQueryParameter('token'), secretOrKey: SECRETS.s1 },
  body: { jwtFromRequest: ExtractJwt.fromBodyField('token'), secretOrKey: SECRETS.s1 },
  extractors: { jwtFromRequest: ExtractJwt.fromExtractors([cookie, bearer]), secretOrKey: SECRETS.s1 },
  audience: { jwtFromRequest: bearer, secretOrKey: SECRETS.s1, audience: ['web', 'mobile'] },
  'audience-web': { jwtFromRequest: bearer, secretOrKey: SECRETS.s1, audience: 'web' },
  issuer: { jwtFromRequest: bearer, secretOrKey: SECRETS.s1, issuer: 'https://auth.example' },
  provider: {
    jwtFromRequest: bearer,
    secretOrKeyProvider: (req, raw, done) => {
      const iss = jsonwebtoken.decode(raw)?.iss;
      if (iss === 'a') return done(null, SECRETS.sa);
      if (iss === 'b') return done(null, SECRETS.sb);
      return done(new Error('unknown issuer'));
    },
  },
  'provider-error': { jwtFromRequest: bearer, secretOrKeyProvider: (req, raw, done) => done(new Error('no key')) },
  'provider-rsa': { jwtFromRequest: bearer, secretOrKeyProvider: (req, raw, done) => done(null, RSA_PUBLIC) },
  rs256: { jwtFromRequest: bearer, secretOrKey: RSA_PUBLIC },
  es256: { jwtFromRequest: bearer, secretOrKey: EC_PUBLIC },
  hs256: { jwtFromRequest: bearer, secretOrKey: SECRETS.s1, algorithms: ['HS256'] },
  'ignore-exp': { jwtFromRequest: bearer, secretOrKey: SECRETS.s1, ignoreExpiration: true },
  tolerance: { jwtFromRequest: bearer, secretOrKey: SECRETS.s1, jsonWebTokenOptions: { clockTolerance: 30 } },
  big: { jwtFromRequest: bearer, secretOrKey: SECRETS.s1 },
};

for (const [name, options] of Object.entries(STRATEGIES)) {
  class Registered extends PassportStrategy(JwtStrategy, name) {
    constructor() {
      super({ ...options, passReqToCallback: true });
    }
    validate(req, payload) {
      if (payload.sub === 'nobody') return null;
      return { id: payload.sub, role: payload.role };
    }
  }
  new Registered();
}

function defineController(Class, routes) {
  for (const [name, method, path, guard] of routes) {
    const proto = Class.prototype;
    proto[name] = function (req) {
      return { user: req.user ?? null };
    };
    const descriptor = Object.getOwnPropertyDescriptor(proto, name);
    (method === 'POST' ? Post : Get)(path)(proto, name, descriptor);
    Req()(proto, name, 0);
    UseGuards(AuthGuard(guard))(proto, name, descriptor);
  }
  Controller('')(Class);
}
class Routes {}
defineController(
  Routes,
  Object.keys(STRATEGIES).map((name) => [name.replace(/-/g, '_'), name === 'body' ? 'POST' : 'GET', name, name]),
);

// ---------------------------------------------------------------------------------------------------------------
// Tokens
// ---------------------------------------------------------------------------------------------------------------
const FAR = 4102444800; // 2100-01-01
const now = (n) => ({ $now: n });
const hs = (key, claims, alg = 'HS256') => ({ alg, key, claims });
const OK = { sub: '7', role: 'admin', exp: FAR };
const b64 = (text) => Buffer.from(text).toString('base64url');
const unsigned = (header, claims) => `${b64(JSON.stringify(header))}.${b64(JSON.stringify(claims))}.`;

function claimsAt(claims, at) {
  const out = {};
  for (const [k, v] of Object.entries(claims)) out[k] = v && typeof v === 'object' && '$now' in v ? at + v.$now : v;
  return out;
}

function sign(spec, at) {
  if (spec.raw !== undefined) return spec.raw;
  const alg = spec.alg ?? 'HS256';
  const header = { alg, typ: 'JWT' };
  const payload = spec.payloadText ?? JSON.stringify(claimsAt({ ...spec.claims, ...(spec.pad ? { pad: 'x'.repeat(spec.pad) } : {}) }, at));
  const keys = { rsa: RSA_PRIVATE, ec: EC_PRIVATE };
  if (spec.key in keys) return jws.sign({ header, payload, privateKey: keys[spec.key] });
  return jws.sign({ header, payload, secret: SECRETS[spec.key] });
}

const get = (strategy, headers, token, extra = {}) => ({
  request: { method: 'GET', path: `/${strategy}`, headers, ...extra },
  token,
});
const viaBearer = (strategy, token) => get(strategy, { authorization: 'Bearer {{token}}' }, token);

const CASES = [
  ['payload passed through', viaBearer('bearer', hs('s1', OK))],
  ['validator returns no user', viaBearer('bearer', hs('s1', { sub: 'nobody', exp: FAR }))],
  ['no token', get('bearer', {}, null)],
  ['wrong secret', viaBearer('bearer', hs('other', OK))],
  ['algorithm none', viaBearer('bearer', { raw: unsigned({ alg: 'none', typ: 'JWT' }, OK) })],
  ['algorithm NONE', viaBearer('bearer', { raw: unsigned({ alg: 'NONE', typ: 'JWT' }, OK) })],
  ['empty signature with HS256', viaBearer('bearer', { raw: unsigned({ alg: 'HS256', typ: 'JWT' }, OK) })],
  ['not a token', viaBearer('bearer', { raw: 'abc' })],
  ['HS512 with the default list', viaBearer('bearer', hs('s1', OK, 'HS512'))],
  ['HS512 with an explicit HS256 list', viaBearer('hs256', hs('s1', OK, 'HS512'))],
  ['HS256 with an explicit HS256 list', viaBearer('hs256', hs('s1', OK))],
  ['tab between scheme and token', get('bearer', { authorization: 'Bearer\t{{token}}' }, hs('s1', OK))],
  ['extra words after the token', get('bearer', { authorization: 'Bearer {{token}} extra' }, hs('s1', OK))],
  ['lowercase scheme', get('bearer', { authorization: 'bearer {{token}}' }, hs('s1', OK))],
  ['other scheme', viaBearer('scheme', hs('s1', OK))],
  ['configured scheme', get('scheme', { authorization: 'JWT {{token}}' }, hs('s1', OK))],
  ['scheme without a token', get('bearer', { authorization: 'Bearer' }, null)],
  ['basic scheme', get('bearer', { authorization: 'Basic {{token}}' }, hs('s1', OK))],
  ['custom header', get('header', { 'x-token': '{{token}}' }, hs('s1', OK))],
  [
    'uppercase header name',
    get('header-upper', { 'x-token': '{{token}}' }, hs('s1', OK)),
    { reason: 'the header name is matched case-insensitively; the original only finds names written in lowercase', status: 200 },
  ],
  ['repeated custom header', get('header', { 'x-token': '{{token}}, {{token}}' }, hs('s1', OK))],
  ['cookie', get('cookie', { cookie: 'jwt={{token}}' }, hs('s1', OK))],
  ['query parameter', { request: { method: 'GET', path: '/query?token={{token}}', headers: {} }, token: hs('s1', OK) }],
  [
    'body field',
    { request: { method: 'POST', path: '/body', headers: { 'content-type': 'application/json' }, body: { token: '{{token}}' } }, token: hs('s1', OK) },
  ],
  [
    'non-string body field',
    { request: { method: 'POST', path: '/body', headers: { 'content-type': 'application/json' }, body: { token: 5 } }, token: null },
  ],
  [
    'token in a place that is not configured',
    { request: { method: 'GET', path: '/bearer?token={{token}}', headers: { cookie: 'jwt={{token}}' } }, token: hs('s1', OK) },
  ],
  ['extractors: only the header', viaBearer('extractors', hs('s1', OK))],
  ['extractors: the cookie', get('extractors', { cookie: 'jwt={{token}}' }, hs('s1', OK))],
  [
    'extractors: the first non-empty one wins',
    {
      request: { method: 'GET', path: '/extractors', headers: { cookie: 'jwt={{wrong}}', authorization: 'Bearer {{token}}' } },
      token: hs('s1', OK),
      wrong: hs('other', OK),
    },
  ],
  [
    'oversized token',
    viaBearer('bearer', { alg: 'HS256', key: 's1', claims: OK, pad: 9000 }),
    { reason: 'the original has no token length limit; the default limit is 8192 bytes', status: 401 },
  ],
  ['raised limit', viaBearer('big', { alg: 'HS256', key: 's1', claims: OK, pad: 10000 })],
  ['expired token', viaBearer('bearer', hs('s1', { sub: '7', exp: now(-10) }))],
  ['expired within the tolerance', viaBearer('tolerance', hs('s1', { sub: '7', exp: now(-10) }))],
  ['expired beyond the tolerance', viaBearer('tolerance', hs('s1', { sub: '7', exp: now(-60) }))],
  ['not yet valid', viaBearer('bearer', hs('s1', { sub: '7', exp: FAR, nbf: now(3600) }))],
  ['not yet valid within the tolerance', viaBearer('tolerance', hs('s1', { sub: '7', exp: FAR, nbf: now(10) }))],
  ['ignore_expiration', viaBearer('ignore-exp', hs('s1', { sub: '7', exp: now(-3600) }))],
  ['non-numeric exp', viaBearer('bearer', hs('s1', { sub: '7', exp: '2099-01-01' }))],
  [
    'token without exp',
    viaBearer('bearer', hs('s1', { sub: '7', role: 'admin' })),
    { reason: 'exp is required by default; the original accepts tokens without exp, which never expire', status: 401 },
  ],
  [
    'string iat',
    viaBearer('bearer', hs('s1', { sub: '7', exp: FAR, iat: 'yesterday' })),
    { reason: 'a non-numeric iat fails; the original checks iat only with maxAge', status: 401 },
  ],
  ['audience in the list', viaBearer('audience', hs('s1', { ...OK, aud: 'mobile' }))],
  ['audience not in the list', viaBearer('audience', hs('s1', { ...OK, aud: 'admin' }))],
  ['audience missing', viaBearer('audience', hs('s1', OK))],
  ['audience array in the token', viaBearer('audience-web', hs('s1', { ...OK, aud: ['admin', 'web'] }))],
  ['audience array without a match', viaBearer('audience-web', hs('s1', { ...OK, aud: ['admin', 'ops'] }))],
  ['issuer matches', viaBearer('issuer', hs('s1', { ...OK, iss: 'https://auth.example' }))],
  ['wrong issuer', viaBearer('issuer', hs('s1', { ...OK, iss: 'https://evil.example' }))],
  ['issuer missing', viaBearer('issuer', hs('s1', OK))],
  ['key chosen by the issuer', viaBearer('provider', hs('sa', { ...OK, iss: 'a' }))],
  ['key chosen by another issuer', viaBearer('provider', hs('sb', { ...OK, iss: 'b' }))],
  ['token signed with the other issuer key', viaBearer('provider', hs('sb', { ...OK, iss: 'a' }))],
  ['key provider error', viaBearer('provider', hs('sa', { ...OK, iss: 'unknown' }))],
  ['key provider that always fails', viaBearer('provider-error', hs('s1', OK))],
  ['provider returns an RSA key for an HMAC token', viaBearer('provider-rsa', hs('s1', OK))],
  ['provider returns an RSA key for an RSA token', viaBearer('provider-rsa', { alg: 'RS256', key: 'rsa', claims: OK })],
  ['RS256', viaBearer('rs256', { alg: 'RS256', key: 'rsa', claims: OK })],
  ['PS256', viaBearer('rs256', { alg: 'PS256', key: 'rsa', claims: OK })],
  ['HMAC token signed with the RSA public key', viaBearer('rs256', hs('rsa-public', OK))],
  ['HMAC token against an RSA key', viaBearer('rs256', hs('s1', OK))],
  ['ES256', viaBearer('es256', { alg: 'ES256', key: 'ec', claims: OK })],
  ['RS256 token against an EC key', viaBearer('es256', { alg: 'RS256', key: 'rsa', claims: OK })],
  [
    'string payload',
    viaBearer('bearer', { alg: 'HS256', key: 's1', payloadText: '"hello"' }),
    { reason: 'a payload that is not an object fails; the original passes the string to validate()', status: 401 },
  ],
  ['two strategies: the second secret on the second name', viaBearer('refresh', hs('s2', OK))],
  ['two strategies: the first secret on the second name', viaBearer('refresh', hs('s1', OK))],
  ['two strategies: the second secret on the first name', viaBearer('bearer', hs('s2', OK))],
];

// ---------------------------------------------------------------------------------------------------------------
async function main() {
  const imports = [];
  class AppModule {}
  Module({ imports, controllers: [Routes] })(AppModule);
  const app = await NestFactory.create(AppModule, { logger: false });
  app.use(cookieParser());
  await app.listen(0, '127.0.0.1');
  const base = `http://127.0.0.1:${app.getHttpServer().address().port}`;
  const cases = [];
  try {
    for (const [name, definition, divergence = null] of CASES) {
      const at = Math.floor(Date.now() / 1000);
      const token = definition.token ? sign(definition.token, at) : '';
      const wrong = definition.wrong ? sign(definition.wrong, at) : '';
      const fill = (text) => text.replaceAll('{{token}}', token).replaceAll('{{wrong}}', wrong);
      const { method, path, headers, body } = definition.request;
      const sent = Object.fromEntries(Object.entries(headers).map(([k, v]) => [k, fill(v)]));
      const res = await fetch(base + fill(path), {
        method,
        headers: sent,
        body: body === undefined ? undefined : fill(JSON.stringify(body)),
        redirect: 'manual',
      });
      const text = await res.text();
      let responseBody;
      try {
        responseBody = JSON.parse(text);
      } catch {
        responseBody = text;
      }
      cases.push({
        name,
        request: definition.request,
        token: definition.token ?? null,
        wrong: definition.wrong ?? null,
        response: { status: res.status, headers: { 'www-authenticate': res.headers.get('www-authenticate') }, body: responseBody },
        divergence: divergence && { reason: divergence.reason, status: divergence.status },
      });
    }
  } finally {
    await app.close();
  }
  const version = (pkg) => JSON.parse(readFileSync(new URL(`./node_modules/${pkg}/package.json`, import.meta.url), 'utf8')).version;
  writeFileSync(
    new URL('./jwt.json', import.meta.url),
    JSON.stringify(
      {
        versions: { 'passport-jwt': version('passport-jwt'), jsonwebtoken: version('jsonwebtoken'), '@nestjs/passport': version('@nestjs/passport') },
        cases,
      },
      null,
      2,
    ) + '\n',
  );
}

await main();
