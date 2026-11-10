// A broadcast with acknowledgements: every client answers; one stays silent and the call fails at its timeout with the
// answers that arrived; and without a timeout (Node fails at once, Corvis waits for ack_timeout).
import { command, crowd, round } from "../../../lib/rooms.mjs";

export default {
  name: "socket-io/rooms/broadcast-ack",
  requires: ["socket-io"],
  async run({ url, record, step }) {
    const group = await crowd(step, url, { A: "/", B: "/", C: "/" }, { answers: { A: "a", B: "b" } });
    try {
      for (const name of ["A", "B", "C"]) await command(group, step, name, "join", { rooms: ["r"] });
      // C never answers: with three answers expected, the call fails at the timeout with the two that came.
      record("broadcast-ack", "one-silent", await command(group, step, "A", "broadcast-ack", { to: ["r"], timeout: 200 }));
      // Without C in the broadcast, everyone answers.
      record("broadcast-ack", "every-recipient-answers", await command(group, step, "A", "broadcast-ack", { to: ["r"], except: [group.members.C.socket.id], timeout: 5000 }));
      record("broadcast-ack", "nobody", await command(group, step, "A", "broadcast-ack", { to: ["nobody"], timeout: 5000 }));
    } finally {
      group.close();
    }
    // The clients answer after 100 ms: Node's call without a timeout is already over (its timer is not set), Corvis
    // waits for ack_timeout and collects them.
    const slow = await crowd(step, url, { A: "/", B: "/" }, { answers: { A: "a", B: "b" }, delay: 100 });
    try {
      record("broadcast-ack", "no-timeout", await command(slow, step, "A", "broadcast-ack", {}));
    } finally {
      slow.close();
    }
  },
};
