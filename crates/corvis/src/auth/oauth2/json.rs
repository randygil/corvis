//! Parsing of the provider's JSON, which is untrusted.
//!
//! `serde_json::Value` (and `IgnoredAny`) treat the key `$serde_json::private::RawValue` specially when the
//! `raw_value` feature is on, which an application that uses the ORM has (sqlx turns it on): such a key makes the
//! parser read a nested document again with a fresh recursion budget. This visitor has no special key, so the parser's
//! own depth limit (128) is the only budget whatever features are enabled. A duplicate key keeps the last value.

use serde_json::{Map, Value};

struct Plain(Value);

impl<'de> serde::Deserialize<'de> for Plain {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::{MapAccess, SeqAccess, Visitor};
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Value;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("any JSON value")
            }
            fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
                Ok(Value::Bool(v))
            }
            fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
                Ok(Value::from(v))
            }
            fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
                Ok(Value::from(v))
            }
            fn visit_f64<E>(self, v: f64) -> Result<Value, E> {
                Ok(Value::from(v))
            }
            fn visit_str<E>(self, v: &str) -> Result<Value, E> {
                Ok(Value::String(v.to_owned()))
            }
            fn visit_string<E>(self, v: String) -> Result<Value, E> {
                Ok(Value::String(v))
            }
            fn visit_unit<E>(self) -> Result<Value, E> {
                Ok(Value::Null)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
                let mut items = Vec::new();
                while let Some(Plain(item)) = seq.next_element()? {
                    items.push(item);
                }
                Ok(Value::Array(items))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
                let mut out = Map::new();
                while let Some((key, Plain(value))) = map.next_entry::<String, Plain>()? {
                    out.insert(key, value);
                }
                Ok(Value::Object(out))
            }
        }
        deserializer.deserialize_any(V).map(Plain)
    }
}

/// Parses untrusted JSON bytes: `None` for anything that is not one complete valid document.
pub(crate) fn parse(bytes: &[u8]) -> Option<Value> {
    serde_json::from_slice::<Plain>(bytes).ok().map(|p| p.0)
}

/// Parses untrusted JSON bytes that must be an object.
pub(crate) fn parse_object(bytes: &[u8]) -> Option<Map<String, Value>> {
    match parse(bytes)? {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_documents_and_refuses_the_rest() {
        assert_eq!(parse(br#"{"a":[1,"x",null,true,1.5]}"#), Some(serde_json::json!({ "a": [1, "x", null, true, 1.5] })));
        assert!(parse(b"{").is_none());
        assert!(parse(b"{} x").is_none());
        assert!(parse(b"").is_none());
        assert!(parse_object(b"[1]").is_none());
        assert!(parse_object(b"\"s\"").is_none());
        assert!(parse_object(br#"{"a":1}"#).is_some());
    }

    #[test]
    fn the_raw_value_key_is_an_ordinary_key() {
        let doc = br#"{"$serde_json::private::RawValue":{"a":1}}"#;
        let v = parse(doc).unwrap();
        assert_eq!(v["$serde_json::private::RawValue"]["a"], 1);
    }

    #[test]
    fn deep_nesting_is_an_error_not_a_stack_overflow() {
        let deep = format!("{}{}", "[".repeat(1000), "]".repeat(1000));
        assert!(parse(deep.as_bytes()).is_none());
        let objects = format!("{}1{}", "{\"a\":".repeat(1000), "}".repeat(1000));
        assert!(parse(objects.as_bytes()).is_none());
    }

    #[test]
    fn a_duplicate_key_keeps_the_last_value() {
        assert_eq!(parse(br#"{"a":1,"a":2}"#), Some(serde_json::json!({ "a": 2 })));
    }
}
