// Connect middleware: a refusal reaches `connect_error` with its message and data, a pass connects, and on the wire the
// CONNECT_ERROR has a `data` key only when the error carries data.
import { client, connected } from "../../lib/official-client.mjs";
import { RawSio } from "../../lib/socket-client.mjs";

export default {
  name: "socket-io/middleware",
  requires: ["socket-io"],
  async run({ url, record, step }) {
    const refused = client(url, "/admin", { auth: { token: "nope" } });
    try {
      const outcome = await connected(step, refused, "refused by middleware");
      record("refused", "connect_error", { message: outcome.message, data: outcome.data ?? null });
    } finally {
      refused.close();
    }

    const allowed = client(url, "/admin", { auth: { token: "admin" } });
    try {
      const outcome = await connected(step, allowed, "accepted by middleware");
      record("allowed", "connect", { outcome: outcome === "connect" ? "connect" : outcome.message });
    } finally {
      allowed.close();
    }

    // The CONNECT_ERROR as it is written.
    const raw = await RawSio.connect(url);
    try {
      const withData = await raw.join("/admin", { token: "x" });
      record("raw", "with-data", { text: withData?.text ?? null });
      const withoutData = await raw.join("/denied");
      record("raw", "without-data", { text: withoutData?.text ?? null });
    } finally {
      raw.close();
    }
  },
};
