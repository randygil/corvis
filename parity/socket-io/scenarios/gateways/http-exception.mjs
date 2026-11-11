// An HTTP exception thrown by a handler: the reference answers `Internal server error` (it is not a WebSocket
// exception); Corvis forwards the exception's own message and fields. An expected difference.
import { connected } from "../../lib/official-client.mjs";
import { gateway, settle, watch } from "../../lib/gateways.mjs";

export default {
  name: "gateways/http-exception",
  requires: ["gateways"],
  async run({ url, record, step }) {
    const socket = gateway(url);
    const seen = watch(socket);
    try {
      await connected(step, socket);
      socket.emit("fail-http");
      record("client", "fail-http", { events: await settle(step, socket, seen) });
    } finally {
      socket.close();
    }
  },
};
