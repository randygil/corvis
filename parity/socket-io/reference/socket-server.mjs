// Node reference server of the Socket.IO scenarios: a plain `socket.io` server with the application every scenario
// talks to. The Corvis side is the same application written with `IoServer` (crates/corvis/tests/engine_support/io_app.rs);
// change one and you change the other.
//
//   node reference/socket-server.mjs [--ack-delay <ms>] [--suite]
//
// Binds to 127.0.0.1 on a port chosen by the operating system and prints `{"address":...,"port":N}` as its first stdout
// line. Exits on SIGTERM and when stdin closes, so it dies with the process that started it (also on Windows).
//
// Servers (a scenario picks one with the `path` option of the client; each has its own Socket.IO server):
//   /sio/        namespaces `/`, `/admin` (middleware: `auth.token` must be "admin", data on refusal) and `/slow`
//                (middleware that takes one second) and `/denied` (middleware that always refuses, without data)
//   /sio-short/  the same, with `connectTimeout` 1000 ms
//
// Handlers of every namespace: echo, sum, emit-update, ask, ask-timeout, bin-emit, leave, kick, emit-reserved, and the
// room and broadcast commands (each takes one JSON argument and acknowledges with one JSON value): join, room-leave,
// rooms, broadcast, broadcast-ack, sockets-join, sockets-leave, disconnect-sockets, fetch-sockets. A broadcast is
// followed by the event `done` to every namespace of the server, which the scenarios wait on instead of sleeping.
// What happened on the server side is kept in a log, served to loopback clients by `GET /harness/log` and cleared by
// `POST /harness/reset`.
//
// With `--suite` it is the server the upstream `socket.io-protocol` suite requires instead: path `/socket.io/`, ping
// interval 300 ms, ping timeout 200 ms, connect timeout 1000 ms, CORS for any origin, `auth` emitted on connect,
// `message` answered with `message-back` and `message-with-ack` acknowledged, and the namespace `/custom`.
import { createServer } from "node:http";
import { Server } from "socket.io";

function numberArg(name, fallback) {
  const at = process.argv.indexOf(name);
  if (at === -1) return fallback;
  const ms = Number(process.argv[at + 1]);
  if (!Number.isInteger(ms) || ms < 0) {
    console.error(`${name} expects a number of milliseconds`);
    process.exit(2);
  }
  return ms;
}

const delay = numberArg("--ack-delay", 0);
const suite = process.argv.includes("--suite");
const log = [];

const httpServer = createServer((req, res) => {
  const loopback = ["127.0.0.1", "::1", "::ffff:127.0.0.1"].includes(req.socket.remoteAddress);
  if (req.url === "/harness/log" && req.method === "GET" && loopback) {
    res.writeHead(200, { "content-type": "application/json" });
    res.end(JSON.stringify(log));
    return;
  }
  if (req.url === "/harness/reset" && req.method === "POST" && loopback) {
    log.length = 0;
    res.writeHead(204);
    res.end();
    return;
  }
  res.writeHead(404, { "content-type": "text/plain" });
  res.end("not found");
});

const servers = [];

// The rooms of a socket with its own id replaced by `<id>` (ids are random), sorted.
function roomsOf(socket) {
  return [...socket.rooms].map((room) => (room === socket.id ? "<id>" : room)).sort();
}

function markDone(io) {
  for (const name of ["/", "/admin"]) io.of(name).emit("done");
}

// `emptyTo` is a `to` given an empty list: what an application gets when the list came from a client.
function selection(operator, { to = [], except = [], emptyTo = false }) {
  if (emptyTo) operator = operator.to([]);
  if (to.length > 0) operator = operator.to(to);
  if (except.length > 0) operator = operator.except(except);
  return operator;
}

