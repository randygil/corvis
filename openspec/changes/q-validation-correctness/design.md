# Design

## Context

Source: the engineering-quality review of the validation library recorded in the issue filed from
`quality/q-validation-correctness.issue.md`. The reviewer read the code only; nothing was compiled or measured. Every line
below was read on `dev` at `d46758a` (2026-11-11); **task 1.1 re-checks them before any edit** and notes moved lines in
"Implementation notes". The decisions depend on behavior, not on line numbers.

The module is a port of zod in two versions (zod 4 mode, the default, and zod 3 mode selected with `ZodVersion::V3`),
verified by differential fixtures: `tests/fixtures/zod4/cases.mjs` runs zod 4.4.3 and its `zod/v3` build and writes
`parse.json`, which `tests/zod4_fixtures.rs` replays by case name (`cases` with V4, `casesV3` with V3);
`tests/fixtures/zod4/pipe.mjs` records the original pipe integration 5.5.0 for `tests/zod_pipe.rs`. The class-validator
rules (`validation/rules.rs`) and the built-in pipes (`common/pipe.rs`) have no differential fixture today.

Release builds: the workspace `Cargo.toml` has no `[profile.release]`, so `overflow-checks` is off (Cargo's default);
the port apps consi and houseplan declare a `[profile.release]` that does not enable it, and facten has none. An `i64`
overflow is a panic in `cargo test` and a silent wrap in every release build.

Which version the port apps use (read in `corvis-tests`, read-only): consi and facten declare zod `^4.4.3` / `4.4.3` in
their originals and build every schema with `import * as z from 'zod'` or `import { z } from 'zod'` (no `zod/v3` import
in any of them); their ports never call `set_version`, `with_version` or `ParseOptions::v3()`. houseplan has no schema
library (its original lists neither zod nor class-validator; its port uses `ParseIntPipe`). The parity check therefore
exercises zod 4 mode only, and compares those validation error responses byte for byte.

### Findings as checked

| Id | Finding | Code now (`d46758a`) | Verdict |
|---|---|---|---|
| D1 | `i64` overflow reachable from input in the date parser | `js.rs:470` (a digit run parsed into `i64`, any length), `553-577` (the year taken as is), `277-285` (`days_from_civil`: `era * 146_097`), `301-304` (`make_utc`: `days * 86_400_000`); reached through `parse_date` (`328-333`) from `to_date` (`611-619`, `z.coerce.date()`), `ZDate` with `date_strings` (`types.rs:2222-2227`) and `ZDate::min/max` (`types.rs:2128,2134`) | **Confirmed by arithmetic.** `"999999999999999999-01-01"` fails `parse_iso_date` (four digits, then a digit where `-` is expected) and reaches the legacy parser: `era ≈ 2.5e15`, `era * 146_097 ≈ 3.65e20 > i64::MAX`. Debug: panic (the pipe catches it, 400, hook message printed). Release: wraps (see above). Siblings found: the offset `off * 60_000` (`js.rs:519-521,604`) overflows for `"+9999999999999999"`; a digit run longer than `i64` becomes `0` (`unwrap_or(0)`, `js.rs:470,484,519`), which can turn `"1/1/99999999999999999999"` into a valid 2000-01-01. A failing test first (Decision 2) |
| D4a | zod 3 mode counts code points | `types.rs:1158` (`s.chars().count()`, recomputed per check) against `js::js_len` (`js.rs:98-100`) used by zod 4 mode (`core.rs:198-204`) | Confirmed in the code; the difference from zod 3 is settled by fixture (Decision 1, case `string.min.astral` and siblings) |
| D4b | zod 3 `multipleOf` tolerance grows with the value | `types.rs:1787` (`1e-9 * n.abs().max(1.0)`) against `float_safe_remainder` (`types.rs:1394-1399`) used by zod 4 mode (`1546`) | **Confirmed by arithmetic**: `12345.00001 % 1` gives a difference of 1e-5 under a tolerance of 1.2345e-5, accepted; `1000000000000.5 % 1`, difference 0.5 under 1000, accepted. The original's answer: fixture |
| D4c | zod 3 email, cuid and url patterns | `types.rs:18-23`, used at `1210-1213` | Confirmed in the code: the email pattern lacks `(?!\.)(?!.*\.\.)` (the `regex` crate cannot express lookarounds; `regexes.rs:42` has them as a `JsRegex`), the cuid pattern has no case-insensitive flag, the url pattern requires `://`. The original's answer: fixture |
| D4d | zod 3 `z.coerce.number()` uses Rust parsing | `types.rs:1702-1703` | Confirmed: `"inf"` parses to infinity, `num_value` turns it into `Value::Null`, the parse succeeds with `null`; `"0x10"` is refused. `js::string_to_number` (`js.rs:124-154`) is the zod 4 mode rule. The original's answer: fixture |
| D4e | zod 3 `.trim()` is Rust's | `types.rs:1259` | Confirmed: Rust trims Unicode `White_Space` (U+0085 included, U+FEFF not); `js_trim` (`js.rs:84-95`) the opposite. Fixture |
| D4f (new) | zod 3 messages format numbers the Rust way | `fmt_num` (`types.rs:31-33`), used in the messages at `1754-1791` | Found while confirming: `format!("{n}")` for `|n| ≥ 1e15` or fractions gives `1000000000000000000000` and `0.0000001` where JavaScript gives `1e+21` and `1e-7`. Fixture |
| D4g (new) | zod 3 `z.coerce.string()` | `types.rs:1144-1150` | Found while confirming: `n.to_string()` (serde_json text) and the JSON text of arrays and objects, where `String(x)` gives `1e+21`, `1,2`, `[object Object]`. Fixture |
| D4h (new) | zod 3 `ip` format | `types.rs:1222` (`s.parse::<IpAddr>()`) | Found while confirming: Rust's address grammar, not zod 3's patterns. Fixture |
| S3a | `.overwrite()` and `.check()` drop the wrapped schema's structure | `ZCheckFn` (`wrappers.rs:853-892`) has no `prop_values`, `pattern`, `accepts_undefined`; `ZOverwrite` (`902-941`) has no `prop_values`, `values`, `pattern`. A discriminated union reads `prop_values` and panics at `compound.rs:1323-1328`; a template literal reads `pattern` and panics at `types.rs:2842` | Confirmed in the code (both panic paths). Whether the original accepts these schemas: fixture (zod 4 adds a check to the same schema, so it is expected to) |
| S3b | `ZRefine`/`ZSuperRefine` do not forward `unwrap_optional` and `literal_values` | `wrappers.rs:707-843`; `.required()` (`compound.rs:896-911`); the zod 3 discriminated union reads the discriminator with `literal_values` (`compound.rs:1339`) | **Narrowed.** In zod 3 a refinement is a different schema type and `.required()` unwraps only optional wrappers, so not unwrapping may be exactly zod 3; but a refined literal as the discriminator gives the zod 3 union no values for that option. Both settled by fixture (cases `object.required.refine`, `discriminatedUnion.refinedKey`) |
| S4a | Number helpers duplicated | `fmt_num`, `num_value` (`types.rs:31-37`) against `num_to_string`, `num_json` (`js.rs:30-77`) | Confirmed; `fmt_num` is also D4f |
| S4b | `ZDate` parses its bounds with two parsers | `types.rs:2099-2100` (chrono bounds), `2106-2114` (chrono parser: a zone-less date-time is UTC), `2126-2147` (`min`/`max` store both), `2155-2167` (zod 3 mode parses input with chrono) | Confirmed: zod 3 mode reads `"2024-01-01T10:00:00"` as UTC, zod 4 mode (`js::parse_date`) as local time; the `msg` slot of the chrono bound is always `None`. Decision 6 |
| S4c | `is_number_string` accepts `NaN`, `inf`, `1e5` | `rules.rs:92-94` | Confirmed in the code. The original's answer: fixture. Siblings found: `min_length`/`max_length` count `char`s (`rules.rs:116-121`), `is_ip` uses Rust's grammar (`107-109`) |
| S4d | `ParseArrayPipe` turns `"NaN"` into `null` and `"1e20"` into `i64::MAX` | `common/pipe.rs:309-322` (`fract() == 0` then `n as i64`; `from_f64(NaN)` is `None`, mapped to `Null`) | Confirmed in the code. Siblings found: `ParseFloatPipe` (`154`: Rust parsing, `"0x10"`), `ParseIntPipe` (`126-131`: accepts `"+5"` and padded text), `DefaultValuePipe` (`379`: `""` takes the default). The original's answers: fixture |
| S4e | `to_bigint` saturates | `js.rs:234` (`f as i128`) | Confirmed: `1e39` gives `i128::MAX`. Decision 7 |
| S4f | Four string-to-boolean rules | `lenient.rs:48-51`, `rules.rs:285-289`, `common/pipe.rs:177-182`, `stringbool` | **Dropped as a defect.** They serve four contracts (the typed extraction of query values, the DTO conversion of `ValidationPipe`, the original's `ParseBoolPipe`, zod's `stringbool`) that differ in the originals too; `ParseBoolPipe` is pinned by the new pipes fixture. Whether the first two follow the original's implicit conversion is out of scope (proposal) |
| S5 | Panics as control flow | `compound.rs:1596` (unmergeable intersection), `core.rs:441-443` (`throw_encode_error`), caught only at `pipe.rs:113-118` | Confirmed. Reachable from input: an intersection of transforms that disagree only for some inputs. Decision 8 |
| N4 | `ZLiteral::new(json!([]))` panics at the first use | `types.rs:2290-2296` (no check) against `from_vals` (`2298-2300`, `assert!`); indexing at `2304`, `2319` | Confirmed. Decision 9 |
| N7 | `enum_variants` returns `&[]` for untagged and internally tagged enums | `rules.rs:223-228`, `lenient.rs:216-220` (`deserialize_any` captures nothing); used by `ParseEnumPipe::of` (`common/pipe.rs:340-342`) and `z::native_enum` (`types.rs:3189-3191`) | Confirmed: both then reject every value. The derive's `isEnum` message and the OpenAPI `enum` constraint read it too (out of module, proposal Out of scope). Decision 9 |
| T1 | Tests bound wall-clock time | `tests/zod_security.rs:126-155`, `z/regex.rs:510-512`, `validation/rules.rs:381-388` | Confirmed. Decision 10 |

D2, D3, S1, S2, N1, N2 and N3 are in `q-validation-hot-path`; S6, N5, N6 and N8 in `q-validation-api-hygiene`. No finding
of this change is dropped outright except S4f; every divergence claim waits for its fixture, and a claim the fixture
contradicts is dropped then (task 2.2 lists them in "Implementation notes").

## Goals / Non-Goals

**Goals:**

- No input makes the date parser panic or wrap, in any build.
- zod 3 mode gives what the recorded `zod/v3` build gives for every recorded case, using the same JavaScript primitives as
  zod 4 mode.
- No public parse function panics for input a caller passes at run time; what the original throws is an `Err`.
- The built-in pipes and the number, address and length rules answer what the recorded originals answer.
- Tests that observe counts, never time.

**Non-Goals:**

- Rewriting zod 3 mode as a layer over zod 4 mode (Decision 4).
- Performance (the next change), API shape (the third change).
- Changing zod 4 mode output beyond the two bugs that touch it (dates out of range, check-like wrappers that panicked).

## Decisions

### 1. Fixture cases recorded from the originals, before any fix

**zod (`tests/fixtures/zod4/`).** A `package.json` and lockfile pin `zod` 4.4.3, so recording is
`npm ci && TZ=UTC node cases.mjs > parse.json` from that folder (the header comment says so; `TZ=UTC` because V8's legacy
date parser reads local time). The recorder gains two encodings: a factory that throws when the schema is built is
recorded as `{ name, construct: { throws } }` instead of crashing the recorder, and a non-finite number in the output is
written as `{ "$number": "Infinity" | "-Infinity" | "NaN" }`. Re-recording with no new case must change nothing in
`parse.json` (task 2.1 checks the diff; an existing case that changes because of the new encoding is listed). The replay
(`tests/zod4_fixtures.rs`) builds a `construct` case under `catch_unwind` and expects the panic, and compares a `$number`
against the `Val` that `z::parse_val` returns.

New cases (`c` = zod 4 mode, `c3` = zod 3 mode; the same name builds the same Rust schema in both lists):

| Name | Schema | Inputs | Settles |
|---|---|---|---|
| `string.min.astral`, `string.max.astral`, `string.length.astral` | `string().min(2)`, `.max(1)`, `.length(2)` | `"😀"`, `"a"`, `"é"` (precomposed), `"a😀"`, `"ab"` | D4a (c and c3) |
| `number.multipleOf.big` | `number().multipleOf(1)` | `12345.00001`, `1000000000000.5`, `3`, `1e21` | D4b (c and c3) |
| `number.multipleOf.decimal` | `number().multipleOf(0.1)` and `multipleOf(0.01)` as two names | `0.3`, `0.30000000000000004`, `1.1`, `2.55`; `19.99`, `0.07`, `1.005` | D4b (c and c3) |
| `string.email.edges` | `string().email()` | `".a@b.co"`, `"a..b@b.co"`, `"A@B.CO"`, `"a@b.c"`, `"a_b@c-d.ef"` | D4c (c3; c pins zod 4) |
| `string.cuid.edges` | `string().cuid()` | `"CJLD2CJXH0000QZRMN831I7RN"`, `"cjld2cjxh0000qzrmn831i7rn"`, `"c-abcdefgh"`, `"cabc"` | D4c |
| `string.url.edges` | `string().url()` | `"mailto:a@b.co"`, `"http://"`, `"https://corvis.dev"`, `"foo:bar"`, `"//x.com"`, `"javascript:alert(1)"` | D4c |
| `string.ip.edges` (c3 only) | `string().ip()` | `"::ffff:1.2.3.4"`, `"fe80::1%eth0"`, `"01.2.3.4"`, `"1.2.3.4"`, `"::1"` | D4h |
| `coerce.number.edges` | `coerce.number()` | `"inf"`, `"Infinity"`, `"-Infinity"`, `"0x10"`, `" 12 "`, `"1e3"`, `" 5"`, `"1_0"`, `".5"`, `"5."`, `"+5"` | D4d (c and c3) |
| `coerce.string.edges` | `coerce.string()` | `1e21`, `1.5`, `[1, 2]`, `{}`, `true`, `0.30000000000000004` | D4g (c and c3) |
| `string.trim.edges` | `string().trim()` | `"﻿a﻿"`, `"\u0085a\u0085"`, `" a　"` | D4e (c and c3) |
| `number.min.big`, `number.max.small` | `number().min(1e21)`, `number().max(1e-7)` | `0`; `1` | D4f (c and c3) |
| `object.required.refine` (c3) | `object({ a: string().optional().refine(() => true) }).required()` | `{}` | S3b |
| `discriminatedUnion.refinedKey` (c3) | `discriminatedUnion("type", [object({ type: literal("a").refine(() => true), a: string() })])` | `{ type: "a", a: "x" }`, `{ type: "b" }` | S3b |
| `du.overwrite`, `du.check`, `du.refine` | `discriminatedUnion("kind", [object({ kind: literal("a") }).overwrite(v => v)])`, the same with `.check(() => {})` and `.refine(() => true)` | `{ kind: "a" }`, `{ kind: "b" }` | S3a |
| `du.optional`, `du.nullable`, `du.default`, `du.catch`, `du.readonly` | the option wrapped by each | `{ kind: "a" }` (or a `construct` record when the original throws) | S3a siblings: Corvis must throw where the original throws and accept where it accepts |
| `templateLiteral.overwrite`, `templateLiteral.check` | `templateLiteral(["id-", number().overwrite(v => v)])`, and with `.check(() => {})` | `"id-5"`, `"id-x"` | S3a (pattern) |
| `literal.empty` | `literal([])` | `"a"` (or a `construct` record) | N4 |
| `date.coerce.edges` | `coerce.date()` | `"999999999999999999-01-01"`, `"Jan 1 999999999999999999"`, `"1/1/99999999999999999999 UTC"`, `"1/1/2000 10:00 +9999999999999999"`, `"275761-01-01"`, `"+275760-09-13T00:00:00.000Z"`, `"+275760-09-13T00:00:00.001Z"`, `"-271821-04-20T00:00:00.000Z"` | D1 and siblings (c and c3). Every input either carries an explicit zone or is expected invalid, so the replay does not depend on the machine's time zone |

**Built-in pipes and rules (`tests/fixtures/builtin_pipes/`, new).** A fixture package in the shape of the others
(`package.json` with pinned versions: the original framework's common package 11.2.1, the version of the existing
fixtures and of facten's original; class-validator 0.14.4, the version of the OpenAPI shim fixture; class-transformer
0.5.1 and reflect-metadata 0.2.2, which the original's array pipe loads). `generate.mjs` writes `builtin_pipes.json`:

- `pipes`: `{ pipe, options, input, result }` where `result` is `{ ok }` or `{ status, body }`, as `pipe.mjs` records them,
  for `ParseArrayPipe({ items: Number })` (`"1,2,3"`, `"1,NaN"`, `"1e20"`, `"inf"`, `"Infinity"`, `" 1 , 2 "`, `""`,
  `"1,,2"`, `"0x10"`, `"1.5"`, `"-0"`, `"9007199254740993"`), `ParseArrayPipe({ items: Number, separator: ";" })`
  (`"1;2"`), `ParseArrayPipe()` (`"a, b"`, `""`), `ParseIntPipe` (`"5"`, `"+5"`, `" 5"`, `"5 "`, `"-0"`, `"007"`,
  `"1e3"`, `"9007199254740993"`, `"99999999999999999999"`, `""`, `"0x10"`), `ParseFloatPipe` (`"1.5"`, `" 1.5 "`,
  `"0x10"`, `"1e3"`, `"Infinity"`, `"NaN"`, `""`, `".5"`, `"5."`, `"1,5"`, `"1.5abc"`), `ParseBoolPipe` (`"true"`,
  `"false"`, `"TRUE"`, `"1"`, `""`, `true`) and `DefaultValuePipe(7)` (missing, `null`, `""`, `"x"`, `0`). A missing
  value is recorded as `{ "$undefined": true }` and replayed as `Value::Null`, the convention of the pipes.
- `rules`: `{ rule, args, input, result }` for `isNumberString` (`"1"`, `"1.5"`, `"-1"`, `"+1"`, `" 1"`, `"1 "`, `"NaN"`,
  `"inf"`, `"Infinity"`, `"1e5"`, `".5"`, `"5."`, `"0x10"`, `""`, `"١٢٣"`, `"1,5"`), `isIP` (the inputs of
  `string.ip.edges` plus `"1.2.3.4 "`), `minLength(_, 2)` and `maxLength(_, 1)` (`"❤️"`, `"😀"`, `"é"`,
  `"a️"`, `"ab"`).

`tests/builtin_pipes_fixtures.rs` replays every record against `common::pipe` and `validation::rules` and reports all
mismatches at once, as `zod4_fixtures.rs` does. houseplan's original declares the common package `^12.0.4`; task 2.3 runs
the generator a second time with 12.0.4 and records any difference (the fixture stays on 11.2.1).

**Rule for every claimed divergence.** The fix lands only with its fixture case failing first. A case where Corvis
already matches the original drops that part of the finding (listed in "Implementation notes" and in the issue); the
case stays as a regression pin.

Alternative considered: unit tests with expected values taken from the reviewer's report or from memory of the
libraries. Rejected: that is how D4 happened (patterns and rules written from the library's documentation, not its
output).

### 2. A date parser that cannot overflow

First, failing tests in `js.rs` (task 3.1), run in debug and in release:

- `"999999999999999999-01-01"`, `"Jan 1 999999999999999999"` and `"1/1/2000 10:00 +9999999999999999"` are invalid
  dates. On the code before the fix they panic in debug.
- **The release case.** The test computes, with the same formula in `wrapping_*` arithmetic, a year above 10^17 whose
  wrapped time value lands inside ±8.64e15 (a bounded search; about one candidate in a thousand qualifies), and asserts
  `parse_date(&format!("{y}-01-01"))` is NaN. Before the fix it panics in debug and returns a finite time in release, so
  `cargo test --release -p corvis --lib z::js` fails too.

Then the fix:

- Numerals of the legacy parser are read with checked accumulation; a numeral that does not fit gives what the fixture
  records for it (expected: an invalid date), never `0`.
- A year outside the ECMAScript range (|y| > 275,760) gives an invalid date before `make_utc`.
- `days_from_civil` and `make_utc` compute in `i128`, so they are total for any `i64` arguments; `time_clip` keeps
  deciding the range.
- Offsets are applied in `f64` (`t - off as f64 * 60_000.0`), which cannot overflow and is exact inside the range.

Totality, pinned by a proptest property in `fuzz_tests.rs` (`js_date_parse_is_total`): for strings drawn from digits,
separators, signs, month names, `T`, `Z` and long digit runs, `js::parse_date` returns NaN or an integral value with
`|t| ≤ 8.64e15`, and does not panic in the debug test build; the same property for `string_to_number` (no panic) and
`string_to_bigint` (no panic, `None` or a value). The date cases of Decision 1 pin the edges against V8.

Alternative considered: enable `overflow-checks` in the release profile. Rejected: the wrap becomes a panic, which the
pipe turns into a 400 but which still unwinds any other caller, and it costs every arithmetic operation of the framework.

### 3. zod 3 mode uses the JavaScript primitives of zod 4 mode

Each item lands with its `casesV3` case failing first (Decision 1):

- **Length**: `js::js_len`, computed once before the checks and again only after a transform (`trim`, `toLowerCase`,
  `toUpperCase`, `normalize`, `slugify`) changes the string; zod 3 interleaves its transforms with its checks in order.
- **Trim**: `js::js_trim`.
- **`z.coerce.number()`**: `js::string_to_number` for strings (`js::to_number` for the other JSON values), so `"inf"`
  is NaN (an `invalid_type` issue with `received: "nan"`) and `"0x10"` is 16, as recorded.
- **`z.coerce.string()`**: `js::to_js_string` (the zod 4 mode `js_string_of`).
- **Numbers in messages**: `js::num_to_string`; `fmt_num` is removed and `num_value` replaced by `js::num_json` (S4a).
- **`multipleOf`**: if every recorded case gives the same result as `float_safe_remainder`, zod 3 mode uses it; otherwise
  the zod 3 algorithm is ported beside it as `float_safe_remainder_v3`, with a comment naming the cases that differ.
- **Patterns**: the zod 3 email, cuid and ip patterns become `JsRegex` statics with zod 3's sources and flags (the email
  lookaheads included); a source identical to a `regexes::` static reuses that static. `url` follows what the fixture shows
  (expected: a WHATWG URL parse, which accepts `mailto:`; `url::Url::parse` is already used by zod 4 mode's
  `url_check`). The `regex`-crate statics at `types.rs:18-23` that are no longer used are removed; `UUID`, `DATETIME`,
  `DATE` and `TIME` are not part of any claim and stay.

Verification: `zod3_parse_fixtures` passes with the new cases, `zod4_parse_fixtures` still passes, and a unit test
asserts that the zod 3 email pattern is the `JsRegex` of zod 3's source (so a later edit cannot silently fall back to the
`regex` crate).

