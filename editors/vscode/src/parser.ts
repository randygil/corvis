// Lightweight scanner for Corvis sources. It does not depend on `vscode`, so it can be
// exercised from plain Node (see test/parser.test.mjs).
//
// It is not a Rust parser: it tokenizes just enough (comments, strings, chars, lifetimes,
// attributes with balanced brackets, identifiers and braces) to find the Corvis items
// declared through attribute macros.

export interface Attr {
  /** Full path as written: `get`, `corvis::main`, `api_ok_response`. */
  path: string;
  /** Last segment of `path`. */
  name: string;
  /** Raw text inside the parentheses (`""` when there are none). */
  args: string;
  hasArgs: boolean;
  /** Offset of the leading `#`. */
  start: number;
  /** Offset just after the closing `]`. */
  end: number;
  nameStart: number;
  nameEnd: number;
  inner: boolean;
}

export interface RouteDef {
  /** `GET`, `POST`, ..., `ALL`, `SEARCH`. */
  method: string;
  /** Attribute that declared it (`get`, `sse`, ...). */
  attrName: string;
  paths: string[];
  handler: string;
  versions: string[];
  /** Offset of the handler name. */
  offset: number;
  attrStart: number;
  attrEnd: number;
  isAsync: boolean;
}

export interface RoutesImpl {
  owner: string;
  routes: RouteDef[];
  offset: number;
}

export interface ControllerDef {
  name: string;
  paths: string[];
  versions: string[];
  host?: string;
  offset: number;
  attrStart: number;
}

export interface ModuleDef {
  name: string;
  imports: string[];
  controllers: string[];
  providers: string[];
  exports: string[];
  global: boolean;
  offset: number;
  attrStart: number;
}

export type ProviderKind = 'injectable' | 'gateway';

export interface ProviderDef {
  name: string;
  kind: ProviderKind;
  transient: boolean;
  /** `path = "/chat"` of a gateway. */
  path?: string;
  offset: number;
  attrStart: number;
}

export interface HandlerDef {
  owner: string;
  /** `on_event`, `cron`, `interval`, `timeout`, `subscribe_message`. */
  kind: string;
  args: string;
  handler: string;
  offset: number;
}

export interface AppSetup {
  rootModule?: string;
  globalPrefix?: string;
  uriVersioning: boolean;
  defaultVersion?: string;
  offset: number;
}

export interface ParsedFile {
  attrs: Attr[];
  controllers: ControllerDef[];
  modules: ModuleDef[];
  providers: ProviderDef[];
  routeImpls: RoutesImpl[];
  handlers: HandlerDef[];
  app?: AppSetup;
}

export const HTTP_ATTRS: Record<string, string> = {
  get: 'GET',
  post: 'POST',
  put: 'PUT',
  delete: 'DELETE',
  patch: 'PATCH',
  options: 'OPTIONS',
  head: 'HEAD',
  all: 'ALL',
  search: 'SEARCH',
  sse: 'GET',
};

export const HANDLER_ATTRS = new Set(['on_event', 'cron', 'interval', 'timeout', 'subscribe_message']);

type Tok =
  | { k: 'attr'; a: Attr; s: number }
  | { k: 'ident'; v: string; s: number }
  | { k: 'punct'; v: string; s: number }
  | { k: 'str'; v: string; s: number };

const isIdentStart = (c: string) => /[A-Za-z_]/.test(c) || c > '\x7f';
const isIdent = (c: string) => /[A-Za-z0-9_]/.test(c) || c > '\x7f';

/** Skips a string literal starting at `i` (at the opening quote or `r`/`b` prefix). Returns [end, value]. */
function readString(src: string, i: number): [number, string] | undefined {
  let j = i;
  if (src[j] === 'b' || src[j] === 'c') j++;
  if (src[j] === 'r') {
    j++;
    let hashes = 0;
    while (src[j] === '#') {
      hashes++;
      j++;
    }
    if (src[j] !== '"') return undefined;
    const close = '"' + '#'.repeat(hashes);
    const end = src.indexOf(close, j + 1);
    if (end < 0) return [src.length, src.slice(j + 1)];
    return [end + close.length, src.slice(j + 1, end)];
  }
  if (src[j] !== '"') return undefined;
  j++;
  let out = '';
  while (j < src.length && src[j] !== '"') {
    if (src[j] === '\\') {
      const n = src[j + 1];
      if (n === 'n') out += '\n';
      else if (n === 't') out += '\t';
      else if (n === '\n' || n === '\r') {
        // line continuation: skip the newline and leading whitespace
        j += 2;
        while (j < src.length && /\s/.test(src[j])) j++;
        continue;
      } else out += n ?? '';
      j += 2;
      continue;
    }
    out += src[j];
    j++;
  }
  return [j + 1, out];
}

