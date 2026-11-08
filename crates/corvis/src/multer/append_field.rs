//! Port of the `append-field` package that multer uses to build `req.body` from the text fields
//! (`a[b][0]=x` → `{ a: { b: ['x'] } }`, `tags[]=x` → `{ tags: ['x'] }`, repeated keys → arrays).
//!
//! Array holes (`items[2]` before `items[0]`) are kept as `null`, which is what `JSON.stringify`
//! prints for them.
//!
//! JavaScript arrays are sparse: `x[1000000]=v` costs one slot in V8, but a `Vec<Value>` would
//! reserve a million. The holes a request may create are therefore bounded by an
//! [`ArrayBudget`] (1000 per request); an index that needs more holes turns that array into an
//! object with the indices as keys (`{ "1000000": "v" }`), like `qs` does past its `arrayLimit`.

use serde_json::{Map, Value};

/// Nesting depth of field names (`a[b][c]` = 2) when `limits.field_nesting_depth` is not set.
/// Deeper names are kept as a literal key (like `qs` past its `depth`), so a name with thousands
/// of `[x]` cannot build a value deep enough to overflow the stack.
pub(crate) const DEFAULT_MAX_FIELD_DEPTH: usize = 32;
/// Hard cap of the nesting depth, even with a bigger `limits.field_nesting_depth`.
pub(crate) const MAX_FIELD_DEPTH: usize = 256;

/// Holes (`null` padding) the fields of one request may create in arrays.
pub(crate) const MAX_ARRAY_HOLES: usize = 1000;

/// What is left of [`MAX_ARRAY_HOLES`] for the current request.
#[derive(Debug, Clone)]
pub(crate) struct ArrayBudget {
    holes_left: usize,
}

impl Default for ArrayBudget {
    fn default() -> Self {
        ArrayBudget { holes_left: MAX_ARRAY_HOLES }
    }
}

#[derive(Debug, Clone)]
enum Key {
    Object(String),
    Array(usize),
}

#[derive(Debug, Clone)]
struct Step {
    key: Key,
    append: bool,
    next_is_array: bool,
    last: bool,
}

fn parse_path(key: &str, max_depth: usize) -> Vec<Step> {
    let failure = || vec![Step { key: Key::Object(key.to_string()), append: false, next_is_array: false, last: true }];
    let first_end = key.find('[').unwrap_or(key.len());
    let first = &key[..first_end];
    if first.is_empty() {
        return failure();
    }
    let mut steps = vec![Step { key: Key::Object(first.to_string()), append: false, next_is_array: false, last: false }];
    let bytes = key.as_bytes();
    let mut pos = first_end;
    while pos < key.len() {
        if steps.len() > max_depth {
            return failure();
        }
        if bytes[pos] == b'[' && bytes.get(pos + 1) == Some(&b']') {
            pos += 2;
            steps.last_mut().unwrap().append = true;
            if pos != key.len() {
                return failure();
            }
            continue;
        }
        let rest = &key[pos..];
        // /^\[(\d+)\]/
        if let Some(inner) = rest.strip_prefix('[') {
            let digits: String = inner.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !digits.is_empty() && inner[digits.len()..].starts_with(']') {
                pos += digits.len() + 2;
                steps.last_mut().unwrap().next_is_array = true;
                // parseInt of a huge index would overflow; treat it like a plain key.
                match digits.parse::<usize>() {
                    Ok(n) => steps.push(Step { key: Key::Array(n), append: false, next_is_array: false, last: false }),
                    Err(_) => return failure(),
                }
                continue;
            }
            // /^\[([^\]]+)\]/
            if let Some(close) = inner.find(']')
                && close > 0
            {
                let name = &inner[..close];
                pos += close + 2;
                steps.last_mut().unwrap().next_is_array = false;
                steps.push(Step { key: Key::Object(name.to_string()), append: false, next_is_array: false, last: false });
                continue;
            }
        }
        return failure();
    }
    steps.last_mut().unwrap().last = true;
    steps
}

