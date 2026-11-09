// Records what the `compression` middleware does on the original stack (platform-express with
// `app.use(compression())`) into compression.json: the headers and the decoded body of every route
// of the replay for several `Accept-Encoding` values, the encoding negotiation and the compressible
// check for a table of inputs. The Rust tests replay this file (crates/corvis/tests/compression.rs
// and the unit tests of src/http/compression/).
// Usage: npm install && node generate.mjs   (the servers bind to 127.0.0.1 only)
import 'reflect-metadata';
import http from 'node:http';
import zlib from 'node:zlib';
import crypto from 'node:crypto';
import os from 'node:os';
import path from 'node:path';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { Controller, Get, Head, HttpCode, Module, Res, Sse } from '@nestjs/common';
import { NestFactory } from '@nestjs/core';
import { interval, map } from 'rxjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);
const version = (name) => JSON.parse(readFileSync(path.join(here, 'node_modules', name, 'package.json'), 'utf8')).version;
const compression = require('compression');
const Negotiator = require('negotiator');
const compressible = require('compressible');

// ---- Content: the Rust test builds the same bytes ----
const FOX = 'The quick brown fox jumps over the lazy dog. ';
const text = (n) => FOX.repeat(Math.ceil(n / FOX.length)).slice(0, n);
const bin = (n) => Buffer.from(Array.from({ length: n }, (_, i) => (i * 31 + 7) % 251));

// ---- Routes: name -> [method, path, handler(res) or return value] ----
class C {}
const routes = {
  // JSON through `res.json`: `{"d":"..."}` is the text length + 8 bytes.
  'json-small': { get: () => ({ d: text(92) }) },
  'json-1023': { get: () => ({ d: text(1015) }) },
  'json-1024': { get: () => ({ d: text(1016) }) },
  'json-large': { get: () => ({ d: text(10232) }) },
  'html-large': { get: () => text(10240) },
  'text-large': { res: (res) => res.type('text/plain').send(text(10240)) },
  png: { res: (res) => res.type('image/png').send(bin(10240)) },
  octet: { res: (res) => res.type('application/octet-stream').send(bin(10240)) },
  'no-transform': { res: (res) => res.set('Cache-Control', 'private, no-transform').json({ d: text(10232) }) },
  encoded: { res: (res) => res.set('Content-Encoding', 'gzip').type('json').send(zlib.gzipSync(text(10240))) },
  'vary-origin': { res: (res) => res.set('Vary', 'Origin').json({ d: text(10232) }) },
  'vary-star': { res: (res) => res.set('Vary', '*').json({ d: text(10232) }) },
  'stream-small': {
    res: (res) => {
      res.type('text/plain');
      res.write(text(40));
      res.write(text(30));
      res.end(text(30));
    },
  },
  'stream-large': {
    res: (res) => {
      res.type('text/plain');
      for (let i = 0; i < 3; i++) res.write(text(4096));
      res.end();
    },
  },
  'no-content': { code: 204, get: () => undefined },
};
for (const [name, r] of Object.entries(routes)) {
  const handler = r.res
    ? function (res) {
        r.res(res);
      }
    : r.get;
  Object.defineProperty(C.prototype, name, { value: handler, writable: true, configurable: true });
  const desc = Object.getOwnPropertyDescriptor(C.prototype, name);
  if (r.res) Res()(C.prototype, name, 0);
  if (r.code) HttpCode(r.code)(C.prototype, name, desc);
  Get(name)(C.prototype, name, desc);
  if (name === 'json-large') {
    // HEAD is routed to the GET handler by Express; `Head` is only needed for explicit routes.
  }
}
void Head;
C.prototype.sse = function () {
  return interval(20).pipe(map((i) => ({ data: `event ${i}` })));
};
Sse('sse')(C.prototype, 'sse', Object.getOwnPropertyDescriptor(C.prototype, 'sse'));
Controller('')(C);

// ---- Requests ----
const AE = [null, 'gzip', 'deflate', 'br', 'gzip, deflate, br', 'br;q=0.5, gzip', 'identity', '*', 'identity;q=0, gzip;q=0'];

function decode(encoding, buf) {
  if (encoding === 'gzip') return zlib.gunzipSync(buf);
  if (encoding === 'deflate') return zlib.inflateSync(buf);
  if (encoding === 'br') return zlib.brotliDecompressSync(buf);
  return buf;
}

const sha = (b) => crypto.createHash('sha256').update(b).digest('hex');

function call(port, { method, url, headers, partial }) {
  return new Promise((resolve, reject) => {
    const req = http.request({ host: '127.0.0.1', port, method, path: url, headers, agent: false }, (res) => {
      const chunks = [];
      let done = false;
      const finish = (raw) => {
        if (done) return;
        done = true;
        resolve({ res, raw });
      };
      res.on('data', (c) => {
        chunks.push(c);
        if (partial) {
          finish(Buffer.concat(chunks));
          req.destroy();
        }
      });
      res.on('end', () => finish(Buffer.concat(chunks)));
      res.on('close', () => finish(Buffer.concat(chunks)));
      res.on('error', () => finish(Buffer.concat(chunks)));
    });
    req.on('error', (e) => {
      if (!partial) reject(e);
    });
    req.end();
  });
}

