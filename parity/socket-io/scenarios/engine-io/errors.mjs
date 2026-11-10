// ws/engine-io: "Error codes", "Duplicate and mismatched parameters" and "Rejected upgrade": the same invalid requests
// to both servers, status, content type and body compared.
import { openPolling, polling, raw, rawSocket, statusAndBody } from "../../lib/engine-client.mjs";

export default {
  name: "engine-io/errors",
  requires: ["engine-io"],
  async run({ url, record, step }) {
    const cases = [
      ["eio-3", `${url}/socket.io/?EIO=3&transport=polling`],
      ["no-eio", `${url}/socket.io/?transport=polling`],
      ["unknown-transport", `${url}/socket.io/?EIO=4&transport=flashsocket`],
      ["unknown-sid", polling(url, "/socket.io/", "&sid=aaaaaaaaaaaaaaaaaaaa")],
      ["websocket-without-upgrade", `${url}/socket.io/?EIO=4&transport=websocket`],
      ["duplicate-eio", `${url}/socket.io/?EIO=4&EIO=4&transport=polling`],
    ];
    for (const [label, target] of cases) {
      const response = await raw(step, label, target);
      // A successful handshake opens a session; only its status matters here.
      const body = response.status === 200 ? "<open packet>" : response.text;
      record("raw", label, { status: response.status, contentType: response.contentType, body });
    }

    const post = await raw(step, "post-handshake", polling(url), { method: "POST", body: "4x" });
    record("raw", "post-handshake", { status: post.status, contentType: post.contentType, body: post.text });

    const { sid } = await openPolling(step, url);
    const mismatch = await raw(step, "mismatch", `${url}/socket.io/?EIO=4&transport=websocket&sid=${sid}`);
    record("raw", "mismatch", { status: mismatch.status, contentType: mismatch.contentType, body: mismatch.text });

    const upgrade = [
      "GET /socket.io/?EIO=4&transport=websocket&sid=aaaaaaaaaaaaaaaaaaaa HTTP/1.1",
      "Host: localhost",
      "Upgrade: websocket",
      "Connection: Upgrade",
      "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==",
      "Sec-WebSocket-Version: 13",
      "",
      "",
    ].join("\r\n");
    const answer = await step("rejected upgrade", rawSocket(url, upgrade));
    record("raw", "rejected-upgrade", statusAndBody(answer));
  },
};
