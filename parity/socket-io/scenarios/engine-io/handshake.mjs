// ws/engine-io: "Polling handshake" and "WebSocket-first handshake". A raw polling handshake, then a WebSocket-first
// connection whose first frame is the open packet.
import WebSocket from "ws";
import { decodePayload, polling, raw } from "../../lib/engine-client.mjs";

export default {
  name: "engine-io/handshake",
  requires: ["engine-io"],
  async run({ url, record, step }) {
    const response = await raw(step, "polling handshake", polling(url));
    record("raw", "http-response", {
      status: response.status,
      contentType: response.contentType,
      cacheControl: response.cacheControl,
      packets: decodePayload(response.text),
    });

    const ws = new WebSocket(`${url.replace("http", "ws")}/socket.io/?EIO=4&transport=websocket`);
    try {
      const first = await step(
        "first websocket frame",
        new Promise((resolve, reject) => {
          ws.once("message", (data) => resolve(data.toString()));
          ws.once("error", reject);
        }),
      );
      record("ws", "frame", { packets: decodePayload(first) });
    } finally {
      ws.close();
    }
  },
};
