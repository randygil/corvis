// The server handle of a gateway broadcasts to a room: only its members receive the event.
import { connected } from "../../lib/official-client.mjs";
import { gateway, settle, watch } from "../../lib/gateways.mjs";

export default {
  name: "gateways/server-emit",
  requires: ["gateways"],
  async run({ url, record, step }) {
    const member = gateway(url);
    const outsider = gateway(url);
    const memberSeen = watch(member);
    const outsiderSeen = watch(outsider);
    try {
      await connected(step, member, "member connects");
      await connected(step, outsider, "outsider connects");
      member.emit("join", { room: "lobby" });
      await settle(step, member, memberSeen, "joined");
      member.emit("shout", { room: "lobby" });
      record("member", "events", { events: await settle(step, member, memberSeen, "member marker") });
      record("outsider", "events", { events: await settle(step, outsider, outsiderSeen, "outsider marker") });
    } finally {
      member.close();
      outsider.close();
    }
  },
};
