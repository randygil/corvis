// Hooks: init before the connection, the connection before the message, one disconnect; the hook sees `auth`; connect
// middleware registered in afterInit refuses a client before the connection hook runs.
import { ask, connected, disconnected } from "../../lib/official-client.mjs";
import { gateway } from "../../lib/gateways.mjs";
import { resetLog, waitForLog } from "../../lib/socket-client.mjs";

export default {
  name: "gateways/lifecycle",
  requires: ["gateways"],
  async run({ url, record, step }) {
    await resetLog(step, url);
    const socket = gateway(url, "", { auth: { token: "t" } });
    try {
      await connected(step, socket);
      await ask(step, socket, "echo", "echo", "hello");
      const closing = disconnected(step, socket);
      socket.disconnect();
      record("client", "disconnect", { reason: await closing });
    } finally {
      socket.close();
    }
    record("server", "log", await waitForLog(step, url, (log) => log.some((entry) => entry.event === "disconnect")));

    await resetLog(step, url);
    const refused = gateway(url, "/mw", { auth: { token: "no" } });
    try {
      const outcome = await connected(step, refused, "refused by the middleware");
      record("refused", "connect", { message: outcome === "connect" ? "connected" : outcome.message });
    } finally {
      refused.close();
    }
    const accepted = gateway(url, "/mw", { auth: { token: "yes" } });
    try {
      await connected(step, accepted);
      record("accepted", "hello", { args: await ask(step, accepted, "hello", "hello") });
    } finally {
      accepted.close();
    }
    record("server", "mw-log", await waitForLog(step, url, (log) => log.some((entry) => entry.event === "connection")));
  },
};
