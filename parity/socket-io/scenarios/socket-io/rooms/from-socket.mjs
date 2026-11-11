// A broadcast started from a socket excludes that socket: `socket.to(room)` and `socket.broadcast`.
import { command, crowd, round } from "../../../lib/rooms.mjs";

export default {
  name: "socket-io/rooms/from-socket",
  requires: ["socket-io"],
  async run({ url, record, step }) {
    const group = await crowd(step, url, { A: "/", B: "/", C: "/", D: "/" });
    try {
      for (const name of ["A", "B", "C"]) await command(group, step, name, "join", { rooms: ["r"] });
      record("from-socket", "to-a-room", await round(group, step, "A", "broadcast", { from: "socket", to: ["r"], event: "s" }));
      record("from-socket", "whole-namespace", await round(group, step, "A", "broadcast", { from: "socket", event: "s" }));
      record("from-socket", "except-a-room", await round(group, step, "A", "broadcast", { from: "socket", except: ["r"], event: "s" }));
    } finally {
      group.close();
    }
  },
};
