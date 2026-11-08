// Records what the original framework answers for each health scenario into terminus.json.
// Usage: npm install && node generate.mjs
import 'reflect-metadata';
import http from 'node:http';
import net from 'node:net';
import { writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { Controller, Get, Inject, Module } from '@nestjs/common';
import { NestFactory } from '@nestjs/core';
import { HttpModule } from '@nestjs/axios';
import {
  TerminusModule,
  HealthCheckService,
  HealthIndicatorService,
  HttpHealthIndicator,
  MemoryHealthIndicator,
} from '@nestjs/terminus';

const here = dirname(fileURLToPath(import.meta.url));

// Local servers the HTTP indicator talks to.
const server = http.createServer((req, res) => {
  if (req.url === '/ok') return res.writeHead(200).end('ok');
  if (req.url === '/error') return res.writeHead(500).end('boom');
  if (req.url === '/start') return res.writeHead(302, { location: '/ok' }).end();
  res.writeHead(404).end();
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const port = server.address().port;

const closed = net.createServer();
await new Promise((r) => closed.listen(0, '127.0.0.1', r));
const closedPort = closed.address().port;
await new Promise((r) => closed.close(r));

const base = `http://127.0.0.1:${port}`;

// scenario name -> (deps) => array of indicator promises
const scenarios = {
  ping_200: (d) => [() => d.http.pingCheck('api', `${base}/ok`)],
  ping_500: (d) => [() => d.http.pingCheck('api', `${base}/error`)],
  ping_302_then_200: (d) => [() => d.http.pingCheck('api', `${base}/start`)],
  ping_closed_port: (d) => [() => d.http.pingCheck('api', `http://127.0.0.1:${closedPort}/`)],
  ping_no_scheme: (d) => [() => d.http.pingCheck('api', `127.0.0.1:${closedPort}`)],
  rss_huge_threshold: (d) => [() => d.memory.checkRSS('memory_rss', Number.MAX_SAFE_INTEGER)],
  rss_tiny_threshold: (d) => [() => d.memory.checkRSS('memory_rss', 1)],
  mixed: (d) => [
    async () => d.indicator.check('db').up(),
    async () => d.indicator.check('redis').down({ message: 'down' }),
  ],
  throws_plain_error: (d) => [
    async () => d.indicator.check('db').up(),
    async () => {
      throw new Error('boom');
    },
  ],
};

let current = [];

class HealthController {
  constructor(health) {
    this.health = health;
  }
  check() {
    return this.health.check(current.map((f) => f));
  }
}
Inject(HealthCheckService)(HealthController, undefined, 0);
Controller('health')(HealthController);
Get()(HealthController.prototype, 'check', Object.getOwnPropertyDescriptor(HealthController.prototype, 'check'));

class AppModule {}
Module({ imports: [TerminusModule, HttpModule], controllers: [HealthController] })(AppModule);

const app = await NestFactory.create(AppModule, { logger: false, abortOnError: false });
await app.listen(0, '127.0.0.1');
const appPort = app.getHttpServer().address().port;

const deps = {
  http: await app.resolve(HttpHealthIndicator),
  memory: await app.resolve(MemoryHealthIndicator),
  indicator: await app.resolve(HealthIndicatorService),
};

const normalize = (text) =>
  text.replaceAll(`127.0.0.1:${port}`, '127.0.0.1:{port}').replaceAll(`127.0.0.1:${closedPort}`, '127.0.0.1:{closed}');

const out = {};
for (const [name, build] of Object.entries(scenarios)) {
  current = build(deps);
  const res = await fetch(`http://127.0.0.1:${appPort}/health`);
  const text = normalize(await res.text());
  out[name] = { status: res.status, body: JSON.parse(text) };
}

await app.close();
server.close();

const meta = {
  generatedWith: {
    '@nestjs/terminus': '11.1.1',
    '@nestjs/core': '11.2.1',
    axios: '1.13.2',
  },
  note: 'Port placeholders: {port} is the local HTTP server, {closed} a port with no listener.',
};
writeFileSync(join(here, 'terminus.json'), JSON.stringify({ meta, scenarios: out }, null, 2) + '\n');
console.log(JSON.stringify(out, null, 2));
