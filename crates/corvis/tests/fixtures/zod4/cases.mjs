// Generates `parse.json` with the results of real zod 4.4.3 for the cases below.
// The Rust side (`tests/zod4_fixtures.rs`) builds the same schemas by name.
//
// Usage (from a folder with `zod@4.4.3` installed):
//   node <repo>/crates/corvis/tests/fixtures/zod4/cases.mjs > <repo>/crates/corvis/tests/fixtures/zod4/parse.json
import { createRequire } from "node:module";

const require = createRequire(process.cwd() + "/");
const { z } = require("zod");
const { z: z3 } = require("zod/v3");
const zodVersion = require(require.resolve("zod").replace(/index\.c?js$/, "package.json")).version;

const U = Symbol("undefined");
const enc = (v) => (v === U || v === undefined ? { $undefined: true } : v);
const dec = (v) => (v === U ? undefined : v);

const cases = [];
/** Parse cases: `inputs` are fed to `safeParse` (or `safeEncode` / `safeDecode`). */
function c(name, schema, inputs, opts = {}) {
  cases.push({ name, schema, inputs, opts });
}

const replacer = (_k, v) => (typeof v === "bigint" ? `${v}n` : v);

// ----- primitives -----------------------------------------------------------
c("string", () => z.string(), ["a", "", 1, null, U, true, [], {}]);
c("string.min", () => z.string().min(3), ["ab", "abc", "日本語"]);
c("string.min.msg", () => z.string().min(3, "muy corto"), ["ab"]);
c("string.min.error", () => z.string().min(3, { error: "corto" }), ["ab"]);
c("string.max", () => z.string().max(2), ["abc", "ab"]);
c("string.length", () => z.string().length(2), ["abc", "a", "ab"]);
c("string.nonempty", () => z.string().nonempty(), ["", "a"]);
c("string.min.max.email", () => z.string().min(3).max(4).email(), ["ab", "abcdefg", "a@b.co"]);
c("string.regex", () => z.string().regex(/^[A-Z]+$/), ["a", "AB"]);
c("string.regex.msg", () => z.string().regex(/^[A-Z]+$/, "solo mayus"), ["a"]);
c("string.regex.flags", () => z.string().regex(/^a+$/i), ["b", "AA"]);
c("string.startsWith", () => z.string().startsWith("J"), ["a", "Ja"]);
c("string.endsWith", () => z.string().endsWith("z"), ["a", "az"]);
c("string.includes", () => z.string().includes("mid"), ["a", "amidb"]);
c("string.includes.pos", () => z.string().includes("a", { position: 2 }), ["abc", "bca"]);
c("string.lowercase", () => z.string().lowercase(), ["aB", "ab"]);
c("string.uppercase", () => z.string().uppercase(), ["aB", "AB"]);
c("string.trim.min", () => z.string().trim().min(1), ["   ", "  a "]);
c("string.toLowerCase.email", () => z.string().toLowerCase().email(), ["A@B.CO", "X"]);
c("string.toUpperCase", () => z.string().toUpperCase(), ["abc"]);
c("string.normalize", () => z.string().normalize(), ["Å"]);
c("string.slugify", () => z.string().slugify(), ["  Hola Mundo! Ñandú  "]);
c("string.email", () => z.string().email(), ["x", "a@b.co", "a..b@c.co", ".a@b.co", "a@b.c"]);
c("email", () => z.email(), ["x", "user+tag@example.com"]);
c("email.min", () => z.email().min(30), ["bad"]);
c("email.html5", () => z.email({ pattern: z.regexes.html5Email }), ["a@b", "x"]);
c("email.unicode", () => z.email({ pattern: z.regexes.unicodeEmail }), ["ñ@ñ.co", "x"]);
c("uuid", () => z.uuid(), ["x", "123e4567-e89b-12d3-a456-426614174000", "00000000-0000-0000-0000-000000000000", "ffffffff-ffff-ffff-ffff-ffffffffffff"]);
c("uuidv4", () => z.uuidv4(), ["123e4567-e89b-12d3-a456-426614174000", "123e4567-e89b-42d3-a456-426614174000"]);
c("uuidv7", () => z.uuidv7(), ["x", "01890a5d-ac96-774b-bcce-b302099a8057"]);
c("string.uuid", () => z.string().uuid(), ["x"]);
c("guid", () => z.guid(), ["123e4567-e89b-12d3-a456-426614174000", "x"]);
c("url", () => z.url(), ["x", "https://corvis.dev/a?b=1", "mailto:a@b.co", "http://", "javascript:alert(1)"]);
c("url.hostname", () => z.url({ hostname: /^corvis\.dev$/ }), ["https://corvis.dev", "https://x.dev"]);
c("url.protocol", () => z.url({ protocol: /^https$/ }), ["https://corvis.dev", "http://corvis.dev"]);
c("url.normalize", () => z.url({ normalize: true }), ["HTTPS://Corvis.dev/a/../b"]);
c("httpUrl", () => z.httpUrl(), ["ftp://a.co", "https://a.co"]);
c("string.url", () => z.string().url(), ["x"]);
c("emoji", () => z.emoji(), ["😀", "a"]);
c("nanoid", () => z.nanoid(), ["V1StGXR8_Z5jdHi6B-myT", "x"]);
c("cuid", () => z.cuid(), ["cjld2cjxh0000qzrmn831i7rn", "x"]);
c("cuid2", () => z.cuid2(), ["tz4a98xxat96iws9zmbrgj3a", "X!"]);
c("ulid", () => z.ulid(), ["01ARZ3NDEKTSV4RRFFQ69G5FAV", "x"]);
c("xid", () => z.xid(), ["9m4e2mr0ui3e8a215n4g", "x"]);
c("ksuid", () => z.ksuid(), ["0ujtsYcgvSTl8PAuAdqWYSMnLOv", "x"]);
c("ipv4", () => z.ipv4(), ["127.0.0.1", "256.0.0.1", "x"]);
c("ipv6", () => z.ipv6(), ["::1", "2001:db8::1", "x", "1:2:3"]);
c("cidrv4", () => z.cidrv4(), ["10.0.0.0/8", "10.0.0.0/33"]);
c("cidrv6", () => z.cidrv6(), ["2001:db8::/32", "x"]);
c("mac", () => z.mac(), ["00:1A:2B:3C:4D:5E", "00-1A-2B-3C-4D-5E", "x"]);
c("mac.dash", () => z.mac({ delimiter: "-" }), ["00-1A-2B-3C-4D-5E", "00:1A:2B:3C:4D:5E"]);
c("base64", () => z.base64(), ["aGVsbG8=", "aGVsbG8", "!!", ""]);
c("base64url", () => z.base64url(), ["aGVsbG8", "aGVsbG8=", "a+b"]);
c("e164", () => z.e164(), ["+14155552671", "123"]);
c("jwt", () => z.jwt(), ["eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxIn0.sig", "x"]);
c("jwt.alg", () => z.jwt({ alg: "HS512" }), ["eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxIn0.sig"]);
c("hostname", () => z.hostname(), ["corvis.dev", "-bad-", "localhost"]);
c("hex", () => z.hex(), ["deadBEEF", "xyz", ""]);
c("hash.sha256", () => z.hash("sha256"), ["e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855", "abc"]);
c("hash.md5.base64", () => z.hash("md5", { enc: "base64" }), ["1B2M2Y8AsgTpgAmY7PhCfg==", "x"]);
c("stringFormat", () => z.stringFormat("cod", /^[A-Z]{3}$/), ["ABC", "abc"]);
c("stringFormat.fn", () => z.stringFormat("even-len", (v) => v.length % 2 === 0), ["ab", "abc"]);
c("iso.datetime", () => z.iso.datetime(), ["2026-01-01", "2026-01-01T10:00:00Z", "2026-01-01T10:00:00.123Z", "2026-01-01T10:00:00+02:00", "2026-01-01T10:00Z"]);
c("iso.datetime.offset", () => z.iso.datetime({ offset: true }), ["2026-01-01T10:00:00+02:00", "2026-01-01T10:00:00+0200", "2026-01-01T10:00:00"]);
c("iso.datetime.local", () => z.iso.datetime({ local: true }), ["2026-01-01T10:00:00", "2026-01-01T10:00", "2026-01-01T10"]);
c("iso.datetime.precision", () => z.iso.datetime({ precision: 3 }), ["2026-01-01T10:00:00.123Z", "2026-01-01T10:00:00Z"]);
c("iso.datetime.precision.minute", () => z.iso.datetime({ precision: -1 }), ["2026-01-01T10:00Z", "2026-01-01T10:00:00Z"]);
c("iso.date", () => z.iso.date(), ["2026-02-28", "2026-02-30", "2024-02-29", "2026-1-1"]);
c("iso.time", () => z.iso.time(), ["10:00:00", "10:00", "25:00", "10:00:00.5"]);
c("iso.time.precision", () => z.iso.time({ precision: 0 }), ["10:00:00", "10:00:00.1"]);
c("iso.duration", () => z.iso.duration(), ["P3Y6M4DT12H30M5S", "P", "x"]);
c("string.datetime", () => z.string().datetime(), ["x"]);
c("string.date", () => z.string().date(), ["x"]);
c("number", () => z.number(), [1, 1.5, "1", null, U, true]);
c("number.min.max", () => z.number().min(1).max(10), [0, 11, 5]);
c("number.gt.lt", () => z.number().gt(0).lt(5), [0, 5, 2]);
c("number.positive", () => z.number().positive(), [0, -1, 1]);
c("number.nonnegative", () => z.number().nonnegative(), [-1, 0]);
c("number.negative", () => z.number().negative(), [0]);
c("number.nonpositive", () => z.number().nonpositive(), [1]);
c("number.multipleOf", () => z.number().multipleOf(5), [7, 10]);
c("number.multipleOf.dec", () => z.number().multipleOf(0.01), [0.07, 1.234, 3.3]);
c("number.step", () => z.number().step(0.5), [1.5, 1.2]);
c("number.int", () => z.number().int(), [1.5, 1, 2 ** 60]);
c("number.int.positive", () => z.number().int().positive(), [-1.5]);
c("number.safe", () => z.number().safe(), [2 ** 60, 1]);
c("number.min.msg", () => z.number().min(1, { message: "al menos 1" }), [0]);
c("int", () => z.int(), [1.5, "a", 3, 2 ** 60]);
c("int.min", () => z.int().min(5), [1]);
c("int32", () => z.int32(), [2 ** 31, -(2 ** 31), 1.5]);
c("uint32", () => z.uint32(), [-1, 2 ** 32]);
c("float32", () => z.float32(), [1e39, 1.5]);
c("float64", () => z.float64(), [1.5]);
c("boolean", () => z.boolean(), [true, "true", 0, U]);
c("null", () => z.null(), [null, U, 0]);
c("undefined", () => z.undefined(), [U, null]);
c("void", () => z.void(), [U, null]);
c("any", () => z.any(), [1, U]);
c("unknown", () => z.unknown(), [{ a: 1 }, U]);
c("never", () => z.never(), [1, U]);
c("literal", () => z.literal("a"), ["a", "c", U]);
c("literal.number", () => z.literal(3), [3, "3"]);
c("literal.multi", () => z.literal(["a", 2, null]), ["c", 2, null]);
c("literal.bool", () => z.literal(true), [false]);
c("enum", () => z.enum(["a", "b"]), ["c", "a", 1, U]);
c("enum.msg", () => z.enum(["a", "b"], { message: "opcion invalida" }), ["c"]);
c("enum.extract", () => z.enum(["a", "b", "c"]).extract(["a"]), ["b", "a"]);
c("enum.exclude", () => z.enum(["a", "b", "c"]).exclude(["a"]), ["a", "b"]);
c("enum.object", () => z.enum({ A: "x", B: "y" }), ["A", "x"]);
c("nativeEnum", () => z.nativeEnum({ Up: 1, Down: "down" }), [1, "down", "Up"]);
c("stringbool", () => z.stringbool(), ["true", "yes", "1", "on", "enabled", "Y", "false", "No", "0", "off", "disabled", "maybe", true, 1]);
c("stringbool.custom", () => z.stringbool({ truthy: ["si"], falsy: ["no"], case: "sensitive" }), ["si", "SI", "no"]);
c("templateLiteral", () => z.templateLiteral(["id-", z.number(), "-", z.enum(["a", "b"])]), ["id-1-a", "id-x-a", 5]);
c("templateLiteral.str", () => z.templateLiteral([z.string().min(2), "@", z.literal("x")]), ["ab@x", "a@x", "ab@y"]);
c("nan", () => z.nan(), [1]);