const one = (h) => (h === undefined ? null : String(h));

async function record(app, port, c) {
  const headers = { ...(c.headers || {}) };
  if (c.ae !== null && c.ae !== undefined) headers['Accept-Encoding'] = c.ae;
  const { res, raw } = await call(port, { method: c.method, url: c.url, headers, partial: c.headersOnly });
  const h = res.headers;
  const encoding = h['content-encoding'] ?? null;
  let body = null;
  if (!c.headersOnly && c.method !== 'HEAD' && res.statusCode !== 304 && res.statusCode !== 204) {
    body = c.rawBody ? zlib.gunzipSync(raw) : decode(encoding, raw);
  }
  return {
    app,
    name: c.name,
    method: c.method,
    url: c.url,
    headers: c.headers || {},
    ae: c.ae ?? null,
    headersOnly: !!c.headersOnly,
    rawBody: !!c.rawBody,
    etagExact: c.etagExact !== false,
    divergence: c.divergence || null,
    status: res.statusCode,
    contentEncoding: encoding,
    vary: one(h.vary),
    contentType: one(h['content-type']),
    contentLengthPresent: h['content-length'] !== undefined,
    contentLength: h['content-length'] !== undefined ? Number(h['content-length']) : null,
    contentRange: one(h['content-range']),
    acceptRanges: one(h['accept-ranges']),
    cacheControl: one(h['cache-control']),
    etag: one(h.etag),
    bodyLength: body ? body.length : null,
    bodySha256: body ? sha(body) : null,
  };
}

const routeNames = [...Object.keys(routes), 'sse'];
const staticNames = ['big.txt', 'small.txt', 'big.json'];

function cases(threshold0) {
  const out = [];
  const aes = threshold0 ? ['gzip', null] : AE;
  for (const name of routeNames) {
    for (const ae of aes) {
      out.push({
        name,
        method: 'GET',
        url: `/${name}`,
        ae,
        headersOnly: name === 'sse',
        rawBody: name === 'encoded',
        etagExact: name !== 'encoded',
      });
    }
  }
  for (const f of staticNames) {
    for (const ae of aes) {
      out.push({ name: `static:${f}`, method: 'GET', url: `/static/${f}`, ae, etagExact: false });
    }
  }
  if (!threshold0) {
    out.push({ name: 'json-large', method: 'HEAD', url: '/json-large', ae: 'gzip' });
    out.push({ name: 'json-large', method: 'HEAD', url: '/json-large', ae: null });
    out.push({ name: 'json-small', method: 'HEAD', url: '/json-small', ae: 'gzip' });
    out.push({ name: 'static:big.txt', method: 'HEAD', url: '/static/big.txt', ae: 'gzip', etagExact: false });
    // Compression of a 206 is a documented divergence of Corvis (Content-Range responses are not compressed).
    out.push({
      name: 'static:big.txt',
      method: 'GET',
      url: '/static/big.txt',
      ae: 'gzip',
      headers: { Range: 'bytes=0-49999' },
      etagExact: false,
      divergence: 'range',
    });
    out.push({
      name: 'static:big.txt',
      method: 'GET',
      url: '/static/big.txt',
      ae: null,
      headers: { Range: 'bytes=0-49999' },
      etagExact: false,
    });
    out.push({
      name: 'static:big.txt',
      method: 'GET',
      url: '/static/big.txt',
      ae: 'gzip',
      headers: { Range: 'bytes=100000-200000' },
      etagExact: false,
      divergence: 'range',
    });
  }
  return out;
}

async function runApp(label, options) {
  const dir = mkdtempSync(path.join(os.tmpdir(), 'corvis-compression-'));
  writeFileSync(path.join(dir, 'big.txt'), text(102400));
  writeFileSync(path.join(dir, 'small.txt'), text(100));
  writeFileSync(path.join(dir, 'big.json'), JSON.stringify({ d: text(51192) }));
  class M {}
  Module({ controllers: [C] })(M);
  const app = await NestFactory.create(M, { logger: false });
  app.use(compression(options));
  app.useStaticAssets(dir, { prefix: '/static/' });
  await app.listen(0, '127.0.0.1');
  const port = app.getHttpServer().address().port;
  const list = cases(label === 'threshold0');
  const results = [];
  for (const c of list) results.push(await record(label, port, c));
  // Conditional GET: the ETag the plain response carries, then a 304 with each encoding.
  if (label === 'default') {
    for (const url of ['/json-large', '/html-large']) {
      const first = await record(label, port, { name: url.slice(1), method: 'GET', url, ae: null });
      for (const ae of ['gzip', null]) {
        results.push(
          await record(label, port, {
            name: url.slice(1),
            method: 'GET',
            url,
            ae,
            headers: { 'If-None-Match': first.etag },
          }),
        );
      }
    }
  }
  await app.close();
  rmSync(dir, { recursive: true, force: true });
  return results;
}