function scenarioNamespace(io, name) {
  const nsp = io.of(name);
  if (name === "/admin") {
    nsp.use((socket, next) => {
      if (socket.handshake.auth.token !== "admin") {
        const error = new Error("not authorized");
        error.data = { content: "retry later" };
        return next(error);
      }
      next();
    });
  }
  if (name === "/denied") {
    nsp.use((socket, next) => next(new Error("not authorized")));
  }
  if (name === "/slow") {
    nsp.use((socket, next) => setTimeout(next, 1000));
  }
  nsp.on("connection", (socket) => {
    log.push({ event: "connection", nsp: name, auth: socket.handshake.auth });
    // The rooms the socket was in while it disconnected, and after.
    let roomsDuring = [];
    socket.on("disconnecting", () => {
      roomsDuring = roomsOf(socket);
      socket.to([...socket.rooms]).emit("left");
    });
    socket.on("disconnect", (reason) =>
      log.push({ event: "disconnect", nsp: name, reason, roomsDuring, roomsAfter: roomsOf(socket) }),
    );
    socket.on("echo", (...args) => {
      const ack = typeof args[args.length - 1] === "function" ? args.pop() : null;
      if (!ack) return;
      if (delay > 0) setTimeout(() => ack(...args), delay);
      else ack(...args);
    });
    socket.on("sum", (...args) => {
      const ack = args.pop();
      ack(args.reduce((a, b) => a + b, 0), "ok");
    });
    socket.on("emit-update", () => socket.emit("update", 1, "two", { three: 3 }));
    // The callback form: `emitWithAck` hands back only the first value of the client's answer.
    for (const [event, ms, result] of [
      ["ask", 5000, "ask-result"],
      ["ask-timeout", 200, "ask-timeout-result"],
    ]) {
      socket.on(event, () => {
        socket.timeout(ms).emit("question", (error, ...args) => {
          if (error) log.push({ event: result, nsp: name, ok: false, message: error.message });
          else log.push({ event: result, nsp: name, ok: true, args });
        });
      });
    }
    socket.on("bin-emit", (...args) => socket.emit("bin", ...args));
    socket.on("leave", () => socket.disconnect());
    socket.on("kick", () => socket.disconnect(true));
    socket.on("join", (arg, ack) => {
      socket.join(arg.rooms);
      ack({ ok: true, rooms: roomsOf(socket) });
    });
    socket.on("room-leave", (arg, ack) => {
      socket.leave(arg.room);
      ack({ rooms: roomsOf(socket) });
    });
    socket.on("rooms", (ack) => ack({ rooms: roomsOf(socket) }));
    socket.on("broadcast", (arg, ack) => {
      const { from = "server", event = "event", args = [] } = arg;
      selection(from === "socket" ? socket.broadcast : io.of(name), arg).emit(event, ...args);
      markDone(io);
      ack({ ok: true });
    });
    socket.on("broadcast-ack", async (arg, ack) => {
      // Node keeps the first value of each client's answer.
      const first = (responses) => responses.map((r) => JSON.stringify(r)).sort();
      // Without `timeout()` the flag is undefined, which is what Node fails on at once.
      const operator = io.of(name).timeout(arg.timeout);
      try {
        const responses = await selection(operator, arg).emitWithAck("question");
        ack({ ok: true, responses: first(responses) });
      } catch (error) {
        ack({ ok: false, message: error.message, responses: first(error.responses ?? []) });
      }
    });
    socket.on("sockets-join", (arg, ack) => {
      selection(io.of(name), arg).socketsJoin(arg.rooms);
      ack({ ok: true });
    });
    socket.on("sockets-leave", (arg, ack) => {
      selection(io.of(name), arg).socketsLeave(arg.rooms);
      ack({ ok: true });
    });
    socket.on("disconnect-sockets", (arg, ack) => {
      selection(io.of(name), arg).disconnectSockets(arg.close === true);
      ack({ ok: true });
    });
    socket.on("fetch-sockets", async (arg, ack) => {
      const sockets = await selection(io.of(name), arg).fetchSockets();
      ack({ sockets: sockets.map((s) => ({ id: s.id, rooms: [...s.rooms] })) });
    });
    socket.on("emit-reserved", () => {
      try {
        socket.emit("disconnect");
        log.push({ event: "emit-reserved", nsp: name, ok: true });
      } catch (error) {
        log.push({ event: "emit-reserved", nsp: name, ok: false, message: error.message });
      }
    });
  });
}

if (suite) {
  const io = new Server(httpServer, {
    path: "/socket.io/",
    serveClient: false,
    pingInterval: 300,
    pingTimeout: 200,
    connectTimeout: 1000,
    cors: { origin: "*" },
  });
  servers.push(io);
  io.on("connection", (socket) => {
    socket.emit("auth", socket.handshake.auth);
    socket.on("message", (...args) => socket.emit("message-back", ...args));
    socket.on("message-with-ack", (...args) => {
      const ack = args.pop();
      ack(...args);
    });
  });
  io.of("/custom").on("connection", (socket) => {
    socket.emit("auth", socket.handshake.auth);
    socket.on("message", (...args) => socket.emit("message-back", ...args));
    socket.on("message-with-ack", (...args) => {
      const ack = args.pop();
      ack(...args);
    });
  });
} else {
  for (const [path, connectTimeout] of [
    ["/sio/", 45000],
    ["/sio-short/", 1000],
  ]) {
    const io = new Server(httpServer, { path, serveClient: false, connectTimeout });
    servers.push(io);
    for (const name of ["/", "/admin", "/slow", "/denied"]) scenarioNamespace(io, name);
  }
}

function shutdown() {
  Promise.all(servers.map((io) => new Promise((resolve) => io.close(resolve)))).then(() => process.exit(0));
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
