// A handler's return value answers the client's acknowledgement, a falsy value included; no argument, one and several.
import { ask, connected } from "../../lib/official-client.mjs";
import { gateway } from "../../lib/gateways.mjs";

export default {
  name: "gateways/ack",
  requires: ["gateways"],
  async run({ url, record, step }) {
    const socket = gateway(url);
    try {
      await connected(step, socket);
      record("client", "echo", { args: await ask(step, socket, "echo", "echo", "ping") });
      record("client", "echo-object", { args: await ask(step, socket, "echo object", "echo", { a: 1 }) });
      record("client", "falsy", { args: await ask(step, socket, "falsy", "falsy") });
      record("client", "args-one", { args: await ask(step, socket, "args one", "args", "x") });
      record("client", "args-three", { args: await ask(step, socket, "args three", "args", 1, "two", { three: 3 }) });
      record("client", "args-none", { args: await ask(step, socket, "args none", "args") });
    } finally {
      socket.close();
    }
  },
};
