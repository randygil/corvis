import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { diff } from "../lib/compare.mjs";
import { normalize } from "../lib/normalize.mjs";
import { checkInstalledVersions, corvisTarget, plan, runScenario, startNodeTarget } from "../lib/targets.mjs";
import handshake from "../scenarios/socket-io/handshake-and-ack.mjs";

test("Corvis target without the capability: runs on Node only and is reported as skipped", () => {
  const result = plan(handshake, { env: {}, second: corvisTarget("http://127.0.0.1:1", ["engine-io"]) });
  assert.deepEqual(result.run, ["node"]);
  assert.equal(result.skipped[0].target, "corvis");
  assert.match(result.skipped[0].reason, /socket-io/);
});

test("a Corvis target with the capability runs the scenario on both targets", () => {
  const result = plan(handshake, { env: {}, second: corvisTarget("http://127.0.0.1:1", ["socket-io"]) });
  assert.deepEqual(result.run, ["node", "corvis"]);
  assert.deepEqual(result.skipped, []);
});

test("Redis scenario without Redis: skipped, whatever the targets can do", () => {
  const scenario = { name: "r", requires: ["redis"], run() {} };
  const second = { name: "node-2", capabilities: ["engine-io", "socket-io", "redis"] };
  assert.deepEqual(plan(scenario, { env: {}, second }).run, []);
  assert.match(plan(scenario, { env: {}, second }).skipped[0].reason, /CORVIS_REDIS_URL/);
  assert.deepEqual(plan(scenario, { env: { CORVIS_REDIS_URL: "redis://127.0.0.1:6379" }, second }).run, ["node", "node-2"]);
});

test("Stale node_modules: a version other than the pinned one is refused with the npm ci message", () => {
  const root = mkdtempSync(join(tmpdir(), "socket-io-parity-"));
  try {
    writeFileSync(join(root, "package.json"), JSON.stringify({ dependencies: { "socket.io": "4.8.4", "engine.io": "6.6.11" } }));
    for (const [name, version] of [["socket.io", "4.8.3"], ["engine.io", "6.6.11"]]) {
      mkdirSync(join(root, "node_modules", name), { recursive: true });
      writeFileSync(join(root, "node_modules", name, "package.json"), JSON.stringify({ version }));
    }
    const problems = checkInstalledVersions(root);
    assert.equal(problems.length, 1);
    assert.match(problems[0], /socket\.io 4\.8\.3 is installed but package\.json pins 4\.8\.4.*npm ci/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("the installed packages match package.json", () => {
  assert.deepEqual(checkInstalledVersions(), []);
});

test("Loopback address: the reference server reports 127.0.0.1 and a non-zero port", async () => {
  const target = await startNodeTarget("loopback");
  try {
    assert.equal(target.address, "127.0.0.1");
    assert.ok(target.port > 0);
  } finally {
    await target.stop();
  }
});

test("Slow machine: a two second late acknowledgement gives the same transcript", { timeout: 60_000 }, async () => {
  const fast = await startNodeTarget("fast");
  const slow = await startNodeTarget("slow", { ackDelay: 2000 });
  try {
    const a = await runScenario(handshake, fast);
    const b = await runScenario(handshake, slow);
    assert.deepEqual(diff(normalize(a), normalize(b)), []);
    assert.deepEqual(b.entries.find((e) => e.kind === "ack").data.args, ["hello", 42]);
  } finally {
    await Promise.all([fast.stop(), slow.stop()]);
  }
});
