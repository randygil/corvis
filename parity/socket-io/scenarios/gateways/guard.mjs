// A gateway guard runs for each message, never for the connection: a client without a token connects and its messages
// are denied (`Forbidden resource`); with a token the handler runs. What the guard saw is read from the server log.
import { ask, connected, next } from "../../lib/official-client.mjs";
import { gateway } from "../../lib/gateways.mjs";
import { readLog, resetLog } from "../../lib/socket-client.mjs";

export default {
  name: "gateways/guard",
  requires: ["gateways"],
  async run({ url, record, step }) {
    await resetLog(step, url);
    const denied = gateway(url, "/rules");
    try {
      record("denied", "connect", { outcome: await connected(step, denied) });
      // Every message of the gateway is guarded, the marker included: the denial itself is the event to wait for.
      const exception = next(step, denied, "exception");
      denied.emit("guarded", { n: 1 });
      record("denied", "message", { exception: await exception });
    } finally {
      denied.close();
    }
    const allowed = gateway(url, "/rules", { auth: { token: "t" } });
    try {
      await connected(step, allowed);
      record("allowed", "guarded", { args: await ask(step, allowed, "guarded", "guarded") });
    } finally {
      allowed.close();
    }
    record("server", "guard-log", (await readLog(step, url)).filter((entry) => entry.event === "guard"));
  },
};
