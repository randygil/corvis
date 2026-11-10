// Acknowledgements the server asks for: the client answers with several values, or never answers and the server's
// call fails with `operation has timed out`.
import { client, connected } from "../../lib/official-client.mjs";
import { resetLog, waitForLog } from "../../lib/socket-client.mjs";

export default {
  name: "socket-io/acks",
  requires: ["socket-io"],
  async run({ url, record, step }) {
    await resetLog(step, url);
    const socket = client(url);
    try {
      await connected(step, socket);
      socket.once("question", (ack) => ack("yes", 7));
      socket.emit("ask");
      let log = await waitForLog(step, url, (entries) => entries.some((e) => e.event === "ask-result"));
      record("server", "ask-result", log.find((e) => e.event === "ask-result"));

      // The client hears the question and never answers.
      let heard = false;
      socket.once("question", () => (heard = true));
      socket.emit("ask-timeout");
      log = await waitForLog(step, url, (entries) => entries.some((e) => e.event === "ask-timeout-result"));
      record("server", "ask-timeout-result", log.find((e) => e.event === "ask-timeout-result"));
      record("client", "heard-the-question", { heard });
    } finally {
      socket.close();
    }
  },
};
