// Records what `AuthGuard(name)` of @nestjs/passport 12.0.1 over passport 0.7.0 answers for API keys: the header
// strategy `passport-headerapikey` 1.2.2 and hand-written `passport-custom` 1.2.1 strategies (query and bearer keys),
// for the API key strategy of Corvis. Run once with `npm ci && node api_key.mjs`; the output (`api_key.json`) is
// committed and replayed by `tests/auth_api_key.rs`. The server binds to 127.0.0.1 only. Every key below is a fake
// test string of at least 32 bytes.
//
// One route per strategy (`GET /<route>`); the strategy names are the route names. The verify function decides from the
// key text: `test-key-null...` is no user, `test-key-throw...` a ForbiddenException ("Key suspended"),
// `test-key-boom...` a plain Error, `test-key-info...` a user with info, any other `test-key-...` the user
// `{ id: 'service-1', keyLength }`, and everything else no user.
//
// Strategies (route: how the key is read):
//   header       passport-headerapikey, header X-Api-Key
//   custom       passport-headerapikey, header X-Service-Key
//   prefixed     passport-headerapikey, header Authorization with the prefix "Api-Key "
//   plus         passport-headerapikey, header X-Key with the prefix "Key+" (a regular expression character)
//   query        passport-custom, req.query.api_key when it is a string
//   bearer       passport-custom, passport-jwt's fromAuthHeaderAsBearerToken
//   chain        AuthGuard(['bearer-user', 'header']): a bearer-user strategy first, then the header key
//   two-places   passport-custom, the header X-Api-Key and, when absent, the query parameter api_key (the first wins)
//
// A case with a `divergence` is a behavior Corvis changes on purpose: `divergence.reason` says why and
// `divergence.status` is what Corvis answers; `divergence.validatorKey`, when present, is the key the validator must
// receive.
import 'reflect-metadata';
import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { Module, Controller, Get, Req, UseGuards, ForbiddenException } from '@nestjs/common';
import { NestFactory } from '@nestjs/core';
import { AuthGuard } from '@nestjs/passport';

const require = createRequire(import.meta.url);
const passport = require('passport');
const HeaderApiKeyStrategy = require('passport-headerapikey').HeaderAPIKeyStrategy;
const CustomStrategy = require('passport-custom');
const { ExtractJwt } = require('passport-jwt');
const version = (name) => JSON.parse(readFileSync(new URL(`./node_modules/${name}/package.json`, import.meta.url), 'utf8')).version;

const K1 = 'test-key-000000000000000000000000000001';
const K2 = 'test-key-000000000000000000000000000002';
const KNULL = 'test-key-null-00000000000000000000001';
const KTHROW = 'test-key-throw-0000000000000000000001';
const KBOOM = 'test-key-boom-00000000000000000000001';
const KINFO = 'test-key-info-00000000000000000000001';
const WRONG = 'wrong-key-000000000000000000000000000001';

function verifyKey(key, done) {
  if (key.startsWith('test-key-null')) return done(null, false);
  if (key.startsWith('test-key-throw')) return done(new ForbiddenException('Key suspended'));
  if (key.startsWith('test-key-boom')) return done(new Error('db down'));
  if (key.startsWith('test-key-info')) return done(null, { id: 'service-1', keyLength: key.length }, { scope: 'read' });
  if (key.startsWith('test-key-')) return done(null, { id: 'service-1', keyLength: key.length });
  return done(null, false);
}

const header = (header, prefix) => new HeaderApiKeyStrategy({ header, prefix }, true, (key, done) => verifyKey(key, done));
passport.use('header', header('X-Api-Key', ''));
passport.use('custom', header('X-Service-Key', ''));
passport.use('prefixed', header('Authorization', 'Api-Key '));
passport.use('plus', header('X-Key', 'Key+'));
passport.use(
  'query',
  new CustomStrategy((req, done) => {
    const key = req.query.api_key;
    if (typeof key !== 'string' || key === '') return done(null, false);
    verifyKey(key, done);
  }),
);
passport.use(
  'bearer',
  new CustomStrategy((req, done) => {
    const key = ExtractJwt.fromAuthHeaderAsBearerToken()(req);
    if (!key) return done(null, false);
    verifyKey(key, done);
  }),
);
passport.use(
  'bearer-user',
  new CustomStrategy((req, done) => {
    const token = ExtractJwt.fromAuthHeaderAsBearerToken()(req);
    if (token === 'user-token-0000000000000000000000000001') return done(null, { id: 'user-1' });
    done(null, false);
  }),
);
passport.use(
  'two-places',
  new CustomStrategy((req, done) => {
    const key = req.headers['x-api-key'] || req.query.api_key;
    if (typeof key !== 'string' || key === '') return done(null, false);
    verifyKey(key, done);
  }),
);

