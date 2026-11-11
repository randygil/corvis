// An event no handler subscribes to is dropped: no reply, no exception, the callback is never called.
import { connected } from "../../lib/official-client.mjs";
import { askAndWatch, gateway, settle, watch } from "../../lib/gateways.mjs";

export default {
  name: "gateways/unknown-event",
  requires: ["gateways"],
  async run({ url, record, step }) {
    const socket = gateway(url);
    const seen = watch(socket);
    try {
      await connected(step, socket);
      const callback = askAndWatch(socket, "nobody-listens", 1, 2);
      record("client", "events", { events: await settle(step, socket, seen) });
      record("client", "callback", callback);
    } finally {
      socket.close();
    }
  },
};
