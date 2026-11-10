// Three CONNECT packets for a namespace whose middleware takes a second. Node starts one pending socket per packet and
// answers all three; Corvis allows one pending attempt per namespace and closes the connection at the second packet
// (expected difference, ws/socket-io: One socket per namespace per connection).
import { RawSio } from "../../../lib/socket-client.mjs";

export default {
  name: "socket-io/wire/pending-connect",
  requires: ["socket-io"],
  async run({ url, record, step }) {
    const raw = await step("raw connection", RawSio.connect(url));
    try {
      for (let i = 0; i < 3; i++) raw.send("0/slow,");
      // Either the connection is closed, or all three attempts are answered (the middleware takes a second).
      await step(
        "answers or close",
        (async () => {
          const deadline = Date.now() + 6000;
          while (Date.now() < deadline) {
            if (raw.closedAt !== null) return;
            if (raw.messages.filter((m) => m.text?.startsWith("0/slow,")).length >= 3) return;
            await new Promise((resolve) => setTimeout(resolve, 25));
          }
        })(),
      );
      record("raw", "outcome", {
        connects: raw.messages.filter((m) => m.text?.startsWith("0/slow,")).length,
        closed: raw.closedAt !== null,
      });
    } finally {
      raw.close();
    }
  },
};
