// Helpers of the Engine.IO wire scenarios: raw HTTP requests, payload decoding and the control route of the
// reference servers. Waits are on events or on a bounded poll of the control route, never on a fixed sleep.
import net from "node:net";
import { setTimeout as delay } from "node:timers/promises";

export const RECORD_SEPARATOR = "\x1e";
const PACKET_TYPES = ["open", "close", "ping", "pong", "message", "upgrade", "noop"];

export function decodePayload(text) {
  return text.split(RECORD_SEPARATOR).map((raw) => {
    if (raw[0] === "b") return { type: "message", data: { binaryBase64: raw.slice(1) } };
    const type = PACKET_TYPES[Number(raw[0])] ?? `unknown:${raw[0]}`;
    const body = raw.slice(1);
    if (type === "open") return { type, data: JSON.parse(body) };
    return { type, data: body };
  });
}

export const polling = (url, preset = "/socket.io/", extra = "") => `${url}${preset}?EIO=4&transport=polling${extra}`;

/** One raw request; the answer reduced to what a scenario compares. */
export async function raw(step, label, url, { method = "GET", headers = {}, body } = {}) {
  const response = await step(label, fetch(url, { method, headers, body }));
  const text = await step(`${label} body`, response.text());
  return {
    status: response.status,
    contentType: response.headers.get("content-type"),
    cacheControl: response.headers.get("cache-control"),
    contentEncoding: response.headers.get("content-encoding"),
    allowOrigin: response.headers.get("access-control-allow-origin"),
    text,
  };
}

/** Opens a polling session and returns its id and the open packet. */
export async function openPolling(step, url, preset) {
  const response = await raw(step, "handshake", polling(url, preset));
  if (response.status !== 200) throw new Error(`handshake answered ${response.status}: ${response.text}`);
  const [open] = decodePayload(response.text);
  return { sid: open.data.sid, open: open.data, response };
}

/** The reason the reference server closed the session with, or `open`. Bounded wait for it to change. */
export async function closeReason(step, url, sid, { waitMs = 5000 } = {}) {
  const deadline = Date.now() + waitMs;
  for (;;) {
    const response = await step("close reason", fetch(`${url}/harness/close-reason/${sid}`));
    const reason = await step("close reason body", response.text());
    if (reason !== "open" || Date.now() >= deadline) return reason;
    await delay(50);
  }
}

/** An HTTP/1.1 request written by hand, read until the server closes the connection. */
export function rawSocket(url, request, { timeoutMs = 5000 } = {}) {
  const { hostname, port } = new URL(url);
  return new Promise((resolve, reject) => {
    const socket = net.connect({ host: hostname, port: Number(port) });
    const chunks = [];
    const timer = setTimeout(() => {
      socket.destroy();
      reject(new Error("the raw socket did not close in time"));
    }, timeoutMs);
    socket.on("connect", () => socket.write(request));
    socket.on("data", (chunk) => chunks.push(chunk));
    socket.on("error", () => {});
    socket.on("close", () => {
      clearTimeout(timer);
      resolve(Buffer.concat(chunks).toString("latin1"));
    });
  });
}

/** Status line and body of a response read with `rawSocket`. */
export function statusAndBody(text) {
  const [head, ...rest] = text.split("\r\n\r\n");
  return { statusLine: head.split("\r\n")[0], body: rest.join("\r\n\r\n") };
}
