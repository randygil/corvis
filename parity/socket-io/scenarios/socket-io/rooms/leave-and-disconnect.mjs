// Leaving a room takes effect at once; a disconnecting socket is still in its rooms while `disconnecting` runs and in
// none at `disconnect`, and `disconnecting` can say goodbye to the people in them.
import { command, crowd, round } from "../../../lib/rooms.mjs";
import { resetLog, sortedLog, waitForLog } from "../../../lib/socket-client.mjs";

export default {
  name: "socket-io/rooms/leave-and-disconnect",
  requires: ["socket-io"],
  async run({ url, record, step }) {
    await resetLog(step, url);
    const group = await crowd(step, url, { A: "/", B: "/", C: "/" });
    try {
      for (const name of ["A", "B", "C"]) await command(group, step, name, "join", { rooms: ["r"] });
      record("leave", "rooms-after-leave", await command(group, step, "A", "room-leave", { room: "r" }));
      record("leave", "broadcast-after-leave", await round(group, step, "C", "broadcast", { to: ["r"], event: "e" }));

      // B disconnects from the namespace while it is in `r`: C hears the goodbye, A (who left) does not.
      const goodbye = step("goodbye on C", new Promise((resolve) => group.members.C.socket.once("left", () => resolve(true))));
      const gone = step("B disconnected", new Promise((resolve) => group.members.B.socket.once("disconnect", resolve)));
      group.members.B.socket.emit("leave");
      await gone;
      record("disconnect", "goodbye", { heard: await goodbye });
      const log = await waitForLog(step, url, (entries) => entries.some((e) => e.event === "disconnect"));
      record("disconnect", "server-log", sortedLog(log.filter((e) => e.event === "disconnect")));
      record("disconnect", "A-heard-nothing", { received: group.members.A.received });
      record("disconnect", "after", await round(group, step, "C", "broadcast", { to: ["r"], event: "e" }, ["A", "C"]));
    } finally {
      group.close();
    }
  },
};
