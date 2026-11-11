// Errors are emitted as the event `exception` (a WebSocket exception with a string or an object, an unknown error) and
// the callback stays unanswered; unknown errors say `Internal server error` and nothing of the original text.
import { connected } from "../../lib/official-client.mjs";
import { askAndWatch, gateway, settle, watch } from "../../lib/gateways.mjs";

export default {
  name: "gateways/exceptions",
  requires: ["gateways"],
  async run({ url, record, step }) {
    const socket = gateway(url);
    const seen = watch(socket);
    try {
      await connected(step, socket);
      for (const event of ["fail-ws", "fail-ws-object", "fail-unknown"]) {
        const callback = askAndWatch(socket, event);
        record("client", event, { events: await settle(step, socket, seen, event), callbackCalled: callback.called });
      }
      // The connection survives.
      socket.emit("echo", "still here");
      record("client", "after", { events: await settle(step, socket, seen, "after") });
    } finally {
      socket.close();
    }
  },
};