const ROUTES = [
  ['header', 'header'],
  ['custom', 'custom'],
  ['prefixed', 'prefixed'],
  ['plus', 'plus'],
  ['query', 'query'],
  ['bearer', 'bearer'],
  ['two-places', 'two-places'],
];

class Routes {}
for (const [route, strategy] of ROUTES) {
  Routes.prototype[route] = function (req) {
    return { user: req.user ?? null };
  };
  const d = Object.getOwnPropertyDescriptor(Routes.prototype, route);
  Get(route)(Routes.prototype, route, d);
  Req()(Routes.prototype, route, 0);
  UseGuards(AuthGuard(strategy))(Routes.prototype, route, d);
}
Routes.prototype.chain = function (req) {
  return { user: req.user ?? null };
};
{
  const d = Object.getOwnPropertyDescriptor(Routes.prototype, 'chain');
  Get('chain')(Routes.prototype, 'chain', d);
  Req()(Routes.prototype, 'chain', 0);
  UseGuards(AuthGuard(['bearer-user', 'header']))(Routes.prototype, 'chain', d);
}
Controller('')(Routes);

const long = (n) => 'k'.repeat(n);
const PLUS_DIVERGENCE = {
  reason:
    'passport-headerapikey removes the prefix with an unescaped regular expression (`^Key+` removes only "Key", leaving "+<key>"); Corvis removes the literal prefix',
  status: 200,
  validatorKey: K1,
};
const OVERSIZED = 'the original has no limit of its own; Corvis refuses a key over 512 bytes before the validator runs';
const CONTROL = 'the original passes a NUL character on to the validator; Corvis refuses control characters in a key';
const CONFLICT = 'the original has one source (or the first one wins); Corvis fails when two sources carry different keys';

