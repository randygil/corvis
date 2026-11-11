// Records what the built-in pipes of the original framework and the class-validator rules answer, into
// builtin_pipes.json, for `tests/builtin_pipes_fixtures.rs`.
//
// Usage, from this folder (`package.json` pins the versions): npm ci && node generate.mjs [output file]
//
// Encodings: a missing value is `{ "$undefined": true }` (the Rust pipes receive `null` for it); a non-finite number
// is `{ "$number": "Infinity" | "-Infinity" | "NaN" }`. A pipe result is `{ ok }` or `{ status, body }`.
//
// `divergence`: the original answers with a value Corvis cannot represent (a non-finite number, an integer beyond
// i64). Corvis refuses it with the pipe's own error instead; `divergence.result` is that error, recorded from the
// original for an input it refuses the same way.
import { createRequire } from 'node:module';
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const require = createRequire(import.meta.url);
require('reflect-metadata');
const common = require('@nestjs/common');
const cv = require('class-validator');

const here = dirname(fileURLToPath(import.meta.url));
const version = (pkg) => JSON.parse(readFileSync(join(here, 'node_modules', pkg, 'package.json'), 'utf8')).version;
const out = process.argv[2] ?? join(here, 'builtin_pipes.json');

const U = Symbol('undefined');
const enc = (v) => {
  if (v === U || v === undefined) return { $undefined: true };
  if (typeof v === 'number' && !Number.isFinite(v)) return { $number: String(v) };
  if (Array.isArray(v)) return v.map(enc);
  return v;
};

/** The pipes by the name the Rust side builds; `options` is JSON (`items: "Number"` stands for the constructor). */
const PIPES = {
  ParseArrayPipe: (o) => new common.ParseArrayPipe({ ...o, ...(o.items === 'Number' ? { items: Number } : {}) }),
  ParseIntPipe: () => new common.ParseIntPipe(),
  ParseFloatPipe: () => new common.ParseFloatPipe(),
  ParseBoolPipe: () => new common.ParseBoolPipe(),
  DefaultValuePipe: (o) => new common.DefaultValuePipe(o.default),
};

async function answer(pipe, options, input) {
  try {
    return { ok: enc(await PIPES[pipe](options).transform(input === U ? undefined : input, { type: 'query' })) };
  } catch (e) {
    return { status: e.getStatus(), body: e.getResponse() };
  }
}

const hasNonFinite = (v) => (Array.isArray(v) ? v.some(hasNonFinite) : typeof v === 'number' && !Number.isFinite(v));

/** Why Corvis cannot hold the original's value, and the refused input whose error Corvis answers instead. */
function unrepresentable(pipe, input, value) {
  if (hasNonFinite(value)) {
    const index = value.findIndex((v) => !Number.isFinite(v));
    const items = input.trim().split(',');
    items[index] = 'NaN';
    return { reason: 'a non-finite item', refusedAs: items.join(',') };
  }
  if (pipe === 'ParseIntPipe' && Math.abs(value) >= 2 ** 63) return { reason: 'an integer beyond i64', refusedAs: 'x' };
  return null;
}

const pipes = [];
async function p(pipe, options, inputs) {
  for (const input of inputs) {
    const record = { pipe, options, input: enc(input), result: await answer(pipe, options, input) };
    if ('ok' in record.result) {
      const raw = await PIPES[pipe](options).transform(input === U ? undefined : input, { type: 'query' });
      const why = unrepresentable(pipe, input, raw);
      if (why) record.divergence = { reason: why.reason, result: await answer(pipe, options, why.refusedAs) };
    }
    pipes.push(record);
  }
}

await p('ParseArrayPipe', { items: 'Number' }, ['1,2,3', '1,NaN', '1e20', 'inf', 'Infinity', ' 1 , 2 ', '', '1,,2', '0x10', '1.5', '-0', '9007199254740993']);
await p('ParseArrayPipe', { items: 'Number', separator: ';' }, ['1;2']);
await p('ParseArrayPipe', {}, ['a, b', '']);
await p('ParseIntPipe', {}, ['5', '+5', ' 5', '5 ', '-0', '007', '1e3', '9007199254740993', '99999999999999999999', '', '0x10']);
await p('ParseFloatPipe', {}, ['1.5', ' 1.5 ', '0x10', '1e3', 'Infinity', 'NaN', '', '.5', '5.', '1,5', '1.5abc']);
await p('ParseBoolPipe', {}, ['true', 'false', 'TRUE', '1', '', true]);
await p('DefaultValuePipe', { default: 7 }, [U, null, '', 'x', 0]);

const RULES = {
  isNumberString: (v) => cv.isNumberString(v),
  isIP: (v) => cv.isIP(v),
  minLength: (v, n) => cv.minLength(v, n),
  maxLength: (v, n) => cv.maxLength(v, n),
};
const rules = [];
function r(rule, args, inputs) {
  for (const input of inputs) rules.push({ rule, args, input, result: RULES[rule](input, ...args) });
}
r('isNumberString', [], ['1', '1.5', '-1', '+1', ' 1', '1 ', 'NaN', 'inf', 'Infinity', '1e5', '.5', '5.', '0x10', '', '١٢٣', '1,5']);
r('isIP', [], ['::ffff:1.2.3.4', 'fe80::1%eth0', '01.2.3.4', '1.2.3.4', '::1', '1.2.3.4 ']);
// A heart with a presentation selector, an astral emoji, a precomposed é, a letter with a presentation selector.
const lengths = ['❤️', '\u{1F600}', 'é', 'a️', 'ab'];
r('minLength', [2], lengths);
r('maxLength', [1], lengths);

const fixture = {
  versions: { common: version('@nestjs/common'), classValidator: version('class-validator'), classTransformer: version('class-transformer') },
  pipes,
  rules,
};
writeFileSync(out, JSON.stringify(fixture, null, 1) + '\n');
