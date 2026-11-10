// An event whose argument is a lone surrogate (`"\ud800"`). JavaScript strings are UTF-16 and `JSON.parse` keeps it, so
// Node delivers the event and acknowledges it; a Rust string cannot hold one, so Corvis refuses the packet and closes
// the connection (expected difference, ws/socket-io: Lone surrogates are refused). A pair of surrogates that makes a
// character is accepted by both.
import { RawSio } from "../../../lib/socket-client.mjs";

export default {
  name: "socket-io/wire/surrogate",
  requires: ["socket-io"],
  async run({ url, record, step }) {
    const raw = await step("raw connection", RawSio.connect(url));
    try {
      await raw.join("/");
      raw.send('27["echo","\\ud83d\\ude00"]');
      const pair = await raw.waitFor((m) => m.text?.startsWith("37"));
      record("raw", "surrogate-pair", { text: pair?.text ?? null });
      raw.messages.length = 0;
      raw.send('28["echo","\\ud800"]');
      await raw.waitFor((m) => m.text?.startsWith("38"), "answer", 1500);
      const answered = raw.messages.some((m) => m.text?.startsWith("38"));
      if (!answered) await raw.waitClosed(3000);
      record("raw", "lone-surrogate", { answered, closed: raw.closedAt !== null });
    } finally {
      raw.close();
    }
  },
};
