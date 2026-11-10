// Prints, as JSON, what `socket.io-parser` produces for the packets the Rust codec tests use, so the expected strings in
// crates/corvis/src/socket_io/parser.rs come from the Node reference and not from the Rust code.
//
//   node scripts/record-parser.mjs > ../../crates/corvis/src/socket_io/fixtures/parser.json
//
// `encode`: each case is a packet; the output is the Encoder's result, the text first and then the attachments in hex.
// `decode`: each case is a list of frames (a string, or `{"$binary": "<hex>"}`) fed one after the other to one Decoder;
// the output is the packets it emitted and, when it threw, the error message. Binary values inside packet data are
// written `{"$binary": "<hex>"}` on both sides.
import { readFileSync } from "node:fs";
import { Decoder, Encoder, PacketType } from "socket.io-parser";

const version = JSON.parse(readFileSync(new URL("../node_modules/socket.io-parser/package.json", import.meta.url), "utf8")).version;

const revive = (value) => {
  if (Array.isArray(value)) return value.map(revive);
  if (value !== null && typeof value === "object") {
    if (Object.keys(value).length === 1 && typeof value.$binary === "string") return Buffer.from(value.$binary, "hex");
    return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, revive(v)]));
  }
  return value;
};

const neutral = (value) => {
  if (Buffer.isBuffer(value)) return { $binary: value.toString("hex") };
  if (Array.isArray(value)) return value.map(neutral);
  if (value !== null && typeof value === "object") return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, neutral(v)]));
  return value;
};

const ph = (n) => ({ _placeholder: true, num: n });

const encodeCases = [
  { name: "connect main", packet: { type: "CONNECT", nsp: "/" } },
  { name: "connect main with data", packet: { type: "CONNECT", nsp: "/", data: { sid: "abc" } } },
  { name: "connect custom", packet: { type: "CONNECT", nsp: "/admin" } },
  { name: "connect custom with data", packet: { type: "CONNECT", nsp: "/admin", data: { token: "abc", n: 1 } } },
  { name: "disconnect main", packet: { type: "DISCONNECT", nsp: "/" } },
  { name: "disconnect custom", packet: { type: "DISCONNECT", nsp: "/admin" } },
  { name: "event", packet: { type: "EVENT", nsp: "/", data: ["hi"] } },
  { name: "event several values", packet: { type: "EVENT", nsp: "/", data: ["hi", 1, "two", { three: 3 }, null, true, [4, [5]]] } },
  { name: "event custom with id", packet: { type: "EVENT", nsp: "/admin", id: 12, data: ["hi", { a: [1, 2, { b: null }] }] } },
  { name: "event id zero", packet: { type: "EVENT", nsp: "/", id: 0, data: ["hi"] } },
  { name: "event control characters", packet: { type: "EVENT", nsp: "/", data: ["hi", "a\u001eb\u0000\"\\\n\t\u007f é€"] } },
  { name: "event keys in insertion order", packet: { type: "EVENT", nsp: "/", data: ["hi", { z: 1, a: 2, m: { y: 1, b: 2 } }] } },
  { name: "ack", packet: { type: "ACK", nsp: "/", id: 3, data: ["ok", 1] } },
  { name: "ack custom empty", packet: { type: "ACK", nsp: "/admin", id: 9, data: [] } },
  { name: "connect error object", packet: { type: "CONNECT_ERROR", nsp: "/", data: { message: "no" } } },
  { name: "connect error with data", packet: { type: "CONNECT_ERROR", nsp: "/admin", data: { message: "no", data: { content: "retry" } } } },
  { name: "connect error string", packet: { type: "CONNECT_ERROR", nsp: "/", data: "text" } },
  { name: "binary event", packet: { type: "EVENT", nsp: "/", data: ["up", { $binary: "010203" }] } },
  { name: "binary event nested", packet: { type: "EVENT", nsp: "/x", id: 4, data: ["up", { first: { $binary: "01" }, list: [{ $binary: "0202" }, 5] }, { $binary: "030303" }] } },
  { name: "binary event several attachments", packet: { type: "EVENT", nsp: "/", data: ["up", { $binary: "00" }, { $binary: "01" }, { $binary: "02" }, { $binary: "" }] } },
  { name: "binary ack", packet: { type: "ACK", nsp: "/", id: 1, data: [{ $binary: "7a" }] } },
  { name: "binary ack custom", packet: { type: "ACK", nsp: "/admin", id: 2, data: ["x", { b: { $binary: "7a7b" } }] } },
];

const hex = (s) => ({ $binary: s });
const placeholderEvent = (n, ...refs) => `5${n}-["x",${refs.map((r) => JSON.stringify(ph(r))).join(",")}]`;

