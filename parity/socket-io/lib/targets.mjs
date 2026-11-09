// Targets (what a scenario runs against), capability rules and process management.
import { spawn } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

export const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
export const ALL_CAPABILITIES = ["engine-io", "socket-io", "redis"];
export const STEP_TIMEOUT_MS = 10_000;

const NPM_CI_HINT = "run `npm ci` in parity/socket-io/";

/**
 * Refuses stale installs: the installed `socket.io` and `engine.io` must equal the versions pinned in package.json.
 * Returns the list of problems (empty when everything matches).
 */
export function checkInstalledVersions(root = ROOT) {
  const pinned = JSON.parse(readFileSync(join(root, "package.json"), "utf8")).dependencies ?? {};
  const problems = [];
  for (const name of ["socket.io", "engine.io"]) {
    const file = join(root, "node_modules", name, "package.json");
    if (!existsSync(file)) {
      problems.push(`${name} is not installed (${NPM_CI_HINT})`);
      continue;
    }
    const installed = JSON.parse(readFileSync(file, "utf8")).version;
    if (installed !== pinned[name]) {
      problems.push(`${name} ${installed} is installed but package.json pins ${pinned[name]} (${NPM_CI_HINT})`);
    }
  }
  return problems;
}

const children = new Set();

function terminate(child) {
  if (child.exitCode === null && child.signalCode === null) child.kill();
}

let hooked = false;
function hookCleanup() {
  if (hooked) return;
  hooked = true;
  process.on("exit", () => children.forEach(terminate));
  for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
    process.on(signal, () => {
      children.forEach(terminate);
      process.exit(130);
    });
  }
}

/** Starts a Node reference server and resolves once it has reported its port. */
export function startNodeTarget(name, { ackDelay = 0, root = ROOT } = {}) {
  hookCleanup();
  const args = [join(root, "reference", "socket-server.mjs")];
  if (ackDelay > 0) args.push("--ack-delay", String(ackDelay));
  const child = spawn(process.execPath, args, { stdio: ["pipe", "pipe", "pipe"], windowsHide: true });
  children.add(child);
  let stderr = "";
  child.stderr.on("data", (chunk) => (stderr += chunk));

  const exited = new Promise((resolve) => {
    child.once("exit", () => {
      children.delete(child);
      resolve();
    });
  });

  const ready = new Promise((resolve, reject) => {
    let buffered = "";
    const timer = setTimeout(() => reject(new Error(`reference server ${name} did not report a port in time`)), STEP_TIMEOUT_MS);
    child.stdout.on("data", (chunk) => {
      buffered += chunk;
      const newline = buffered.indexOf("\n");
      if (newline === -1) return;
      clearTimeout(timer);
      try {
        const { address, port } = JSON.parse(buffered.slice(0, newline));
        if (address !== "127.0.0.1" || !Number.isInteger(port) || port <= 0) {
          throw new Error(`unexpected address ${address}:${port}`);
        }
        resolve({ address, port });
      } catch (error) {
        reject(new Error(`reference server ${name} printed an invalid first line: ${error.message}`));
      }
    });
    child.once("exit", (code) => {
      clearTimeout(timer);
      reject(new Error(`reference server ${name} exited with code ${code}: ${stderr.trim()}`));
    });
  });

  return ready.then(
    ({ address, port }) => ({
      name,
      kind: "node",
      url: `http://${address}:${port}`,
      address,
      port,
      capabilities: [...ALL_CAPABILITIES],
      async stop() {
        child.stdin.end();
        const force = setTimeout(() => terminate(child), 2000);
        await exited;
        clearTimeout(force);
      },
    }),
    async (error) => {
      terminate(child);
      await exited;
      throw error;
    },
  );
}

/** A Corvis server that is already running (started by the Rust test, or by hand). */
export function corvisTarget(url, capabilities) {
  return { name: "corvis", kind: "corvis", url, capabilities, stop: async () => {} };
}

/**
 * Decides where a scenario runs. `corvisTargetOrNull` is the second target (Corvis, or a second Node server when there
 * is no Corvis target). A scenario that needs `redis` runs only with `CORVIS_REDIS_URL`; a scenario whose capabilities
 * the second target lacks runs on Node only and is reported as skipped on it.
 */
export function plan(scenario, { env = process.env, second } = {}) {
  const requires = scenario.requires ?? [];
  if (requires.includes("redis") && !env.CORVIS_REDIS_URL) {
    return { run: [], skipped: [{ target: "all", reason: "needs CORVIS_REDIS_URL" }] };
  }
  const missing = requires.filter((c) => !second.capabilities.includes(c));
  if (missing.length > 0) {
    return { run: ["node"], skipped: [{ target: second.name, reason: `lacks capability ${missing.join(", ")}` }] };
  }
  return { run: ["node", second.name], skipped: [] };
}

export function withTimeout(promise, label, ms = STEP_TIMEOUT_MS) {
  let timer;
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`step "${label}" timed out after ${ms} ms`)), ms);
  });
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}

/** Runs one scenario against one target and returns its transcript (no timings). */
export async function runScenario(scenario, target) {
  const entries = [];
  const record = (client, kind, data = null) => entries.push({ client, kind, data: structuredClone(data) });
  const step = (label, promise) => withTimeout(Promise.resolve(promise), label);
  await scenario.run({ url: target.url, record, step });
  return { scenario: scenario.name, entries };
}
