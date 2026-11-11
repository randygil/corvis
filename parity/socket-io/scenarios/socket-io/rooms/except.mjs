// `except` takes rooms, socket ids included, and removes their members from whatever the broadcast reaches.
import { command, crowd, round } from "../../../lib/rooms.mjs";

export default {
  name: "socket-io/rooms/except",
  requires: ["socket-io"],
  async run({ url, record, step }) {
    const group = await crowd(step, url, { A: "/", B: "/", C: "/", D: "/" });
    try {
      for (const name of ["A", "B", "C"]) await command(group, step, name, "join", { rooms: ["r"] });
      await command(group, step, "A", "join", { rooms: ["admins"] });
      record("except", "a-room", await round(group, step, "D", "broadcast", { except: ["admins"], event: "e" }));
      record("except", "an-id-inside-a-room", await round(group, step, "D", "broadcast", { to: ["r"], except: [group.members.B.socket.id], event: "e" }));
      record("except", "a-room-inside-a-room", await round(group, step, "D", "broadcast", { to: ["r"], except: ["admins"], event: "e" }));
      record("except", "everything", await round(group, step, "D", "broadcast", { except: ["r"], event: "e" }));
    } finally {
      group.close();
    }
  },
};