### 4. The zod 3 engine stays separate and reuses the primitives (the design question)

Two ways were weighed:

- **(A) Keep two engines; zod 3 mode reuses zod 4 mode's primitives** (lengths, remainder, patterns, coercion, number
  text) now. This change.
- **(B) Rewrite zod 3 mode as a message and issue-shape layer over the zod 4 engine.**

Chosen: **A**. Reasons:

- **Who runs which version.** The three port apps run zod 4 mode only (Context), and the parity check compares their
  zod 4 error responses byte for byte. B would edit every zod 4 node to give it a zod 3 behavior, so every line of zod 4
  output the parity check guards would be touched to serve a mode none of the apps use. A leaves zod 4 nodes alone.
- **zod 3 differs in more than messages.** Its checks run to the end and collect (zod 4 aborts per `abort`/`when`);
  its union returns the issues of a "dirty" option (zod 4: the only non-aborted one); `.default()` parses the default;
  issues carry `unionErrors` and per-code key orders; coercion and `z.date()` follow other rules. A "layer" would need a
  zod 3 branch inside most zod 4 nodes: two semantics in one engine instead of two engines that share their primitives.
- **Public contract.** `ZodType::check` (zod 3 semantics) is the required method custom types implement, and `run_v4`
  adapts it. B would invert that contract; that is an API decision of its own (Open question 1).
