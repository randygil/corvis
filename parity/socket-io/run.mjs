// Runs every scenario against two targets, compares what the clients observed and exits non-zero on a difference.
//
//   node run.mjs [--corvis <url> --corvis-capabilities engine-io,socket-io] [--corvis-gateways <url>] [--scenario <substring>]
//
// Without --corvis the second target is another Node reference server (the harness then checks itself).
// `--corvis-gateways` is the Corvis application of the gateway scenarios (capability `gateways`); the scenarios that
// require it are reported as skipped on a Corvis target without it, and, when `gateway-reference/` is not installed,
// as skipped on Node too (with `CORVIS_NODE_DIFF=1` that is an error that names `npm ci`).
import { mkdirSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { pathToFileURL } from "node:url";
import { diff, evaluate, formatReport, loadExpected } from "./lib/compare.mjs";
import { normalize } from "./lib/normalize.mjs";
import {
  ALL_CAPABILITIES,
  GATEWAY_NPM_CI_HINT,
  ROOT,
  checkGatewayReference,
  checkInstalledVersions,
  corvisTarget,
  modeOf,
  plan,
  runScenario,
  startNodeTarget,
} from "./lib/targets.mjs";

function parseArgs(argv) {
  const options = { corvis: null, capabilities: [], corvisGateways: null, filter: null };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--corvis") options.corvis = argv[++i];
    else if (arg === "--corvis-capabilities") options.capabilities = (argv[++i] ?? "").split(",").filter(Boolean);
    else if (arg === "--corvis-gateways") options.corvisGateways = argv[++i];
    else if (arg === "--scenario") options.filter = argv[++i];
    else throw new Error(`unknown argument ${arg}`);
  }
  for (const c of options.capabilities) {
    if (!ALL_CAPABILITIES.includes(c)) throw new Error(`unknown capability ${c} (known: ${ALL_CAPABILITIES.join(", ")})`);
  }
  if (!options.corvis && options.capabilities.length > 0) throw new Error("--corvis-capabilities needs --corvis <url>");
  return options;
}

function findScenarioFiles(dir) {
  const files = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) files.push(...findScenarioFiles(path));
    else if (entry.name.endsWith(".mjs")) files.push(path);
  }
  return files.sort();
}

function writeTranscript(target, scenario, transcript) {
  const file = join(ROOT, "out", target, `${scenario}.json`);
  mkdirSync(dirname(file), { recursive: true });
  writeFileSync(file, `${JSON.stringify(transcript, null, 2)}\n`);
}

function line(scenario, target, status) {
  console.log(`${scenario.padEnd(40)} ${target.padEnd(8)} ${status}`);
}

async function main() {
  const options = parseArgs(process.argv.slice(2));

  const problems = checkInstalledVersions();
  if (problems.length > 0) {
    console.error(`the installed packages do not match package.json:\n  ${problems.join("\n  ")}`);
    return 1;
  }
  const expected = loadExpected(join(ROOT, "expected-differences.json"));

  const scenarios = [];
  for (const file of findScenarioFiles(join(ROOT, "scenarios"))) {
    const module = (await import(pathToFileURL(file))).default;
    if (!module?.name || typeof module.run !== "function") {
      throw new Error(`${relative(ROOT, file)} must export { name, requires, run }`);
    }
    if (!options.filter || module.name.includes(options.filter)) scenarios.push(module);
  }

  const started = [];
  try {
    // The reference servers of a mode (socket.io, engine.io) start when the first scenario of that mode runs.
    const corvis = options.corvis ? corvisTarget(options.corvis, options.capabilities) : null;
    const gatewayCorvis = options.corvisGateways ? corvisTarget(options.corvisGateways, ["engine-io", "socket-io", "gateways"]) : null;
    const secondOfMode = (mode) => (mode === "gateways" && gatewayCorvis ? gatewayCorvis : corvis);
    const byMode = new Map();
    async function targetsOf(mode) {
      if (!byMode.has(mode)) {
        const node = await startNodeTarget("node", { mode });
        started.push(node);
        let second = secondOfMode(mode);
        if (!second) {
          second = await startNodeTarget("node-2", { mode });
          started.push(second);
        }
        byMode.set(mode, { second, targets: { [node.name]: node, [second.name]: second } });
      }
      return byMode.get(mode);
    }
    const secondFor = (scenario) => secondOfMode(modeOf(scenario)) ?? { name: "node-2", capabilities: ALL_CAPABILITIES };
    const gatewayProblems = checkGatewayReference();

    const results = [];
    let failed = false;
    for (const scenario of scenarios) {
      if (modeOf(scenario) === "gateways" && gatewayProblems.length > 0) {
        if (process.env.CORVIS_NODE_DIFF === "1") {
          failed = true;
          line(scenario.name, "all", `FAILED: the gateway reference is not installed (${GATEWAY_NPM_CI_HINT}): ${gatewayProblems.join("; ")}`);
        } else {
          line(scenario.name, "all", `skipped (the gateway reference is not installed: ${GATEWAY_NPM_CI_HINT})`);
        }
        continue;
      }
      const where = plan(scenario, { second: secondFor(scenario) });
      for (const skip of where.skipped) {
        line(scenario.name, skip.target, `skipped (${skip.reason})`);
      }
      if (where.run.length === 0) continue;
      const { targets } = await targetsOf(modeOf(scenario));

      const transcripts = {};
      for (const name of where.run) {
        try {
          transcripts[name] = await runScenario(scenario, targets[name]);
          writeTranscript(name, scenario.name, transcripts[name]);
          line(scenario.name, name, "ran");
        } catch (error) {
          failed = true;
          line(scenario.name, name, `FAILED: ${error.message}`);
        }
      }
      if (where.run.length === 2 && transcripts[where.run[0]] && transcripts[where.run[1]]) {
        const [a, b] = where.run;
        results.push({
          scenario: scenario.name,
          names: { left: a, right: b },
          bothTargets: true,
          differences: diff(normalize(transcripts[a]), normalize(transcripts[b])),
        });
      }
    }

    const outcome = evaluate(results, expected);
    const names = results[0]?.names ?? { left: "node", right: corvis?.name ?? "node-2" };
    const report = formatReport(outcome, names);
    if (report) console.log(report);
    const compared = results.length;
    console.log(
      `${compared} scenario(s) compared, ${outcome.unexpected.length} unexpected, ${outcome.accepted.length} expected, ${outcome.stale.length} stale difference(s)`,
    );
    return failed || !outcome.ok ? 1 : 0;
  } finally {
    await Promise.all(started.map((target) => target.stop()));
  }
}

main().then(
  (code) => {
    process.exitCode = code;
  },
  (error) => {
    console.error(error.message);
    process.exitCode = 1;
  },
);
