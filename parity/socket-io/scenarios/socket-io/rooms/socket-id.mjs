// Every socket is in the room of its own id from the moment it connects, so emitting to an id reaches that socket.
import { command, crowd, round } from "../../../lib/rooms.mjs";

export default {
  name: "socket-io/rooms/socket-id",
  requires: ["socket-io"],
  async run({ url, record, step }) {
    const group = await crowd(step, url, { A: "/", B: "/", C: "/" });
    try {
      record("socket-id", "rooms-right-after-connecting", await command(group, step, "A", "rooms"));
      record("socket-id", "emit-to-an-id", await round(group, step, "A", "broadcast", { to: [group.members.B.socket.id], event: "direct" }));
      record("socket-id", "emit-to-two-ids", await round(group, step, "C", "broadcast", { to: [group.members.A.socket.id, group.members.C.socket.id], event: "two" }));
    } finally {
      group.close();
    }
  },
};
