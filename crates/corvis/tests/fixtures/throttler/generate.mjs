// Records the rate limit headers `@nestjs/throttler` sends for a fixed request sequence. Run once with
// `npm install && node generate.mjs`; the output (`throttler.json`) is committed and replayed by
// `tests/throttler_headers.rs`. The servers bind to 127.0.0.1 only.
//
// Three apps, all with long windows so the recorded values do not depend on timing:
//   named   short (3 per 30 s) and long (5 per 60 s)
//   second  short (100 per 30 s) and long (2 per 60 s): the second throttler is the one that blocks
//   default one throttler named `default` (3 per 60 s); `/override` has `@Throttle` limit 1
//   skips-named    short (3 per 30 s) and long (5 per 60 s) with the skip routes of #152
//   skips-default  default (3 per 60 s) and short (3 per 30 s) with the same skip routes
//   overrides-named    short (3 per 30 s) and long (5 per 60 s) with per-name `@Throttle` overrides (#156)
//   overrides-default  default (3 per 60 s) and short (3 per 30 s); `@Throttle({ default })` next to a named throttler
//   module-tracker     default (1 per 60 s) with a module-level `getTracker` that reads the `x-client` header
//   module-key         default (1 per 60 s) with a module-level `generateKey` that drops the route
//   shared-key         short (100 per 30 s) and long (2 per 60 s) whose `generateKey` ignores the throttler name: the
//                      original shares the reporting window and the block between them, Corvis keeps them apart
//   throttler-tracker  short (1 per 30 s) with its own `getTracker` and long (3 per 60 s), under a module `getTracker` (#162)
//   throttler-key      default (1 per 60 s) with its own `generateKey`, under a module `generateKey`
//   dynamic-limit      default (60 s) with `limit` and `blockDuration` functions
//   dynamic-ttl-order  a (60 s, 2) and b (`ttl` function, 1): the function throttler is checked last
//   function-error     default whose `limit` function throws an Error, a ForbiddenException, or returns 1
//
// The functions of the module options (`getTracker`, `generateKey`) cannot be stored in the JSON; the Rust replay
// rebuilds each one from the case name (see `tests/throttler_headers.rs`).
import 'reflect-metadata';
import { writeFileSync } from 'node:fs';
import { Module, Controller, Get, ForbiddenException } from '@nestjs/common';
import { NestFactory, APP_GUARD } from '@nestjs/core';
import { ThrottlerModule, ThrottlerGuard, SkipThrottle, Throttle } from '@nestjs/throttler';

class Routes {
  plain() {
    return 'ok';
  }
  skip() {
    return 'ok';
  }
  override() {
    return 'ok';
  }
  bare() {
    return 'ok';
  }
  byname() {
    return 'ok';
  }
}
// A controller with `@SkipThrottle()` whose handler `on` undoes it for `default`.
class Undo {
  on() {
    return 'ok';
  }
  off() {
    return 'ok';
  }
}
// A controller that skips `short` while its handler has a bare `@SkipThrottle()`.
class Mixed {
  h() {
    return 'ok';
  }
}
// Decorators are applied by hand: the fixture is plain JavaScript.
const proto = Routes.prototype;
const method = (name) => Object.getOwnPropertyDescriptor(proto, name);
Get('plain')(proto, 'plain', method('plain'));
Get('skip')(proto, 'skip', method('skip'));
// `@SkipThrottle()` alone only skips the throttler named `default`; every throttler of the apps is listed.
SkipThrottle({ default: true, short: true, long: true })(proto.skip);
Get('override')(proto, 'override', method('override'));
Throttle({ default: { limit: 1, ttl: 60000 } })(proto.override);
// `@SkipThrottle()` with no argument is `@SkipThrottle({ default: true })`.
Get('bare')(proto, 'bare', method('bare'));
SkipThrottle()(proto, 'bare', method('bare'));
Get('byname')(proto, 'byname', method('byname'));
SkipThrottle({ short: true })(proto, 'byname', method('byname'));
Controller('')(Routes);