// ----- coerce ---------------------------------------------------------------
c("coerce.number", () => z.coerce.number(), ["abc", "42", "4.5", "", " 7 ", true, null, U, "1e3", "0x10", [], [5]]);
c("coerce.number.int.min", () => z.coerce.number().int().min(1), ["0", "1.5", "3"]);
c("coerce.string", () => z.coerce.string(), [12, true, null, U, [1, 2], { a: 1 }]);
c("coerce.boolean", () => z.coerce.boolean(), ["false", "", 0, 1, null, U, "x"]);
c("coerce.bigint", () => z.coerce.bigint(), ["12", "x"]);
c("coerce.date", () => z.coerce.date(), [0, "2024-01-01", "abc"]);
c("coerce.date.min.max", () => z.coerce.date().min(new Date("2024-01-01")).max(new Date("2024-12-31")), ["2023-06-01", "2025-01-01", "2024-05-05T10:00:00Z", "x"]);
c("coerce.bigint.min", () => z.coerce.bigint().min(5n).max(10n), ["3", "11", "7", "1.5"]);
c("coerce.number.default", () => z.coerce.number().int().min(1).max(100).default(20), [U, "5", "500"]);

// ----- objects --------------------------------------------------------------
c("object", () => z.object({ a: z.string(), b: z.number().optional() }), [{ a: "x", extra: 1 }, {}, null, U, "x", [], { a: 1, b: "y" }]);
c("object.strict", () => z.object({ a: z.string() }).strict(), [{ a: "x", b: 1 }, { a: "x", b: 1, c: 2 }, { b: 1 }]);
c("strictObject", () => z.strictObject({ a: z.string() }), [{ a: "x", b: 1 }]);
c("looseObject", () => z.looseObject({ a: z.string() }), [{ a: "x", b: 1 }]);
c("object.passthrough", () => z.object({ a: z.string() }).passthrough(), [{ a: "x", b: 1 }]);
c("object.catchall", () => z.object({ a: z.string() }).catchall(z.number()), [{ a: "x", b: 1, c: "y" }]);
c("object.optional.keys", () => z.object({ a: z.string().optional(), b: z.undefined(), c: z.unknown(), d: z.any(), e: z.void() }), [{}, { a: undefined }]);
c("object.exactOptional", () => z.object({ a: z.string().exactOptional() }), [{}, { a: "x" }]);
c("object.default", () => z.object({ a: z.string().default("d"), b: z.number().prefault(2) }), [{}, { a: "x", b: 3 }]);
c("object.default.order", () => z.object({ x: z.string(), a: z.string().default("d"), b: z.string().optional() }), [{ x: "1", b: "2" }, { b: "2", x: "1" }]);
c("object.index.keys", () => z.object({ b: z.string(), 10: z.string(), 2: z.string() }), [{ b: "x", 10: "y", 2: "z" }]);
c("record.index.keys", () => z.record(z.string(), z.number()), [{ b: 1, 10: 2, 2: 3 }]);
c("object.nested", () => z.object({ items: z.array(z.object({ q: z.number().gt(0) })) }), [{ items: [{ q: 0 }, { q: 1 }, {}] }]);
c("object.extend", () => z.object({ a: z.string() }).extend({ b: z.number() }), [{ a: "x" }]);
c("object.safeExtend", () => z.object({ a: z.string() }).safeExtend({ a: z.string().min(2) }), [{ a: "x" }]);
c("object.merge", () => z.object({ a: z.string() }).merge(z.object({ b: z.number() })), [{}]);
c("object.pick.strict", () => z.object({ a: z.string(), b: z.number() }).strict().pick({ a: true }), [{ a: "x", b: 1 }]);
c("object.omit", () => z.object({ a: z.string(), b: z.number() }).omit({ b: true }), [{ b: 1 }]);
c("object.partial", () => z.object({ a: z.string(), b: z.number() }).partial(), [{}, { a: 1 }]);
c("object.partial.keys", () => z.object({ a: z.string(), b: z.number() }).partial({ a: true }), [{}]);
c("object.required", () => z.object({ a: z.string().optional(), b: z.number().optional() }).required(), [{}]);
c("object.required.keys", () => z.object({ a: z.string().optional(), b: z.number().optional() }).required({ a: true }), [{}]);
c("object.keyof", () => z.object({ a: z.string(), b: z.number() }).keyof(), ["c", "a"]);
c("object.refine", () => z.object({ a: z.number(), b: z.number() }).refine((v) => v.a <= v.b, { message: "a<=b", path: ["b"] }), [{ a: 2, b: 1 }, { a: "x", b: 1 }]);
c("object.refine.skip", () => z.object({ a: z.number(), b: z.number() }).refine(() => false, { message: "never" }), [{ a: "x", b: 1 }, { a: 1, b: 1 }]);
c("object.refine.when", () => z.object({ a: z.number(), b: z.number() }).refine(() => false, { message: "always", when: () => true }), [{ a: "x", b: 1 }]);
c("object.superRefine", () => z.object({ a: z.string() }).superRefine((v, ctx) => {
  ctx.addIssue({ code: "custom", message: "x", path: ["q"] });
  ctx.addIssue("s");
  ctx.addIssue({ code: "too_small", origin: "string", minimum: 3, inclusive: true, input: v.a, path: ["a"] });
}), [{ a: "b" }]);
c("object.check", () => z.object({ a: z.string() }).check((ctx) => {
  if (ctx.value.a === "x") ctx.issues.push({ code: "custom", message: "no x", input: ctx.value, path: ["a"] });
}), [{ a: "x" }, { a: "y" }]);

