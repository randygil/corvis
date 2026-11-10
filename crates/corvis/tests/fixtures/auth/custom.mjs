// Records what `AuthGuard(name)` of @nestjs/passport 12.0.1 over passport 0.7.0 answers for a hand-written
// `passport-custom` 1.2.1 strategy that uses an injected service, for the guide "Writing a strategy" of Corvis. Run once
// with `npm ci && node custom.mjs`; the output (`custom.json`) is committed and replayed by `tests/auth_custom.rs`.
// The server binds to 127.0.0.1 only.
//
// The `device` strategy reads the header `x-device-id` and asks `DevicesService.find(id)`: a device is a success (the
// device `dev-info` also reports info), no device is a failure, `dev-blocked` is a ForbiddenException ("Device
// blocked") and `dev-broken` a plain Error. Routes: `GET /device` (AuthGuard('device')) and `GET /chain`
// (AuthGuard(['bearer-user', 'device']): a strategy that accepts one bearer token, then the device strategy). The
// `falsy` strategy hands whatever `x-falsy` names to `done` as the user (false, 0, '', null, undefined, NaN or a real
// user), alone on `GET /falsy` and before the device strategy on `GET /falsy-chain`.
import 'reflect-metadata';
import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { Module, Controller, Get, Req, UseGuards, ForbiddenException } from '@nestjs/common';
import { NestFactory } from '@nestjs/core';
import { AuthGuard } from '@nestjs/passport';

const require = createRequire(import.meta.url);
const passport = require('passport');
const CustomStrategy = require('passport-custom');
const { ExtractJwt } = require('passport-jwt');
const version = (name) => JSON.parse(readFileSync(new URL(`./node_modules/${name}/package.json`, import.meta.url), 'utf8')).version;

class DevicesService {
  async find(id) {
    if (id === 'dev-blocked') throw new ForbiddenException('Device blocked');
    if (id === 'dev-broken') throw new Error('devices store down');
    if (id === 'dev-1' || id === 'dev-info') return { id, name: `device ${id}` };
    return null;
  }
}

// The injected service: the strategy is built with it, as the container builds the class with its dependencies.
class DeviceStrategy extends CustomStrategy {
  constructor(devices) {
    super(async (req, done) => {
      try {
        const id = req.headers['x-device-id'];
        if (typeof id !== 'string' || id === '') return done(null, false);
        const device = await devices.find(id);
        if (!device) return done(null, false);
        done(null, device, id === 'dev-info' ? { scope: 'telemetry' } : undefined);
      } catch (err) {
        done(err);
      }
    });
  }
}
passport.use('device', new DeviceStrategy(new DevicesService()));
passport.use(
  'bearer-user',
  new CustomStrategy((req, done) => {
    const token = ExtractJwt.fromAuthHeaderAsBearerToken()(req);
    done(null, token === 'user-token-0000000000000000000000000001' ? { id: 'user-1' } : false);
  }),
);

// A validator written as `return isValid`: the header `x-falsy` picks what `done` receives. A falsy user is a failure
// of the strategy (`if (!user) return self.fail()` in passport-custom), and the guard's `handleRequest` throws a 401
// for a missing user, so none of them authenticates.
const FALSY = { false: false, zero: 0, empty: '', null: null, undefined: undefined, nan: NaN, truthy: { id: 'falsy-1' } };
passport.use(
  'falsy',
  new CustomStrategy((req, done) => {
    const pick = req.headers['x-falsy'];
    if (!Object.hasOwn(FALSY, pick)) return done(null, false);
    done(null, FALSY[pick]);
  }),
);

class Routes {}
for (const [route, strategy] of [
  ['device', 'device'],
  ['chain', ['bearer-user', 'device']],
  ['falsy', 'falsy'],
  ['falsy-chain', ['falsy', 'device']],
]) {
  Routes.prototype[route] = function (req) {
    return { user: req.user ?? null };
  };
  const d = Object.getOwnPropertyDescriptor(Routes.prototype, route);
  Get(route)(Routes.prototype, route, d);
  Req()(Routes.prototype, route, 0);
  UseGuards(AuthGuard(strategy))(Routes.prototype, route, d);
}
Controller('')(Routes);

const BEARER = { authorization: 'Bearer user-token-0000000000000000000000000001' };
const CASES = [
  ['device found', '/device', { 'x-device-id': 'dev-1' }],
  ['device found with info', '/device', { 'x-device-id': 'dev-info' }],
  ['device unknown', '/device', { 'x-device-id': 'dev-404' }],
  ['no device header', '/device', {}],
  ['empty device header', '/device', { 'x-device-id': '' }],
  ['device blocked', '/device', { 'x-device-id': 'dev-blocked' }],
  ['devices store fails', '/device', { 'x-device-id': 'dev-broken' }],
  ['chain, only the device', '/chain', { 'x-device-id': 'dev-1' }],
  ['chain, only the bearer user', '/chain', BEARER],
  ['chain, both: the first strategy wins', '/chain', { ...BEARER, 'x-device-id': 'dev-1' }],
  ['chain, neither', '/chain', {}],
  ['chain, device unknown', '/chain', { 'x-device-id': 'dev-404' }],
  ['chain, device blocked', '/chain', { 'x-device-id': 'dev-blocked' }],
  ['chain, devices store fails', '/chain', { 'x-device-id': 'dev-broken' }],
  ['chain, bearer user and a failing device store', '/chain', { ...BEARER, 'x-device-id': 'dev-broken' }],
  ['falsy user: done(null, false)', '/falsy', { 'x-falsy': 'false' }],
  ['falsy user: done(null, 0)', '/falsy', { 'x-falsy': 'zero' }],
  ["falsy user: done(null, '')", '/falsy', { 'x-falsy': 'empty' }],
  ['falsy user: done(null, null)', '/falsy', { 'x-falsy': 'null' }],
  ['falsy user: done(null, undefined)', '/falsy', { 'x-falsy': 'undefined' }],
  ['falsy user: done(null, NaN)', '/falsy', { 'x-falsy': 'nan' }],
  ['a real user is the control', '/falsy', { 'x-falsy': 'truthy' }],
  ['falsy user then a device that exists: the chain goes on', '/falsy-chain', { 'x-falsy': 'false', 'x-device-id': 'dev-1' }],
  ['falsy user then no device', '/falsy-chain', { 'x-falsy': 'zero' }],
];

async function main() {
  class AppModule {}
  Module({ controllers: [Routes] })(AppModule);
  const app = await NestFactory.create(AppModule, { logger: false });
  await app.listen(0, '127.0.0.1');
  const base = `http://127.0.0.1:${app.getHttpServer().address().port}`;
  const cases = [];
  try {
    for (const [name, path, headers] of CASES) {
      const res = await fetch(base + path, { method: 'GET', headers });
      const text = await res.text();
      let parsed;
      try {
        parsed = JSON.parse(text);
      } catch {
        parsed = text;
      }
      cases.push({ name, request: { method: 'GET', path, headers }, response: { status: res.status, body: parsed } });
    }
  } finally {
    await app.close();
  }
  writeFileSync(
    new URL('./custom.json', import.meta.url),
    JSON.stringify(
      { versions: { 'passport-custom': version('passport-custom'), '@nestjs/passport': version('@nestjs/passport') }, cases },
      null,
      2,
    ) + '\n',
  );
}
await main();
