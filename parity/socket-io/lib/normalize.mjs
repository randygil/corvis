// Normalization of a transcript before comparison.
//
// Session ids (the `sid` of an Engine.IO open packet and of a Socket.IO CONNECT answer) are random by design. They are
// recognized by the field they come from (a property named `sid`), numbered `<id:1>`, `<id:2>`, ... in order of first
// appearance, and every later occurrence of the same string is replaced by the same placeholder, so equal ids stay
// equal and different ids stay different. A string that merely looks like an id is never rewritten.
//
// Entries of different clients are independent: they are ordered by client name (keeping each client's own order), so
// the interleaving between clients never shows up as a difference.

function sortedByClient(entries) {
  return entries
    .map((entry, index) => ({ entry, index }))
    .sort((a, b) => {
      if (a.entry.client < b.entry.client) return -1;
      if (a.entry.client > b.entry.client) return 1;
      return a.index - b.index;
    })
    .map(({ entry }) => entry);
}

function collectIds(value, ids) {
  if (Array.isArray(value)) {
    for (const item of value) collectIds(item, ids);
  } else if (value !== null && typeof value === "object") {
    for (const [key, item] of Object.entries(value)) {
      if (key === "sid" && typeof item === "string" && item !== "" && !ids.has(item)) {
        ids.set(item, `<id:${ids.size + 1}>`);
      }
      collectIds(item, ids);
    }
  }
}

function rewrite(value, ids) {
  if (typeof value === "string") {
    let out = value;
    for (const [id, placeholder] of ids) out = out.split(id).join(placeholder);
    return out;
  }
  if (Array.isArray(value)) return value.map((item) => rewrite(item, ids));
  if (value !== null && typeof value === "object") {
    const out = {};
    for (const [key, item] of Object.entries(value)) out[key] = rewrite(item, ids);
    return out;
  }
  return value;
}

export function normalize(transcript) {
  const entries = sortedByClient(transcript.entries);
  const ids = new Map();
  collectIds(entries, ids);
  return { scenario: transcript.scenario, entries: rewrite(entries, ids) };
}
