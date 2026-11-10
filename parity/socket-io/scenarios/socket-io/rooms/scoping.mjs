// Rooms belong to a namespace: the same room name in `/` and in `/admin` is two rooms, and a broadcast on a namespace
// reaches the sockets of that namespace only.
import { command, crowd, round } from "../../../lib/rooms.mjs";

export default {
  name: "socket-io/rooms/scoping",
  requires: ["socket-io"],
  async run({ url, record, step }) {
    const group = await crowd(step, url, { A: "/", B: "/", C: "/admin" });
    try {
      await command(group, step, "A", "join", { rooms: ["r"] });
      await command(group, step, "C", "join", { rooms: ["r"] });
      record("scoping", "same-room-two-namespaces", await round(group, step, "A", "broadcast", { to: ["r"], event: "hit" }));
      record("scoping", "whole-namespace", await round(group, step, "A", "broadcast", { event: "all" }));
      record("scoping", "other-namespace", await round(group, step, "C", "broadcast", { event: "admin-only" }));
    } finally {
      group.close();
    }
  },
};