// ----- arrays / tuples / records ---------------------------------------------
c("array", () => z.array(z.string()), [["a"], [1, "a", 2], {}, "x", U]);
c("array.min.max", () => z.array(z.string()).min(1).max(2), [[], ["a", "b", "c"]]);
c("array.length", () => z.array(z.number()).length(2), [[1]]);
c("array.nonempty", () => z.array(z.number()).nonempty(), [[]]);
c("array.method", () => z.string().array().max(1), [["a", "b"]]);
c("tuple", () => z.tuple([z.string(), z.number()]), [["a", 1], ["a"], ["a", 1, 2], [1, "a"], "x"]);
c("tuple.rest", () => z.tuple([z.string()], z.number()), [["a", 1, 2], ["a", "b"]]);
c("tuple.optional", () => z.tuple([z.string(), z.number().optional()]), [["a"], []]);
c("record", () => z.record(z.string(), z.number()), [{ a: 1 }, { a: "x" }, [], null]);
c("record.keymin", () => z.record(z.string().min(2), z.number()), [{ a: 1, bb: 2 }]);
c("record.enum", () => z.record(z.enum(["a", "b"]), z.number()), [{ a: 1 }, { a: 1, b: 2, c: 3 }]);
c("record.nested", () => z.record(z.enum(["x"]), z.record(z.string(), z.string())), [{ x: { k: "v" } }, { x: { k: 1 } }]);
c("partialRecord", () => z.partialRecord(z.enum(["a", "b"]), z.number()), [{ a: 1 }, { c: 1 }]);
c("looseRecord", () => z.looseRecord(z.string().regex(/^a/), z.number()), [{ ab: 1, b: "x" }, { ab: "x" }]);
c("record.number.key", () => z.record(z.number(), z.string()), [{ 1: "a" }, { x: "a" }]);
c("record.literal.key", () => z.record(z.literal(["a", "b"]), z.string()), [{ a: "1" }]);

