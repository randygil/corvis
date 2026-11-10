// ws/engine-io: the bounds Corvis keeps where Node has none, and the ones both share. Each observable difference has
// an entry in expected-differences.json: packets per POST, bytes per poll response, the send buffer, the session cap
// and compression of polling responses. The WebSocket message bound is the same on both.
import WebSocket from "ws";
import { closeReason, openPolling, polling, raw } from "../../lib/engine-client.mjs";

const headers = { "content-type": "text/plain;charset=UTF-8" };

async function closeSession(step, url, preset, sid) {
  await raw(step, "cleanup", polling(url, preset, `&sid=${sid}`), { method: "POST", headers, body: "1" }).catch(() => {});
}

export default {
  name: "engine-io/bounds",
  requires: ["engine-io"],
  async run({ url, record, step }) {
    // Packet flood in one POST: 10,001 one-byte message packets, well under maxHttpBufferSize.
    {
      const { sid } = await openPolling(step, url);
      const target = polling(url, "/socket.io/", `&sid=${sid}`);
      const flood = await raw(step, "flood", target, { method: "POST", headers, body: Array(10_001).fill("4").join("\x1e") });
      record("flood", "post", { status: flood.status });
      // Node delivers every packet and keeps the session; Corvis refuses the body and closes it.
      record("flood", "reason", { reason: await closeReason(step, url, sid, { waitMs: 1000 }) });
      await closeSession(step, url, "/socket.io/", sid);
    }

    // Large backlog: 3 MB queued for a session that then keeps polling.
    {
      const { sid } = await openPolling(step, url);
      const target = polling(url, "/socket.io/", `&sid=${sid}`);
      await raw(step, "flood command", target, { method: "POST", headers, body: "4flood:3000000" });
      let messages = 0;
      let polls = 0;
      let ordered = true;
      let largest = 0;
      while (messages < 30) {
        const poll = await raw(step, "backlog poll", target);
        polls += 1;
        largest = Math.max(largest, poll.text.length);
        for (const packet of poll.text.split("\x1e")) {
          ordered &&= packet === `4${"x".repeat(100_000)}`;
          messages += 1;
        }
      }
      record("backlog", "received", { messages, ordered, polls, withinLimit: largest <= 1_000_000 + 100_001 });
      await closeSession(step, url, "/socket.io/", sid);
    }

    // Slow consumer: 9.5 MB queued for a client that never polls.
    {
      const { sid } = await openPolling(step, url);
      const target = polling(url, "/socket.io/", `&sid=${sid}`);
      await raw(step, "slow consumer flood", target, { method: "POST", headers, body: "4flood:9500000" });
      record("slow-consumer", "reason", { reason: await closeReason(step, url, sid, { waitMs: 2000 }) });
      await closeSession(step, url, "/socket.io/", sid);
    }

    // Session cap (preset `cap`): a third handshake while two sessions are open.
    {
      const first = await openPolling(step, url, "/cap/");
      const second = await openPolling(step, url, "/cap/");
      const third = await raw(step, "third handshake", polling(url, "/cap/"));
      record("cap", "third", { status: third.status });
      // After one session closes, a new handshake succeeds (on Node it is simply a third open session).
      await closeSession(step, url, "/cap/", first.sid);
      const fourth = await raw(step, "fourth handshake", polling(url, "/cap/"));
      record("cap", "fourth", { status: fourth.status });
      await closeSession(step, url, "/cap/", second.sid);
    }

    // Sessions per address (preset `ipcap`: 2): every request here comes from 127.0.0.1.
    {
      const first = await openPolling(step, url, "/ipcap/");
      const second = await openPolling(step, url, "/ipcap/");
      const third = await raw(step, "third handshake from the address", polling(url, "/ipcap/"));
      record("ip-cap", "third", { status: third.status, body: third.status === 503 ? third.text : null });
      await closeSession(step, url, "/ipcap/", first.sid);
      const fourth = await raw(step, "fourth handshake from the address", polling(url, "/ipcap/"));
      record("ip-cap", "fourth", { status: fourth.status });
      await closeSession(step, url, "/ipcap/", second.sid);
    }

    // A polling session that never makes a second request (preset `first`: Corvis closes it after 400 ms).
    {
      const { sid } = await openPolling(step, url, "/first/");
      record("first-request", "silent", { reason: await closeReason(step, url, sid, { waitMs: 1500 }) });
    }

    // Compression: a 2,000-byte polling response to a client that accepts gzip.
    {
      const { sid } = await openPolling(step, url);
      const target = polling(url, "/socket.io/", `&sid=${sid}`);
      await raw(step, "echo big", target, { method: "POST", headers, body: `4${"y".repeat(2000)}` });
      const poll = await raw(step, "compressed poll", target, { headers: { "accept-encoding": "gzip" } });
      record("compression", "poll", { status: poll.status, contentEncoding: poll.contentEncoding, length: poll.text.length });
      await closeSession(step, url, "/socket.io/", sid);
    }

    // WebSocket message above maxHttpBufferSize: the connection is closed with 1009.
    {
      const ws = new WebSocket(`${url.replace("http", "ws")}/socket.io/?EIO=4&transport=websocket`);
      const sid = await step(
        "websocket open",
        new Promise((resolve, reject) => {
          ws.once("message", (data) => resolve(JSON.parse(data.toString().slice(1)).sid));
          ws.once("error", reject);
        }),
      );
      const closed = new Promise((resolve) => ws.once("close", (code) => resolve(code)));
      ws.send(`4${"z".repeat(1_000_000)}`);
      record("websocket", "oversized", { code: await step("websocket close", closed), reason: await closeReason(step, url, sid) });
    }
  },
};
