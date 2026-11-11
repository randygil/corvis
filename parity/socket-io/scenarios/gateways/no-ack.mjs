// Without a callback nothing is sent for a returned value; observed through the marker event.
import { connected } from "../../lib/official-client.mjs";
import { gateway, settle, watch } from "../../lib/gateways.mjs";

export default {
  name: "gateways/no-ack",
  requires: ["gateways"],
  async run({ url, record, step }) {
    const socket = gateway(url);
    const seen = watch(socket);
    try {
      await connected(step, socket);
      socket.emit("echo", "x");
      socket.emit("falsy");
      record("client", "before-the-marker", { events: await settle(step, socket, seen) });
    } finally {
      socket.close();
    }
  },
};