// ---- Library tables ----
const SUPPORTED = ['br', 'gzip', 'deflate', 'identity'];
const PREFERRED = ['br', 'gzip'];
const acceptEncodingValues = [
  null,
  '',
  '*',
  'identity',
  'gzip',
  'GZIP',
  'Gzip, DEFLATE',
  'gzip;q=0',
  'gzip;q=0.5, br;q=0.5',
  'gzip, deflate, br',
  'br;q=0.5, gzip',
  'deflate',
  'identity;q=0, gzip;q=0',
  'gzip;q=0.001',
  'gzip;q=0.0001',
  'gzip;q=0.123, br;q=0.122',
  'gzip;q=abc',
  'gzip;q=',
  'gzip;q',
  'gzip;q=1.5',
  'gzip;q=-1',
  'gzip;q=.5',
  'gzip;q=1e-1',
  'gzip;q=0.5abc',
  'gzip;q=Infinity',
  'gzip;q=NaN',
  'gzip; q=0.5',
  'gzip ;q=0.5',
  'gzip;q= 0.5',
  'gzip;Q=0.5',
  'gzip;q=0.5;q=1',
  'gzip;level=5;q=0.3, br',
  'x-gzip',
  'compress',
  'compress, gzip',
  'br;q=0, *',
  '*;q=0.5, gzip',
  'gzip;q=0.5, *',
  '*;q=0, gzip',
  '*;q=0',
  'identity;q=0',
  'identity;q=0.5, gzip;q=0.4',
  'gzip, identity;q=0.5',
  'q=1',
  ',gzip',
  'gzip,,deflate',
  ';gzip',
  'gzip;;q=0.5',
  ' gzip ',
  '\tgzip',
  'deflate, gzip',
  'deflate;q=0.9, gzip;q=0.9',
  'BR',
  'Br;q=0.9, gzip;q=0.8',
  'br, gzip;q=0',
  '*, gzip;q=0',
  'gzip;q=0, deflate;q=0, br;q=0',
  'gzip;q=0.5, deflate;q=0.5, br;q=0.5',
  'deflate;q=1, gzip;q=1, br;q=1, identity;q=1',
  'identity, *;q=0',
  'br;q=0.0, gzip;q=0.00',
  'gzip;q=1.0',
  'a'.repeat(50),
];
const negotiation = acceptEncodingValues.map((v) => {
  const headers = v === null ? {} : { 'accept-encoding': v };
  const result = new Negotiator({ headers }).encodings(SUPPORTED, PREFERRED)[0] ?? null;
  return { acceptEncoding: v, result };
});

const types = [
  null,
  '',
  'application/json',
  'application/json; charset=utf-8',
  'Application/JSON',
  '  application/json ; charset=utf-8',
  'application/octet-stream',
  'application/x-ndjson',
  'application/vnd.api+json',
  'application/vnd.example+xml',
  'application/vnd.example+text',
  'application/vnd.example+yaml',
  'application/foo+JSON',
  'application/pdf',
  'application/zip',
  'application/gzip',
  'application/javascript',
  'application/xml',
  'application/wasm',
  'application/x-www-form-urlencoded',
  'application/problem+json',
  'text/plain',
  'text/html; charset=utf-8',
  'TEXT/CSS',
  'text/event-stream',
  'text/x-unknown',
  'text/',
  'text',
  'image/png',
  'image/jpeg',
  'image/svg+xml',
  'image/x-icon',
  'image/bmp',
  'image/vnd.microsoft.icon',
  'audio/mpeg',
  'video/mp4',
  'font/woff2',
  'font/ttf',
  'multipart/form-data; boundary=abc',
  'x-unknown/thing',
  'unknown',
  'constructor',
  'toString',
  'application/json;charset=utf-8',
  'application/json\t;x=y',
  ';application/json',
];
const compressibleTable = types.map((t) => {
  const r = compressible(t);
  return { type: t, result: r === undefined ? null : r };
});

const out = {
  versions: {
    compression: version('compression'),
    negotiator: version('negotiator'),
    compressible: version('compressible'),
    'mime-db': version('mime-db'),
    core: version('@nestjs/core'),
    express: version('express'),
    node: process.version,
  },
  negotiation,
  compressible: compressibleTable,
  cases: [...(await runApp('default', {})), ...(await runApp('threshold0', { threshold: 0 }))],
};
writeFileSync(path.join(here, 'compression.json'), JSON.stringify(out, null, 2) + '\n');
console.log('recorded', out.cases.length, 'cases,', negotiation.length, 'negotiations,', compressibleTable.length, 'types');
process.exit(0);
