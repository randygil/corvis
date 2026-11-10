// Events in both directions with several arguments, an acknowledgement with several values, and the reserved event
// names that server code may not emit.
import { ask, client, connected, next } from "../../lib/official-client.mjs";
import { resetLog, waitForLog } from "../../lib/socket-client.mjs";

export default {
  name: "socket-io/events",
  requires: ["socket-io"],
  async run({ url, record, step }) {
    await resetLog(step, url);
    const socket = client(url);
    try {
      await connected(step, socket);
      const update = next(step, socket, "update");
      socket.emit("emit-update");
      record("client", "update", { args: await update });

      record("client", "sum", { args: await ask(step, socket, "sum", "sum", 1, 2) });
      record("client", "echo", { args: await ask(step, socket, "echo", "echo", "a", 1, { b: [2, null] }, [true]) });
      record("client", "echo-none", { args: await ask(step, socket, "echo without values", "echo") });

      socket.emit("emit-reserved");
      const log = await waitForLog(step, url, (entries) => entries.some((e) => e.event === "emit-reserved"));
      record("server", "emit-reserved", log.find((e) => e.event === "emit-reserved"));

      // An event nobody listens to is ignored and the connection stays usable.
      socket.emit("nobody-listens", 1, 2);
      record("client", "after-unhandled", { args: await ask(step, socket, "echo after an unhandled event", "echo", "still here") });
    } finally {
      socket.close();
    }
  },
};
