// Packets that cannot be decoded close the connection on both servers, and the socket reports `forced close`.
import { RawSio, resetLog, sortedLog, waitForLog } from "../../../lib/socket-client.mjs";

export default {
  name: "socket-io/wire/malformed",
  requires: ["socket-io"],
  async run({ url, record, step }) {
    await resetLog(step, url);
    for (const [index, packet] of ["9", '2{"a":1}', '2["disconnect"]', '2abc["x"]'].entries()) {
      const raw = await step("raw connection", RawSio.connect(url));
      try {
        await raw.join("/");
        raw.send(packet);
        record("raw", `closed-${index}`, { packet, closed: await raw.waitClosed(5000) });
      } finally {
        raw.close();
      }
    }
    const log = await waitForLog(step, url, (entries) => entries.filter((e) => e.event === "disconnect").length >= 4);
    record("server", "log", sortedLog(log.filter((e) => e.event === "disconnect")));
  },
};
