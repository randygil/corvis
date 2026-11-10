// Node reference server in Engine.IO mode: `engine.io` attached to a plain HTTP server, like Corvis' Engine.IO server.
//
//   node reference/engine-server.mjs [--suite]
//
// Binds to 127.0.0.1 on a port chosen by the operating system and prints `{"address":"127.0.0.1","port":N}` as its
// first stdout line. Exits on SIGTERM and when stdin closes.
//
// The same application exists on the Corvis side (crates/corvis/tests/engine_support/harness_app.rs): the presets
// below are mirrored there, option for option, because the scenarios pick a preset by its path.
//
//   preset   path        what is different
//   default  /socket.io/ Node's defaults
//   fast     /fast/      pingInterval 300 ms, pingTimeout 200 ms
//   cors     /cors/      cors: origin https://app.example
//   burst    /burst/     sends 100 messages as soon as a session opens
//   small    /small/     maxHttpBufferSize 1000
//   cap      /cap/       Node defaults (Corvis caps this one at 2 sessions; Node has no cap, see expected-differences.json)
//   ipcap    /ipcap/     Node defaults (Corvis allows 2 sessions per address here; Node has no such cap)
//   first    /first/     Node defaults (Corvis closes a polling session that makes no second request within 400 ms)
//
// Every session echoes what it receives (text and binary), and understands these text messages:
//   burst        queues three messages, one of them binary
//   flood:<n>    sends <n> bytes as messages of 100 000 bytes
//
// A loopback-only control route answers `GET /harness/close-reason/<sid>` with the reason the session was closed
// with, or `open` while it is not closed.
//
// `--suite` serves the options the upstream `engine.io-protocol` suite requires (ping 300/200, cors, echo) at
// `/engine.io/` instead of the presets.
import { createServer } from "node:http";
import { attach } from "engine.io";

const PRESETS = [
  { name: "default", path: "/socket.io/", options: {} },
  { name: "fast", path: "/fast/", options: { pingInterval: 300, pingTimeout: 200 } },
  { name: "cors", path: "/cors/", options: { cors: { origin: "https://app.example" } } },
  { name: "burst", path: "/burst/", options: {}, burst: true },
  { name: "small", path: "/small/", options: { maxHttpBufferSize: 1000 } },
  { name: "cap", path: "/cap/", options: {} },
  { name: "ipcap", path: "/ipcap/", options: {} },
  { name: "first", path: "/first/", options: {} },
];

const suite = process.argv.includes("--suite");
const reasons = new Map();

const httpServer = createServer((req, res) => {
  const match = /^\/harness\/close-reason\/([^/?]+)$/.exec(req.url ?? "");
  if (req.method === "GET" && match && req.socket.remoteAddress === "127.0.0.1") {
    res.writeHead(200, { "content-type": "text/plain" });
    res.end(reasons.get(match[1]) ?? "open");
    return;
  }
  res.writeHead(404, { "content-type": "text/plain" });
  res.end("not found");
});

function behave(socket, burst) {
  socket.on("close", (reason) => reasons.set(socket.id, reason));
  socket.on("message", (data) => {
    if (typeof data === "string") {
      if (data === "burst") {
        socket.send("one");
        socket.send(Buffer.from([1, 2, 3]));
        socket.send("three");
        return;
      }
      const flood = /^flood:(\d+)$/.exec(data);
      if (flood) {
        let left = Number(flood[1]);
        while (left > 0) {
          const n = Math.min(left, 100_000);
          socket.send("x".repeat(n));
          left -= n;
        }
        return;
      }
    }
    socket.send(data);
  });
  if (burst) {
    for (let i = 0; i < 100; i++) socket.send(`m${i}`);
  }
}

const servers = [];
if (suite) {
  const server = attach(httpServer, { path: "/engine.io/", pingInterval: 300, pingTimeout: 200, cors: { origin: "*" } });
  server.on("connection", (socket) => behave(socket, false));
  servers.push(server);
} else {
  for (const preset of PRESETS) {
    const server = attach(httpServer, { path: preset.path, ...preset.options });
    server.on("connection", (socket) => behave(socket, preset.burst === true));
    servers.push(server);
  }
}

function shutdown() {
  for (const server of servers) server.close();
  httpServer.close(() => process.exit(0));
  httpServer.closeAllConnections?.();
  setTimeout(() => process.exit(0), 500).unref();
}

process.on("SIGTERM", shutdown);
process.on("SIGINT", shutdown);
process.stdin.on("end", shutdown);
process.stdin.on("close", shutdown);
process.stdin.resume();

httpServer.listen(0, "127.0.0.1", () => {
  const { address, port } = httpServer.address();
  process.stdout.write(`${JSON.stringify({ address, port })}\n`);
});
