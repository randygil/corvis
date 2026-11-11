// Records what the original `JwtService.sign` of the JWT module (@nestjs/jwt 11.0.2, over `jsonwebtoken` 9.0.3, the
// versions the original applications of the port apps use) returns or throws, called directly:
// `new JwtService(moduleOptions).sign(payload, callOptions)`. No server is involved. Run once with
// `npm ci && node jwt_sign.mjs`; the output (`jwt_sign.json`) is committed and replayed by the unit test
// `sign_matches_the_recorded_original` of `crates/corvis/src/jwt.rs`.
//
// The clock is fixed: `Date.now` returns CLOCK * 1000 during each call (CLOCK = 1767225600, 2026-01-01T00:00:00Z) and
// is restored after it, so every token is the same on every run (HS256 and RS256 with PKCS#1 v1.5 are deterministic).
//
// Format: { versions, clock, cases: [{ name, module, call, payload, result, payloadAfter, divergence }] }
//   module       the module options with keys by name: { secret: "s1" } or { privateKey: "rsa", algorithm: "RS256" },
//                plus expiresIn, issuer, audience (the module's signOptions, in that order after algorithm)
//   call         the per-call options { expiresIn, subject, jwtid, notBefore, mutatePayload }, or null
//   result       { token } or { error } (the message the original threw)
//   payloadAfter the caller's value after the call
//   divergence   null, or { reason, result } with what Corvis returns instead
// Keys: s1 (the first secret of jwt.mjs) and rsa (`tests/fixtures/jwt_rsa_private.pem`).
import 'reflect-metadata';
import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const { JwtService } = require('@nestjs/jwt');
const moduleRequire = createRequire(require.resolve('@nestjs/jwt'));

const CLOCK = 1767225600;
const KEYS = {
  s1: 'corvis-jwt-secret-one-0123456789abcdef',
  rsa: readFileSync(new URL('../jwt_rsa_private.pem', import.meta.url), 'utf8'),
};

// The modules of design.md Decision 2.
const B = { secret: 's1' };
const A = { secret: 's1', expiresIn: '1h' };
const C = { secret: 's1', expiresIn: '1h', issuer: 'https://auth.example', audience: 'web' };
const R = { privateKey: 'rsa', algorithm: 'RS256', expiresIn: '1h' };

const CASES = [
  ['iat appended at the clock', B, null, { sub: '7', role: 'admin' }],
  ['exp counted from the clock iat', A, null, { sub: '7', role: 'admin' }],
  ['payload iat kept, exp counted from it', A, null, { iat: 1700000000, sub: '7' }],
  ['payload iat of 0', A, null, { iat: 0, sub: '7' }],
  ['fractional payload iat', A, null, { iat: 1700000000.5, sub: '7' }],
  ['non-numeric iat', B, null, { iat: 'yesterday' }],
  ['payload exp without expiresIn', B, null, { sub: '7', exp: 1900000000 }],
  ['payload exp with the module expiresIn', A, null, { sub: '7', exp: 1900000000 }],
  ['payload exp with a per-call expiresIn', B, { expiresIn: '10m' }, { exp: 1900000000 }],
  ['non-numeric exp', B, null, { exp: '2030' }],
  ['null exp with expiresIn', A, null, { exp: null }],
  ['per-call expiresIn overrides the module', A, { expiresIn: '10m' }, { sub: '7' }],
  ['numeric expiresIn', { ...B, expiresIn: 3600 }, null, { iat: 1700000000 }],
  ['aud and iss appended after exp', C, null, { sub: '7' }],
  ['payload iss with the module issuer', C, null, { sub: '7', iss: 'x' }],
  ['payload aud with the module audience', C, null, { sub: '7', aud: 'x' }],
  ['sub appended', B, { subject: '9' }, { role: 'admin' }],
  ['payload sub with subject', B, { subject: '9' }, { sub: '7' }],
  ['jti appended', B, { jwtid: 'j1' }, { sub: '7' }],
  ['payload jti with jwtid', B, { jwtid: 'j1' }, { sub: '7', jti: 'x' }],
  ['nbf counted from the clock iat', B, { notBefore: '30s' }, { sub: '7' }],
  ['nbf counted from the payload iat', A, { notBefore: '30s' }, { iat: 1700000000 }],
  ['payload nbf with notBefore', B, { notBefore: '30s' }, { nbf: 1700000000 }],
  ['non-numeric nbf', B, null, { nbf: 'soon' }],
  ['the full claim order', C, { subject: '9', jwtid: 'j1', notBefore: 60 }, { iat: 1700000000, role: 'admin' }],
  ['two conflicts: the one reported', C, null, { iss: 'x', exp: 1900000000 }],
  ['string payload', B, null, 'hello'],
  ['string payload with a sign option', A, null, 'hello'],
  ['array payload', B, null, [1, 2]],
  ['number payload', B, null, 5],
  ['mutatePayload', B, { mutatePayload: true }, { sub: '7' }],
  ['the payload is copied by default', B, null, { sub: '7' }],
  ['RS256', R, null, { iat: 1700000000, sub: '7' }],
  ['the payload text', B, null, { iat: 1700000000, name: 'Zoë "x"', n: 0.1, big: 9007199254740991, nested: { a: [1, 'é'] } }],
  // Branches found while reading the original that the table of the design does not cover.
  ['string payload with an asymmetric module', R, null, 'hello'],
  ['number payload with a sign option', A, null, 5],
  ['boolean payload', B, null, true],
  ['null payload', B, null, null],
  ['negative payload iat', A, null, { iat: -5, sub: '7' }],
  ['payload claim set to null with its option', B, { subject: '9' }, { sub: null }],
];

// What Corvis does instead, by case name.
const DIVERGENCES = {
  mutatePayload: {
    reason:
      'Corvis borrows the payload and never changes the caller\'s value: there is no mutate_payload option. The token is the same; payloadAfter is not replayed.',
  },
};

function moduleOptions(m) {
  const options = {};
  if (m.secret) options.secret = KEYS[m.secret];
  if (m.privateKey) options.privateKey = KEYS[m.privateKey];
  const signOptions = {};
  for (const key of ['algorithm', 'expiresIn', 'issuer', 'audience']) {
    if (m[key] !== undefined) signOptions[key] = m[key];
  }
  if (Object.keys(signOptions).length > 0) options.signOptions = signOptions;
  return options;
}

const clone = (v) => (v === undefined ? undefined : JSON.parse(JSON.stringify(v)));

const realNow = Date.now;
const cases = CASES.map(([name, module, call, payload]) => {
  const service = new JwtService(moduleOptions(module));
  const given = clone(payload);
  let result;
  Date.now = () => CLOCK * 1000;
  try {
    result = { token: call ? service.sign(given, { ...call }) : service.sign(given) };
  } catch (e) {
    result = { error: e.message };
  } finally {
    Date.now = realNow;
  }
  const divergence = DIVERGENCES[name] ? { ...DIVERGENCES[name], result } : null;
  return { name, module, call, payload, result, payloadAfter: given === undefined ? null : given, divergence };
});

const versions = {
  '@nestjs/jwt': require('@nestjs/jwt/package.json').version,
  jsonwebtoken: moduleRequire('jsonwebtoken/package.json').version,
};
writeFileSync(new URL('./jwt_sign.json', import.meta.url), JSON.stringify({ versions, clock: CLOCK, cases }, null, 2) + '\n');
console.log(`jwt_sign.json: ${cases.length} cases (${JSON.stringify(versions)})`);
