// The four disconnections a client can see, with the reason the server reports for each, read from the server log.
import { Manager } from "socket.io-client";
import { client, connected, disconnected } from "../../lib/official-client.mjs";
import { SIO, resetLog, sortedLog, waitForLog } from "../../lib/socket-client.mjs";

async function pair(step, url) {
  const manager = new Manager(url, { path: SIO, forceNew: true, reconnection: false });
  const main = manager.socket("/");
  const admin = manager.socket("/admin", { auth: { token: "admin" } });
  await connected(step, main, "connect /");
  await connected(step, admin, "connect /admin");
  return { manager, main, admin };
}

const disconnects = (entries) => entries.filter((e) => e.event === "disconnect");

export default {
  name: "socket-io/disconnect",
  requires: ["socket-io"],
  async run({ url, record, step }) {
    // 1. The client leaves one namespace.
    await resetLog(step, url);
    let { manager, main, admin } = await pair(step, url);
    admin.disconnect();
    let log = await waitForLog(step, url, (entries) => disconnects(entries).length >= 1);
    record("leave", "server-log", sortedLog(disconnects(log)));
    record("leave", "main-still-connected", { connected: main.connected });
    manager.engine.close();
    await waitForLog(step, url, (entries) => disconnects(entries).length >= 2);

    // 2. The server leaves one namespace.
    await resetLog(step, url);
    ({ manager, main, admin } = await pair(step, url));
    const left = disconnected(step, admin, "server leaves /admin");
    admin.emit("leave");
    record("server-leaves", "client-reason", { reason: await left });
    log = await waitForLog(step, url, (entries) => disconnects(entries).length >= 1);
    record("server-leaves", "server-log", sortedLog(disconnects(log)));
    record("server-leaves", "main-still-connected", { connected: main.connected });
    manager.engine.close();
    await waitForLog(step, url, (entries) => disconnects(entries).length >= 2);

    // 3. The server closes the connection.
    await resetLog(step, url);
    ({ manager, main, admin } = await pair(step, url));
    const closedMain = disconnected(step, main, "server closes the connection");
    main.emit("kick");
    record("kick", "client-reason", { reason: await closedMain });
    log = await waitForLog(step, url, (entries) => disconnects(entries).length >= 2);
    record("kick", "server-log", sortedLog(disconnects(log)));

    // 4. The client closes the transport.
    await resetLog(step, url);
    const socket = client(url);
    await connected(step, socket);
    const gone = disconnected(step, socket, "transport closed by the client");
    socket.io.engine.close();
    record("transport", "client-reason", { reason: await gone });
    log = await waitForLog(step, url, (entries) => disconnects(entries).length >= 1);
    record("transport", "server-log", sortedLog(disconnects(log)));
  },
};
