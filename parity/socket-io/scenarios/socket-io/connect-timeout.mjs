// A raw client completes the Engine.IO handshake and sends no CONNECT: the server closes the session when
// `connectTimeout` (1000 ms on this server) passes, not before.
import { RawSio } from "../../lib/socket-client.mjs";

export default {
  name: "socket-io/connect-timeout",
  requires: ["socket-io"],
  async run({ url, record, step }) {
    const raw = await step("raw connection", RawSio.connect(url, "/sio-short/"));
    const startedAt = Date.now();
    try {
      const closed = await step("closed by the server", raw.waitClosed(8000));
      record("raw", "closed", { closed, notBefore1000ms: raw.closedAt !== null && raw.closedAt - startedAt >= 950 });
    } finally {
      raw.close();
    }
  },
};