/// A container being navigated: an object or an array.
enum Ctx<'a> {
    Obj(&'a mut Map<String, Value>),
    Arr(&'a mut Vec<Value>),
}

impl Ctx<'_> {
    fn get(&self, key: &Key) -> Option<&Value> {
        match (self, key) {
            (Ctx::Obj(m), Key::Object(k)) => m.get(k),
            (Ctx::Obj(m), Key::Array(i)) => m.get(&i.to_string()),
            (Ctx::Arr(a), Key::Array(i)) => a.get(*i).filter(|v| !v.is_null()),
            (Ctx::Arr(_), Key::Object(_)) => None,
        }
    }

    fn set(&mut self, key: &Key, v: Value) {
        match (self, key) {
            (Ctx::Obj(m), Key::Object(k)) => {
                m.insert(k.clone(), v);
            }
            (Ctx::Obj(m), Key::Array(i)) => {
                m.insert(i.to_string(), v);
            }
            (Ctx::Arr(a), Key::Array(i)) => {
                if a.len() <= *i {
                    a.resize(*i + 1, Value::Null);
                }
                a[*i] = v;
            }
            (Ctx::Arr(_), Key::Object(_)) => {}
        }
    }

    fn slot(&mut self, key: &Key) -> Option<&mut Value> {
        match (self, key) {
            (Ctx::Obj(m), Key::Object(k)) => m.get_mut(k),
            (Ctx::Obj(m), Key::Array(i)) => m.get_mut(&i.to_string()),
            (Ctx::Arr(a), Key::Array(i)) => a.get_mut(*i),
            (Ctx::Arr(_), Key::Object(_)) => None,
        }
    }
}

fn set_last(ctx: &mut Ctx<'_>, step: &Step, entry: Value) {
    match ctx.get(&step.key).cloned() {
        None => {
            if step.append {
                ctx.set(&step.key, Value::Array(vec![entry]));
            } else {
                ctx.set(&step.key, entry);
            }
        }
        Some(Value::Array(_)) => {
            if let Some(Value::Array(a)) = ctx.slot(&step.key) {
                a.push(entry);
            }
        }
        Some(Value::Object(_)) => {
            if let Some(Value::Object(o)) = ctx.slot(&step.key) {
                let mut inner = Ctx::Obj(o);
                set_last(&mut inner, &Step { key: Key::Object(String::new()), append: false, next_is_array: false, last: true }, entry);
            }
        }
        Some(scalar) => ctx.set(&step.key, Value::Array(vec![scalar, entry])),
    }
}

/// `appendField(store, key, value)`.
///
/// `max_depth` bounds the nesting of `key` (see [`DEFAULT_MAX_FIELD_DEPTH`]).
pub(crate) fn append_field(store: &mut Map<String, Value>, budget: &mut ArrayBudget, key: &str, value: Value, max_depth: usize) {
    let steps = parse_path(key, max_depth.min(MAX_FIELD_DEPTH));
    apply(&mut Ctx::Obj(store), &steps, value, budget);
}

/// Before descending into `slot` with the index `next`: if it is an array and reaching the index
/// needs more holes than the budget allows, the array becomes an object keyed by index
/// (the existing items keep their positions as keys, holes are dropped).
fn reserve_holes(slot: &mut Value, next: &Key, budget: &mut ArrayBudget) {
    let (Value::Array(a), Key::Array(i)) = (&*slot, next) else { return };
    if *i < a.len() {
        return;
    }
    let holes = *i - a.len();
    if holes <= budget.holes_left {
        budget.holes_left -= holes;
        return;
    }
    let Value::Array(items) = std::mem::take(slot) else { return };
    let mut obj = Map::new();
    for (idx, item) in items.into_iter().enumerate() {
        if !item.is_null() {
            obj.insert(idx.to_string(), item);
        }
    }
    *slot = Value::Object(obj);
}