// ----- unions ---------------------------------------------------------------
c("union", () => z.union([z.string(), z.number()]), [true, "a", 1]);
c("union.dirty", () => z.union([z.string().max(3), z.number()]), ["abcdef"]);
c("union.objects", () => z.union([z.object({ a: z.string() }), z.object({ b: z.number() })]), [{ c: 1 }, { b: 1 }]);
c("or", () => z.string().or(z.number()), [null]);
c("xor", () => z.xor([z.string(), z.string().min(2)]), ["ab", "a", 1]);
c("discriminatedUnion", () => z.discriminatedUnion("type", [z.object({ type: z.literal("a"), a: z.string() }), z.object({ type: z.literal("b"), b: z.number() })]), [{ type: "c" }, {}, 5, { type: "a" }, { type: "b", b: 1 }]);
c("discriminatedUnion.enum", () => z.discriminatedUnion("k", [z.object({ k: z.enum(["x", "y"]) }), z.object({ k: z.literal(3) })]), [{ k: "y" }, { k: 4 }]);
c("intersection", () => z.intersection(z.object({ a: z.string() }), z.object({ b: z.number() })), [{ a: "x", b: 1 }, { a: 1 }]);
c("intersection.conflict", () => z.intersection(z.string().transform(() => 1), z.string().transform(() => 2)), ["x"]);
c("and", () => z.object({ a: z.string() }).and(z.object({ a: z.string().min(2) })), [{ a: "x" }]);

c("union.literals", () => z.union([z.literal("a"), z.literal("b")]), ["c"]);
c("union.nested.du", () => z.object({ shape: z.discriminatedUnion("kind", [z.object({ kind: z.literal("circle"), r: z.number().positive() }), z.object({ kind: z.literal("square"), side: z.number() })]) }), [{ shape: { kind: "circle", r: -1 } }, { shape: { kind: "tri" } }, { shape: null }]);
c("object.catch.field", () => z.object({ a: z.number().catch(-1), b: z.string() }), [{ a: "x", b: "y" }, { a: "x" }]);
c("array.objects.strict", () => z.array(z.strictObject({ id: z.uuid() })).min(1), [[{ id: "x", y: 1 }], []]);

