// Structural comparison of two normalized transcripts, expected differences and the report.
// No dependencies: a package update must never change what counts as a difference.
import { readFileSync } from "node:fs";

const ABSENT = "<absent>";

function isObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function pathKey(parent, key) {
  return /^[A-Za-z_][A-Za-z0-9_]*$/.test(key) ? `${parent}.${key}` : `${parent}[${JSON.stringify(key)}]`;
}

/** Differences between two JSON values, as `{ path, left, right }` with JSONPath-like paths. */
export function diff(left, right, path = "$", out = []) {
  if (Array.isArray(left) && Array.isArray(right)) {
    const length = Math.max(left.length, right.length);
    for (let i = 0; i < length; i++) {
      const l = i < left.length ? left[i] : ABSENT;
      const r = i < right.length ? right[i] : ABSENT;
      if (i >= left.length || i >= right.length) out.push({ path: `${path}[${i}]`, left: l, right: r });
      else diff(l, r, `${path}[${i}]`, out);
    }
  } else if (isObject(left) && isObject(right)) {
    const keys = [...new Set([...Object.keys(left), ...Object.keys(right)])].sort();
    for (const key of keys) {
      const child = pathKey(path, key);
      if (!(key in left)) out.push({ path: child, left: ABSENT, right: right[key] });
      else if (!(key in right)) out.push({ path: child, left: left[key], right: ABSENT });
      else diff(left[key], right[key], child, out);
    }
  } else if (left !== right) {
    out.push({ path, left, right });
  }
  return out;
}

/** Loads and validates `expected-differences.json`: an array of `{ scenario, path, reason, spec }`. */
export function loadExpected(file) {
  let entries;
  try {
    entries = JSON.parse(readFileSync(file, "utf8"));
  } catch (error) {
    throw new Error(`cannot read ${file}: ${error.message}`);
  }
  return validateExpected(entries, file);
}

export function validateExpected(entries, label = "expected differences") {
  if (!Array.isArray(entries)) throw new Error(`${label}: must be an array`);
  entries.forEach((entry, i) => {
    const where = `${label}[${i}]`;
    if (!isObject(entry)) throw new Error(`${where}: must be an object`);
    for (const field of ["scenario", "path", "reason", "spec"]) {
      if (typeof entry[field] !== "string" || entry[field].trim() === "") {
        throw new Error(`${where}: "${field}" must be a non-empty string (every expected difference needs a reason and a spec requirement)`);
      }
    }
  });
  return entries;
}

/**
 * Classifies the outcome of a run. `results` is `[{ scenario, bothTargets, differences }]`.
 * An unlisted difference fails; a listed difference that did not occur in a scenario that ran on both targets is stale
 * and fails too.
 */
export function evaluate(results, expected) {
  const unexpected = [];
  const accepted = [];
  const matched = new Set();
  for (const result of results) {
    for (const difference of result.differences) {
      const index = expected.findIndex((e) => e.scenario === result.scenario && e.path === difference.path);
      if (index === -1) {
        unexpected.push({ scenario: result.scenario, ...difference });
      } else {
        matched.add(index);
        accepted.push({ scenario: result.scenario, ...difference, reason: expected[index].reason, spec: expected[index].spec });
      }
    }
  }
  const ranOnBoth = new Set(results.filter((r) => r.bothTargets).map((r) => r.scenario));
  const stale = expected.filter((e, index) => ranOnBoth.has(e.scenario) && !matched.has(index));
  return { unexpected, accepted, stale, ok: unexpected.length === 0 && stale.length === 0 };
}

function show(value) {
  return value === ABSENT ? ABSENT : JSON.stringify(value);
}

/** Human-readable report: scenario, path and both values. */
export function formatReport(outcome, names = { left: "left", right: "right" }) {
  const lines = [];
  for (const d of outcome.accepted) {
    lines.push(`expected difference in ${d.scenario} at ${d.path}`);
    lines.push(`    ${names.left}: ${show(d.left)}`);
    lines.push(`    ${names.right}: ${show(d.right)}`);
    lines.push(`    reason: ${d.reason} (${d.spec})`);
  }
  for (const d of outcome.unexpected) {
    lines.push(`DIFFERENCE in ${d.scenario} at ${d.path}`);
    lines.push(`    ${names.left}: ${show(d.left)}`);
    lines.push(`    ${names.right}: ${show(d.right)}`);
  }
  for (const e of outcome.stale) {
    lines.push(`STALE expected difference in ${e.scenario} at ${e.path}: it no longer occurs (reason: ${e.reason}); remove it from expected-differences.json`);
  }
  return lines.join("\n");
}