- **Where the bugs come from.** Every D4 bug is a primitive re-implemented with Rust semantics. Sharing the primitive
  removes the cause; the engine's structure was not wrong.

Revisit B if zod 3 mode gains users who need new zod 3 features, or if the fixture coverage of `casesV3` grows to the
size of `cases`; until then the cost is a second engine whose primitives cannot drift.

### 5. Check-like wrappers forward the structure of what they wrap

In zod 4, `.refine()`, `.superRefine()`, `.check()` and `.overwrite()` return the same schema with one more check, so
every structural property is the inner schema's. A crate-internal macro, `check_wrapper_structure!(Type)`, implements for
`ZRefine`, `ZSuperRefine`, `ZCheckFn` and `ZOverwrite` the zod 4 structural methods by forwarding to `inner`: `optin`,
`optout`, `values`, `prop_values`, `pattern`, `def_type`, `object_shape`, `inner_schema`, `parent`, `string_patterns`.
Each wrapper keeps writing only `check`, `run_v4`, `type_name` and `json_schema`.

The zod 3 methods (`accepts_undefined`, `unwrap_optional`, `literal_values`) are not in the macro: in zod 3 a refinement
is a separate schema type, so each wrapper implements them as the zod 3 fixture shows (`object.required.refine`,
`discriminatedUnion.refinedKey`).

