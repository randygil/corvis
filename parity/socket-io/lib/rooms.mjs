// Helpers of the room and broadcast scenarios: a crowd of named official clients, commands sent to the scenario app
// (events with an acknowledgement, see reference/socket-server.mjs), and a round: one command followed by the event
// `done` that the server sends to every namespace, so a scenario learns what each client received without waiting for
// time to pass. Socket ids are random: everything a scenario records has them replaced by the name of the client.
import { client, connected } from "./official-client.mjs";

const ADMIN_AUTH = { token: "admin" };

/** Replaces the id of every client in `value` by `<name>`. */
export function alias(crowd, value) {
  if (typeof value === "string") {
    for (const member of Object.values(crowd.members)) {
      if (member.socket.id && value === member.socket.id) return `<${member.name}>`;
    }
    return value;
  }
  if (Array.isArray(value)) return value.map((item) => alias(crowd, item));
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, alias(crowd, item)]));
  }
  return value;
}

/**
 * Connects official clients: `spec` maps a name to its namespace (`{ A: "/", B: "/", C: "/admin" }`). Each collects the
 * events it receives (but `done`) in `received`. A client named in `answers` answers the server's `question` with
 * `answers[name]`, after `delay` milliseconds when it is given.
 */
export async function crowd(step, url, spec, { answers = {}, delay = 0 } = {}) {
  const members = {};
  for (const [name, nsp] of Object.entries(spec)) {
    const socket = client(url, nsp === "/" ? "" : nsp, nsp === "/admin" ? { auth: ADMIN_AUTH } : {});
    const member = { name, nsp, socket, received: [] };
    socket.onAny((event, ...args) => {
      if (event !== "done") member.received.push([event, ...args]);
    });
    if (name in answers) {
      socket.on("question", (ack) => (delay > 0 ? setTimeout(() => ack(answers[name]), delay) : ack(answers[name])));
    }
    const outcome = await connected(step, socket, `connect ${name}`);
    if (outcome !== "connect") throw new Error(`${name} did not connect: ${outcome.message}`);
    members[name] = member;
  }
  const group = {
    members,
    close() {
      for (const member of Object.values(members)) member.socket.close();
    },
  };
  return group;
}

/** Sends a command from one client and resolves with the acknowledged value, with ids replaced by names. */
export async function command(group, step, from, event, arg) {
  const answer = await step(
    `${event} from ${from}`,
    new Promise((resolve) => (arg === undefined ? group.members[from].socket.emit(event, resolve) : group.members[from].socket.emit(event, arg, resolve))),
  );
  return alias(group, answer);
}

/**
 * One command, then the `done` event on every client in `waitFor` (all of them by default). Resolves with the answer
 * and what each client received meanwhile, in the order it arrived, ids replaced by names.
 */
export async function round(group, step, from, event, arg, waitFor = Object.keys(group.members)) {
  for (const member of Object.values(group.members)) member.received.length = 0;
  const dones = waitFor.map((name) => step(`done on ${name}`, new Promise((resolve) => group.members[name].socket.once("done", resolve))));
  const answer = await command(group, step, from, event, arg);
  await Promise.all(dones);
  const got = {};
  for (const member of Object.values(group.members)) got[member.name] = alias(group, member.received.splice(0));
  return { answer, got };
}

/** Sorts a list of strings, for answers whose order is not defined. */
export function sorted(list) {
  return [...list].sort();
}
