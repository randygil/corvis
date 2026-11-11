// `cors` declared on a gateway: the preflight and the polling response carry the CORS headers of the reference gateway.
const HEADERS = [
  "access-control-allow-origin",
  "access-control-allow-credentials",
  "access-control-allow-methods",
  "access-control-allow-headers",
  "access-control-expose-headers",
  "access-control-max-age",
  "vary",
];

function picked(response) {
  const headers = {};
  for (const name of HEADERS) {
    const value = response.headers.get(name);
    if (value !== null) headers[name] = value;
  }
  return { status: response.status, headers };
}

export default {
  name: "gateways/wire/cors",
  requires: ["gateways"],
  async run({ url, record, step }) {
    const polling = "EIO=4&transport=polling";
    const preflight = await step(
      "preflight",
      fetch(`${url}/cors-one/?${polling}`, {
        method: "OPTIONS",
        headers: { origin: "https://app.example", "access-control-request-method": "GET" },
      }),
    );
    record("client", "one-preflight", picked(preflight));
    const one = await step("polling with the origin", fetch(`${url}/cors-one/?${polling}`, { headers: { origin: "https://app.example" } }));
    record("client", "one-polling", picked(one));
    const any = await step("polling for any origin", fetch(`${url}/cors-any/?${polling}`, { headers: { origin: "https://whoever.example" } }));
    record("client", "any-polling", picked(any));
    const none = await step("a path without cors", fetch(`${url}/socket.io/?${polling}`, { headers: { origin: "https://app.example" } }));
    record("client", "none-polling", picked(none));
  },
};
