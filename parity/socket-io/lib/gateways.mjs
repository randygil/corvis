// Helpers of the gateway scenarios (capability `gateways`): an official client for a gateway namespace on the default
// Engine.IO path, and the marker pattern. The reference application and the Corvis application both have a handler
// `mark` on every namespace used here that emits the event `done`; a scenario sees the absence of a reply by what
// arrived before `done`, never by waiting.
import { client, next } from "./official-client.mjs";

export const GATEWAY_PATH = "/socket.io/";

/** An official client for a namespace of the gateway application (`nsp` like "/rules"). */
export function gateway(url, nsp = "", options = {}) {
  return client(url, nsp, { path: GATEWAY_PATH, ...options });
}

/** Collects every event the client receives (the acknowledgements are not events). */
export function watch(socket) {
  const seen = [];
  socket.onAny((event, ...args) => seen.push({ event, args }));
  return seen;
}

/**
 * Asks the server to emit the marker and returns what arrived up to and including it. Everything a handler did for the
 * events sent before has been sent by then: events of one connection start in order.
 */
export async function settle(step, socket, seen, label = "marker") {
  const done = next(step, socket, "done", label);
  socket.emit("mark");
  await done;
  return seen.splice(0);
}

/** Emits with a callback that records whether it was called and with what, and returns the state for later. */
export function askAndWatch(socket, event, ...args) {
  const state = { called: false, args: null };
  socket.emit(event, ...args, (...answer) => {
    state.called = true;
    state.args = answer;
  });
  return state;
}
