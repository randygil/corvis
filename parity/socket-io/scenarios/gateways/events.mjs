// A WsResponse or an object with an `event` field is emitted (with or without a callback, which stays unanswered);
// nothing returned sends nothing and does not call the callback.
import { connected } from "../../lib/official-client.mjs";
import { askAndWatch, gateway, settle, watch } from "../../lib/gateways.mjs";

export default {
  name: "gateways/events",
  requires: ["gateways"],
  async run({ url, record, step }) {
    const socket = gateway(url);
    const seen = watch(socket);
    try {
      await connected(step, socket);
      const reply = askAndWatch(socket, "reply");
      const duck = askAndWatch(socket, "duck");
      const silent = askAndWatch(socket, "silent");
      const nothing = askAndWatch(socket, "nothing");
      socket.emit("reply");
      record("client", "events", { events: await settle(step, socket, seen) });
      record("client", "callbacks", { reply, duck, silent, nothing });
    } finally {
      socket.close();
    }
  },
};
