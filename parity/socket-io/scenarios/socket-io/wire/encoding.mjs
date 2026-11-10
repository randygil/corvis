// What the servers write, byte for byte: events with several argument types, an acknowledgement id, a string holding
// the record separator, a CONNECT answer and an Engine.IO session over polling carrying the same string.
import { RawSio, SIO, resetLog, withoutSid } from "../../../lib/socket-client.mjs";

const SEP = "\x1e";

export default {
  name: "socket-io/wire/encoding",
  requires: ["socket-io"],
  async run({ url, record, step }) {
    await resetLog(step, url);
    const raw = await step("raw connection", RawSio.connect(url));
    try {
      const answer = await raw.join("/", { k: 1 });
      record("raw", "connect-answer", { text: withoutSid(answer.text) });

      raw.send("0/admin,");
      const refused = await raw.waitFor((m) => m.text?.startsWith("4/admin,"));
      record("raw", "connect-error", { text: refused?.text ?? null });
      raw.messages.length = 0;

      raw.send('2["emit-update"]');
      const update = await raw.waitFor((m) => m.text?.startsWith('2["update"'));
      record("raw", "update", { text: update?.text ?? null });

      const values = '{"a":[1,2,{"b":null}],"c":"é","d":true,"e":-3,"f":"","g":{}}';
      raw.send(`29["echo",${values},"x\\u001ey"]`);
      const echoed = await raw.waitFor((m) => m.text?.startsWith("39"));
      record("raw", "echo-ack", { text: echoed?.text ?? null, hasSeparator: echoed?.text.includes(SEP) ?? null });
    } finally {
      raw.close();
    }

    // The same string over polling: one record, the separator escaped.
    const handshake = await step("polling handshake", fetch(`${url}${SIO}?EIO=4&transport=polling`));
    const open = (await step("open body", handshake.text())).slice(1);
    const sid = JSON.parse(open).sid;
    const post = (body) => fetch(`${url}${SIO}?EIO=4&transport=polling&sid=${sid}`, { method: "POST", body });
    const poll = async () => (await step("poll", fetch(`${url}${SIO}?EIO=4&transport=polling&sid=${sid}`))).text();
    await step("post connect", post("40"));
    record("polling", "connect", { text: withoutSid(await poll()) });
    await step("post echo", post('429["echo","x\\u001ey"]'));
    const payload = await poll();
    record("polling", "echo-ack", { payload, records: payload.split(SEP).length });
    await step("close", post("41"));
  },
};