// ----- wrappers --------------------------------------------------------------
c("optional", () => z.string().optional(), [U, null, 1]);
c("nullable", () => z.string().nullable(), [null, U, 5]);
c("nullish", () => z.string().nullish(), [null, U, 5]);
c("default", () => z.string().min(5).default("ab"), [U, "x"]);
c("default.fn", () => z.array(z.string()).default(() => []), [U]);
c("prefault", () => z.string().min(5).prefault("ab"), [U]);
c("catch", () => z.number().catch(0), ["x", 5, U]);
c("catch.fn", () => z.number().catch((ctx) => ctx.issues.length), ["x"]);
c("nonoptional", () => z.string().optional().nonoptional(), [U, "a"]);
c("nonoptional.msg", () => z.string().optional().nonoptional("requerido"), [U]);
c("readonly", () => z.object({ a: z.string() }).readonly(), [{ a: "x" }]);
c("success", () => z.success(z.string()), ["a", 1]);
c("brand", () => z.string().brand(), ["a"]);
c("transform", () => z.string().transform((v) => v.length), ["abc", 1]);
c("transform.issue", () => z.string().transform((v, ctx) => {
  if (v === "x") { ctx.addIssue({ code: "custom", message: "no x" }); return z.NEVER; }
  return v + "!";
}), ["x", "y"]);
c("pipe", () => z.string().transform((v) => v.length).pipe(z.number().min(5)), ["abc", "abcdef"]);
c("preprocess", () => z.preprocess((v) => (typeof v === "string" ? v.split(",") : v), z.array(z.string())), ["a,b", 5]);
c("refine", () => z.string().refine((v) => v.length > 2), ["a"]);
c("refine.msg", () => z.string().refine((v) => v.length > 2, "muy corto"), ["a"]);
c("refine.abort", () => z.string().refine((v) => v.length > 2, { message: "r1", abort: true }).refine(() => false, "r2"), ["a"]);
c("min.then.refine", () => z.string().min(3).refine(() => false), ["a"]);
c("refine.params", () => z.string().refine(() => false, { params: { foo: 1 } }), ["a"]);
c("superRefine.fatal", () => z.string().superRefine((v, ctx) => { ctx.addIssue({ code: "custom", message: "f", fatal: true }); }).refine(() => false, "after"), ["a"]);
c("overwrite", () => z.number().max(5).overwrite((v) => v * 2), [2, 3]);
c("codec", () => z.codec(z.iso.datetime(), z.number(), { decode: (s) => Date.parse(s), encode: (n) => new Date(n).toISOString() }), ["2026-01-01T00:00:00.000Z", "x"]);
c("codec.encode", () => z.codec(z.iso.datetime(), z.number(), { decode: (s) => Date.parse(s), encode: (n) => new Date(n).toISOString() }), [0, "x"], { mode: "encode" });
c("invertCodec", () => z.invertCodec(z.codec(z.iso.datetime(), z.number(), { decode: (s) => Date.parse(s), encode: (n) => new Date(n).toISOString() })), [0, "x"]);
c("email.idn", () => z.email({ pattern: z.regexes.idnEmail }), ["ñandú@ejemplo.com", "a b@c"]);
c("stringbool.encode", () => z.stringbool(), [true, false, "x"], { mode: "encode" });
c("lazy", () => {
  const Cat = z.object({ name: z.string(), get children() { return z.array(Cat).optional(); } });
  return Cat;
}, [{ name: "a", children: [{ name: "b", children: [{ name: 1 }] }] }]);
c("json", () => z.json(), [{ a: [1, "x", null, true] }]);
c("custom", () => z.custom((v) => typeof v === "string" && v.startsWith("a")), ["ab", "b"]);
c("custom.msg", () => z.custom((v) => v === 1, "debe ser 1"), [2]);

// ----- errors -----------------------------------------------------------------
c("error.param.fn", () => z.string({ error: (iss) => (iss.input === undefined ? "requerido" : "no es string") }), [U, 1]);
c("error.param.str", () => z.string("debe ser texto"), [1]);
c("error.min.fn", () => z.string().min(3, { error: (iss) => `min ${iss.minimum}` }), ["a"]);
c("error.object.fn", () => z.object({ a: z.string() }, { error: () => "obj" }), [5, { a: 1 }]);
c("error.reportInput", () => z.object({ a: z.string().min(2) }), [{ a: "x" }], { reportInput: true });
c("error.parse.error", () => z.object({ a: z.string().min(2), b: z.number() }), [{ a: "x" }], { error: true });
c("error.format", () => z.object({ a: z.string(), b: z.object({ c: z.number() }), d: z.array(z.string()) }).refine(() => false, "raiz"), [{ b: { c: "x" }, d: [1, "a", 2] }], { fmt: true });

// ----- locales ------------------------------------------------------------------
const es = { locale: "es" };
c("es.string", () => z.string(), [1, U, null, [], {}, true], es);
c("es.sizes", () => z.object({ s: z.string().min(3).max(4), a: z.array(z.number()).min(1), n: z.number().gt(1).lte(2), i: z.int() }), [{ s: "a", a: [], n: 1, i: 1.5 }, { s: "abcde", a: [1], n: 3, i: 2 }], es);
c("es.formats", () => z.object({ e: z.email(), u: z.uuid(), d: z.iso.datetime(), r: z.string().regex(/^a$/), sw: z.string().startsWith("x"), ew: z.string().endsWith("y"), inc: z.string().includes("z"), mac: z.mac(), ip: z.ipv4() }), [{ e: "x", u: "x", d: "x", r: "b", sw: "a", ew: "a", inc: "a", mac: "x", ip: "x" }], es);
c("es.values", () => z.object({ e: z.enum(["a", "b"]), l: z.literal("x"), m: z.number().multipleOf(3) }).strict(), [{ e: "c", l: "y", m: 4, extra: 1 }, { e: "a", l: "x", m: 3, k1: 1, k2: 2 }], es);
c("es.misc", () => z.object({ u: z.union([z.string(), z.number()]), r: z.record(z.string().min(2), z.string()), d: z.discriminatedUnion("t", [z.object({ t: z.literal("a") })]) }), [{ u: true, r: { a: "x" }, d: { t: "b" } }], es);

