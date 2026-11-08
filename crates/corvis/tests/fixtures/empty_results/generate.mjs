// Records what the original framework answers for handlers that return null, undefined or nothing
// into empty_results.json. Plain JS: the decorators are applied as functions.
// Usage: npm install && node generate.mjs
import 'reflect-metadata';
import http from 'node:http';
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { Controller, Get, Post, HttpCode, Header, Module } from '@nestjs/common';
import { NestFactory } from '@nestjs/core';

const here = dirname(fileURLToPath(import.meta.url));

class C {
  nul() { return null; }
  undef() { return undefined; }
  void_() {}
  async asyncNul() { return null; }
  postNul() { return null; }
  obj() { return { a: 1 }; }
  str() { return 'hi'; }
  emptyStr() { return ''; }
  code204() { return null; }
  jsonHeader() { return null; }
  arrNull() { return [null]; }
}
const p = C.prototype;
const d = (n) => Object.getOwnPropertyDescriptor(p, n);
Get('null')(p, 'nul', d('nul'));
Get('undefined')(p, 'undef', d('undef'));
Get('void')(p, 'void_', d('void_'));
Get('async-null')(p, 'asyncNul', d('asyncNul'));
Post('null')(p, 'postNul', d('postNul'));
Get('obj')(p, 'obj', d('obj'));
Get('str')(p, 'str', d('str'));
Get('empty-str')(p, 'emptyStr', d('emptyStr'));
Get('null-204')(p, 'code204', d('code204'));
HttpCode(204)(p, 'code204', d('code204'));
Get('null-json-header')(p, 'jsonHeader', d('jsonHeader'));
Header('Content-Type', 'application/json')(p, 'jsonHeader', d('jsonHeader'));
Get('array-null')(p, 'arrNull', d('arrNull'));
Controller('r')(C);
class M {}
Module({ controllers: [C] })(M);

let port;
function call(method, path) {
  return new Promise((resolve, reject) => {
    const r = http.request({ host: '127.0.0.1', port, method, path }, (res) => {
      const chunks = [];
      res.on('data', (c) => chunks.push(c));
      res.on('end', () => {
        const headers = {};
        for (let i = 0; i < res.rawHeaders.length; i += 2) {
          const k = res.rawHeaders[i].toLowerCase();
          if (k === 'date' || k === 'connection' || k === 'keep-alive') continue;
          headers[k] = res.rawHeaders[i + 1];
        }
        resolve({ method, path, status: res.statusCode, headers, body: Buffer.concat(chunks).toString('utf8') });
      });
    });
    r.on('error', reject);
    r.end();
  });
}

const version = (name) =>
  JSON.parse(readFileSync(join(here, 'node_modules', name, 'package.json'), 'utf8')).version;

const app = await NestFactory.create(M, { logger: false });
await app.listen(0, '127.0.0.1');
port = app.getHttpServer().address().port;
const out = {
  versions: {
    core: version('@nestjs/core'),
    platformExpress: version('@nestjs/platform-express'),
    express: version('express'),
    node: process.version,
  },
  cases: [],
};
for (const [m, pth] of [
  ['GET', '/r/null'], ['GET', '/r/undefined'], ['GET', '/r/void'], ['GET', '/r/async-null'],
  ['POST', '/r/null'], ['HEAD', '/r/null'], ['GET', '/r/null-204'], ['GET', '/r/null-json-header'],
  ['GET', '/r/obj'], ['GET', '/r/str'], ['GET', '/r/empty-str'], ['GET', '/r/array-null'],
]) {
  out.cases.push(await call(m, pth));
}
await app.close();
writeFileSync(join(here, 'empty_results.json'), JSON.stringify(out, null, 2) + '\n');
