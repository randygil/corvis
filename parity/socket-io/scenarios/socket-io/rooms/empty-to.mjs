// A `to` given an empty list of rooms. In Node it adds no room, so the broadcast reaches everyone; in Corvis it selects
// no socket, because a list that came from a client must not mean "everyone". Without any `to` both reach everyone, and
// an empty `except` changes nothing in both.
import { command, crowd, round } from "../../../lib/rooms.mjs";

export default {
  name: "socket-io/rooms/empty-to",
  requires: ["socket-io"],
  async run({ url, record, step }) {
    const group = await crowd(step, url, { A: "/", B: "/", C: "/" });
    try {
      await command(group, step, "A", "join", { rooms: ["r"] });
      record("empty-to", "no-to-at-all", await round(group, step, "C", "broadcast", { event: "all" }));
      record("empty-to", "empty-to", await round(group, step, "C", "broadcast", { emptyTo: true, event: "none" }));
      record("empty-to", "empty-to-then-a-room", await round(group, step, "C", "broadcast", { emptyTo: true, to: ["r"], event: "r-only" }));
      record("empty-to", "empty-except", await round(group, step, "C", "broadcast", { to: ["r"], except: [], event: "r-again" }));
    } finally {
      group.close();
    }
  },
};