const CASES = [
  // [name, path, headers, divergence]
  ['default header, valid key', '/header', { 'x-api-key': K1 }],
  ['default header, wrong key', '/header', { 'x-api-key': WRONG }],
  ['default header, missing', '/header', {}],
  ['default header, empty value', '/header', { 'x-api-key': '' }],
  ['default header, two values joined', '/header', { 'x-api-key': `${K1}, ${K2}` }],
  ['default header, key in the query is ignored', `/header?api_key=${K1}`, {}],
  ['default header, key in the Authorization header is ignored', '/header', { authorization: `Bearer ${K1}` }],
  ['default header, key in a cookie is ignored', '/header', { cookie: `x-api-key=${K1}` }],
  ['default header, validator returns no user', '/header', { 'x-api-key': KNULL }],
  ['default header, validator throws', '/header', { 'x-api-key': KTHROW }],
  ['default header, validator fails with a plain error', '/header', { 'x-api-key': KBOOM }],
  ['default header, validator returns a user and info', '/header', { 'x-api-key': KINFO }],
  ['default header, oversized key', '/header', { 'x-api-key': `test-key-${long(600)}` }, OVERSIZED],
  ['default header, key of 512 bytes', '/header', { 'x-api-key': `test-key-${long(503)}` }],
  ['custom header, valid key', '/custom', { 'x-service-key': K1 }],
  ['custom header, the default header is ignored', '/custom', { 'x-api-key': K1 }],
  ['prefixed, prefix and key', '/prefixed', { authorization: `Api-Key ${K1}` }],
  ['prefixed, key without the prefix', '/prefixed', { authorization: K1 }],
  ['prefixed, prefix in another case', '/prefixed', { authorization: `api-key ${K1}` }],
  ['prefixed, bearer scheme', '/prefixed', { authorization: `Bearer ${K1}` }],
  ['prefixed, prefix only', '/prefixed', { authorization: 'Api-Key ' }],
  ['plus prefix, literal prefix', '/plus', { 'x-key': `Key+${K1}` }, PLUS_DIVERGENCE],
  ['plus prefix, a character the pattern would match', '/plus', { 'x-key': `Keyy${K1}` }],
  ['plus prefix, prefix without the plus', '/plus', { 'x-key': `Key${K1}` }],
  ['query, valid key', `/query?api_key=${K1}`, {}],
  ['query, valid key with other parameters', `/query?page=2&api_key=${K1}&sort=asc`, {}],
  ['query, percent-encoded key', `/query?api_key=${K1.replace('-', '%2D')}`, {}],
  ['query, wrong key', `/query?api_key=${WRONG}`, {}],
  ['query, missing', '/query', {}],
  ['query, empty value', '/query?api_key=', {}],
  ['query, repeated parameter', `/query?api_key=${K1}&api_key=${K2}`, {}],
  ['query, bracketed parameter', `/query?api_key[]=${K1}`, {}],
  ['query, key in the header is ignored', '/query', { 'x-api-key': K1 }],
  ['query, control character in the key', '/query?api_key=test-key-%00000000000000000000000000001', {}, CONTROL],
  ['bearer, valid key', '/bearer', { authorization: `Bearer ${K1}` }],
  ['bearer, scheme in lower case', '/bearer', { authorization: `bearer ${K1}` }],
  ['bearer, several spaces', '/bearer', { authorization: `Bearer     ${K1}` }],
  ['bearer, another scheme', '/bearer', { authorization: `Basic ${K1}` }],
  ['bearer, scheme only', '/bearer', { authorization: 'Bearer' }],
  ['bearer, no scheme', '/bearer', { authorization: K1 }],
  ['bearer, missing', '/bearer', {}],
  ['bearer, wrong key', '/bearer', { authorization: `Bearer ${WRONG}` }],
  ['bearer, oversized key', '/bearer', { authorization: `Bearer test-key-${long(600)}` }, OVERSIZED],
  ['chain, only the API key', '/chain', { 'x-api-key': K1 }],
  ['chain, only the bearer user', '/chain', { authorization: 'Bearer user-token-0000000000000000000000000001' }],
  ['chain, both valid: the first strategy wins', '/chain', { authorization: 'Bearer user-token-0000000000000000000000000001', 'x-api-key': K1 }],
  ['chain, bearer wrong and API key valid', '/chain', { authorization: `Bearer ${WRONG}`, 'x-api-key': K1 }],
  ['chain, neither', '/chain', {}],
  ['chain, API key wrong', '/chain', { 'x-api-key': WRONG }],
  ['chain, API key suspended', '/chain', { 'x-api-key': KTHROW }],
  ['two places, header only', '/two-places', { 'x-api-key': K1 }],
  ['two places, query only', `/two-places?api_key=${K1}`, {}],
  ['two places, the same key in both', `/two-places?api_key=${K1}`, { 'x-api-key': K1 }],
  ['two places, different keys', `/two-places?api_key=${K2}`, { 'x-api-key': K1 }, CONFLICT],
  ['two places, neither', '/two-places', {}],
];

async function main() {
  class AppModule {}
  Module({ controllers: [Routes] })(AppModule);
  const app = await NestFactory.create(AppModule, { logger: false });
  await app.listen(0, '127.0.0.1');
  const base = `http://127.0.0.1:${app.getHttpServer().address().port}`;
  const cases = [];
  try {
    for (const [name, path, headers, divergence = null] of CASES) {
      const res = await fetch(base + path, { method: 'GET', headers });
      const text = await res.text();
      let parsed;
      try {
        parsed = JSON.parse(text);
      } catch {
        parsed = text;
      }
      cases.push({ name, request: { method: 'GET', path, headers }, response: { status: res.status, body: parsed }, divergence });
    }
  } finally {
    await app.close();
  }
  writeFileSync(
    new URL('./api_key.json', import.meta.url),
    JSON.stringify(
      {
        versions: {
          'passport-headerapikey': version('passport-headerapikey'),
          'passport-custom': version('passport-custom'),
          '@nestjs/passport': version('@nestjs/passport'),
        },
        cases,
      },
      null,
      2,
    ) + '\n',
  );
}
await main();