Tests: the `du.*` and `templateLiteral.*` fixture cases; a table-driven unit test in `wrappers.rs` that wraps an object, a
literal and a number in each of the four wrappers and asserts every forwarded method returns what the inner schema
returns, so a method added to the trait later and not forwarded fails the test once it is added to the table.

Alternative considered: a `Wrapper` trait with a blanket `ZodType` implementation. Rejected: a blanket implementation
conflicts with the wrappers' own `ZodType` implementations (coherence), and the macro keeps each wrapper readable.

### 6. `ZDate` keeps one bound, parsed once

`min(&str)`/`max(&str)` parse the bound once with `js::parse_date` and store it only in `checks`; the chrono `min`/`max`
fields and their always-`None` message slot are removed. zod 3 mode reads its bounds from `checks`, keeps accepting the
same input shapes it accepts today (an ISO-looking string, or a millisecond number with `coerce`) but parses them with
`js::parse_date` / `js::to_date`, and keeps its message text (the bound written as RFC 3339). A zone-less date-time is
then local time in both modes, as in JavaScript. Test: `z::date().min("2024-01-01T10:00:00")` refuses the instant one
millisecond before that local time in V3 and in V4, and accepts that instant in both.

### 7. `z.coerce.bigint()` refuses what does not fit

`js::to_bigint` returns `None` (JavaScript would succeed; Corvis cannot represent the value) for a finite integral `f64`
outside `i128`, instead of saturating; the schema reports its `invalid_type` issue. Unit test with `1e39` and `-1e39`.
The rustdoc of `z::coerce::bigint()` states the `i128` range.

