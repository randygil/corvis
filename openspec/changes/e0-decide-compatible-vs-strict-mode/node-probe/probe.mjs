// Raw-socket probe of Node's `http` parser. Binds to 127.0.0.1 only.
// Usage: node probe.mjs   (prints one line per case: label => status line or "closed")
import http from 'node:http';
import net from 'node:net';

const server = http.createServer((req, res) => {
  const chunks = [];
  req.on('data', (c) => chunks.push(c));
  req.on('end', () => {
    res.writeHead(200, { 'content-type': 'text/plain', connection: 'close' });
    res.end(Buffer.concat(chunks));
  });
});

const head = 'POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n';
const cases = [
  ['bare LF request line and headers', 'GET / HTTP/1.1\nHost: x\nConnection: close\n\n'],
  ['header block ended by CRLF LF', 'GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\n'],
  ['bare CR inside a header line', 'GET / HTTP/1.1\r\nHost: x\rX: y\r\nConnection: close\r\n\r\n'],
  ['chunk size with leading zeros (26 digits)', head + '00000000000000000000000002\r\nhi\r\n0\r\n\r\n'],
  ['chunk size over 64 bits', head + '10000000000000002\r\nhi\r\n0\r\n\r\n'],
  ['space after chunk size', head + '2 \r\nhi\r\n0\r\n\r\n'],
  ['tab after chunk size', head + '2\t\r\nhi\r\n0\r\n\r\n'],
  ['space before chunk extension', head + '2 ;a=b\r\nhi\r\n0\r\n\r\n'],
  ['bare LF in chunked framing', head + '2\nhi\n0\n\n'],
  ['bare LF inside a trailer line', head + '2\r\nhi\r\n0\r\nX-A: b\nX-C: d\r\n\r\n'],
  [
    'Content-Length and Transfer-Encoding',
    'POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 4\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n2\r\nhi\r\n0\r\n\r\n',
  ],
  ['chunk extension containing LF', head + '2;a=b\nc\r\nhi\r\n0\r\n\r\n'],
];

function send(port, payload) {
  return new Promise((resolve) => {
    const s = net.connect(port, '127.0.0.1');
    let out = '';
    const first = () => (out ? out.split('\r\n')[0] : 'closed');
    const t = setTimeout(() => {
      s.destroy();
      resolve(out ? first() : 'timeout');
    }, 1500);
    s.on('data', (d) => {
      out += d.toString('latin1');
    });
    s.on('close', () => {
      clearTimeout(t);
      resolve(first());
    });
    s.on('error', () => {});
    s.write(payload);
  });
}

server.listen(0, '127.0.0.1', async () => {
  const { port } = server.address();
  console.log(`node ${process.version} llhttp ${process.versions.llhttp}`);
  for (const [label, payload] of cases) console.log(`${label} => ${await send(port, payload)}`);
  server.close();
  server.closeAllConnections();
});
