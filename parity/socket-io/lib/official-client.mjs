// Helpers of the scenarios that use the official `socket.io-client`.
import { io } from "socket.io-client";
import { SIO } from "./socket-client.mjs";

/** An official client for a namespace of the scenario app (`nsp` like "/admin"), not yet connected to anything else. */
export function client(url, nsp = "", options = {}) {
  return io(`${url}${nsp}`, { path: SIO, forceNew: true, reconnection: false, ...options });
}

/** Resolves with `"connect"` or with the `connect_error` error. */
export function connected(step, socket, label = "connect") {
  return step(
    label,
    new Promise((resolve) => {
      if (socket.connected) resolve("connect");
      socket.once("connect", () => resolve("connect"));
      socket.once("connect_error", (error) => resolve(error));
    }),
  );
}

/** Resolves with the arguments of the next `event`. */
export function next(step, socket, event, label = event) {
  return step(label, new Promise((resolve) => socket.once(event, (...args) => resolve(args))));
}

/** Resolves with the reason of the next `disconnect`. */
export function disconnected(step, socket, label = "disconnect") {
  return step(label, new Promise((resolve) => socket.once("disconnect", resolve)));
}

/** Emits with a callback and resolves with the acknowledgement arguments. */
export function ask(step, socket, label, event, ...args) {
  return step(label, new Promise((resolve) => socket.emit(event, ...args, (...answer) => resolve(answer))));
}
