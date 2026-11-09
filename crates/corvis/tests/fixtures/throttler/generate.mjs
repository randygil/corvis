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
import 'reflect-metadata';
import { writeFileSync } from 'node:fs';
import { Module, Controller, Get } from '@nestjs/common';
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

async function record(throttlers, requests) {
  class AppModule {}
  Module({
    imports: [ThrottlerModule.forRoot(throttlers)],
    controllers: [Routes, Undo, Mixed],
    providers: [{ provide: APP_GUARD, useClass: ThrottlerGuard }],
  })(AppModule);
  const app = await NestFactory.create(AppModule, { logger: false });
  await app.listen(0, '127.0.0.1');
  const base = `http://127.0.0.1:${app.getHttpServer().address().port}`;
  const out = [];
  const start = Date.now();
  for (const request of requests) {
    // A request is a path, or { path, at } to send it `at` milliseconds after the start of the sequence.
    const { path, at } = typeof request === 'string' ? { path: request } : request;
    if (at !== undefined) await new Promise((r) => setTimeout(r, Math.max(0, start + at - Date.now())));
    const res = await fetch(base + path);
    const headers = {};
    for (const [k, v] of res.headers) {
      if (k.startsWith('x-ratelimit') || k.startsWith('retry-after')) headers[k] = v;
    }
    out.push(at === undefined ? { path, status: res.status, headers } : { path, at, status: res.status, headers });
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
];
for (const a of apps) a.responses = await record(a.throttlers, a.requests);
writeFileSync(new URL('./throttler.json', import.meta.url), JSON.stringify(apps, null, 2) + '\n');