fn apply(ctx: &mut Ctx<'_>, steps: &[Step], value: Value, budget: &mut ArrayBudget) {
    let Some((step, rest)) = steps.split_first() else { return };
    if step.last {
        set_last(ctx, step, value);
        return;
    }
    // setValue for an intermediate step: make sure the container exists, then descend.
    match ctx.get(&step.key).cloned() {
        None => {
            let v = if step.next_is_array { Value::Array(vec![]) } else { Value::Object(Map::new()) };
            ctx.set(&step.key, v);
        }
        Some(Value::Object(_)) => {}
        Some(Value::Array(arr)) => {
            if !step.next_is_array {
                let mut obj = Map::new();
                for (i, item) in arr.into_iter().enumerate() {
                    if !item.is_null() {
                        obj.insert(i.to_string(), item);
                    }
                }
                ctx.set(&step.key, Value::Object(obj));
            }
        }
        Some(scalar) => {
            let mut obj = Map::new();
            obj.insert(String::new(), scalar);
            ctx.set(&step.key, Value::Object(obj));
        }
    }
    let Some(slot) = ctx.slot(&step.key) else { return };
    if let Some(next) = rest.first() {
        reserve_holes(slot, &next.key, budget);
    }
    match slot {
        Value::Object(o) => apply(&mut Ctx::Obj(o), rest, value, budget),
        Value::Array(a) => apply(&mut Ctx::Arr(a), rest, value, budget),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn build(pairs: &[(&str, &str)]) -> Value {
        let mut m = Map::new();
        let mut budget = ArrayBudget::default();
        for (k, v) in pairs {
            append_field(&mut m, &mut budget, k, json!(v), DEFAULT_MAX_FIELD_DEPTH);
        }
        Value::Object(m)
    }

    /// A name nested thousands of levels deep used to build an equally deep value (recursive
    /// `apply`, `js_key_order`, `Drop`): a stack overflow aborts the process. Past the depth
    /// limit the name is a literal key.
    #[test]
    fn deep_field_name_is_kept_literal() {
        let deep = format!("a{}", "[b]".repeat(100_000));
        let v = build(&[(deep.as_str(), "x")]);
        assert_eq!(v.as_object().unwrap().keys().next().map(|k| k.len()), Some(deep.len()));
        let ok = format!("a{}", "[b]".repeat(DEFAULT_MAX_FIELD_DEPTH));
        let v = build(&[(ok.as_str(), "x")]);
        let mut cur = &v["a"];
        for _ in 0..DEFAULT_MAX_FIELD_DEPTH {
            cur = &cur["b"];
        }
        assert_eq!(cur, &json!("x"));
        let over = format!("a{}", "[b]".repeat(DEFAULT_MAX_FIELD_DEPTH + 1));
        assert!(build(&[(over.as_str(), "x")]).get(&over).is_some());
    }

    /// `x[1000000]=v` used to reserve a dense `Vec` of a million `null`s (~200 MiB from a 71-byte
    /// body). Now holes are bounded per request and a far index becomes an object key.
    #[test]
    fn far_array_index_does_not_allocate_holes() {
        assert_eq!(build(&[("x[1000000]", "v")]), json!({ "x": { "1000000": "v" } }));
        assert_eq!(build(&[("x[99999999999]", "v")]), json!({ "x": { "99999999999": "v" } }));
        // Existing items keep their index as key.
        assert_eq!(build(&[("x[0]", "a"), ("x[5000]", "b")]), json!({ "x": { "0": "a", "5000": "b" } }));
        assert_eq!(build(&[("x[0][k]", "a"), ("x[5000][k]", "b")]), json!({ "x": { "0": { "k": "a" }, "5000": { "k": "b" } } }));
        // The budget is per request: the second far index no longer fits.
        let v = build(&[("a[600]", "1"), ("b[600]", "2")]);
        assert_eq!(v["a"].as_array().map(|a| a.len()), Some(601));
        assert_eq!(v["b"], json!({ "600": "2" }));
        // Many fields cannot add up: total padding stays within the budget.
        let mut m = Map::new();
        let mut budget = ArrayBudget::default();
        for i in 0..1000 {
            append_field(&mut m, &mut budget, &format!("x{i}[999]"), json!("v"), DEFAULT_MAX_FIELD_DEPTH);
        }
        let slots: usize = m.values().map(|v| v.as_array().map_or(0, |a| a.len())).sum();
        assert!(slots <= MAX_ARRAY_HOLES + 1000, "{slots}");
    }

    #[test]
    fn append_field_semantics() {
        assert_eq!(build(&[("a", "1"), ("a", "2"), ("a", "3")]), json!({ "a": ["1", "2", "3"] }));
        assert_eq!(build(&[("tags[]", "x")]), json!({ "tags": ["x"] }));
        assert_eq!(build(&[("user[name]", "ana"), ("user[age]", "3")]), json!({ "user": { "name": "ana", "age": "3" } }));
        assert_eq!(build(&[("items[1]", "b"), ("items[0]", "a")]), json!({ "items": ["a", "b"] }));
        assert_eq!(build(&[("items[1]", "b")]), json!({ "items": [null, "b"] }));
        assert_eq!(build(&[("a[0][x]", "1"), ("a[0][y]", "2")]), json!({ "a": [{ "x": "1", "y": "2" }] }));
        assert_eq!(build(&[("[weird", "1")]), json!({ "[weird": "1" }));
        assert_eq!(build(&[("a[]b", "1")]), json!({ "a[]b": "1" }));
        assert_eq!(build(&[("a", "1"), ("a[b]", "2")]), json!({ "a": { "": "1", "b": "2" } }));
    }
}
