// Records how the original framework applies global guards, pipes, interceptors and filters to a
// gateway, and which `exception` frames it sends. Run once with `npm install && node generate.mjs`;
// the output (`ws_gateways.json`) is committed and replayed by `tests/ws_gateways.rs`.
// The server binds to 127.0.0.1 only.
import 'reflect-metadata';
import { writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { Module, Injectable, UseGuards, UseFilters, UseInterceptors, Catch } from '@nestjs/common';
import { NestFactory, APP_GUARD, APP_FILTER, APP_PIPE, APP_INTERCEPTOR } from '@nestjs/core';
import { WebSocketGateway, SubscribeMessage, MessageBody, ConnectedSocket, WsException } from '@nestjs/websockets';
import { WsAdapter } from '@nestjs/platform-ws';
import { map } from 'rxjs';

const require = createRequire(import.meta.url);
const WebSocket = require('ws');

const log = [];
const record = (name) => log.push(name);

// Decorators are applied by hand: the fixture is plain JavaScript.
const guard = (name, fn) => {
  class G {
    canActivate(ctx) {
      record(name);
      return fn ? fn(ctx) : true;
    }
  }
  Object.defineProperty(G, 'name', { value: name });
  Injectable()(G);
  return G;
};
const filter = (name, catchAll = true) => {
  class F {
    catch(exception, host) {
      record(name);
      const client = host.switchToWs().getClient();
      client.send(JSON.stringify({ event: 'exception', data: { by: name } }));
    }
  }
  Object.defineProperty(F, 'name', { value: name });
  Catch()(F);
  return F;
};
const noCause = (frame) => {
  if (frame && frame.data && typeof frame.data === 'object') delete frame.data.cause;
  return frame;
};

class TrimPipe {
  transform(value) {
    record('TrimPipe');
    return typeof value === 'string' ? value.trim() : value;
  }
}
class WrapInterceptor {
  intercept(ctx, next) {
    record('WrapInterceptor');
    return next.handle().pipe(map((r) => (r && r.event ? { event: r.event, data: { wrapped: r.data } } : r)));
  }
}

function makeGateway({ gatewayGuards = [], gatewayFilters = [], methodFilters = [], methodGuards = [] }) {
  class Gw {
    join(text) {
      record('handler');
      return { event: 'joined', data: text };
    }
    fail(kind) {
      record('handler');
      if (kind === 'ws-string') throw new WsException('boom');
      if (kind === 'ws-object') throw new WsException({ code: 'ROOM_FULL' });
      throw new Error('internal');
    }
    failFiltered(kind) {
      record('handler');
      throw new WsException('boom');
    }
  }
  const proto = Gw.prototype;
  const method = (name, event, params) => {
    const d = Object.getOwnPropertyDescriptor(proto, name);
    SubscribeMessage(event)(proto, name, d);
    params.forEach((p, i) => p(proto, name, i));
  };
  method('join', 'join', [MessageBody()]);
  method('fail', 'fail', [MessageBody()]);
  method('failFiltered', 'failFiltered', [MessageBody()]);
  if (methodFilters.length) UseFilters(...methodFilters)(proto, 'failFiltered', Object.getOwnPropertyDescriptor(proto, 'failFiltered'));
  if (methodGuards.length) UseGuards(...methodGuards)(proto, 'join', Object.getOwnPropertyDescriptor(proto, 'join'));
  if (gatewayGuards.length) UseGuards(...gatewayGuards)(Gw);
  if (gatewayFilters.length) UseFilters(...gatewayFilters)(Gw);
  WebSocketGateway({ path: '/' })(Gw);
  Injectable()(Gw);
  return Gw;
}

async function run(name, { providers = [], global = {}, gateway = {}, messages }) {
  log.length = 0;
  const Gw = makeGateway(gateway);
  class AppModule {}
  Module({ providers: [Gw, ...providers] })(AppModule);
  const app = await NestFactory.create(AppModule, { logger: false });
  app.useWebSocketAdapter(new WsAdapter(app));
  if (global.guards) app.useGlobalGuards(...global.guards.map((G) => new G()));
  if (global.pipes) app.useGlobalPipes(...global.pipes.map((P) => new P()));
  if (global.interceptors) app.useGlobalInterceptors(...global.interceptors.map((I) => new I()));
  if (global.filters) app.useGlobalFilters(...global.filters.map((F) => new F()));
  await app.listen(0, '127.0.0.1');
  const port = app.getHttpServer().address().port;
  const results = [];
  for (const message of messages) {
    log.length = 0;
    const ws = new WebSocket(`ws://127.0.0.1:${port}/`);
    await new Promise((resolve, reject) => {
      ws.once('open', resolve);
      ws.once('error', reject);
    });
    const frames = [];
    ws.on('message', (raw) => frames.push(noCause(JSON.parse(raw.toString()))));
    ws.send(JSON.stringify(message));
    await new Promise((r) => setTimeout(r, 250));
    ws.close();
    results.push({ message, frames, calls: [...log] });
  }
  await app.close();
  return [name, results];
}

const Deny = guard('Deny', () => false);
const A = guard('A');
const B = guard('B');
const WsOnly = guard('WsOnly', (ctx) => ctx.getType() === 'ws');
const msgJoin = { event: 'join', data: '  lobby  ' };
const msgs = {
  join: msgJoin,
  wsString: { event: 'fail', data: 'ws-string' },
  wsObject: { event: 'fail', data: 'ws-object' },
};

const scenarios = Object.fromEntries(
  await (async () => {
    const out = [];
    out.push(await run('global_guard_use_global', { global: { guards: [Deny] }, messages: [msgJoin] }));
    out.push(await run('global_guard_provider', { providers: [{ provide: APP_GUARD, useClass: Deny }], messages: [msgJoin] }));
    out.push(await run('global_guard_before_gateway_guard', { global: { guards: [A] }, gateway: { gatewayGuards: [B] }, messages: [msgJoin] }));
    out.push(await run('global_guard_ws_only', { global: { guards: [WsOnly] }, messages: [msgJoin] }));
    out.push(
      await run('global_pipe_and_interceptor', {
        global: { pipes: [TrimPipe], interceptors: [WrapInterceptor] },
        messages: [msgJoin],
      }),
    );
    out.push(
      await run('global_pipe_provider', {
        providers: [{ provide: APP_PIPE, useClass: TrimPipe }, { provide: APP_INTERCEPTOR, useClass: WrapInterceptor }],
        messages: [msgJoin],
      }),
    );
    const G1 = filter('GlobalFilter');
    const G2 = filter('ProviderFilter');
    out.push(await run('global_filter_use_global', { global: { filters: [G1] }, messages: [msgs.wsString, msgs.wsObject] }));
    out.push(await run('global_filter_provider', { providers: [{ provide: APP_FILTER, useClass: G2 }], messages: [msgs.wsString] }));
    out.push(await run('builtin_frames', { messages: [msgs.wsString, msgs.wsObject, { event: 'fail', data: 'other' }] }));
    const GwF = filter('GatewayFilter');
    const MF = filter('MethodFilter');
    out.push(
      await run('method_filter_wins', {
        gateway: { gatewayFilters: [GwF], methodFilters: [MF] },
        messages: [{ event: 'failFiltered', data: 'x' }],
      }),
    );
    out.push(
      await run('gateway_filter_catches_guard_refusal', {
        gateway: { gatewayFilters: [filter('GatewayFilter')], methodGuards: [Deny] },
        messages: [msgJoin],
      }),
    );
    out.push(
      await run('filters_in_one_use_filters', {
        gateway: { methodFilters: [filter('First'), filter('Second')] },
        messages: [{ event: 'failFiltered', data: 'x' }],
      }),
    );
    return out;
  })(),
);

writeFileSync(new URL('./ws_gateways.json', import.meta.url), JSON.stringify(scenarios, null, 2) + '\n');
console.log('recorded', Object.keys(scenarios).length, 'scenarios');
process.exit(0);
