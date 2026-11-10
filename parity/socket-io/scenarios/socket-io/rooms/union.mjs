// `to` with several rooms reaches the union, each socket once; a room nobody is in selects nobody.
import { command, crowd, round } from "../../../lib/rooms.mjs";

export default {
  name: "socket-io/rooms/union",
  requires: ["socket-io"],
  async run({ url, record, step }) {
    const group = await crowd(step, url, { A: "/", B: "/", C: "/", D: "/" });
    try {
      await command(group, step, "A", "join", { rooms: ["r1", "r2"] });
      await command(group, step, "B", "join", { rooms: ["r1"] });
      await command(group, step, "C", "join", { rooms: ["r2"] });
      record("union", "two-rooms", await round(group, step, "D", "broadcast", { to: ["r1", "r2"], event: "u" }));
      record("union", "one-room", await round(group, step, "D", "broadcast", { to: ["r1"], event: "u" }));
      record("union", "empty-room", await round(group, step, "D", "broadcast", { to: ["nobody"], event: "u" }));
      record("union", "arguments", await round(group, step, "D", "broadcast", { to: ["r2"], event: "u", args: [1, "two", { three: [3] }] }));
    } finally {
      group.close();
    }
  },
};