// ----- zod 3 (`zod/v3`, selected with `z::set_version(ZodVersion::V3)`) -----------
// Same names as above: the Rust schema is the same, only the version changes.
const casesV3 = [];
function c3(name, schema, inputs, opts = {}) {
  casesV3.push({ name, schema, inputs, opts });
}
c3("string", () => z3.string(), ["a", 1, null, U, true]);
c3("string.min", () => z3.string().min(3), ["ab", "abc"]);
c3("string.min.msg", () => z3.string().min(3, "muy corto"), ["ab"]);
c3("string.max", () => z3.string().max(2), ["abc"]);
c3("string.length", () => z3.string().length(2), ["abc", "a"]);
c3("string.nonempty", () => z3.string().nonempty(), [""]);
c3("string.min.max.email", () => z3.string().min(3).max(4).email(), ["ab", "abcdefg"]);
c3("string.regex", () => z3.string().regex(/^[A-Z]+$/), ["a"]);
c3("string.regex.msg", () => z3.string().regex(/^[A-Z]+$/, "solo mayus"), ["a"]);
c3("string.startsWith", () => z3.string().startsWith("J"), ["a"]);
c3("string.endsWith", () => z3.string().endsWith("z"), ["a"]);
c3("string.includes", () => z3.string().includes("mid"), ["a"]);
c3("string.trim.min", () => z3.string().trim().min(1), ["   "]);
c3("string.email", () => z3.string().email(), ["x", "a@b.co"]);
c3("string.uuid", () => z3.string().uuid(), ["x"]);
c3("string.url", () => z3.string().url(), ["x", "https://corvis.dev"]);
c3("string.datetime", () => z3.string().datetime(), ["x", "2026-01-01T00:00:00Z"]);
c3("string.date", () => z3.string().date(), ["x", "2026-02-30"]);
c3("number", () => z3.number(), [1, "1", null, U]);
c3("number.min.max", () => z3.number().min(1).max(10), [0, 11]);
c3("number.gt.lt", () => z3.number().gt(0).lt(5), [0, 5]);
c3("number.positive", () => z3.number().positive(), [0]);
c3("number.nonnegative", () => z3.number().nonnegative(), [-1]);
c3("number.multipleOf", () => z3.number().multipleOf(5), [7]);
c3("number.int", () => z3.number().int(), [1.5]);
c3("number.int.positive", () => z3.number().int().positive(), [-1.5]);
c3("number.min.msg", () => z3.number().min(1, { message: "al menos 1" }), [0]);
c3("boolean", () => z3.boolean(), [true, "true", U]);
c3("null", () => z3.null(), [null, 0]);
c3("literal", () => z3.literal("a"), ["c"]);
c3("literal.number", () => z3.literal(3), ["3"]);
c3("enum", () => z3.enum(["a", "b"]), ["c", 1]);
c3("enum.extract", () => z3.enum(["a", "b", "c"]).extract(["a"]), ["b"]);
c3("object", () => z3.object({ a: z3.string(), b: z3.number().optional() }), [{ a: "x", extra: 1 }, {}, null, U, { a: 1, b: "y" }]);
c3("object.strict", () => z3.object({ a: z3.string() }).strict(), [{ a: "x", b: 1 }, { a: "x", b: 1, c: 2 }]);
c3("object.passthrough", () => z3.object({ a: z3.string() }).passthrough(), [{ a: "x", b: 1 }]);
c3("object.catchall", () => z3.object({ a: z3.string() }).catchall(z3.number()), [{ a: "x", b: 1, c: "y" }]);
c3("object.default.a", () => z3.object({ a: z3.string().default("d") }), [{}]);
c3("object.nested", () => z3.object({ items: z3.array(z3.object({ q: z3.number().gt(0) })) }), [{ items: [{ q: 0 }, { q: 1 }, {}] }]);
c3("object.partial", () => z3.object({ a: z3.string(), b: z3.number() }).partial(), [{}, { a: 1 }]);
c3("object.required", () => z3.object({ a: z3.string().optional(), b: z3.number().optional() }).required(), [{}]);
c3("object.refine", () => z3.object({ a: z3.number(), b: z3.number() }).refine((v) => v.a <= v.b, { message: "a<=b", path: ["b"] }), [{ a: 2, b: 1 }, { a: "x", b: 1 }]);
c3("array", () => z3.array(z3.string()), [["a"], [1, "a", 2], {}]);
c3("array.min.max", () => z3.array(z3.string()).min(1).max(2), [[], ["a", "b", "c"]]);
c3("array.nonempty", () => z3.array(z3.number()).nonempty(), [[]]);
c3("tuple", () => z3.tuple([z3.string(), z3.number()]), [["a"], ["a", 1, 2]]);
c3("record", () => z3.record(z3.string(), z3.number()), [{ a: "x" }]);
c3("union", () => z3.union([z3.string(), z3.number()]), [true]);
c3("union.dirty", () => z3.union([z3.string().max(3), z3.number()]), ["abcdef"]);
c3("discriminatedUnion", () => z3.discriminatedUnion("type", [z3.object({ type: z3.literal("a"), a: z3.string() }), z3.object({ type: z3.literal("b"), b: z3.number() })]), [{ type: "c" }, { type: "a" }]);
c3("intersection", () => z3.intersection(z3.object({ a: z3.string() }), z3.object({ b: z3.number() })), [{ a: "x", b: 1 }, { a: 1 }]);
c3("optional", () => z3.string().optional(), [U, 1]);
c3("nullable", () => z3.string().nullable(), [null, 5]);
c3("nullish", () => z3.string().nullish(), [U, 5]);
c3("default", () => z3.string().min(5).default("ab"), [U]);
c3("catch", () => z3.number().catch(0), ["x"]);
c3("transform", () => z3.string().transform((v) => v.length), ["abc", 1]);
c3("pipe", () => z3.string().transform((v) => v.length).pipe(z3.number().min(5)), ["abc"]);
c3("refine", () => z3.string().refine((v) => v.length > 2), ["a"]);
c3("refine.msg", () => z3.string().refine((v) => v.length > 2, "muy corto"), ["a"]);
c3("coerce.number", () => z3.coerce.number(), ["abc", "42", ""]);
c3("coerce.string", () => z3.coerce.string(), [12, null]);
c3("coerce.boolean", () => z3.coerce.boolean(), ["false", ""]);
c3("error.format", () => z3.object({ a: z3.string(), b: z3.object({ c: z3.number() }), d: z3.array(z3.string()) }).refine(() => false, "raiz"), [{ b: { c: "x" }, d: [1, "a", 2] }], { fmt: true });

