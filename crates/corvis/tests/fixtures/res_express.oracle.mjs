// express 5 oracle for @Res() semantics (plain JS decorators); writes tests/fixtures/res_express.json.
// NM must point to a node_modules with @nestjs/common, @nestjs/core, @nestjs/platform-express 12 and reflect-metadata.
const NM = 'C:/Users/Usuario/Devel/corvis-tests/nest/houseplan/api/node_modules/';
await import('file:///' + NM + 'reflect-metadata/Reflect.js');
const common = await import('file:///' + NM + '@nestjs/common/index.js');
const core = await import('file:///' + NM + '@nestjs/core/index.js');
const { Controller, Get, Post, Res, HttpCode, Header, UnauthorizedException, Module, Query } = common;

class C {
  redirect(res) { res.setHeader('Set-Cookie', 'sid=1; Path=/; HttpOnly'); res.redirect(302, '/?auth=expired'); }
  redirectEnc(res) { res.redirect('/a b/ñ?x=<1>'); }
  async login(res) { res.setHeader('Set-Cookie', 'sid=2; Path=/; HttpOnly; SameSite=Lax; Max-Age=60'); }
  async loginFail(res) { res.setHeader('Set-Cookie', 'try=1'); throw new UnauthorizedException('Clave incorrecta'); }
  sendText(res) { res.status(201).send('hola'); }
  json(res) { res.json({ a: 1, b: [true, null] }); }
  end(res) { res.setHeader('Content-Type', 'text/plain; charset=windows-1252'); res.setHeader('Content-Disposition', 'attachment; filename="r.txt"'); res.end(Buffer.from([0x41, 0xd1])); }
  sendStatus(res) { res.sendStatus(418); }
  cookie(res) { res.cookie('a', 'b c', { httpOnly: true, sameSite: 'lax' }); res.clearCookie('old'); res.send('ok'); }
  pass(res) { res.status(202); return { ok: true }; }
  later(res) { setTimeout(() => res.json({ late: 1 }), 30); return 'ignored'; }
  attachment(res) { res.attachment('informe 2026.pdf'); res.send(Buffer.from('xyz')); }
  buffer(res) { res.send(Buffer.from('raw')); }
  handledThrow(res) { res.setHeader('X-Before', '1'); throw new UnauthorizedException(); }
}
const P = C.prototype;
const route = (dec, name, extra = []) => { const d = Object.getOwnPropertyDescriptor(P, name); for (const x of [dec, ...extra]) x(P, name, d); };
const res = (name, opts) => Res(opts)(P, name, 0);
route(Get('redirect'), 'redirect'); res('redirect');
route(Get('redirect-enc'), 'redirectEnc'); res('redirectEnc');
route(Post('login'), 'login', [HttpCode(204)]); res('login', { passthrough: true });
route(Post('login-fail'), 'loginFail', [HttpCode(204)]); res('loginFail', { passthrough: true });
route(Get('send-text'), 'sendText'); res('sendText');
route(Get('json'), 'json'); res('json');
route(Get('end'), 'end'); res('end');
route(Get('send-status'), 'sendStatus'); res('sendStatus');
route(Get('cookie'), 'cookie'); res('cookie');
route(Post('pass'), 'pass', [Header('X-A', '1'), Header('Content-Type', 'application/json')]); res('pass', { passthrough: true });
route(Get('later'), 'later'); res('later');
route(Get('attachment'), 'attachment'); res('attachment');
route(Get('buffer'), 'buffer'); res('buffer');
route(Get('handled-throw'), 'handledThrow', [Header('X-H', 'h')]); res('handledThrow');
Controller('res')(C);
class M {}
Module({ controllers: [C] })(M);

const app = await core.NestFactory.create(M, { logger: false });
await app.listen(3299, '127.0.0.1');
const base = 'http://127.0.0.1:3299/res/';
const cases = [
  ['GET', 'redirect', {}], ['GET', 'redirect', { accept: 'text/html' }], ['GET', 'redirect', { accept: 'application/json' }],
  ['HEAD', 'redirect', {}], ['GET', 'redirect-enc', {}],
  ['POST', 'login', {}], ['POST', 'login-fail', {}], ['GET', 'send-text', {}], ['GET', 'json', {}], ['GET', 'end', {}],
  ['GET', 'send-status', {}], ['GET', 'cookie', {}], ['POST', 'pass', {}], ['GET', 'later', {}], ['GET', 'attachment', {}],
  ['GET', 'buffer', {}], ['GET', 'handled-throw', {}],
];
const http = await import('node:http');
const call = (method, path, headers) => new Promise((resolve, reject) => {
  const rq = http.request(base + path, { method, headers }, (r) => {
    const chunks = [];
    r.on('data', (c) => chunks.push(c));
    r.on('end', () => {
      const h = { 'set-cookie': [] };
      for (let i = 0; i < r.rawHeaders.length; i += 2) {
        const k = r.rawHeaders[i].toLowerCase();
        const v = Buffer.from(r.rawHeaders[i + 1], 'latin1').toString('utf8');
        if (['date', 'connection', 'keep-alive'].includes(k)) continue;
        if (k === 'set-cookie') h[k].push(v); else h[k] = v;
      }
      resolve({ status: r.statusCode, h, body: Buffer.concat(chunks).toString('latin1') });
    });
  });
  rq.on('error', reject);
  rq.end();
});
const out = [];
for (const [method, path, headers] of cases) out.push({ method, path, headers, ...(await call(method, path, headers)) });
console.log(JSON.stringify(out, null, 1));
await app.close();
