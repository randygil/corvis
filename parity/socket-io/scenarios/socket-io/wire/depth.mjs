// An event whose argument is 200 nested arrays. Node delivers it (and acknowledges it); Corvis refuses JSON nested
// deeper than 127 levels and closes the connection (expected difference, ws/socket-io: Nesting depth is bounded).
import { RawSio } from "../../../lib/socket-client.mjs";

export default {
  name: "socket-io/wire/depth",
  requires: ["socket-io"],
  async run({ url, record, step }) {
    const raw = await step("raw connection", RawSio.connect(url));
    try {
      await raw.join("/");
      const nested = (n) => `${"[".repeat(n)}${"]".repeat(n)}`;
      raw.send(`27["echo",${nested(100)}]`);
      const shallow = await raw.waitFor((m) => m.text?.startsWith("37"));
      record("raw", "depth-100", { answered: shallow !== null });
      raw.send(`28["echo",${nested(200)}]`);
      const deep = await raw.waitFor((m) => m.text?.startsWith("38"), "answer", 1500);
      if (!deep) await raw.waitClosed(3000);
      record("raw", "depth-200", { answered: deep !== null, closed: raw.closedAt !== null });
    } finally {
      raw.close();
    }
  },
};