// ----- json schema ---------------------------------------------------------------
const jsonSchemas = [];
function j(name, schema, params) {
  jsonSchemas.push({ name, schema, params });
}
j("string", () => z.string());
j("string.checks", () => z.string().min(1).max(5).regex(/^a/).startsWith("a"));
j("email", () => z.email());
j("email.regex", () => z.email().regex(/a/));
j("uuid", () => z.uuid());
j("url", () => z.url());
j("iso.datetime", () => z.iso.datetime());
j("iso.date", () => z.iso.date());
j("iso.time", () => z.iso.time());
j("iso.duration", () => z.iso.duration());
j("ipv4", () => z.ipv4());
j("base64", () => z.base64());
j("number", () => z.number().min(5).max(10));
j("number.int.min", () => z.number().min(5).int());
j("int", () => z.int());
j("int.min", () => z.int().min(5));
j("int32", () => z.int32());
j("number.exclusive", () => z.number().gt(0).lt(5).multipleOf(0.5));
j("boolean", () => z.boolean());
j("null", () => z.null());
j("literal", () => z.literal("a"));
j("literal.multi", () => z.literal(["a", 1]));
j("enum", () => z.enum(["a", "b"]));
j("object", () => z.object({ a: z.string(), b: z.number().optional(), c: z.string().default("x"), d: z.string().nullable() }));
j("object.io.input", () => z.object({ c: z.string().default("x") }), { io: "input" });
j("object.strict", () => z.strictObject({ a: z.string() }));
j("object.loose", () => z.looseObject({ a: z.string() }));
j("object.catchall", () => z.object({ a: z.string() }).catchall(z.number()));
j("array", () => z.array(z.string()).min(1).max(3));
j("tuple", () => z.tuple([z.string(), z.number()], z.boolean()));
j("record", () => z.record(z.string(), z.number()));
j("record.enum", () => z.record(z.enum(["a", "b"]), z.number()));
j("partialRecord", () => z.partialRecord(z.enum(["a", "b"]), z.string()));
j("looseRecord", () => z.looseRecord(z.string().regex(/^a/), z.string()));
j("union", () => z.union([z.string(), z.number()]));
j("discriminatedUnion", () => z.discriminatedUnion("t", [z.object({ t: z.literal("a") }), z.object({ t: z.literal("b") })]));
j("intersection", () => z.intersection(z.object({ a: z.string() }), z.object({ b: z.number() })));
j("nullable", () => z.string().nullable());
j("nullable.object", () => z.object({ a: z.string() }).nullable());
j("describe", () => z.string().describe("Un texto"));
j("meta", () => z.object({ a: z.string().meta({ title: "A", examples: ["x"] }) }).meta({ id: "Obj", description: "d" }));
j("default", () => z.string().default("x"));
j("templateLiteral", () => z.templateLiteral(["id-", z.number()]));
j("stringbool", () => z.stringbool(), { io: "input" });
j("pipe.output", () => z.string().transform((v) => v.length).pipe(z.number()), { io: "output", unrepresentable: "any" });
j("pipe.input", () => z.string().transform((v) => v.length).pipe(z.number()), { io: "input" });
j("lazy", () => {
  const Cat = z.object({ name: z.string(), get children() { return z.array(Cat); } });
  return Cat;
});
j("target.draft-07", () => z.object({ a: z.string().nullable(), t: z.tuple([z.string()]) }), { target: "draft-07" });
j("target.openapi-3.0", () => z.object({ a: z.string().nullable(), l: z.literal("x") }), { target: "openapi-3.0" });
j("readonly", () => z.object({ a: z.string() }).readonly());
j("any", () => z.any());
j("unknown", () => z.unknown());
j("never", () => z.never());
j("json", () => z.json());
j("reused.inline", () => { const S = z.string().min(3); return z.object({ a: S, b: S }); });
j("reused.ref", () => { const S = z.string().min(3); return z.object({ a: S, b: S }); }, { reused: "ref" });
j("cycles.throw", () => {
  const Cat = z.object({ name: z.string(), get children() { return z.array(Cat); } });
  return Cat;
}, { cycles: "throw" });
j("lazy.nested", () => {
  const Node = z.object({ v: z.number(), get next() { return Node.optional(); } });
  return z.object({ head: Node });
});
j("unrep.bigint", () => z.bigint());
j("unrep.bigint.any", () => z.object({ a: z.bigint(), b: z.string() }), { unrepresentable: "any" });
j("unrep.date", () => z.date());
j("unrep.transform", () => z.string().transform((v) => v.length));
j("unrep.transform.any", () => z.string().transform((v) => v.length), { unrepresentable: "any" });
j("io.input.object", () => z.object({ a: z.string().default("x"), b: z.string().optional(), c: z.string().prefault("y"), d: z.number().catch(0) }), { io: "input" });
j("io.output.object", () => z.object({ a: z.string().default("x"), b: z.string().optional(), c: z.string().prefault("y"), d: z.number().catch(0) }));
j("meta.id.nested", () => z.object({ name: z.string().meta({ id: "PersonName", description: "nombre" }), tags: z.array(z.string()) }));
j("meta.fields", () => z.string().meta({ title: "T", description: "D", examples: ["a"], deprecated: true, "x-extra": 1 }));
j("describe.nested", () => z.object({ a: z.string().describe("campo a").optional() }));
j("formats", () => z.object({
  cuid: z.cuid(), cuid2: z.cuid2(), ulid: z.ulid(), ipv6: z.ipv6(), cidrv4: z.cidrv4(), e164: z.e164(), jwt: z.jwt(),
  hostname: z.hostname(), emoji: z.emoji(), nanoid: z.nanoid(), base64url: z.base64url(), guid: z.guid(), uuidv4: z.uuidv4(),
  hex: z.hex(), mac: z.mac(), httpUrl: z.httpUrl(), lower: z.string().lowercase(), upper: z.string().uppercase(),
  incl: z.string().includes("x"), ends: z.string().endsWith("z"), len: z.string().length(3),
}));
j("enum.native", () => z.nativeEnum({ A: 1, B: "b" }));
j("literal.null", () => z.literal(null));
j("literal.mixed", () => z.literal(["a", 1, true, null]));
j("tuple.draft07", () => z.tuple([z.string(), z.number()], z.boolean()), { target: "draft-07" });
j("tuple.openapi", () => z.tuple([z.string(), z.number()]), { target: "openapi-3.0" });
j("draft-04", () => z.object({ a: z.number().gt(1).lt(5), b: z.literal("x"), c: z.string().nullable() }), { target: "draft-04" });
j("default.fn", () => z.array(z.string()).default(() => ["a"]));
j("catch", () => z.number().catch(0));
j("readonly.array", () => z.array(z.string()).readonly());
j("union.nullable", () => z.union([z.string(), z.number()]).nullable());
j("object.partial", () => z.object({ a: z.string(), b: z.number() }).partial());
j("object.required", () => z.object({ a: z.string().optional(), b: z.number().optional() }).required());
j("stringbool.output", () => z.stringbool());
j("codec.input", () => z.codec(z.iso.datetime(), z.number(), { decode: (s) => Date.parse(s), encode: (n) => new Date(n).toISOString() }), { io: "input" });
j("codec.output", () => z.codec(z.iso.datetime(), z.number(), { decode: (s) => Date.parse(s), encode: (n) => new Date(n).toISOString() }));
j("preprocess", () => z.preprocess((v) => v, z.number()), { io: "input" });
j("nullable.openapi.object", () => z.object({ a: z.string() }).nullable(), { target: "openapi-3.0" });
j("number.openapi.exclusive", () => z.number().gt(0).lt(5), { target: "openapi-3.0" });
j("int.positive", () => z.int().positive());
j("array.length", () => z.array(z.number()).length(2));
j("record.draft07", () => z.record(z.string().min(1), z.number()), { target: "draft-07" });
j("record.openapi", () => z.record(z.string().min(1), z.number()), { target: "openapi-3.0" });
j("intersection.objects", () => z.object({ a: z.string() }).and(z.object({ b: z.number() }).strict()));
j("success", () => z.success(z.string()));
j("nan.any", () => z.nan(), { unrepresentable: "any" });
j("void.any", () => z.object({ a: z.void(), b: z.undefined() }), { unrepresentable: "any" });
j("custom.any", () => z.custom(() => true), { unrepresentable: "any" });

