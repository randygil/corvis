// Records what `AuthGuard(name)` of @nestjs/passport 12.0.1 over passport 0.7.0 answers when a custom
// `passport-strategy` 1.0.0 strategy ends an attempt in each way (success, fail, redirect, pass, error). Run once with
// `npm ci && node strategies.mjs`; the output (`strategies.json`) is committed and replayed by
// `tests/auth_strategies.rs`. The servers bind to 127.0.0.1 only.
//
// One strategy class serves the names `a` and `b`. The header `x-outcome-<name>` (else `x-outcome`) picks the outcome:
//   success, success-noinfo, fail, fail-challenge, fail-400, error-forbidden, error, redirect, redirect-301,
//   redirect-crlf, pass
//
// Three apps:
//   main    routes with one guard each (see `ROUTES`)
//   filter  a global exception filter that catches UnauthorizedException
//   public  a global guard that skips routes marked `isPublic` (the usual `Reflector` check before `super`)
//
// A case with a `divergence` is a behavior Corvis changes on purpose; the Rust test asserts the specification instead
// of the recorded response.
import 'reflect-metadata';
import { writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { Module, Controller, Get, Req, UseGuards, SetMetadata, Catch, ForbiddenException, UnauthorizedException } from '@nestjs/common';
import { NestFactory, Reflector } from '@nestjs/core';
import { AuthGuard } from '@nestjs/passport';

const require = createRequire(import.meta.url);
const passport = require('passport');
const { Strategy } = require('passport-strategy');
const passportVersion = require('passport/package.json').version;
const nestPassportVersion = require('@nestjs/passport/package.json').version;

class Driven extends Strategy {
  constructor(name) {
    super();
    this.name = name;
  }
  authenticate(req) {
    const outcome = req.headers[`x-outcome-${this.name}`] ?? req.headers['x-outcome'] ?? 'fail';
    switch (outcome) {
      case 'success':
        return this.success({ id: 7, role: 'admin' }, { scope: 'read' });
      case 'success-noinfo':
        return this.success({ id: 7, role: 'admin' });
      case 'fail':
        return this.fail();
      case 'fail-challenge':
        return this.fail('no token');
      case 'fail-400':
        return this.fail({ message: 'Missing credentials' }, 400);
      case 'error-forbidden':
        return this.error(new ForbiddenException('Account locked'));
      case 'error':
        return this.error(new Error('db down'));
      case 'redirect':
        return this.redirect('https://idp.example/authorize?x=1');
      case 'redirect-301':
        return this.redirect('/moved', 301);
      case 'redirect-crlf':
        return this.redirect('/a\r\nSet-Cookie: x=1');
      case 'pass':
        return this.pass();
      default:
        throw new Error(`unknown outcome ${outcome}`);
    }
  }
}
passport.use('a', new Driven('a'));
passport.use('b', new Driven('b'));

// The usual overrides of `handleRequest`. `user || null`, not `user ?? null`: a failed attempt hands the callback
// `false`, which `??` keeps as `req.user`; the optional guard of Corvis leaves no user, which is the `null` here.
class OptionalGuard extends AuthGuard('a') {
  handleRequest(err, user) {
    if (err) throw err;
    return user || null;
  }
}
class OptionalChainGuard extends AuthGuard(['a', 'b']) {
  handleRequest(err, user) {
    if (err) throw err;
    return user || null;
  }
}
class CustomGuard extends AuthGuard('a') {
  handleRequest(err, user) {
    if (err || !user) throw err || new UnauthorizedException('Token required');
    return user;
  }
}
class PublicAwareGuard extends AuthGuard('a') {
  constructor(reflector) {
    super();
    this.reflector = reflector;
  }
  canActivate(context) {
    const isPublic = this.reflector.getAllAndOverride('isPublic', [context.getHandler(), context.getClass()]);
    if (isPublic) return true;
    return super.canActivate(context);
  }
}

// Handlers answer what the guard stored on the request.
function show(req) {
  return { user: req.user ?? null, authInfo: req.authInfo ?? null };
}

function defineController(Class, prefix, routes, classDecorators = []) {
  for (const [name, path, ...decorators] of routes) {
    const proto = Class.prototype;
    proto[name] = function (req) {
      return show(req);
    };
    const descriptor = Object.getOwnPropertyDescriptor(proto, name);
    Get(path)(proto, name, descriptor);
    Req()(proto, name, 0);
    for (const decorator of decorators) decorator(proto, name, descriptor);
  }
  Controller(prefix)(Class);
  for (const decorator of classDecorators) decorator(Class);
}

const use = (guard) => (proto, name, descriptor) => UseGuards(guard)(proto, name, descriptor);

class Routes {}
const ROUTES = [
  ['single', 'single', use(AuthGuard('a'))],
  ['chain', 'chain', use(AuthGuard(['a', 'b']))],
  ['optional', 'optional', use(OptionalGuard)],
  ['optionalChain', 'optional-chain', use(OptionalChainGuard)],
  ['custom', 'custom', use(CustomGuard)],
  ['unknown', 'unknown', use(AuthGuard('typo'))],
];
defineController(Routes, '', ROUTES);

class PublicRoutes {}
defineController(PublicRoutes, '', [
  ['open', 'open', (proto, name, d) => SetMetadata('isPublic', true)(proto, name, d)],
  ['closed', 'closed'],
]);
class PublicController {}
defineController(
  PublicController,
  'ctrl',
  [
    ['open', 'open'],
    ['overridden', 'overridden', (proto, name, d) => SetMetadata('isPublic', false)(proto, name, d)],
  ],
  [(c) => SetMetadata('isPublic', true)(c)],
);

class AllUnauthorized {
  catch(exception, host) {
    host.switchToHttp().getResponse().status(401).json({ error: 'login' });
  }
}
Catch(UnauthorizedException)(AllUnauthorized);

const HEADERS = ['location', 'www-authenticate', 'set-cookie'];

const CASES = [
  // [app, name, path, request headers, divergence]
  ['main', 'success with info', '/single', { 'x-outcome': 'success' }],
  ['main', 'success without info', '/single', { 'x-outcome': 'success-noinfo' }],
  ['main', 'fail without arguments', '/single', { 'x-outcome': 'fail' }],
  ['main', 'fail with a challenge', '/single', { 'x-outcome': 'fail-challenge' }],
  ['main', 'fail with status 400', '/single', { 'x-outcome': 'fail-400' }],
  ['main', 'error with an exception', '/single', { 'x-outcome': 'error-forbidden' }],
  ['main', 'error with an Error', '/single', { 'x-outcome': 'error' }],
  ['main', 'redirect', '/single', { 'x-outcome': 'redirect' }, null, true],
  ['main', 'redirect with status 301', '/single', { 'x-outcome': 'redirect-301' }, null, true],
  ['main', 'redirect URL with a line break', '/single', { 'x-outcome': 'redirect-crlf' }],
  ['main', 'pass', '/single', { 'x-outcome': 'pass' }],
  ['main', 'chain second succeeds', '/chain', { 'x-outcome-a': 'fail', 'x-outcome-b': 'success' }],
  ['main', 'chain first succeeds', '/chain', { 'x-outcome-a': 'success', 'x-outcome-b': 'fail' }],
  ['main', 'chain first errors', '/chain', { 'x-outcome-a': 'error-forbidden', 'x-outcome-b': 'success' }],
  ['main', 'chain first passes', '/chain', { 'x-outcome-a': 'pass', 'x-outcome-b': 'success' }],
  ['main', 'chain first redirects', '/chain', { 'x-outcome-a': 'redirect', 'x-outcome-b': 'success' }, null, true],
  ['main', 'chain all fail', '/chain', { 'x-outcome-a': 'fail-challenge', 'x-outcome-b': 'fail-400' }],
  ['main', 'optional guard without credentials', '/optional', { 'x-outcome': 'fail' }],
  ['main', 'optional guard with a challenge', '/optional', { 'x-outcome': 'fail-challenge' }],
  ['main', 'optional guard with credentials', '/optional', { 'x-outcome': 'success' }],
  ['main', 'optional guard still passes errors on', '/optional', { 'x-outcome': 'error-forbidden' }],
  ['main', 'optional chain all fail', '/optional-chain', { 'x-outcome-a': 'fail-challenge', 'x-outcome-b': 'fail' }],
  ['main', 'optional chain single failure with challenge', '/optional-chain', { 'x-outcome-a': 'fail', 'x-outcome-b': 'fail-challenge' }],
  ['main', 'custom failure exception', '/custom', { 'x-outcome': 'fail' }],
  [
    'main',
    'unknown strategy in a guard',
    '/unknown',
    {},
    'Corvis stops startup with an error naming the route and the strategy; the original answers 500 at request time',
  ],
  ['filter', 'global filter catches the 401', '/single', { 'x-outcome': 'fail' }],
  ['filter', 'global filter does not see a redirect', '/single', { 'x-outcome': 'redirect' }, null, true],
  ['public', 'public route is not authenticated', '/open', { 'x-outcome': 'fail' }],
  ['public', 'other route is authenticated', '/closed', { 'x-outcome': 'fail' }],
  ['public', 'public controller', '/ctrl/open', { 'x-outcome': 'fail' }],
  ['public', 'handler overrides a public controller', '/ctrl/overridden', { 'x-outcome': 'fail' }],
];

async function startApp(name) {
  const imports = [];
  const controllers = { main: [Routes], filter: [Routes], public: [PublicRoutes, PublicController] }[name];
  class AppModule {}
  Module({ imports, controllers })(AppModule);
  const app = await NestFactory.create(AppModule, { logger: false });
  if (name === 'filter') app.useGlobalFilters(new AllUnauthorized());
  if (name === 'public') app.useGlobalGuards(new PublicAwareGuard(app.get(Reflector)));
  await app.listen(0, '127.0.0.1');
  return { app, base: `http://127.0.0.1:${app.getHttpServer().address().port}` };
}

const apps = {};
const cases = [];
try {
  for (const [app, name, path, headers, divergence = null, redirect = false] of CASES) {
    if (!apps[app]) apps[app] = await startApp(app);
    const res = await fetch(apps[app].base + path, { headers, redirect: 'manual' });
    const text = await res.text();
    let body;
    try {
      body = JSON.parse(text);
    } catch {
      body = text;
    }
    const names = redirect ? [...HEADERS, 'content-length'] : HEADERS;
    const recorded = {};
    for (const h of names) recorded[h] = res.headers.get(h);
    cases.push({
      name,
      app,
      request: { method: 'GET', path, headers },
      response: { status: res.status, headers: recorded, body },
      divergence,
    });
  }
} finally {
  for (const { app } of Object.values(apps)) await app.close();
}

writeFileSync(
  new URL('./strategies.json', import.meta.url),
  JSON.stringify({ versions: { '@nestjs/passport': nestPassportVersion, passport: passportVersion }, cases }, null, 2) + '\n',
);
