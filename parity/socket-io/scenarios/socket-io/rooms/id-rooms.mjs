// The room of a socket's id: Node lets a socket join the room of another socket's id (and then it receives what is
// sent to that socket) and leave its own; Corvis refuses both, so a client cannot read another socket's messages by
// naming its id.
import { command, crowd, round } from "../../../lib/rooms.mjs";

export default {
  name: "socket-io/rooms/id-rooms",
  requires: ["socket-io"],
  async run({ url, record, step }) {
    const group = await crowd(step, url, { A: "/", B: "/", C: "/" });
    try {
      record("id-rooms", "join-another-sockets-id", await command(group, step, "A", "join", { rooms: [group.members.B.socket.id] }));
      record("id-rooms", "emit-to-that-id", await round(group, step, "C", "broadcast", { to: [group.members.B.socket.id], event: "private" }));
      record("id-rooms", "leave-its-own-id", await command(group, step, "C", "room-leave", { room: group.members.C.socket.id }));
      record("id-rooms", "emit-to-its-own-id", await round(group, step, "A", "broadcast", { to: [group.members.C.socket.id], event: "own" }));
    } finally {
      group.close();
    }
  },
};
