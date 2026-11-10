// A client acknowledgement id of 20 digits. Node reads it as a float and answers with a rounded id; Corvis refuses ids
// above 2^53 - 1 and closes the connection (expected difference, ws/socket-io: Client acknowledgement ids are
// bounded). The largest accepted id is echoed exactly by both.
import { RawSio } from "../../../lib/socket-client.mjs";

export default {
  name: "socket-io/wire/ack-id",
  requires: ["socket-io"],
  async run({ url, record, step }) {
    const raw = await step("raw connection", RawSio.connect(url));
    try {
      await raw.join("/");
      raw.send('29007199254740991["echo",1]');
      const largest = await raw.waitFor((m) => m.text?.startsWith("3"));
      record("raw", "largest-id", { text: largest?.text ?? null });
      raw.messages.length = 0;
      raw.send('212345678901234567890["echo",1]');
      await raw.waitFor((m) => m.text?.startsWith("3"), "answer", 1500);
      const answered = raw.messages.some((m) => m.text?.startsWith("3"));
      if (!answered) await raw.waitClosed(3000);
      record("raw", "oversized-id", { answered, closed: raw.closedAt !== null });
    } finally {
      raw.close();
    }
  },
};