const undoMethod = (name) => Object.getOwnPropertyDescriptor(Undo.prototype, name);
Get('on')(Undo.prototype, 'on', undoMethod('on'));
SkipThrottle({ default: false })(Undo.prototype, 'on', undoMethod('on'));
Get('off')(Undo.prototype, 'off', undoMethod('off'));
SkipThrottle()(Undo);
Controller('undo')(Undo);

Get('h')(Mixed.prototype, 'h', Object.getOwnPropertyDescriptor(Mixed.prototype, 'h'));
SkipThrottle()(Mixed.prototype, 'h', Object.getOwnPropertyDescriptor(Mixed.prototype, 'h'));
SkipThrottle({ short: true })(Mixed);
Controller('mixed')(Mixed);

// Per-name overrides (#156). New controllers, so the recorded values of the cases above do not move.
class Ovr {
  short1() {
    return 'ok';
  }
  both() {
    return 'ok';
  }
  block() {
    return 'ok';
  }
  unknown() {
    return 'ok';
  }
  default() {
    return 'ok';
  }
  invalid() {
    return 'ok';
  }
  fixed() {
    return 'ok';
  }
  ownKey() {
    return 'ok';
  }
}
const ovr = (name, path, options) => {
  const d = Object.getOwnPropertyDescriptor(Ovr.prototype, name);
  Get(path)(Ovr.prototype, name, d);
  if (options) Throttle(options)(Ovr.prototype, name, d);
};
ovr('short1', 'short1', { short: { limit: 1 } });
ovr('both', 'both', { short: { limit: 2, ttl: 10000 }, long: { limit: 1 } });
ovr('block', 'block', { short: { limit: 1, blockDuration: 5000 } });
ovr('unknown', 'unknown', { shrot: { limit: 1 } });
ovr('default', 'default', { default: { limit: 1 } });
ovr('invalid', 'invalid', { short: { limit: 'abc' } });
ovr('fixed', 'fixed', { default: { getTracker: () => 'fixed' } });
ovr('ownKey', 'own-key', { default: { generateKey: (ctx, tracker) => 'own:' + tracker } });
Controller('ovr')(Ovr);

// The class overrides `short`; the handler `h` overrides only its limit and keeps the class ttl; `sibling` has none.
class OvrClass {
  h() {
    return 'ok';
  }
  sibling() {
    return 'ok';
  }
}
Get('h')(OvrClass.prototype, 'h', Object.getOwnPropertyDescriptor(OvrClass.prototype, 'h'));
Throttle({ short: { limit: 1 } })(OvrClass.prototype, 'h', Object.getOwnPropertyDescriptor(OvrClass.prototype, 'h'));
Get('sibling')(OvrClass.prototype, 'sibling', Object.getOwnPropertyDescriptor(OvrClass.prototype, 'sibling'));
Throttle({ short: { limit: 2, ttl: 20000 } })(OvrClass);
Controller('ovr-class')(OvrClass);

// Per-throttler hooks and function-valued options (#162). A new controller, so the recorded values above do not move.
class Ptr {
  fixed() {
    return 'ok';
  }
  ownKey() {
    return 'ok';
  }
  limit5() {
    return 'ok';
  }
  pro() {
    return 'ok';
  }
}
const ptr = (name, path, options) => {
  const d = Object.getOwnPropertyDescriptor(Ptr.prototype, name);
  Get(path)(Ptr.prototype, name, d);
  if (options) Throttle(options)(Ptr.prototype, name, d);
};
ptr('fixed', 'fixed', { short: { getTracker: () => 'fixed' } });
ptr('ownKey', 'own-key', { default: { generateKey: (ctx, t) => 'own:' + t } });
ptr('limit5', 'limit5', { default: { limit: 5 } });
ptr('pro', 'pro');
Controller('ptr')(Ptr);

