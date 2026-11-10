// Helpers of the Socket.IO scenarios: the control route of the scenario app and a raw Socket.IO client over the
// WebSocket transport (so a scenario can send bytes the official client never would). Waits are on events or on a
// bounded poll of the control route, never on a fixed sleep.
import { setTimeout as delay } from "node:timers/promises";
import WebSocket from "ws";

export const SIO = "/sio/";

/** Clears the server-side log of the scenario app. */
export async function resetLog(step, url) {
  await step("reset the log", fetch(`${url}/harness/reset`, { method: "POST" }));
}

export async function readLog(step, url) {
  const response = await step("read the log", fetch(`${url}/harness/log`));
  return JSON.parse(await step("log body", response.text()));
}

/** Polls the log until `predicate(entries)` holds (bounded) and returns the entries. */
export async function waitForLog(step, url, predicate, { waitMs = 5000 } = {}) {
  const deadline = Date.now() + waitMs;
  for (;;) {
    const entries = await readLog(step, url);
    if (predicate(entries) || Date.now() >= deadline) return entries;
    await delay(25);
  }
}

/** Log entries in a stable order: the scenarios compare sets of events, not the interleaving of sockets. */
export function sortedLog(entries) {
  return [...entries].sort((a, b) => JSON.stringify(a).localeCompare(JSON.stringify(b)));
}

/**
 * A raw Engine.IO v4 client over WebSocket speaking Socket.IO packets as text: `send("0/admin,")` sends the Engine.IO
 * message `40/admin,`. Pings are answered. `messages` holds everything received, as `{ text }` or `{ binary }`.
 */
export class RawSio {
  static async connect(url, path = SIO) {
    const ws = new WebSocket(`${url.replace(/^http/, "ws")}${path}?EIO=4&transport=websocket`);
    const client = new RawSio(ws);
    await client.opened;
    await client.waitFor((m) => m.text !== undefined && m.text.startsWith("0"), "the Engine.IO open packet");
    client.messages.length = 0;
    return client;
  }

  constructor(ws) {
    this.ws = ws;
    this.messages = [];
    this.closedAt = null;
    this.waiters = [];
    this.opened = new Promise((resolve, reject) => {
      ws.once("open", resolve);
      ws.once("error", reject);
    });
    this.closed = new Promise((resolve) => {
      ws.once("close", (code) => {
        this.closedAt = Date.now();
        this.closeCode = code;
        this.notify();
        resolve(code);
      });
    });
    ws.on("error", () => {});
    ws.on("message", (data, isBinary) => {
      if (isBinary) {
        this.push({ binary: Buffer.from(data).toString("hex") });
        return;
      }
      const text = data.toString();
      if (text === "2") {
        ws.send("3");
        return;
      }
      // Engine.IO message packets carry a Socket.IO packet after the `4`; anything else (open, noop) is kept whole.
      this.push({ text: text.startsWith("4") ? text.slice(1) : text, engine: !text.startsWith("4") });
    });
  }

  push(message) {
    this.messages.push(message);
    this.notify();
  }

  notify() {
    for (const waiter of this.waiters.splice(0)) waiter();
  }

  /** Sends a Socket.IO packet as an Engine.IO message. */
  send(socketIoPacket) {
    this.ws.send(`4${socketIoPacket}`);
  }

  sendBinary(buffer) {
    this.ws.send(buffer, { binary: true });
  }

  /** Resolves when `predicate(message)` holds for some received message (or the socket closed); returns it. */
  async waitFor(predicate, label, ms = 5000) {
    const deadline = Date.now() + ms;
    for (;;) {
      const found = this.messages.find(predicate);
      if (found) return found;
      if (this.closedAt !== null || Date.now() >= deadline) return null;
      await new Promise((resolve) => {
        this.waiters.push(resolve);
        setTimeout(resolve, 50);
      });
    }
  }

  /** Joins a namespace and returns the CONNECT answer (a `0...` packet, or a `4...` error). */
  async join(nsp = "/", auth) {
    const body = auth === undefined ? "" : JSON.stringify(auth);
    this.send(nsp === "/" ? `0${body}` : `0${nsp},${body}`);
    const answer = await this.waitFor((m) => m.text !== undefined && /^[04]/.test(m.text), "the CONNECT answer");
    if (answer) this.messages.splice(this.messages.indexOf(answer), 1);
    return answer;
  }

  /** Waits for the connection to close (bounded). Returns whether it did. */
  async waitClosed(ms = 5000) {
    if (this.closedAt !== null) return true;
    await Promise.race([this.closed, delay(ms)]);
    return this.closedAt !== null;
  }

  close() {
    this.ws.terminate();
  }
}

/** Replaces the socket id of a CONNECT answer so two servers compare equal. */
export function withoutSid(text) {
  return text.replace(/"sid":"[^"]*"/, '"sid":"<sid>"');
}
