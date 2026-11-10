// Binary attachments with the official client: an object holding a buffer echoed over polling only, and two buffers
// acknowledged over WebSocket.
import { ask, client, connected, next } from "../../lib/official-client.mjs";

const bytes = (n, seed) => Buffer.from(Array.from({ length: n }, (_, i) => (i * 7 + seed) & 0xff));
const describe = (value) => (Buffer.isBuffer(value) ? { buffer: value.toString("hex") } : value);

export default {
  name: "socket-io/binary",
  requires: ["socket-io"],
  async run({ url, record, step }) {
    const polling = client(url, "", { transports: ["polling"] });
    try {
      await connected(step, polling);
      record("polling", "transport", { name: polling.io.engine.transport.name });
      const payload = { name: "file", data: bytes(1024, 1), nested: [bytes(3, 9), 5] };
      const [echoed] = await ask(step, polling, "binary echo over polling", "echo", payload);
      record("polling", "echo", {
        name: echoed.name,
        data: describe(echoed.data),
        nested: [describe(echoed.nested[0]), echoed.nested[1]],
        equal: Buffer.compare(echoed.data, payload.data) === 0,
      });
      const pushed = next(step, polling, "bin");
      polling.emit("bin-emit", bytes(5, 3), "text");
      record("polling", "bin", { args: (await pushed).map(describe) });
    } finally {
      polling.close();
    }

    const ws = client(url, "", { transports: ["websocket"] });
    try {
      await connected(step, ws);
      record("websocket", "transport", { name: ws.io.engine.transport.name });
      const args = await ask(step, ws, "binary ack over WebSocket", "echo", bytes(10, 1), bytes(20, 2));
      record("websocket", "echo", { args: args.map(describe) });
    } finally {
      ws.close();
    }
  },
};