async function record(throttlers, requests, moduleOptions) {
  class AppModule {}
  Module({
    imports: [ThrottlerModule.forRoot(moduleOptions ? { throttlers, ...moduleOptions } : throttlers)],
    controllers: [Routes, Undo, Mixed, Ovr, OvrClass, Ptr],
    providers: [{ provide: APP_GUARD, useClass: ThrottlerGuard }],
  })(AppModule);
  const app = await NestFactory.create(AppModule, { logger: false });
  await app.listen(0, '127.0.0.1');
  const base = `http://127.0.0.1:${app.getHttpServer().address().port}`;
  const out = [];
  const start = Date.now();
  for (const request of requests) {
    // A request is a path, or { path, at, send } to send it `at` milliseconds after the start of the sequence with the
    // request headers `send`.
    const { path, at, send } = typeof request === 'string' ? { path: request } : request;
    if (at !== undefined) await new Promise((r) => setTimeout(r, Math.max(0, start + at - Date.now())));
    const res = await fetch(base + path, send ? { headers: send } : undefined);
    const headers = {};
    for (const [k, v] of res.headers) {
      if (k.startsWith('x-ratelimit') || k.startsWith('retry-after')) headers[k] = v;
    }
    const entry = { path };
    if (at !== undefined) entry.at = at;
    if (send !== undefined) entry.send = send;
    out.push({ ...entry, status: res.status, headers });
  }
  await app.close();
  return out;
}

