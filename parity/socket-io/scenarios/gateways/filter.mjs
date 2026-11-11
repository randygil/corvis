// Gateway exception filters: one answers with a body of its own, one answers nothing (an empty body sends nothing).
import { connected } from "../../lib/official-client.mjs";
import { gateway, settle, watch } from "../../lib/gateways.mjs";

export default {
  name: "gateways/filter",
  requires: ["gateways"],
  async run({ url, record, step }) {
    const socket = gateway(url, "/rules", { auth: { token: "t" } });
    const seen = watch(socket);
    try {
      await connected(step, socket);
      socket.emit("fail-room");
      record("client", "room", { events: await settle(step, socket, seen, "room") });
      socket.emit("fail-quiet");
      record("client", "quiet", { events: await settle(step, socket, seen, "quiet") });
    } finally {
      socket.close();
    }
  },
};
