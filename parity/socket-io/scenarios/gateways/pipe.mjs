// A pipe on the message body: a failing pipe skips the handler and the client receives `exception`.
import { ask, connected } from "../../lib/official-client.mjs";
import { gateway, settle, watch } from "../../lib/gateways.mjs";

export default {
  name: "gateways/pipe",
  requires: ["gateways"],
  async run({ url, record, step }) {
    const socket = gateway(url, "/rules", { auth: { token: "t" } });
    const seen = watch(socket);
    try {
      await connected(step, socket);
      socket.emit("piped", "x");
      record("client", "invalid", { events: await settle(step, socket, seen, "invalid") });
      record("client", "valid", { args: await ask(step, socket, "valid", "piped", 4) });
    } finally {
      socket.close();
    }
  },
};