const apps = [
  {
    name: 'named',
    throttlers: [
      { name: 'short', ttl: 30000, limit: 3 },
      { name: 'long', ttl: 60000, limit: 5 },
    ],
    requests: ['/plain', '/plain', '/plain', '/plain', '/plain', '/skip'],
  },
  {
    name: 'second',
    throttlers: [
      { name: 'short', ttl: 30000, limit: 100 },
      { name: 'long', ttl: 60000, limit: 2 },
    ],
    requests: ['/plain', '/plain', '/plain'],
  },
  {
    name: 'default',
    throttlers: [{ name: 'default', ttl: 60000, limit: 3 }],
    requests: ['/plain', '/plain', '/plain', '/plain', '/skip', '/override', '/override', '/bare', '/bare'],
  },
  // A bare skip only skips `default`; with named throttlers only it skips nothing (the fourth request is a 429).
  {
    name: 'skips-named',
    throttlers: [
      { name: 'short', ttl: 30000, limit: 3 },
      { name: 'long', ttl: 60000, limit: 5 },
    ],
    requests: [
      '/bare',
      '/bare',
      '/bare',
      '/bare',
      '/byname',
      '/byname',
      '/byname',
      '/byname',
      '/byname',
      '/byname',
      '/mixed/h',
      '/mixed/h',
    ],
  },
  {
    name: 'skips-default',
    throttlers: [
      { name: 'default', ttl: 60000, limit: 3 },
      { name: 'short', ttl: 30000, limit: 3 },
    ],
    requests: [
      '/bare',
      '/bare',
      '/bare',
      '/bare',
      '/undo/on',
      '/undo/on',
      '/undo/off',
      '/undo/off',
      '/mixed/h',
      '/mixed/h',
      '/mixed/h',
      '/mixed/h',
    ],
  },
  // A 2-second throttler that blocks (#135): the block lasts one ttl from the moment of the third request. The retry at
  // 500 ms and the request at 2600 ms are at least 300 ms away from every expiry (about 2000 ms).
  {
    name: 'block',
    throttlers: [{ name: 'default', ttl: 2000, limit: 2 }],
    requests: [
      '/plain',
      '/plain',
      '/plain',
      { path: '/plain', at: 500 },
      { path: '/plain', at: 2600 },
      { path: '/plain', at: 2700 },
    ],
  },
  {
    name: 'overrides-named',
    throttlers: [
      { name: 'short', ttl: 30000, limit: 3 },
      { name: 'long', ttl: 60000, limit: 5 },
    ],
    requests: [
      '/ovr/short1',
      '/ovr/short1',
      '/ovr/both',
      '/ovr/both',
      '/ovr-class/h',
      '/ovr-class/h',
      '/ovr-class/sibling',
      '/ovr-class/sibling',
      '/ovr-class/sibling',
      '/ovr/block',
      '/ovr/block',
      '/ovr/unknown',
      '/ovr/unknown',
      '/ovr/default',
      '/ovr/default',
      '/ovr/invalid',
    ],
  },
  // `@Throttle({ default })` limits `default` only; `short` keeps its own limit (the replay uses a bare THROTTLE_LIMIT).
  {
    name: 'overrides-default',
    throttlers: [
      { name: 'default', ttl: 60000, limit: 3 },
      { name: 'short', ttl: 30000, limit: 3 },
    ],
    requests: ['/ovr/default', '/ovr/default'],
  },
  {
    name: 'module-tracker',
    throttlers: [{ name: 'default', ttl: 60000, limit: 1 }],
    moduleOptions: { getTracker: (req) => req.headers['x-client'] ?? req.ip },
    requests: [
      { path: '/plain', send: { 'x-client': 'a' } },
      { path: '/plain', send: { 'x-client': 'a' } },
      { path: '/plain', send: { 'x-client': 'b' } },
      { path: '/ovr/fixed', send: { 'x-client': 'a' } },
      { path: '/ovr/fixed', send: { 'x-client': 'b' } },
    ],
  },
  {
    name: 'module-key',
    throttlers: [{ name: 'default', ttl: 60000, limit: 1 }],
    moduleOptions: { generateKey: (ctx, tracker, name) => name + ':' + tracker },
    requests: ['/plain', '/byname', '/ovr/own-key'],
  },
  // Divergence: the key is the same for both throttlers. The original then reports one window and one block for
  // both (`x-ratelimit-reset-long` is the short window, `retry-after-short` appears on the blocked requests).
  {
    name: 'shared-key',
    throttlers: [
      { name: 'short', ttl: 30000, limit: 100 },
      { name: 'long', ttl: 60000, limit: 2 },
    ],
    moduleOptions: { generateKey: (ctx, tracker) => tracker },
    requests: ['/plain', '/plain', '/plain', '/plain'],
  },
  {
    name: 'throttler-tracker',
    throttlers: [
      { name: 'short', ttl: 30000, limit: 1, getTracker: (req) => 't:' + (req.headers['x-account'] ?? 'none') },
      { name: 'long', ttl: 60000, limit: 3 },
    ],
    moduleOptions: { getTracker: (req) => 'm:' + (req.headers['x-client'] ?? 'none') },
    requests: [
      { path: '/plain', send: { 'x-client': 'a', 'x-account': '1' } },
      { path: '/plain', send: { 'x-client': 'b', 'x-account': '1' } },
      { path: '/plain', send: { 'x-client': 'a', 'x-account': '2' } },
      { path: '/ptr/fixed', send: { 'x-account': '3' } },
      { path: '/ptr/fixed', send: { 'x-account': '4' } },
    ],
  },
  {
    name: 'throttler-key',
    throttlers: [{ name: 'default', ttl: 60000, limit: 1, generateKey: (ctx, t) => 'shared:' + t }],
    moduleOptions: { generateKey: (ctx, t) => ctx.getHandler().name + ':' + t },
    requests: ['/plain', '/byname', '/ptr/own-key'],
  },
  {
    name: 'dynamic-limit',
    throttlers: [
      {
        name: 'default',
        ttl: 60000,
        limit: (ctx) => (ctx.switchToHttp().getRequest().headers['x-plan'] === 'pro' ? 3 : 1),
        blockDuration: async () => 20000,
      },
    ],
    requests: [
      '/plain',
      '/plain',
      ...Array.from({ length: 4 }, () => ({ path: '/ptr/pro', send: { 'x-plan': 'pro' } })),
      '/ptr/limit5',
    ],
  },
  {
    name: 'dynamic-ttl-order',
    throttlers: [
      { name: 'a', ttl: 60000, limit: 2 },
      { name: 'b', ttl: () => 10000, limit: 1 },
    ],
    requests: ['/plain', '/plain', '/plain'],
  },
  {
    name: 'function-error',
    throttlers: [
      {
        name: 'default',
        ttl: 60000,
        limit: (ctx) => {
          const h = ctx.switchToHttp().getRequest().headers;
          if (h['x-boom']) throw new Error('boom');
          if (h['x-forbid']) throw new ForbiddenException();
          return 1;
        },
      },
    ],
    requests: [
      { path: '/plain', send: { 'x-boom': '1' } },
      { path: '/plain', send: { 'x-forbid': '1' } },
      '/plain',
    ],
  },
];
for (const a of apps) {
  a.responses = await record(a.throttlers, a.requests, a.moduleOptions);
  delete a.moduleOptions;
}
writeFileSync(new URL('./throttler.json', import.meta.url), JSON.stringify(apps, null, 2) + '\n');
