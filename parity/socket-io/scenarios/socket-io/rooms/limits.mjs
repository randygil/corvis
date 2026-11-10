// The bounds Corvis puts on rooms and Node does not have: 1,000 rooms per socket (own id included) and room names of
// at most 256 bytes. Node accepts both; Corvis refuses the join and joins nothing.
import { command, crowd } from "../../../lib/rooms.mjs";

export default {
  name: "socket-io/rooms/limits",
  requires: ["socket-io"],
  async run({ url, record, step }) {
    const group = await crowd(step, url, { A: "/", B: "/" });
    try {
      const many = Array.from({ length: 1000 }, (_, n) => `room-${n}`);
      const first = await command(group, step, "A", "join", { rooms: many });
      record("limits", "a-thousand-rooms", { ok: first.ok, rooms: first.rooms?.length ?? null });
      const after = await command(group, step, "A", "rooms");
      record("limits", "rooms-after", { rooms: after.rooms.length });
      // Room names: 256 bytes are fine, 257 are not.
      const fits = await command(group, step, "B", "join", { rooms: ["x".repeat(256)] });
      record("limits", "name-of-256-bytes", { ok: fits.ok, rooms: fits.rooms.length });
      const long = await command(group, step, "B", "join", { rooms: ["x".repeat(257)] });
      record("limits", "name-of-257-bytes", { ok: long.ok });
      record("limits", "rooms-of-b", await command(group, step, "B", "rooms").then((r) => ({ rooms: r.rooms.length })));
    } finally {
      group.close();
    }
  },
};
