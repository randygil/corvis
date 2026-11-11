// The body of a message: none gives null, one gives that argument, several give the array; a property of the body.
import { ask, connected } from "../../lib/official-client.mjs";
import { gateway, settle, watch } from "../../lib/gateways.mjs";

export default {
  name: "gateways/body-mapping",
  requires: ["gateways"],
  async run({ url, record, step }) {
    const socket = gateway(url);
    const seen = watch(socket);
    try {
      await connected(step, socket);
      // The callback is not an argument.
      record("client", "none", { args: await ask(step, socket, "none", "args") });
      record("client", "one", { args: await ask(step, socket, "one", "args", { a: 1 }) });
      record("client", "several", { args: await ask(step, socket, "several", "args", 1, "two", { three: 3 }) });
      record("client", "array", { args: await ask(step, socket, "an array", "args", [1, 2]) });
      // `#[message_body("room")]`: a property of the body.
      socket.emit("join", { room: "r1" });
      await settle(step, socket, seen);
      record("client", "property", { args: await ask(step, socket, "whoami", "whoami") });
    } finally {
      socket.close();
    }
  },
};
