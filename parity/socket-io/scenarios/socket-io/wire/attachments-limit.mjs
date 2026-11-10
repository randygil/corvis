// A binary packet that declares 11 attachments (the decoder allows 10) closes the connection on both servers, before
// any attachment is sent.
import { RawSio } from "../../../lib/socket-client.mjs";

export default {
  name: "socket-io/wire/attachments-limit",
  requires: ["socket-io"],
  async run({ url, record, step }) {
    const raw = await step("raw connection", RawSio.connect(url));
    try {
      await raw.join("/");
      const placeholders = Array.from({ length: 11 }, (_, i) => `{"_placeholder":true,"num":${i}}`).join(",");
      raw.send(`511-["x",${placeholders}]`);
      record("raw", "eleven", { closed: await raw.waitClosed(5000) });
    } finally {
      raw.close();
    }
    const ten = await step("raw connection", RawSio.connect(url));
    try {
      await ten.join("/");
      const placeholders = Array.from({ length: 10 }, (_, i) => `{"_placeholder":true,"num":${i}}`).join(",");
      ten.send(`510-["echo",${placeholders}]`);
      // Nine attachments are not enough: the packet waits and the connection stays open.
      for (let i = 0; i < 9; i++) ten.sendBinary(Buffer.from([i]));
      record("raw", "nine-of-ten", { closedWhileWaiting: await ten.waitClosed(500) });
    } finally {
      ten.close();
    }
  },
};
