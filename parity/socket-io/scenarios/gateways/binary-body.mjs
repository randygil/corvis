// A binary argument: the reference passes the buffer through; Corvis' pipeline has no binary type, so the handler sees
// null. An expected difference (a handler registered on the socket receives the real bytes).
import { ask, connected } from "../../lib/official-client.mjs";
import { gateway } from "../../lib/gateways.mjs";

export default {
  name: "gateways/binary-body",
  requires: ["gateways"],
  async run({ url, record, step }) {
    const socket = gateway(url);
    try {
      await connected(step, socket);
      const [answer] = await ask(step, socket, "upload", "upload", Buffer.from([1, 2, 3]));
      record("client", "upload", { isBuffer: Buffer.isBuffer(answer?.body), body: Buffer.isBuffer(answer?.body) ? [...answer.body] : answer?.body });
    } finally {
      socket.close();
    }
  },
};
