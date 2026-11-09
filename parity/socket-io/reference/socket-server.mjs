// Node reference server: a plain `socket.io` server with default options.
//
//   node reference/socket-server.mjs [--ack-delay <ms>]
//
// Binds to 127.0.0.1 on a port chosen by the operating system and prints `{"port":N}` as its first stdout line.
// Exits on SIGTERM and when stdin closes, so it dies with the process that started it (also on Windows).
import { createServer } from "node:http";
import { Server } from "socket.io";

function ackDelay() {
  const at = process.argv.indexOf("--ack-delay");
  if (at === -1) return 0;
  const ms = Number(process.argv[at + 1]);
  if (!Number.isInteger(ms) || ms < 0) {
    console.error("--ack-delay expects a number of milliseconds");
    process.exit(2);
  }
  return ms;
}

const delay = ackDelay();

const httpServer = createServer((req, res) => {
  res.writeHead(404, { "content-type": "text/plain" });
  res.end("not found");
});

const io = new Server(httpServer);

io.on("connection", (socket) => {
  socket.on("echo", (...args) => {
    const ack = typeof args[args.length - 1] === "function" ? args.pop() : null;
    if (!ack) return;
    if (delay > 0) setTimeout(() => ack(...args), delay);
    else ack(...args);
  });
});

function shutdown() {
  io.close(() => process.exit(0));
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
