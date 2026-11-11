// Bulk operations over a selection: `socketsJoin`, `socketsLeave`, `disconnectSockets` and `fetchSockets`.
import { command, crowd, round } from "../../../lib/rooms.mjs";

export default {
  name: "socket-io/rooms/bulk",
  requires: ["socket-io"],
  async run({ url, record, step }) {
    const group = await crowd(step, url, { A: "/", B: "/", C: "/", D: "/admin" });
    try {
      await command(group, step, "A", "join", { rooms: ["r1"] });
      await command(group, step, "B", "join", { rooms: ["r1"] });
      // Make a room join another.
      await command(group, step, "C", "sockets-join", { to: ["r1"], rooms: ["r2"] });
      record("bulk", "after-sockets-join", await round(group, step, "C", "broadcast", { to: ["r2"], event: "e" }));
      await command(group, step, "C", "sockets-leave", { to: ["r1"], rooms: ["r2"] });
      record("bulk", "after-sockets-leave", await round(group, step, "C", "broadcast", { to: ["r2"], event: "e" }));
      // Fetch the sockets of a room except one.
      const fetched = await command(group, step, "C", "fetch-sockets", { to: ["r1"], except: [group.members.B.socket.id] });
      record(
        "bulk",
        "fetch-sockets",
        fetched.sockets.map((s) => ({ id: s.id, rooms: [...s.rooms].sort() })).sort((a, b) => a.id.localeCompare(b.id)),
      );
      record("bulk", "fetch-everyone", (await command(group, step, "C", "fetch-sockets", {})).sockets.map((s) => s.id).sort());
      // Disconnect a room without closing the connection: only that namespace's sockets hear it.
      const left = ["A", "B"].map((name) => step(`${name} disconnected`, new Promise((resolve) => group.members[name].socket.once("disconnect", resolve))));
      await command(group, step, "C", "disconnect-sockets", { to: ["r1"] });
      record("bulk", "disconnect-reasons", await Promise.all(left));
      record("bulk", "still-connected", { A: group.members.A.socket.connected, B: group.members.B.socket.connected, C: group.members.C.socket.connected, D: group.members.D.socket.connected });
      record("bulk", "after-disconnect", await round(group, step, "C", "broadcast", { event: "e" }, ["C", "D"]));
    } finally {
      group.close();
    }
  },
};
