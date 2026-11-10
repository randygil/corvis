// ws/engine-io: "No CORS by default" and "Configured origin".
import { polling, raw } from "../../lib/engine-client.mjs";

async function preflight(step, label, target, origin) {
  const response = await step(
    label,
    fetch(target, {
      method: "OPTIONS",
      headers: { origin, "access-control-request-method": "GET", "access-control-request-headers": "content-type" },
    }),
  );
  const body = await step(`${label} body`, response.text());
  return {
    status: response.status,
    allowOrigin: response.headers.get("access-control-allow-origin"),
    allowMethods: response.headers.get("access-control-allow-methods"),
    allowHeaders: response.headers.get("access-control-allow-headers"),
    vary: response.headers.get("vary"),
    body,
  };
}

export default {
  name: "engine-io/cors",
  requires: ["engine-io"],
  async run({ url, record, step }) {
    // Default options: no CORS headers, and an OPTIONS handshake is a bad handshake method.
    record("default", "preflight", await preflight(step, "default preflight", polling(url), "https://evil.example"));
    const plain = await raw(step, "default handshake", polling(url), { headers: { origin: "https://evil.example" } });
    record("default", "handshake", { status: plain.status, allowOrigin: plain.allowOrigin });

    // Preset `cors`: origin https://app.example.
    record("configured", "preflight", await preflight(step, "configured preflight", polling(url, "/cors/"), "https://app.example"));
    const allowed = await raw(step, "allowed handshake", polling(url, "/cors/"), { headers: { origin: "https://app.example" } });
    record("configured", "handshake", { status: allowed.status, allowOrigin: allowed.allowOrigin });
    const other = await raw(step, "other handshake", polling(url, "/cors/"), { headers: { origin: "https://evil.example" } });
    record("configured", "other-origin", { status: other.status, allowOrigin: other.allowOrigin });
  },
};
