// Two gateways are two namespaces of one session; a namespace no gateway declares is refused.
import { Manager } from "socket.io-client";
import { ask, connected } from "../../lib/official-client.mjs";
import { GATEWAY_PATH, gateway } from "../../lib/gateways.mjs";

export default {
  name: "gateways/namespaces",
  requires: ["gateways"],
  async run({ url, record, step }) {
    const manager = new Manager(url, { path: GATEWAY_PATH, forceNew: true, reconnection: false });
    const a = manager.socket("/a");
    const b = manager.socket("/b");
    try {
      await connected(step, a, "connect /a");
      await connected(step, b, "connect /b");
      record("pair", "session", { sameEngine: a.io === b.io });
      record("pair", "a", { args: await ask(step, a, "ping a", "ping") });
      record("pair", "b", { args: await ask(step, b, "ping b", "ping") });
    } finally {
      a.close();
      b.close();
    }
    const missing = gateway(url, "/missing");
    try {
      const outcome = await connected(step, missing, "connect to a namespace nobody declares");
      record("missing", "connect_error", { message: outcome === "connect" ? "connected" : outcome.message });
    } finally {
      missing.close();
    }
  },
};