/** At a `'`: returns the end of a char literal, or of a lifetime/label. */
function skipQuote(src: string, i: number): number {
  if (src[i + 1] === '\\') {
    let j = i + 2;
    while (j < src.length && src[j] !== "'" && src[j] !== '\n') j++;
    return j + 1;
  }
  if (src[i + 2] === "'") return i + 3;
  // astral-plane char (surrogate pair)
  const code = src.charCodeAt(i + 1);
  if (code >= 0xd800 && code <= 0xdbff && src[i + 3] === "'") return i + 4;
  let j = i + 1;
  while (j < src.length && isIdent(src[j])) j++;
  return j;
}

function skipLineComment(src: string, i: number): number {
  const nl = src.indexOf('\n', i);
  return nl < 0 ? src.length : nl;
}

function skipBlockComment(src: string, i: number): number {
  let depth = 0;
  let j = i;
  while (j < src.length) {
    if (src[j] === '/' && src[j + 1] === '*') {
      depth++;
      j += 2;
    } else if (src[j] === '*' && src[j + 1] === '/') {
      depth--;
      j += 2;
      if (depth === 0) return j;
    } else j++;
  }
  return j;
}

/** Given the offset of an opening bracket, returns the offset of its matching closing bracket. */
export function matchBracket(src: string, open: number): number {
  const stack: string[] = [];
  const pairs: Record<string, string> = { '(': ')', '[': ']', '{': '}' };
  let i = open;
  while (i < src.length) {
    const c = src[i];
    if (c === '/' && src[i + 1] === '/') {
      i = skipLineComment(src, i);
      continue;
    }
    if (c === '/' && src[i + 1] === '*') {
      i = skipBlockComment(src, i);
      continue;
    }
    if (c === '"' || ((c === 'r' || c === 'b' || c === 'c') && /^(?:b|c)?r?#*"/.test(src.slice(i, i + 6)) && !isIdent(src[i - 1] ?? ' '))) {
      const r = readString(src, i);
      if (r) {
        i = r[0];
        continue;
      }
    }
    if (c === "'") {
      i = skipQuote(src, i);
      continue;
    }
    if (pairs[c]) stack.push(pairs[c]);
    else if (c === ')' || c === ']' || c === '}') {
      stack.pop();
      if (stack.length === 0) return i;
    }
    i++;
  }
  return src.length - 1;
}

/** Removes line and block comments, keeping string literals intact. */
export function stripComments(s: string): string {
  let out = '';
  let i = 0;
  while (i < s.length) {
    const c = s[i];
    if (c === '/' && s[i + 1] === '/') {
      i = skipLineComment(s, i);
      continue;
    }
    if (c === '/' && s[i + 1] === '*') {
      i = skipBlockComment(s, i);
      out += ' ';
      continue;
    }
    if (c === '"' || (c === 'r' && /^r#*"/.test(s.slice(i, i + 5)) && !isIdent(s[i - 1] ?? ' '))) {
      const r = readString(s, i);
      if (r) {
        out += s.slice(i, r[0]);
        i = r[0];
        continue;
      }
    }
    if (c === "'") {
      const e = skipQuote(s, i);
      out += s.slice(i, e);
      i = e;
      continue;
    }
    out += c;
    i++;
  }
  return out;
}

function parseAttr(src: string, hash: number, open: number, close: number, inner: boolean): Attr {
  const body = src.slice(open + 1, close);
  const m = /^\s*((?:r#)?[A-Za-z_][\w]*(?:\s*::\s*[A-Za-z_][\w]*)*)/.exec(body);
  const path = m ? m[1].replace(/\s+/g, '') : '';
  const lead = m ? m[0].length - m[1].length : 0;
  const nameStart = open + 1 + lead;
  const nameEnd = nameStart + (m ? m[1].length : 0);
  const segs = path.split('::');
  const name = segs[segs.length - 1] ?? '';
  let args = '';
  let hasArgs = false;
  let k = nameEnd;
  while (k < close && /\s/.test(src[k])) k++;
  if (src[k] === '(') {
    const end = matchBracket(src, k);
    args = stripComments(src.slice(k + 1, Math.min(end, close)));
    hasArgs = true;
  } else if (src[k] === '=') {
    args = src.slice(k + 1, close).trim();
    hasArgs = true;
  }
  return { path, name, args, hasArgs, start: hash, end: close + 1, nameStart, nameEnd, inner };
}

export function tokenize(src: string): Tok[] {
  const toks: Tok[] = [];
  let i = 0;
  const n = src.length;
  while (i < n) {
    const c = src[i];
    if (c === ' ' || c === '\t' || c === '\n' || c === '\r') {
      i++;
      continue;
    }
    if (c === '/' && src[i + 1] === '/') {
      i = skipLineComment(src, i);
      continue;
    }
    if (c === '/' && src[i + 1] === '*') {
      i = skipBlockComment(src, i);
      continue;
    }
    if (c === '#') {
      let j = i + 1;
      let inner = false;
      if (src[j] === '!') {
        inner = true;
        j++;
      }
      while (j < n && /\s/.test(src[j])) j++;
      if (src[j] === '[') {
        const close = matchBracket(src, j);
        toks.push({ k: 'attr', a: parseAttr(src, i, j, close, inner), s: i });
        i = close + 1;
        continue;
      }
    }
    if (c === '"' || ((c === 'r' || c === 'b' || c === 'c') && /^(?:b|c)?r?#*"/.test(src.slice(i, i + 6)))) {
      const r = readString(src, i);
      if (r) {
        toks.push({ k: 'str', v: r[1], s: i });
        i = r[0];
        continue;
      }
    }
    if (c === "'") {
      i = skipQuote(src, i);
      continue;
    }
    if (isIdentStart(c)) {
      let j = i;
      if (c === 'r' && src[i + 1] === '#' && isIdentStart(src[i + 2] ?? '')) j = i + 2;
      const s = j;
      while (j < n && isIdent(src[j])) j++;
      toks.push({ k: 'ident', v: src.slice(s, j), s: i });
      i = j;
      continue;
    }
    if (/[0-9]/.test(c)) {
      let j = i;
      while (j < n && (isIdent(src[j]) || (src[j] === '.' && /[0-9]/.test(src[j + 1] ?? '')))) j++;
      i = j;
      continue;
    }
    toks.push({ k: 'punct', v: c, s: i });
    i++;
  }
  return toks;
}

// ------------------------------------------------------------------ argument helpers

/** Splits `a, b(c, d), [e, f]` on top-level commas. */
export function splitTopLevel(s: string, sep = ','): string[] {
  const out: string[] = [];
  let depth = 0;
  let angle = 0;
  let cur = '';
  let i = 0;
  while (i < s.length) {
    const c = s[i];
    if (c === '"' || (c === 'r' && /^r#*"/.test(s.slice(i, i + 5)) && !isIdent(s[i - 1] ?? ' '))) {
      const r = readString(s, i);
      if (r) {
        cur += s.slice(i, r[0]);
        i = r[0];
        continue;
      }
    }
    if (c === "'") {
      const e = skipQuote(s, i);
      cur += s.slice(i, e);
      i = e;
      continue;
    }
    if (c === '(' || c === '[' || c === '{') depth++;
    else if (c === ')' || c === ']' || c === '}') depth--;
    else if (c === '<' && /[\w:]$/.test(cur)) {
      // generic arguments: `Provider::app_guard::<G>()`, `Vec<Arc<T>>`
      angle++;
    } else if (c === '>' && angle > 0 && s[i - 1] !== '-' && s[i - 1] !== '=') angle--;
    if (c === sep && depth === 0 && angle === 0) {
      out.push(cur.trim());
      cur = '';
      i++;
      continue;
    }
    cur += c;
    i++;
  }
  if (cur.trim()) out.push(cur.trim());
  return out;
}

/** Returns the string value of a literal (`"cats"` or `r"cats"`), or undefined. */
export function stringLiteral(expr: string): string | undefined {
  const t = expr.trim();
  if (!/^(?:r#*)?"/.test(t)) return undefined;
  const r = readString(t, 0);
  if (!r || r[0] !== t.length) return undefined;
  return r[1];
}

/** `"a"` → [a]; `["a", "b"]` → [a, b]; anything else → []. */
export function stringList(expr: string): string[] {
  const t = expr.trim();
  if (t.startsWith('[') && t.endsWith(']')) {
    return splitTopLevel(t.slice(1, -1))
      .map(stringLiteral)
      .filter((s): s is string => s !== undefined);
  }
  const s = stringLiteral(t);
  return s === undefined ? [] : [s];
}

/** `imports = [A, B]` style key/value pairs of an attribute. Bare keys map to `""`. */
export function keyValues(args: string): { key?: string; value: string }[] {
  return splitTopLevel(args).map((part) => {
    const m = /^([A-Za-z_]\w*)\s*=(?!=)\s*([\s\S]*)$/.exec(part);
    if (m) return { key: m[1], value: m[2].trim() };
    return { value: part };
  });
}

function arrayItems(value: string): string[] {
  const t = value.trim();
  if (t.startsWith('[') && t.endsWith(']')) return splitTopLevel(t.slice(1, -1));
  return t ? [t] : [];
}

/** Version values of `version = "1"`, `#[version("1")]`, `#[version(["1", "2"])]`. */
function versionList(expr: string): string[] {
  const list = stringList(expr);
  if (list.length) return list;
  const t = expr.trim();
  if (/VERSION_NEUTRAL/.test(t)) return ['neutral'];
  return t ? [t] : [];
}

/** Short display form of a module list element: `ConfigModule::for_root(…)`. */
export function displayItem(expr: string): string {
  const t = expr.replace(/\s+/g, ' ').trim();
  const p = t.indexOf('(');
  if (p < 0) return t;
  const close = matchBracket(t, p);
  const inside = t.slice(p + 1, close).trim();
  const head = t.slice(0, p);
  const tail = t.slice(close + 1).trim();
  const firstArg = splitTopLevel(inside)[0];
  const lit = firstArg !== undefined ? stringLiteral(firstArg) : undefined;
  let shown: string;
  if (!inside) shown = '()';
  else if (lit !== undefined) shown = `("${lit}"${splitTopLevel(inside).length > 1 ? ', …' : ''})`;
  else shown = '(…)';
  return head + shown + (tail ? ' ' + tail.slice(0, 20) + (tail.length > 20 ? '…' : '') : '');
}

/** The type an element refers to: `UsersModule`, `ConfigModule`, `ThrottlerGuard`. */
export function referencedType(expr: string): string | undefined {
  const t = expr.trim();
  const generic = /::<\s*([A-Za-z_][\w:]*)/.exec(t);
  const head = t.split(/[(<\s]/)[0];
  const segs = head.split('::').filter(Boolean);
  // `Provider::app_guard::<G>()` → G; `ConfigModule::for_root(...)` → ConfigModule
  if (segs[0] === 'Provider' && generic) return generic[1].split('::').pop();
  const pascal = segs.filter((s) => /^[A-Z]/.test(s));
  return pascal.length ? pascal[pascal.length - 1] : undefined;
}

// ------------------------------------------------------------------ item scanner

function selfTypeOfImpl(toks: Tok[], from: number, to: number): string | undefined {
  // tokens between `impl` and `{`
  let angle = 0;
  let afterFor = -1;
  const idents: { v: string; depth: number; i: number }[] = [];
  for (let i = from; i < to; i++) {
    const t = toks[i];
    if (t.k === 'punct') {
      if (t.v === '<') angle++;
      else if (t.v === '>' && !(toks[i - 1]?.k === 'punct' && (toks[i - 1] as { v: string }).v === '-')) angle--;
    } else if (t.k === 'ident') {
      if (t.v === 'for' && angle === 0) afterFor = idents.length;
      else if (t.v === 'where' && angle === 0) break;
      else idents.push({ v: t.v, depth: angle, i });
    }
  }
  const candidates = (afterFor >= 0 ? idents.slice(afterFor) : idents).filter((x) => x.depth === 0 && x.v !== 'dyn' && x.v !== 'unsafe');
  // last segment of the first path at depth 0
  if (!candidates.length) return undefined;
  let name = candidates[0].v;
  for (let k = 1; k < candidates.length; k++) {
    const prev = candidates[k - 1].i;
    const between = toks.slice(prev + 1, candidates[k].i);
    const isPath = between.length === 2 && between.every((b) => b.k === 'punct' && b.v === ':');
    if (!isPath) break;
    name = candidates[k].v;
  }
  return name;
}

const RESETTING_KEYWORDS = new Set(['enum', 'mod', 'trait', 'type', 'const', 'static', 'use', 'let', 'union', 'macro_rules']);

export function parseRust(src: string): ParsedFile {
  const toks = tokenize(src);
  const out: ParsedFile = { attrs: [], controllers: [], modules: [], providers: [], routeImpls: [], handlers: [] };
  let pending: Attr[] = [];
  let depth = 0;
  // stack of impl blocks: owner + body depth + kind
  const impls: { owner: string; depth: number; kind: 'routes' | 'handlers' | 'plain'; ref?: RoutesImpl }[] = [];

  for (let i = 0; i < toks.length; i++) {
    const t = toks[i];
    if (t.k === 'attr') {
      out.attrs.push(t.a);
      if (!t.a.inner) pending.push(t.a);
      continue;
    }
    if (t.k === 'punct') {
      if (t.v === '{') {
        depth++;
        pending = [];
      } else if (t.v === '}') {
        depth--;
        while (impls.length && impls[impls.length - 1].depth > depth) impls.pop();
        pending = [];
      } else if (t.v === ';' || t.v === ',') pending = [];
      continue;
    }
    if (t.k !== 'ident') continue;

    if (t.v === 'struct') {
      const nameTok = toks[i + 1];
      if (nameTok?.k === 'ident') handleStruct(nameTok.v, nameTok.s, pending, out);
      pending = [];
      continue;
    }
    if (t.v === 'impl') {
      // find the `{` of the impl block
      let j = i + 1;
      while (j < toks.length && !(toks[j].k === 'punct' && ((toks[j] as { v: string }).v === '{' || (toks[j] as { v: string }).v === ';'))) j++;
      const owner = selfTypeOfImpl(toks, i + 1, j);
      const kind = pending.some((a) => a.name === 'routes') ? 'routes' : pending.some((a) => a.name === 'handlers') ? 'handlers' : 'plain';
      if (owner && j < toks.length && (toks[j] as { v: string }).v === '{') {
        let ref: RoutesImpl | undefined;
        if (kind === 'routes') {
          ref = { owner, routes: [], offset: toks[i].s };
          out.routeImpls.push(ref);
        }
        impls.push({ owner, depth: depth + 1, kind, ref });
      }
      pending = [];
      continue;
    }
    if (t.v === 'fn') {
      const nameTok = toks[i + 1];
      const top = impls[impls.length - 1];
      if (nameTok?.k === 'ident' && top && top.depth === depth) {
        const isAsync = toks[i - 1]?.k === 'ident' && (toks[i - 1] as { v: string }).v === 'async';
        if (top.kind === 'routes' && top.ref) {
          const route = routeFromAttrs(pending, nameTok.v, nameTok.s, isAsync);
          if (route) top.ref.routes.push(route);
        } else if (top.kind === 'handlers') {
          for (const a of pending) {
            if (HANDLER_ATTRS.has(a.name)) out.handlers.push({ owner: top.owner, kind: a.name, args: a.args.trim(), handler: nameTok.v, offset: nameTok.s });
          }
        }
      }
      pending = [];
      continue;
    }
    if (RESETTING_KEYWORDS.has(t.v)) pending = [];
  }

  out.app = scanAppSetup(src);
  return out;
}

function routeFromAttrs(attrs: Attr[], handler: string, offset: number, isAsync: boolean): RouteDef | undefined {
  const http = attrs.find((a) => HTTP_ATTRS[a.name] !== undefined && (a.path === a.name || a.path.startsWith('corvis::')));
  if (!http) return undefined;
  let paths = http.hasArgs ? stringList(http.args.trim()) : [];
  if (!paths.length) {
    // `#[get(path = "x")]` is not supported by the macro, but a non-literal expression is:
    paths = http.hasArgs && http.args.trim() ? [http.args.trim()] : [''];
  }
  const versionAttr = attrs.find((a) => a.name === 'version');
  return {
    method: HTTP_ATTRS[http.name],
    attrName: http.name,
    paths,
    handler,
    versions: versionAttr ? versionList(versionAttr.args) : [],
    offset,
    attrStart: http.start,
    attrEnd: http.end,
    isAsync,
  };
}

function handleStruct(name: string, offset: number, attrs: Attr[], out: ParsedFile) {
  for (const a of attrs) {
    if (a.name === 'controller' && (a.path === 'controller' || a.path === 'corvis::controller')) {
      const def: ControllerDef = { name, paths: [], versions: [], offset, attrStart: a.start };
      for (const kv of a.hasArgs ? keyValues(a.args) : []) {
        if (!kv.key) def.paths.push(...stringList(kv.value));
        else if (kv.key === 'path') def.paths.push(...stringList(kv.value));
        else if (kv.key === 'version') def.versions = versionList(kv.value);
        else if (kv.key === 'host') def.host = stringLiteral(kv.value) ?? kv.value;
      }
      const v = attrs.find((x) => x.name === 'version');
      if (v) def.versions = versionList(v.args);
      out.controllers.push(def);
      return;
    }
    if (a.name === 'module' && (a.path === 'module' || a.path === 'corvis::module')) {
      const def: ModuleDef = { name, imports: [], controllers: [], providers: [], exports: [], global: false, offset, attrStart: a.start };
      for (const kv of a.hasArgs ? keyValues(a.args) : []) {
        if (!kv.key) {
          if (kv.value === 'global') def.global = true;
          continue;
        }
        switch (kv.key) {
          case 'imports':
            def.imports.push(...arrayItems(kv.value));
            break;
          case 'controllers':
            def.controllers.push(...arrayItems(kv.value));
            break;
          case 'providers':
            def.providers.push(...arrayItems(kv.value));
            break;
          case 'exports':
            def.exports.push(...arrayItems(kv.value));
            break;
          case 'global':
            def.global = kv.value === 'true';
            break;
        }
      }
      if (attrs.some((x) => x.name === 'global' && !x.hasArgs)) def.global = true;
      out.modules.push(def);
      return;
    }
    if (a.name === 'injectable' && (a.path === 'injectable' || a.path === 'corvis::injectable')) {
      out.providers.push({ name, kind: 'injectable', transient: /transient/i.test(a.args), offset, attrStart: a.start });
      return;
    }
    if (a.name === 'websocket_gateway') {
      let path: string | undefined;
      for (const kv of a.hasArgs ? keyValues(a.args) : []) if (kv.key === 'path') path = stringLiteral(kv.value);
      out.providers.push({ name, kind: 'gateway', transient: false, path, offset, attrStart: a.start });
      return;
    }
  }
}

function scanAppSetup(src: string): AppSetup | undefined {
  const create = /CorvisFactory\s*::\s*create(?:_application_context)?\s*::\s*<\s*([A-Za-z_][\w:]*)\s*>/.exec(src);
  const prefix = /set_global_prefix\s*\(\s*"([^"]*)"/.exec(src);
  const uri = /enable_versioning\s*\(\s*VersioningOptions\s*::\s*uri\s*\(\s*\)([^;]*)/.exec(src);
  if (!create && !prefix && !uri) return undefined;
  let defaultVersion: string | undefined;
  if (uri) {
    const dv = /default_version\s*\(\s*"([^"]*)"/.exec(uri[1]);
    if (dv) defaultVersion = dv[1];
  }
  return {
    rootModule: create ? create[1].split('::').pop() : undefined,
    globalPrefix: prefix ? prefix[1] : undefined,
    uriVersioning: !!uri,
    defaultVersion,
    offset: (create ?? prefix ?? uri)!.index,
  };
}

// ------------------------------------------------------------------ paths

/** Joins path segments into a route path: `/api/v1/cats/:id`. */
export function joinRoute(...parts: (string | undefined)[]): string {
  const segs = parts
    .filter((p): p is string => p !== undefined)
    .map((p) => p.replace(/^\/+|\/+$/g, ''))
    .filter((p) => p.length > 0);
  return '/' + segs.join('/');
}

export interface RouteContext {
  globalPrefix?: string;
  uriVersioning?: boolean;
  defaultVersion?: string;
}

/** Full path of a route as the app would map it (first controller path and first version). */
export function fullRoutePath(route: RouteDef, controller: ControllerDef | undefined, ctx: RouteContext, pathIndex = 0): string {
  let version: string | undefined;
  if (ctx.uriVersioning) {
    const v = route.versions[0] ?? controller?.versions[0] ?? ctx.defaultVersion;
    if (v && v !== 'neutral' && stringLiteral(`"${v}"`) === v && !/[^\w.-]/.test(v)) version = 'v' + v;
  }
  return joinRoute(ctx.globalPrefix, version, controller?.paths[0], route.paths[pathIndex] ?? route.paths[0]);
}

// ------------------------------------------------------------------ attribute categories

export type AttrCategory = 'di' | 'http' | 'enhancer' | 'param' | 'event' | 'swagger' | 'validation' | 'orm';

const DI = new Set(['module', 'controller', 'injectable', 'routes', 'handlers', 'websocket_gateway', 'global', 'inject', 'init', 'optional', 'websocket_server']);
const ENHANCERS = new Set(['use_guards', 'use_interceptors', 'use_pipes', 'use_filters', 'set_metadata', 'apply_decorators', 'http_code', 'header', 'redirect', 'version']);
const PARAMS = new Set(['body', 'param', 'query', 'headers', 'ip', 'req', 'request', 'ctx', 'res', 'response', 'uploaded_file', 'uploaded_files', 'message_body', 'connected_socket']);
const VALIDATION = new Set([
  'is_string', 'is_int', 'is_number', 'is_boolean', 'is_array', 'is_object', 'is_defined', 'is_optional', 'is_not_empty', 'is_empty',
  'is_email', 'is_url', 'is_uuid', 'is_alpha', 'is_alphanumeric', 'is_number_string', 'is_numeric_string', 'is_date_string',
  'is_iso8601', 'is_hex_color', 'is_jwt', 'is_mongo_id', 'is_ip', 'is_lowercase', 'is_uppercase', 'is_positive', 'is_negative',
  'min_length', 'max_length', 'length', 'min', 'max', 'matches', 'contains', 'not_contains', 'is_in', 'is_not_in', 'equals',
  'not_equals', 'array_min_size', 'array_max_size', 'array_not_empty', 'array_unique', 'is_enum', 'validate_nested',
  'validate_with', 'validate_if',
]);
const ORM = new Set(['id', 'unique', 'relation', 'updated_at', 'map', 'db', 'db_type', 'model', 'orm_enum']);

export interface CategorizeOptions {
  /** The file derives `Validate` (enables class-validator attributes). */
  validation: boolean;
  /** The file derives `Model` (enables corvis-orm attributes). */
  orm: boolean;
}

export function fileCategorizeOptions(src: string): CategorizeOptions {
  return {
    validation: /derive\s*\([^)]*\bValidate\b/.test(src),
    orm: /derive\s*\([^)]*\b(?:Model|OrmEnum)\b/.test(src),
  };
}

export function categorize(a: Attr, opts: CategorizeOptions): AttrCategory | undefined {
  if (a.inner) return undefined;
  const { name, path } = a;
  if (path === 'corvis::main' || path === 'corvis::test') return 'di';
  if (path.includes('::') && !path.startsWith('corvis::')) return undefined;
  if (HTTP_ATTRS[name] !== undefined) return 'http';
  if (DI.has(name)) return 'di';
  if (ENHANCERS.has(name)) return 'enhancer';
  if (PARAMS.has(name)) return 'param';
  if (HANDLER_ATTRS.has(name)) return 'event';
  if (name.startsWith('api_')) return 'swagger';
  if (opts.validation && VALIDATION.has(name)) return 'validation';
  if (opts.orm && ORM.has(name)) return 'orm';
  return undefined;
}

/** 0-based line of an offset, using a precomputed table of line starts. */
export function lineOffsets(src: string): number[] {
  const starts = [0];
  for (let i = 0; i < src.length; i++) if (src.charCodeAt(i) === 10) starts.push(i + 1);
  return starts;
}

export function positionOf(starts: number[], offset: number): { line: number; character: number } {
  let lo = 0;
  let hi = starts.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (starts[mid] <= offset) lo = mid;
    else hi = mid - 1;
  }
  return { line: lo, character: offset - starts[lo] };
}