### 8. What the original throws is returned as an error

- A private payload type `Thrown(String)` in `core.rs`. `throw_encode_error` and the unmergeable intersection call
  `std::panic::resume_unwind(Box::new(Thrown(message)))`: the stack unwinds (through `DepthGuard`, which restores the
  depth) but the panic hook is not called, so nothing is printed.
- Every public parse entry point (`parse_val`, and through it `parse`, `parse_with`, `parse_opt`, `safe_parse*`,
  `encode`, `decode`, and zod 3's `run_v3`) runs the engine inside `catch_unwind(AssertUnwindSafe(..))`, turns a `Thrown`
  payload into `Err(ZodError::thrown(message))`, and resumes any other payload (a panic in user code keeps propagating
  to its caller, as today).
- `ZodError` gains a private `thrown: Option<String>` and `pub fn thrown(&self) -> Option<&str>`; `ZodError::new` sets
  `None`; its issues are empty when thrown; `Display` writes the message. This makes `ZodError` impossible to build with a
  struct literal outside the crate: a public API change, made now, before 1.0.
- `validate_opt` answers a thrown error with the same 400 body as today (`pipeThrown`: no `errors`) and hands a custom
  `createValidationException` the thrown `ZodError` (today an empty one); the serializer answers 500 as today.
- `catch_thrown` keeps catching panics of user code (a transform that unwraps). It now logs one `error` line, at most
  once a minute through `auth::warn_limit::WarnLimiter`, naming the pipe (`ZodValidationPipe` or the serializer) and the
  parameter kind, never the panic's message, which can carry request data.
- The fixture replay's `throws` branch expects `Err(e)` with `e.thrown() == Some(message)` and compares the message with
  the recorded one. Construction errors (`construct` records, a discriminated union option without a discriminator) stay
  panics at build time: the original throws when the schema is created, and schemas are built by application code.

Tests in a binary of their own, `tests/zod_thrown.rs` (the panic hook is process-wide): a counting panic hook stays at 0
while an unmergeable intersection and an encode through a transform return `Err` with the recorded messages; the same
parse inside `z.lazy` twice in a row reaches the full nesting limit the second time (the guard restored the depth); a
transform that panics inside `ZodValidationPipe` gives the 400 and one log line without the panic text, and a second
panic within the minute adds no line.

Alternative considered: carry a `thrown` flag in `Payload` and check it after every child call. Rejected: every combinator
(object, array, tuple, record, union, pipe, catch, lazy, ...) would need the check, and one missed check would let a
union or a `.catch()` swallow what the original propagates; unwinding gives exactly the original's semantics.

### 9. Construction errors where the original raises them

- `ZLiteral::new` with an empty array panics with "Cannot create literal schema with no valid values", like
  `from_vals`, and documents it under `# Panics`; the `literal.empty` fixture case decides the message.
- `z::native_enum::<T>()` and `ParseEnumPipe::of::<T>()` panic when `enum_variants::<T>()` is empty, with a message that
  names `std::any::type_name::<T>()` and says that only enums of unit variants with the default representation are
  supported; `# Panics` in both rustdocs. `enum_variants` keeps its signature (the derive macros call it).
- Tests: `#[should_panic]` for an empty literal, an untagged enum and an internally tagged enum; a plain enum keeps
  working.

These panic while application code builds its schemas and pipes, as the original throws when its schemas are declared;
none is reachable from request input.

### 10. Tests that count instead of timing

- `recursive_traversal_is_linear` moves to `tests/zod_linear_traversal.rs`, a binary of its own with a counting global
  allocator (the only test of that binary, as `auth_session_memory_heap.rs` does): bytes allocated by a parse of depth
  2n are at most 2.2 times those of depth n, for the `z.lazy` object schema and for `z.json()`. A quadratic traversal
  gives about 4.
- `word_boundary_patterns_are_linear_and_exact`: the elapsed-time assertion goes; `is_linear()` and the result stay.
- `array_unique_is_linear`: a `cfg(test)` counter of element comparisons in `array_unique` stays below `2 * n` for
  100,000 distinct numbers.

## Security invariants this change comes near

From the earlier security audit of the module (nesting limit, quadratic recursion, `array_unique`, the regex backtrack
bound) and the pipe fixture:

| Invariant | Decisions near it | Pinned by |
|---|---|---|
| Recursive schemas stop at `max_depth` with one `custom` issue whose input is undefined, not wrapped in `invalid_union` | 3, 5 (zod 3 code), 8 (unwinding through `DepthGuard`) | `json_deep_input_is_an_issue_not_a_stack_overflow`, `lazy_deep_input_is_an_issue_not_a_stack_overflow`, `too_deep_issue_is_not_wrapped_in_invalid_union` (unchanged); new: depth restored after a thrown error (Decision 8) |
| Recursive traversal is linear (children moved, not copied) | 10 | `tests/zod_linear_traversal.rs` (allocation ratio instead of time) |
| Patterns run on the linear engine when they can; the backtracking engine is bounded by `BACKTRACK_LIMIT` and answers no match at the limit | 3 (zod 3 patterns become `JsRegex`; the email lookaheads use the bounded engine, as zod 4 mode's email already does) | `regex.rs` tests (without the timing assertion); new unit test naming which zod 3 patterns are linear |
| `array_unique` is linear | 10 | comparison counter test |
| An error thrown while parsing answers 400 without `errors` through the pipe, 500 through the serializer | 8 | `zod_pipe.rs` `pipeThrown`; `tests/zod_thrown.rs` |
| Log lines a client can provoke are rate limited and carry no request data | 8 | `tests/zod_thrown.rs` (one line per minute, no panic text) |
| The date parser is total (new) | 2 | `js_date_parse_is_total` (proptest), the D1 unit tests |

## Behavior visible to applications

| What | Before | After | Consequence |
|---|---|---|---|
| `z.coerce.date()` (and `date_strings`, `ZDate` bounds) with a year outside ±275,760, a huge offset or an over-long numeral | Debug: panic (400 through the pipe, hook message printed). Release: wrapped value, sometimes a valid date; an over-long numeral read as `0` | Invalid date (`invalid_type`, `received: "Invalid Date"`), as recorded | Inputs that were wrongly accepted in release are refused |
| zod 3 mode: string lengths, `multipleOf`, email, cuid, url, ip, `coerce.number`, `coerce.string`, `trim`, numbers in messages | Rust semantics | As the recorded `zod/v3` build, case by case | Only where the fixture showed a difference; listed in Implementation notes. zod 4 mode unchanged |
| `.overwrite()` / `.check()` object as a discriminated union option or template literal part | Panic when the schema is built | Built and parsed as recorded | Schemas that could not be built now work |
| zod 3 mode, refined discriminator or `.required()` over a refined optional | As today | As recorded | Only where the fixture showed a difference |
| `ZDate` bound or zod 3 input without a time zone | zod 3 mode: UTC; zod 4 mode: local time | Local time in both | zod 3 mode only, on servers not running in UTC |
| Unmergeable intersection; encode through a transform | Panic (unwind, hook message); the pipe answered 400, the serializer 500 | `Err(ZodError)` with `thrown()`, no hook message; the pipe and serializer answer the same bodies | Direct callers of `parse` get an `Err` instead of an unwind; `safe_parse` returns `success: false`. `ZodError` cannot be built with a struct literal |
| A custom `createValidationException` and a thrown error | Received an empty `ZodError` | Receives the thrown `ZodError` (empty issues, `thrown()` set) | More information, same shape |
| A user callback panicking inside the pipe | 400, panic hook line only | 400, plus one rate-limited `error` line without the panic text | Operators can see it |
| `z.coerce.bigint()` of a number outside `i128` | Saturated value | `invalid_type` issue | Divergence from JavaScript (no arbitrary precision), documented |
| `ParseArrayPipe`, `ParseIntPipe`, `ParseFloatPipe`, `ParseBoolPipe`, `DefaultValuePipe` | Rust parsing | As recorded from the original; values Corvis cannot represent refused with the pipe's 400 and listed as divergences | Only where the fixture showed a difference |
| `is_number_string`, `is_ip`, `min_length`, `max_length` (`#[derive(Validate)]`) | Rust parsing and `char` counts | As recorded from class-validator 0.14.4 | Only where the fixture showed a difference |
| `z::literal(json!([]))`, `z::native_enum::<T>()` / `ParseEnumPipe::of::<T>()` for an untagged enum | Panic at the first parse; silently rejects everything | Panic when built, with a message | Fails at startup instead of per request |

## Risks / Trade-offs

- The fixtures may show divergences beyond the claimed ones → fix those inside the areas of this change (the same
  primitives, the same pipes); record any other as a follow-up in "Implementation notes", not in this change.
- Re-recording `parse.json` needs exactly zod 4.4.3 → the new `package.json` and lockfile pin it; task 2.1 proves that
  re-recording without new cases changes nothing.
- zod 3 mode users see behavior changes → each is a recorded zod 3 behavior and is listed in the docs page.
- Unwinding with a private payload depends on `panic = "unwind"` → with `panic = "abort"` the parse aborts the process,
  as the current panic does; the docs page says that thrown errors need the default unwinding.
- A value Corvis cannot represent (an integer beyond `i64` in `ParseIntPipe`, a BigInt beyond `i128`) cannot match the
  original → refused with the same error shape and recorded as a divergence in the fixture and the docs.

## Open questions

1. Should `ZodType` make `run_v4` its required method and `check` the adapter, now that zod 4 mode is the default and the
   one every port app uses? It changes what every custom type implements. Left to the maintainer.
2. Should zod 3 mode be deprecated or moved behind a Cargo feature, since no port app uses it?
3. zod 3 mode date messages embed `new Date(..)` text, which depends on the server's time zone name: fix them?
4. houseplan's original declares the common package `^12.0.4`; record the pipes fixture with 11.2.1 or 12.0.4?
5. A value the original accepts but Corvis cannot represent (Decision 7, the pipes): refuse, or accept with a lossy
   value?

## Review notes

These settle the open questions and override the design where they differ.

1. Open question 1 is not settled here: it is a public trait change outside the findings. This change keeps `check` as
   the required method; the maintainer decides whether a later change flips it.
2. Open question 2 is not settled here either: removing or gating a documented mode is a product decision. This change
   fixes zod 3 mode where the fixture shows it wrong and keeps it.
3. zod 3 mode date messages are out of scope: their original text depends on the time zone name, which a fixture recorded
   on one machine cannot pin; the bounds themselves are unified (Decision 6).
4. The pipes fixture is recorded with 11.2.1 (the version of facten's original and of the existing fixtures); task 2.3
   re-runs the generator with 12.0.4 and records any difference in "Implementation notes"; a difference for
   `ParseIntPipe` (the pipe houseplan uses) is raised with the maintainer before the change is archived.
5. Values Corvis cannot represent are refused with the original's error shape and recorded as divergences, never accepted
   with a different value (a silent wrong value is what D1, S4d and S4e are about).
6. The security review is mandatory before archiving: the date parser, unwinding through depth guards, the error
   mapping and logs of the pipe and serializer, the engines of the zod 3 patterns, and the number parsing of the pipes.
