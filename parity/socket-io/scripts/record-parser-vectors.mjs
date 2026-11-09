// Prints the `engine.io-parser` output for the packets the Rust codec tests use, so the expected byte strings in
// crates/corvis/src/socket_io/engine/packet.rs come from the Node reference and not from the Rust code.
//
//   node scripts/record-parser-vectors.mjs
import { decodePayload, encodePayload } from "engine.io-parser";

const packets = [
  { type: "message", data: "hello" },
  { type: "message", data: "héllo €" },
  { type: "message", data: Buffer.from([1, 2, 3, 4]) },
  { type: "message", data: Buffer.from([255, 0, 7]) },
  { type: "ping", data: "probe" },
  { type: "pong", data: "probe" },
  { type: "upgrade" },
  { type: "noop" },
  { type: "close" },
];

const out = { payload: await new Promise((resolve) => encodePayload(packets, resolve)) };
out.decoded = decodePayload(out.payload).map((p) => ({
  type: p.type,
  data: Buffer.isBuffer(p.data) ? { binary: [...p.data] } : (p.data ?? null),
}));
out.decodedInvalid = decodePayload("4a\x1e9\x1e4b").map((p) => p.type);
console.log(JSON.stringify(out, null, 2));
