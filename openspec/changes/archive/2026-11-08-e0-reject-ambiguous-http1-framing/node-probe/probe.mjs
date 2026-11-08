// Raw-socket probe of HTTP/1 framing. Superset of the probe of e0-decide-compatible-vs-strict-mode.
// Binds and connects to 127.0.0.1 only.
//
//   node probe.mjs                         probe Node's own `http` module (echo server started here)
//   node probe.mjs --target 3000 --path /x probe a server already listening on 127.0.0.1:3000
//   node probe.mjs --json                  print the cases and statuses as JSON (fixture for the Rust test)
//
// Each case is sent on a fresh connection. Most cases are followed by a pipelined
// `GET <path>` with `Connection: close`, so the output shows whether the server went on
// reading the connection after the first request: one status means it did not.
import http from 'node:http';
import net from 'node:net';

const args = process.argv.slice(2);
const opt = (name) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : undefined;
};
const target = opt('--target');
const path = opt('--path') || '/';
const json = args.includes('--json');

const H = 'Host: x\r\n';
const next = `GET ${path} HTTP/1.1\r\n${H}Connection: close\r\n\r\n`;
const chunked = `POST ${path} HTTP/1.1\r\n${H}Transfer-Encoding: chunked\r\n\r\n`;

// [id, label, bytes, followed by the pipelined request]
const cases = [
  ['ok-get', 'valid GET', `GET ${path} HTTP/1.1\r\n${H}\r\n`, true],
  ['bare-lf-head', 'bare LF request line and headers', `GET ${path} HTTP/1.1\nHost: x\n\n`, true],
  ['bare-lf-request-line', 'bare LF after the request line only', `GET ${path} HTTP/1.1\n${H}\r\n`, true],
  ['crlf-lf-head-end', 'header block ended by CRLF LF', `GET ${path} HTTP/1.1\r\n${H}\n`, true],
  ['lf-crlf-head-end', 'last header ended by LF, then CRLF', `GET ${path} HTTP/1.1\r\nHost: x\n\r\n`, true],
  ['bare-cr-header', 'bare CR inside a header line', `GET ${path} HTTP/1.1\r\nHost: x\rX: y\r\n\r\n`, true],
  ['leading-crlf', 'empty line (CRLF) before the request line', `\r\nGET ${path} HTTP/1.1\r\n${H}\r\n`, true],
  ['leading-lf', 'empty line (bare LF) before the request line', `\nGET ${path} HTTP/1.1\r\n${H}\r\n`, true],
  ['obs-fold', 'obsolete line folding in a header', `GET ${path} HTTP/1.1\r\n${H}X-A: b\r\n c\r\n\r\n`, true],
  ['chunked-ok', 'valid chunked body', chunked + '2\r\nhi\r\n0\r\n\r\n', true],
  ['chunk-leading-zeros', 'chunk size with leading zeros (26 digits)', chunked + '00000000000000000000000002\r\nhi\r\n0\r\n\r\n', true],
  ['chunk-overflow', 'chunk size over 64 bits', chunked + '10000000000000002\r\nhi\r\n0\r\n\r\n', true],
  ['chunk-size-space', 'space after chunk size', chunked + '2 \r\nhi\r\n0\r\n\r\n', true],
  ['chunk-size-tab', 'tab after chunk size', chunked + '2\t\r\nhi\r\n0\r\n\r\n', true],
  ['chunk-size-space-ext', 'space before chunk extension', chunked + '2 ;a=b\r\nhi\r\n0\r\n\r\n', true],
  ['chunk-ext-ok', 'chunk extension without whitespace', chunked + '2;a=b\r\nhi\r\n0\r\n\r\n', true],
  ['chunk-ext-lf', 'chunk extension containing LF', chunked + '2;a=b\nc\r\nhi\r\n0\r\n\r\n', true],
  ['chunked-bare-lf', 'bare LF in the chunked framing', chunked + '2\nhi\n0\n\n', true],
  ['trailer-ok', 'valid trailer field', chunked + '2\r\nhi\r\n0\r\nX-A: b\r\n\r\n', true],
  ['trailer-bare-lf', 'bare LF inside a trailer line (two fields)', chunked + '2\r\nhi\r\n0\r\nX-A: b\nX-C: d\r\n\r\n', true],
  ['trailer-bare-lf-end', 'bare LF LF inside the trailer section', chunked + '2\r\nhi\r\n0\r\nX-A: b\n\nGARBAGE\r\n\r\n', true],
  ['cl-then-te', 'Content-Length, then Transfer-Encoding: chunked', `POST ${path} HTTP/1.1\r\n${H}Content-Length: 4\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nhi\r\n0\r\n\r\n`, true],
  ['te-then-cl', 'Transfer-Encoding: chunked, then Content-Length', `POST ${path} HTTP/1.1\r\n${H}Transfer-Encoding: chunked\r\nContent-Length: 4\r\n\r\n2\r\nhi\r\n0\r\n\r\n`, true],
  ['dup-cl-identical', 'two identical Content-Length headers', `POST ${path} HTTP/1.1\r\n${H}Content-Length: 2\r\nContent-Length: 2\r\n\r\nhi`, true],
  ['te-chunked-twice', 'Transfer-Encoding: chunked, chunked', `POST ${path} HTTP/1.1\r\n${H}Transfer-Encoding: chunked, chunked\r\n\r\n2\r\nhi\r\n0\r\n\r\n`, true],
  ['http10-te', 'HTTP/1.0 request with Transfer-Encoding: chunked', `POST ${path} HTTP/1.0\r\n${H}Transfer-Encoding: chunked\r\n\r\n2\r\nhi\r\n0\r\n\r\n`, false],
  ['pipelined-valid-then-bare-lf', 'valid GET, then a pipelined GET with bare LF', `GET ${path} HTTP/1.1\r\n${H}\r\nGET ${path} HTTP/1.1\nHost: x\n\n`, true],
];

function send(port, payload) {
  return new Promise((resolve) => {
    const s = net.connect(port, '127.0.0.1');
    let out = '';
    const done = () => resolve(out.match(/HTTP\/1\.[01] \d{3}/g) || []);
    const t = setTimeout(() => {
      s.destroy();
      done();
    }, 1500);
    s.on('data', (d) => {
      out += d.toString('latin1');
    });
    s.on('close', () => {
      clearTimeout(t);
      done();
    });
    s.on('error', () => {});
    s.write(Buffer.from(payload, 'latin1'));
  });
}

async function run(port, header) {
  const results = [];
  for (const [id, label, bytes, pipelined] of cases) {
    const payload = pipelined ? bytes + next : bytes;
    const statuses = (await send(port, payload)).map((l) => Number(l.slice(9)));
    results.push({ id, label, request: payload, pipelined, statuses });
  }
  if (json) {
    console.log(JSON.stringify({ server: header, path, cases: results }, null, 2));
  } else {
    console.log(header);
    for (const r of results) console.log(`${r.id}: ${r.label} => ${r.statuses.join(' | ') || 'closed, no response'}`);
  }
}

if (target) {
  await run(Number(target), `127.0.0.1:${target} ${path}`);
} else {
  const server = http.createServer((req, res) => {
    const chunks = [];
    req.on('data', (c) => chunks.push(c));
    req.on('end', () => {
      res.writeHead(200, { 'content-type': 'text/plain' });
      res.end(Buffer.concat(chunks));
    });
  });
  server.listen(0, '127.0.0.1', async () => {
    await run(server.address().port, `node ${process.version} llhttp ${process.versions.llhttp}`);
    server.close();
    server.closeAllConnections();
  });
}
