// Records which filter answers a failing request when several filters are declared at one level
// (one decorator, stacked decorators, class and method together, global). Output: http_filter_order.json.
// Plain JS: the decorators are applied as functions, the lower stacked decorator first.
// Usage: npm install && node generate.mjs   (the server binds to 127.0.0.1 only)
import 'reflect-metadata';
import http from 'node:http';
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import {
  Controller,
  Get,
  Module,
  UseFilters,
  Catch,
  ForbiddenException,
  ConflictException,
  NotFoundException,
  BadRequestException,
} from '@nestjs/common';
import { NestFactory } from '@nestjs/core';

const here = dirname(fileURLToPath(import.meta.url));
const version = (name) =>
  JSON.parse(readFileSync(join(here, 'node_modules', name, 'package.json'), 'utf8')).version;

const EXC = {
  forbidden: ForbiddenException,
  conflict: ConflictException,
  notfound: NotFoundException,
  bad: BadRequestException,
};

// A filter named `name` that catches the listed exception kinds (all of them when the list is empty).
const filter = (name, kinds = []) => {
  class F {
    catch(exception, host) {
      host.switchToHttp().getResponse().status(200).json({ by: name });
    }
  }
  Object.defineProperty(F, 'name', { value: name });
  Catch(...kinds.map((k) => EXC[k]))(F);
  return F;
};

// `classGroups` and `methodGroups` list the decorators top to bottom; they are applied bottom to top.
function makeController({ classGroups = [], methodGroups = [] }) {
  class C {
    forbidden() { throw new ForbiddenException(); }
    conflict() { throw new ConflictException(); }
    notfound() { throw new NotFoundException(); }
    bad() { throw new BadRequestException(); }
  }
  const p = C.prototype;
  for (const n of Object.keys(EXC)) {
    const d = Object.getOwnPropertyDescriptor(p, n);
    Get(n)(p, n, d);
    for (const g of [...methodGroups].reverse()) UseFilters(...g)(p, n, d);
  }
  for (const g of [...classGroups].reverse()) UseFilters(...g)(C);
  Controller('c')(C);
  return C;
}

const call = (port, path) =>
  new Promise((resolve, reject) => {
    http
      .get({ host: '127.0.0.1', port, path }, (res) => {
        const chunks = [];
        res.on('data', (c) => chunks.push(c));
        res.on('end', () => resolve({ status: res.statusCode, body: Buffer.concat(chunks).toString('utf8') }));
      })
      .on('error', reject);
  });

async function run(name, { global = [], ...opts }) {
  const C = makeController(opts);
  class M {}
  Module({ controllers: [C] })(M);
  const app = await NestFactory.create(M, { logger: false });
  if (global.length) app.useGlobalFilters(...global.map((F) => new F()));
  await app.listen(0, '127.0.0.1');
  const port = app.getHttpServer().address().port;
  const requests = [];
  for (const kind of Object.keys(EXC)) {
    const path = `/c/${kind}`;
    const r = await call(port, path);
    requests.push({ path, kind, status: r.status, body: JSON.parse(r.body) });
  }
  await app.close();
  return [name, requests];
}

const out = {
  versions: {
    core: version('@nestjs/core'),
    common: version('@nestjs/common'),
    node: process.version,
  },
  scenarios: {},
};
const add = ([name, requests]) => (out.scenarios[name] = requests);

add(await run('method_one_attribute', { methodGroups: [[filter('First'), filter('Second')]] }));
add(await run('class_one_attribute', { classGroups: [[filter('First'), filter('Second')]] }));
add(
  await run('method_stacked_attributes', {
    methodGroups: [
      [filter('A', ['forbidden', 'conflict']), filter('B', ['forbidden'])],
      [filter('C')],
    ],
  }),
);
add(
  await run('class_stacked_attributes', {
    classGroups: [
      [filter('A', ['forbidden', 'conflict']), filter('B', ['forbidden'])],
      [filter('C')],
    ],
  }),
);
add(
  await run('method_before_class', {
    classGroups: [[filter('C1', ['conflict', 'notfound']), filter('C2', ['forbidden', 'notfound'])]],
    methodGroups: [[filter('M1', ['forbidden']), filter('M2', ['conflict'])]],
  }),
);
add(
  await run('global_last', {
    global: [filter('G')],
    classGroups: [[filter('K', ['conflict'])]],
    methodGroups: [[filter('M', ['forbidden'])]],
  }),
);

writeFileSync(join(here, 'http_filter_order.json'), JSON.stringify(out, null, 2) + '\n');
console.log('recorded', Object.keys(out.scenarios).length, 'scenarios');
process.exit(0);
