// A raw polling handshake, then the official client connecting with an auth object, emitting one event with
// arguments and an acknowledgement, and disconnecting. Waits are on events, never sleeps.
import { io } from "socket.io-client";

const RECORD_SEPARATOR = "\x1e";
const PACKET_TYPES = ["open", "close", "ping", "pong", "message", "upgrade", "noop"];

function decodePayload(text) {
  return text.split(RECORD_SEPARATOR).map((raw) => {
    const type = PACKET_TYPES[Number(raw[0])] ?? `unknown:${raw[0]}`;
    const body = raw.slice(1);
    if (type === "open") return { type, data: JSON.parse(body) };
    return { type, data: body };
  });
}

export default {
  name: "socket-io/handshake-and-ack",
  requires: ["socket-io"],
  async run({ url, record, step }) {
    const response = await step("raw polling handshake", fetch(`${url}/sio/?EIO=4&transport=polling`));
    const text = await step("raw polling body", response.text());
    record("raw", "http-response", {
      status: response.status,
      contentType: response.headers.get("content-type"),
      cacheControl: response.headers.get("cache-control"),
      packets: decodePayload(text),
    });

    const socket = io(url, { path: "/sio/", auth: { token: "t" }, forceNew: true, reconnection: false });
    try {
      const outcome = await step(
        "connect",
        new Promise((resolve) => {
          socket.once("connect", () => resolve("connect"));
          socket.once("connect_error", (error) => resolve(error));
        }),
      );
      if (outcome !== "connect") {
        record("client", "connect_error", { message: outcome.message });
        return;
      }
      record("client", "connect", { sid: socket.id, engine: { sid: socket.io.engine.id } });

      const ack = new Promise((resolve) => socket.emit("echo", "hello", 42, (...args) => resolve(args)));
      record("client", "ack", { args: await step("acknowledgement", ack) });

      const closed = new Promise((resolve) => socket.once("disconnect", resolve));
      socket.disconnect();
      record("client", "disconnect", { reason: await step("disconnect", closed) });
    } finally {
      socket.close();
    }
  },
};