// ----- run -----------------------------------------------------------------------
function issuesOf(error) {
  return JSON.parse(JSON.stringify(error.issues, replacer));
}

const out = { zod: zodVersion, cases: [], casesV3: [], jsonSchemas: [] };
for (const [list, target, v3] of [[cases, out.cases, false], [casesV3, out.casesV3, true]])
for (const { name, schema, inputs, opts } of list) {
  const s = schema();
  const results = [];
  for (const raw of inputs) {
    const input = dec(raw);
    const params = opts.reportInput ? { reportInput: true } : undefined;
    let r;
    const res = { input: enc(raw) };
    try {
      if (opts.locale) z.config(z.locales[opts.locale]());
      if (opts.mode === "encode") r = z.safeEncode(s, input, params);
      else if (opts.error) r = s.safeParse(input, { error: (iss) => `global ${iss.code}` });
      else r = s.safeParse(input, params);
    } catch (e) {
      res.throws = String(e.message);
      results.push(res);
      continue;
    } finally {
      if (opts.locale) z.config(z.locales.en());
    }
    if (r.success) {
      res.success = true;
      if (r.data === undefined) res.undefined = true;
      else res.data = JSON.parse(JSON.stringify(r.data, replacer));
    } else {
      res.success = false;
      res.issues = issuesOf(r.error);
      if (opts.fmt && v3) {
        res.flatten = r.error.flatten();
        res.format = r.error.format();
        res.message = r.error.message;
      } else if (opts.fmt) {
        res.flatten = z.flattenError(r.error);
        res.treeify = z.treeifyError(r.error);
        res.format = z.formatError(r.error);
        res.prettify = z.prettifyError(r.error);
        res.message = r.error.message;
      }
    }
    results.push(res);
  }
  target.push({ name, mode: opts.mode ?? "parse", locale: opts.locale ?? null, reportInput: !!opts.reportInput, globalError: !!opts.error, results });
}
for (const { name, schema, params } of jsonSchemas) {
  let result;
  try {
    result = { schema: z.toJSONSchema(schema(), params) };
  } catch (e) {
    result = { error: String(e.message) };
  }
  out.jsonSchemas.push({ name, params: params ?? null, ...result });
}
process.stdout.write(JSON.stringify(out, null, 1) + "\n");
