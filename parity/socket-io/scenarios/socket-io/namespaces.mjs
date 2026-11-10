// Joining namespaces with the official client: `auth` reaches the server, an unknown namespace is refused, two
// namespaces share one Engine.IO session and receive only their own events, and the ids are three different strings.
import { Manager } from "socket.io-client";
import { client, connected, next } from "../../lib/official-client.mjs";
import { SIO, resetLog, sortedLog, waitForLog } from "../../lib/socket-client.mjs";

export default {
  name: "socket-io/namespaces",
  requires: ["socket-io"],
  async run({ url, record, step }) {
    await resetLog(step, url);

    const main = client(url, "", { auth: { token: "abc" } });
    try {
      const outcome = await connected(step, main);
      record("main", "connect", { outcome: outcome === "connect" ? "connect" : outcome.message, sid: main.id });
    } finally {
      main.close();
    }
    record("server", "log", sortedLog(await waitForLog(step, url, (log) => log.length >= 1)));

    const missing = client(url, "/missing");
    try {
      const outcome = await connected(step, missing, "connect to a namespace that does not exist");
      record("missing", "connect_error", { message: outcome === "connect" ? "connected" : outcome.message });
    } finally {
      missing.close();
    }

    // Two sockets of one manager: one Engine.IO session.
    const manager = new Manager(url, { path: SIO, forceNew: true, reconnection: false });
    const first = manager.socket("/");
    const second = manager.socket("/admin", { auth: { token: "admin" } });
    try {
      await connected(step, first, "connect /");
      await connected(step, second, "connect /admin");
      record("pair", "session", {
        sameEngine: first.io === second.io,
        distinctIds: new Set([first.id, second.id, manager.engine.id]).size,
      });
      // Each namespace receives its own events only.
      const mainUpdate = next(step, first, "update", "update on /");
      const adminUpdate = next(step, second, "update", "update on /admin");
      first.emit("emit-update");
      record("pair", "main-update", { args: await mainUpdate });
      second.emit("emit-update");
      record("pair", "admin-update", { args: await adminUpdate });
      let stray = 0;
      first.on("update", () => stray++);
      const again = next(step, second, "update", "second update on /admin");
      second.emit("emit-update");
      await again;
      record("pair", "cross-talk", { strayOnMain: stray });
    } finally {
      manager.engine?.close();
      first.close();
      second.close();
    }
  },
};