const decodeCases = [
  { name: "connect main", frames: ["0"] },
  { name: "connect with auth", frames: ['0{"token":"abc"}'] },
  { name: "connect custom", frames: ["0/admin,"] },
  { name: "connect custom without comma", frames: ["0/admin"] },
  { name: "connect custom with auth", frames: ['0/admin,{"token":"abc"}'] },
  { name: "disconnect", frames: ["1"] },
  { name: "disconnect custom", frames: ["1/admin,"] },
  { name: "event", frames: ['2["hi",1,"two"]'] },
  { name: "event with id", frames: ['213["hi"]'] },
  { name: "event leading zeros in id", frames: ['2007["hi"]'] },
  { name: "event custom with id", frames: ['2/admin,5["hi",{"a":[1,null]}]'] },
  { name: "event numeric name", frames: ['2[7,"x"]'] },
  { name: "event largest id", frames: ['29007199254740991["x"]'] },
  { name: "ack", frames: ['37["x",1]'] },
  { name: "ack custom", frames: ['3/admin,2[]'] },
  { name: "connect error object", frames: ['4{"message":"no"}'] },
  { name: "connect error string", frames: ['4"text"'] },
  { name: "binary event", frames: [placeholderEvent(1, 0), hex("010203")] },
  { name: "binary event nested", frames: ['52-/x,9["up",{"a":{"_placeholder":true,"num":1}},[{"_placeholder":true,"num":0}]]', hex("00"), hex("0101")] },
  { name: "binary event repeated placeholder", frames: [placeholderEvent(1, 0, 0, 0), hex("aa")] },
  { name: "binary event unused attachment", frames: [placeholderEvent(2, 1), hex("00"), hex("01")] },
  { name: "binary ack", frames: ['61-4[{"_placeholder":true,"num":0}]', hex("7a")] },
  { name: "two packets in a row", frames: ['2["a"]', placeholderEvent(1, 0), hex("01"), '2["b"]'] },
  { name: "unknown type 9", frames: ["9"] },
  { name: "unknown type 7", frames: ['7["x"]'] },
  { name: "unknown type letter", frames: ['a["x"]'] },
  { name: "event object payload", frames: ['2{"a":1}'] },
  { name: "event empty array", frames: ["2[]"] },
  { name: "event null name", frames: ["2[null]"] },
  { name: "event reserved name disconnect", frames: ['2["disconnect"]'] },
  { name: "event reserved name connect", frames: ['2["connect"]'] },
  { name: "event reserved name connect_error", frames: ['2["connect_error"]'] },
  { name: "event reserved name disconnecting", frames: ['2["disconnecting"]'] },
  { name: "event reserved name newListener", frames: ['2["newListener"]'] },
  { name: "event reserved name removeListener", frames: ['2["removeListener"]'] },
  { name: "event bad json", frames: ['2abc["x"]'] },
  { name: "event truncated json", frames: ['2["x"'] },
  { name: "event trailing text", frames: ['2["x"] x'] },
  { name: "connect array payload", frames: ["0[]"] },
  { name: "connect string payload", frames: ['0"x"'] },
  { name: "disconnect with payload", frames: ["1{}"] },
  { name: "connect error array payload", frames: ["4[]"] },
  { name: "binary without dash", frames: ['5["x"]'] },
  { name: "binary zero attachments", frames: ['50-["x"]'] },
  { name: "binary empty count", frames: ['5-["x"]'] },
  { name: "binary eleven attachments", frames: ['511-["x"]'] },
  { name: "binary ten attachments declared", frames: ['510-["x"]'] },
  { name: "binary huge count", frames: ['5999999999999999999999999-["x"]'] },
  { name: "text while reconstructing", frames: [placeholderEvent(1, 0), '2["y"]'] },
  { name: "binary without a header", frames: [hex("01")] },
  { name: "placeholder out of range", frames: [placeholderEvent(1, 3), hex("01")] },
  { name: "placeholder negative", frames: ['51-["x",{"_placeholder":true,"num":-1}]', hex("01")] },
  { name: "ack without id", frames: ['3["x"]'] },
  { name: "event without payload", frames: ["2"] },
  { name: "empty string", frames: [""] },
  { name: "id on connect", frames: ["00{}"] },
  { name: "ack id above 2^53", frames: ['29007199254740992["x"]'] },
  { name: "ack id of 22 digits", frames: ['2123456789012345678901["x"]'] },
];

const out = { parser: "socket.io-parser", version, encode: [], decode: [] };

for (const { name, packet } of encodeCases) {
  const { type, nsp, id, data } = packet;
  const encoded = new Encoder().encode({ type: PacketType[type], nsp, id, data: revive(data) });
  out.encode.push({
    name,
    packet: { type, nsp, ...(id === undefined ? {} : { id }), ...(data === undefined ? {} : { data }) },
    encoded: encoded.map((part) => (typeof part === "string" ? part : { $binary: Buffer.from(part).toString("hex") })),
  });
}

for (const { name, frames } of decodeCases) {
  const decoder = new Decoder();
  const packets = [];
  decoder.on("decoded", (p) => packets.push({ type: PacketType[p.type], nsp: p.nsp, ...(p.id === undefined ? {} : { id: p.id }), ...(p.data === undefined ? {} : { data: neutral(p.data) }) }));
  const result = { name, frames, packets };
  try {
    for (const frame of frames) decoder.add(typeof frame === "string" ? frame : Buffer.from(frame.$binary, "hex"));
  } catch (error) {
    result.error = error.message;
  }
  out.decode.push(result);
}

console.log(JSON.stringify(out, null, 2));
