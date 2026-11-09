import assert from "node:assert/strict";
import { test } from "node:test";
import { diff, evaluate, formatReport, validateExpected } from "../lib/compare.mjs";
import { normalize } from "../lib/normalize.mjs";

const make = (sid, ackArgs) => ({
  scenario: "socket-io/handshake-and-ack",
  entries: [
    { client: "raw", kind: "http-response", data: { packets: [{ type: "open", data: { sid } }] } },
    { client: "client", kind: "ack", data: { args: ackArgs } },
  ],
});

const same = () => [normalize(make("AAAAAAAAAAAAAAAAAAAA", ["hello", 42])), normalize(make("BBBBBBBBBBBBBBBBBBBB", ["hello", 42]))];
const changed = () => [normalize(make("AAAAAAAAAAAAAAAAAAAA", ["hello", 42])), normalize(make("BBBBBBBBBBBBBBBBBBBB", ["hello", 43]))];

const run = (pair, expected = [], bothTargets = true) =>
  evaluate([{ scenario: "socket-io/handshake-and-ack", bothTargets, differences: diff(pair[0], pair[1]) }], expected);

test("Node against Node with different ids has no difference", () => {
  const outcome = run(same());
  assert.equal(outcome.ok, true);
  assert.deepEqual(outcome.unexpected, []);
});

test("A behavior difference is reported with scenario, path and both values", () => {
  const outcome = run(changed());
  assert.equal(outcome.ok, false);
  assert.equal(outcome.unexpected.length, 1);
  assert.equal(outcome.unexpected[0].path, "$.entries[0].data.args[1]");
  const report = formatReport(outcome, { left: "node", right: "corvis" });
  assert.match(report, /socket-io\/handshake-and-ack/);
  assert.match(report, /\$\.entries\[0\]\.data\.args\[1\]/);
  assert.match(report, /node: 42/);
  assert.match(report, /corvis: 43/);
});

test("Id identity is kept: a reused id against two different ids is a difference", () => {
  const reused = {
    scenario: "s",
    entries: [{ client: "a", kind: "k", data: { sid: "AAAAAAAAAAAAAAAAAAAA", again: { sid: "AAAAAAAAAAAAAAAAAAAA" } } }],
  };
  const distinct = {
    scenario: "s",
    entries: [{ client: "a", kind: "k", data: { sid: "BBBBBBBBBBBBBBBBBBBB", again: { sid: "CCCCCCCCCCCCCCCCCCCC" } } }],
  };
  const differences = diff(normalize(reused), normalize(distinct));
  assert.equal(differences.length, 1);
  assert.equal(differences[0].path, "$.entries[0].data.again.sid");
});

test("Listed difference: passes and is printed as expected", () => {
  const expected = [
    { scenario: "socket-io/handshake-and-ack", path: "$.entries[0].data.args[1]", reason: "test", spec: "ws/socket-io-parity" },
  ];
  const outcome = run(changed(), expected);
  assert.equal(outcome.ok, true);
  assert.equal(outcome.accepted.length, 1);
  assert.match(formatReport(outcome), /expected difference in socket-io\/handshake-and-ack/);
});

test("Stale entry: a listed difference that does not occur fails and is named", () => {
  const expected = [
    { scenario: "socket-io/handshake-and-ack", path: "$.entries[0].data.args[1]", reason: "test", spec: "ws/socket-io-parity" },
  ];
  const outcome = run(same(), expected);
  assert.equal(outcome.ok, false);
  assert.equal(outcome.stale.length, 1);
  assert.match(formatReport(outcome), /STALE.*socket-io\/handshake-and-ack.*args\[1\]/);
});

test("an entry is not stale when its scenario did not run on both targets", () => {
  const expected = [
    { scenario: "socket-io/handshake-and-ack", path: "$.entries[0].data.args[1]", reason: "test", spec: "ws/socket-io-parity" },
  ];
  const outcome = evaluate([], expected);
  assert.equal(outcome.ok, true);
  const nodeOnly = run(same(), expected, false);
  assert.equal(nodeOnly.ok, true);
});

test("entries without a reason (or spec) are rejected", () => {
  assert.throws(() => validateExpected([{ scenario: "s", path: "$", reason: "", spec: "x" }]), /reason/);
  assert.throws(() => validateExpected([{ scenario: "s", path: "$", reason: "r" }]), /spec/);
  assert.throws(() => validateExpected({}), /array/);
  assert.deepEqual(validateExpected([]), []);
});

test("missing and extra entries and keys are differences", () => {
  const differences = diff({ entries: [1], a: 1 }, { entries: [1, 2], b: 1 });
  assert.deepEqual(
    differences.map((d) => d.path),
    ["$.a", "$.b", "$.entries[1]"],
  );
});
