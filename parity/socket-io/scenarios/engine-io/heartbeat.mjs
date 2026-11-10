// ws/engine-io: "Client stops answering pings". Preset `fast`: pingInterval 300 ms, pingTimeout 200 ms. A raw client
// receives the first ping and never answers; the server closes the session with `ping timeout`, not before the
// interval plus the timeout and within a few seconds.
import { closeReason, decodePayload, openPolling, polling, raw } from "../../lib/engine-client.mjs";

export default {
  name: "engine-io/heartbeat",
  requires: ["engine-io"],
  async run({ url, record, step }) {
    const { sid, open } = await openPolling(step, url, "/fast/");
    const started = Date.now();
    record("raw", "open", { pingInterval: open.pingInterval, pingTimeout: open.pingTimeout });

    const poll = await raw(step, "ping poll", polling(url, "/fast/", `&sid=${sid}`));
    record("raw", "ping", { status: poll.status, packets: decodePayload(poll.text) });

    const reason = await closeReason(step, url, sid, { waitMs: 8000 });
    const elapsed = Date.now() - started;
    record("raw", "closed", { reason, notBeforeIntervalPlusTimeout: elapsed >= 450, within: elapsed < 8000 });
  },
};
