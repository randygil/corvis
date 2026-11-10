// ws/engine-io: "Upgrade with queued packets". The official `engine.io-client` connects to preset `burst` (the server
// emits 100 messages as soon as the session opens) over polling and upgrades to WebSocket while they are in flight.
// Each message must arrive exactly once and in emission order.
import { Socket } from "engine.io-client";

export default {
  name: "engine-io/upgrade",
  requires: ["engine-io"],
  async run({ url, record, step }) {
    const socket = new Socket(`${url}/burst/`, { transports: ["polling", "websocket"], path: "/burst/", rememberUpgrade: false });
    const received = [];
    let upgraded = false;
    socket.on("upgrade", () => {
      upgraded = true;
    });
    try {
      await step(
        "100 messages",
        new Promise((resolve, reject) => {
          socket.on("message", (data) => {
            received.push(String(data));
            if (received.length === 100) resolve();
          });
          socket.on("error", reject);
          socket.on("close", () => reject(new Error("closed before 100 messages")));
        }),
      );
      // The upgrade may finish after the last message: wait for it, it must happen.
      if (!upgraded) {
        await step("upgrade", new Promise((resolve) => socket.once("upgrade", resolve)));
      }
      record("client", "messages", {
        count: received.length,
        inOrder: received.every((m, i) => m === `m${i}`),
        upgradedTo: socket.transport.name,
      });
    } finally {
      socket.close();
    }
  },
};
