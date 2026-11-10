// Records what `AuthGuard('local')` of the original passport integration answers when the strategy is `passport-local`
// 1.0.0, for the credential-reading cases of the local strategy of Corvis. Run once with `npm ci && node local.mjs`;
// the output (`local.json`) is committed and replayed by `tests/auth_local.rs`. The server binds to 127.0.0.1 only.
//
// Four strategies share one `validate`: `local` (default fields), `nested` (`user[email]`), `userobj` (`user`) and
// `indexed` (`names[1]`). `validate(req, username, password)` answers the user `{ username, usernameType, password }`
// unless the header `x-validate` says otherwise: `null` (no user) or `throw` (a ForbiddenException).
//
// A case with a `divergence` is a behavior Corvis changes on purpose; the Rust test asserts the specification instead
// of the recorded response.
import 'reflect-metadata';
import { writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { Module, Controller, Post, Req, UseGuards, ForbiddenException } from '@nestjs/common';
import { NestFactory } from '@nestjs/core';
import { AuthGuard, PassportStrategy } from '@nestjs/passport';

const require = createRequire(import.meta.url);
const { Strategy } = require('passport-local');
const passportLocalVersion = JSON.parse(
  (await import('node:fs')).readFileSync(new URL('./node_modules/passport-local/package.json', import.meta.url), 'utf8'),
).version;

function strategyClass(name, fields) {
  class S extends PassportStrategy(Strategy, name) {
    constructor() {
      super({ ...fields, passReqToCallback: true });
    }
    validate(req, username, password) {
      const mode = req.headers['x-validate'];
      if (mode === 'null') return null;
      if (mode === 'throw') throw new ForbiddenException('Account locked');
      return { username, usernameType: typeof username, password };
    }
  }
  return S;
}
new (strategyClass('local', {}))();
new (strategyClass('nested', { usernameField: 'user[email]' }))();
new (strategyClass('userobj', { usernameField: 'user' }))();
new (strategyClass('indexed', { usernameField: 'names[1]' }))();

class Routes {}
for (const [method, path, strategy] of [
  ['login', 'login', 'local'],
  ['nested', 'nested', 'nested'],
  ['userobj', 'userobj', 'userobj'],
  ['indexed', 'indexed', 'indexed'],
]) {
  Routes.prototype[method] = function (req) {
    return { user: req.user ?? null };
  };
  const d = Object.getOwnPropertyDescriptor(Routes.prototype, method);
  Post(path)(Routes.prototype, method, d);
  Req()(Routes.prototype, method, 0);
  UseGuards(AuthGuard(strategy))(Routes.prototype, method, d);
}
Controller('')(Routes);

const JSON_H = { 'content-type': 'application/json' };
const FORM_H = { 'content-type': 'application/x-www-form-urlencoded' };
const j = (v) => JSON.stringify(v);
const long = (n) => 'p'.repeat(n);
// Cases whose divergence starts with this text are replayed twice: refused by default, accepted with allow_query_credentials().
const QDIV = 'the original also reads credentials from the query string; Corvis does so only with allow_query_credentials() (URLs end up in logs), so by default the credential is missing';

const CASES = [
  // [name, path, headers, body, divergence]
  ['json body', '/login', JSON_H, j({ username: 'ana', password: 'secret' })],
  ['form body', '/login', FORM_H, 'username=ana&password=secret'],
  ['form body with percent-encoding', '/login', FORM_H, 'username=ana%40x.io&password=p%C3%A4ss+word%26'],
  ['query string only', '/login?username=ana&password=secret', {}, undefined, QDIV],
  ['query string with an empty JSON body', '/login?username=ana&password=secret', JSON_H, j({}), QDIV],
  ['body value wins over the query', '/login?username=eve', JSON_H, j({ username: 'ana', password: 'secret' })],
  ['username in the body, password in the query', '/login?password=secret', JSON_H, j({ username: 'ana' }), QDIV],
  ['validator returns no user', '/login', { ...JSON_H, 'x-validate': 'null' }, j({ username: 'ana', password: 'bad' })],
  ['validator throws', '/login', { ...JSON_H, 'x-validate': 'throw' }, j({ username: 'ana', password: 'secret' })],
  ['missing password', '/login', JSON_H, j({ username: 'ana' })],
  ['missing username', '/login', JSON_H, j({ password: 'secret' })],
  ['empty password', '/login', JSON_H, j({ username: 'ana', password: '' })],
  ['empty username', '/login', JSON_H, j({ username: '', password: 'secret' })],
  ['no body at all', '/login', {}, undefined],
  ['password zero', '/login', JSON_H, j({ username: 'ana', password: 0 })],
  ['password false', '/login', JSON_H, j({ username: 'ana', password: false })],
  ['password null', '/login', JSON_H, j({ username: 'ana', password: null })],
  ['empty body password falls back to the query', '/login?password=secret', JSON_H, j({ username: 'ana', password: '' }), QDIV],
  ['username is an array', '/login', JSON_H, j({ username: ['ana'], password: 'secret' })],
  ['username is an object', '/login', JSON_H, j({ username: { name: 'ana' }, password: 'secret' })],
  [
    'non-string username',
    '/login',
    JSON_H,
    j({ username: 123, password: 'secret' }),
    'the original passes the number to validate(); Corvis treats a non-string credential as missing (type confusion in user lookups)',
  ],
  [
    'non-string password',
    '/login',
    JSON_H,
    j({ username: 'ana', password: 123 }),
    'the original passes the number to validate(); Corvis treats a non-string credential as missing',
  ],
  ['custom nested field', '/nested', JSON_H, j({ user: { email: 'ana@x.io' }, password: 'secret' })],
  ['custom nested field absent', '/nested', JSON_H, j({ user: {}, password: 'secret' })],
  ['string before the end of the path', '/nested', JSON_H, j({ user: 'ana', password: 'secret' })],
  ['object at the end of the path', '/userobj', JSON_H, j({ user: { email: 'ana@x.io' }, password: 'secret' })],
  ['string at a one-segment path', '/userobj', JSON_H, j({ user: 'ana', password: 'secret' })],
  ['array index in the path', '/indexed', JSON_H, j({ names: ['x', 'ana'], password: 'secret' })],
  ['array index out of range', '/indexed', JSON_H, j({ names: ['x'], password: 'secret' })],
  ['array index on a string', '/indexed', JSON_H, j({ names: 'ana', password: 'secret' })],
  ['password of 1024 bytes', '/login', JSON_H, j({ username: 'ana', password: long(1024) })],
  [
    'oversized password',
    '/login',
    JSON_H,
    j({ username: 'ana', password: long(1025) }),
    'the original has no limit besides the body size and calls validate(); Corvis refuses a credential over 1024 bytes',
  ],
  [
    'oversized username',
    '/login',
    JSON_H,
    j({ username: long(1025), password: 'secret' }),
    'the original has no limit besides the body size and calls validate(); Corvis refuses a credential over 1024 bytes',
  ],
];

async function main() {
  class AppModule {}
  Module({ controllers: [Routes] })(AppModule);
  const app = await NestFactory.create(AppModule, { logger: false });
  await app.listen(0, '127.0.0.1');
  const base = `http://127.0.0.1:${app.getHttpServer().address().port}`;
  const cases = [];
  try {
    for (const [name, path, headers, body, divergence = null] of CASES) {
      const res = await fetch(base + path, { method: 'POST', headers, body });
      const text = await res.text();
      let parsed;
      try {
        parsed = JSON.parse(text);
      } catch {
        parsed = text;
      }
      cases.push({
        name,
        request: { method: 'POST', path, headers, body: body ?? null },
        response: { status: res.status, body: parsed },
        divergence,
      });
    }
  } finally {
    await app.close();
  }
  writeFileSync(
    new URL('./local.json', import.meta.url),
    JSON.stringify({ versions: { 'passport-local': passportLocalVersion }, cases }, null, 2) + '\n',
  );
}
await main();
