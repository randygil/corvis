// An interceptor sees and replaces the handler's result: the callback receives the wrapped value.
import { ask, connected } from "../../lib/official-client.mjs";
import { gateway } from "../../lib/gateways.mjs";

export default {
  name: "gateways/interceptor",
  requires: ["gateways"],
  async run({ url, record, step }) {
    const socket = gateway(url, "/rules", { auth: { token: "t" } });
    try {
      await connected(step, socket);
      record("client", "wrapped", { args: await ask(step, socket, "wrapped", "wrapped") });
    } finally {
      socket.close();
    }
  },
};
