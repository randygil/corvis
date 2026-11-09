import assert from "node:assert/strict";
import { test } from "node:test";
import { normalize } from "../lib/normalize.mjs";

const transcript = (sid, other = sid) => ({
  scenario: "s",
  entries: [
    { client: "a", kind: "open", data: { sid, pingInterval: 25000 } },
    { client: "a", kind: "echo", data: { args: [`sid=${sid}`] } },
    { client: "a", kind: "connect", data: { sid: other } },
  ],
});

test("Ids are normalized: different random ids give the same transcript", () => {
  const left = normalize(transcript("AAAAAAAAAAAAAAAAAAAA", "BBBBBBBBBBBBBBBBBBBB"));
  const right = normalize(transcript("CCCCCCCCCCCCCCCCCCCC", "DDDDDDDDDDDDDDDDDDDD"));
  assert.deepEqual(left, right);
  assert.equal(left.entries[0].data.sid, "<id:1>");
  assert.equal(left.entries[1].data.args[0], "sid=<id:1>");
  assert.equal(left.entries[2].data.sid, "<id:2>");
});

test("Id identity is kept: equal ids stay equal, different ids stay different", () => {
  const same = normalize(transcript("AAAAAAAAAAAAAAAAAAAA", "AAAAAAAAAAAAAAAAAAAA"));
  const different = normalize(transcript("AAAAAAAAAAAAAAAAAAAA", "BBBBBBBBBBBBBBBBBBBB"));
  assert.equal(same.entries[2].data.sid, "<id:1>");
  assert.equal(different.entries[2].data.sid, "<id:2>");
  assert.notDeepEqual(same, different);
});

test("a value that looks like an id but is not in an id field is left alone", () => {
  const out = normalize({
    scenario: "s",
    entries: [{ client: "a", kind: "echo", data: { args: ["NzXDSKcgv9FHvfTOAAAA"] } }],
  });
  assert.equal(out.entries[0].data.args[0], "NzXDSKcgv9FHvfTOAAAA");
});

test("entries of different clients are compared per client, whatever the interleaving", () => {
  const x = { client: "x", kind: "k", data: 1 };
  const y = { client: "y", kind: "k", data: 2 };
  const x2 = { client: "x", kind: "k", data: 3 };
  const one = normalize({ scenario: "s", entries: [x, y, x2] });
  const two = normalize({ scenario: "s", entries: [y, x, x2] });
  assert.deepEqual(one, two);
});

test("the order within one client is kept", () => {
  const one = normalize({
    scenario: "s",
    entries: [
      { client: "x", kind: "k", data: 1 },
      { client: "x", kind: "k", data: 2 },
    ],
  });
  const two = normalize({
    scenario: "s",
    entries: [
      { client: "x", kind: "k", data: 2 },
      { client: "x", kind: "k", data: 1 },
    ],
  });
  assert.notDeepEqual(one, two);
});
