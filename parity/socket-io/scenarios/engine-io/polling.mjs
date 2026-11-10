// ws/engine-io: "Several packets in one poll", "Overlapping polls", "Oversized POST" and "Client close".
import { setTimeout as delay } from "node:timers/promises";
import { closeReason, openPolling, polling, raw } from "../../lib/engine-client.mjs";

const headers = { "content-type": "text/plain;charset=UTF-8" };

export default {
  name: "engine-io/polling",
  requires: ["engine-io"],
  async run({ url, record, step }) {
    // Several packets in one poll: the server queues three messages (one binary) while no poll is pending.
    {
      const { sid } = await openPolling(step, url);
      const post = await raw(step, "burst", polling(url, "/socket.io/", `&sid=${sid}`), { method: "POST", headers, body: "4burst" });
      record("burst", "post", { status: post.status, contentType: post.contentType, cacheControl: post.cacheControl, body: post.text });
      const poll = await raw(step, "burst poll", polling(url, "/socket.io/", `&sid=${sid}`));
      record("burst", "poll", { status: poll.status, contentType: poll.contentType, cacheControl: poll.cacheControl, body: poll.text });
      await raw(step, "burst close", polling(url, "/socket.io/", `&sid=${sid}`), { method: "POST", headers, body: "1" });
    }

    // Overlapping polls: the second GET is a 400 and the session closes with `transport error`.
    {
      const { sid } = await openPolling(step, url);
      const target = polling(url, "/socket.io/", `&sid=${sid}`);
      const first = raw(step, "pending poll", target);
      // Nothing tells a client that the server holds its poll: a short, fixed head start (a lower bound, far above a
      // loopback round trip) puts the first request ahead of the second.
      await delay(300);
      const second = await raw(step, "overlapping poll", target);
      record("overlap", "second", { status: second.status, body: second.text });
      const answered = await first;
      record("overlap", "first", { status: answered.status, body: answered.text });
      record("overlap", "reason", { reason: await closeReason(step, url, sid) });
    }

    // Oversized POST: 413 and the session keeps working (preset `small`, maxHttpBufferSize 1000).
    {
      const { sid } = await openPolling(step, url, "/small/");
      const target = polling(url, "/small/", `&sid=${sid}`);
      const big = await raw(step, "oversized", target, { method: "POST", headers, body: `4${"x".repeat(1000)}` });
      record("oversized", "post", { status: big.status });
      const small = await raw(step, "after oversized", target, { method: "POST", headers, body: "4hi" });
      record("oversized", "next-post", { status: small.status, body: small.text });
      const echoed = await raw(step, "echo", target);
      record("oversized", "poll", { status: echoed.status, body: echoed.text });
      await raw(step, "oversized close", target, { method: "POST", headers, body: "1" });
    }

    // Client close: a close packet ends the session with `transport close`.
    {
      const { sid } = await openPolling(step, url);
      const post = await raw(step, "close", polling(url, "/socket.io/", `&sid=${sid}`), { method: "POST", headers, body: "1" });
      record("close", "post", { status: post.status, body: post.text });
      record("close", "reason", { reason: await closeReason(step, url, sid) });
    }
  },
};
