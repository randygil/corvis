//! Port of the `append-field` package that multer uses to build `req.body` from the text fields
//! (`a[b][0]=x` → `{ a: { b: ['x'] } }`, `tags[]=x` → `{ tags: ['x'] }`, repeated keys → arrays).
//!
//! Array holes (`items[2]` before `items[0]`) are kept as `null`, which is what `JSON.stringify`
//! prints for them.

use serde_json::{Map, Value};

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

fn parse_path(key: &str) -> Vec<Step> {
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
                    Ok(n) if n <= 1_000_000 => steps.push(Step { key: Key::Array(n), append: false, next_is_array: false, last: false }),
                    _ => return failure(),
                }
                continue;
            }
            // /^\[([^\]]+)\]/
            if let Some(close) = inner.find(']') {
                if close > 0 {
                    let name = &inner[..close];
                    pos += close + 2;
                    steps.last_mut().unwrap().next_is_array = false;
                    steps.push(Step { key: Key::Object(name.to_string()), append: false, next_is_array: false, last: false });
                    continue;
                }
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
pub(crate) fn append_field(store: &mut Map<String, Value>, key: &str, value: Value) {
    let steps = parse_path(key);
    apply(&mut Ctx::Obj(store), &steps, value);
}

fn apply(ctx: &mut Ctx<'_>, steps: &[Step], value: Value) {
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
    match ctx.slot(&step.key) {
        Some(Value::Object(o)) => apply(&mut Ctx::Obj(o), rest, value),
        Some(Value::Array(a)) => apply(&mut Ctx::Arr(a), rest, value),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn build(pairs: &[(&str, &str)]) -> Value {
        let mut m = Map::new();
        for (k, v) in pairs {
            append_field(&mut m, k, json!(v));
        }
        Value::Object(m)
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
