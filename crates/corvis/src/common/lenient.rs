//! A type-driven deserializer over `serde_json::Value` that parses strings into numbers
//! and booleans when the target type asks for them. Used for route params, query
//! strings and headers (the implicit conversion of `ValidationPipe({ transform: true })`).

use serde::de::{self, DeserializeOwned, DeserializeSeed, IntoDeserializer, MapAccess, SeqAccess, Visitor};
use serde::forward_to_deserialize_any;
use serde_json::{Map, Value};

pub fn from_value<T: DeserializeOwned>(value: Value) -> Result<T, serde_json::Error> {
    T::deserialize(Lenient(value))
}

pub struct Lenient(pub Value);

type E = serde_json::Error;

fn parse_err(v: &str, ty: &str) -> E {
    de::Error::custom(format!("invalid value: \"{v}\", expected {ty}"))
}

macro_rules! lenient_number {
    ($method:ident, $visit:ident, $ty:ty) => {
        fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, E> {
            match self.0 {
                Value::String(s) => match s.trim().parse::<$ty>() {
                    Ok(n) => visitor.$visit(n),
                    Err(_) => Err(parse_err(&s, stringify!($ty))),
                },
                other => other.$method(visitor),
            }
        }
    };
}

impl<'de> de::Deserializer<'de> for Lenient {
    type Error = E;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, E> {
        match self.0 {
            Value::Array(a) => visitor.visit_seq(LenientSeq(a.into_iter())),
            Value::Object(o) => visitor.visit_map(LenientMap::new(o)),
            other => other.deserialize_any(visitor),
        }
    }

    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, E> {
        match self.0 {
            Value::String(s) => match s.as_str() {
                "true" | "1" | "on" | "yes" => visitor.visit_bool(true),
                "false" | "0" | "off" | "no" | "" => visitor.visit_bool(false),
                _ => Err(parse_err(&s, "a boolean")),
            },
            other => other.deserialize_bool(visitor),
        }
    }

    lenient_number!(deserialize_i8, visit_i8, i8);
    lenient_number!(deserialize_i16, visit_i16, i16);
    lenient_number!(deserialize_i32, visit_i32, i32);
    lenient_number!(deserialize_i64, visit_i64, i64);
    lenient_number!(deserialize_i128, visit_i128, i128);
    lenient_number!(deserialize_u8, visit_u8, u8);
    lenient_number!(deserialize_u16, visit_u16, u16);
    lenient_number!(deserialize_u32, visit_u32, u32);
    lenient_number!(deserialize_u64, visit_u64, u64);
    lenient_number!(deserialize_u128, visit_u128, u128);
    lenient_number!(deserialize_f32, visit_f32, f32);
    lenient_number!(deserialize_f64, visit_f64, f64);

    fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, E> {
        self.0.deserialize_char(visitor)
    }

    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, E> {
        self.deserialize_string(visitor)
    }

    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, E> {
        match self.0 {
            Value::Number(n) => visitor.visit_string(n.to_string()),
            Value::Bool(b) => visitor.visit_string(b.to_string()),
            Value::Array(mut a) if a.len() == 1 => Lenient(a.remove(0)).deserialize_string(visitor),
            other => other.deserialize_string(visitor),
        }
    }

    fn deserialize_bytes<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, E> {
        self.0.deserialize_bytes(visitor)
    }

    fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, E> {
        self.0.deserialize_byte_buf(visitor)
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, E> {
        match self.0 {
            Value::Null => visitor.visit_none(),
            other => visitor.visit_some(Lenient(other)),
        }
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, E> {
        self.0.deserialize_unit(visitor)
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(self, name: &'static str, visitor: V) -> Result<V::Value, E> {
        self.0.deserialize_unit_struct(name, visitor)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(self, _name: &'static str, visitor: V) -> Result<V::Value, E> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, E> {
        match self.0 {
            Value::Array(a) => visitor.visit_seq(LenientSeq(a.into_iter())),
            Value::Null => visitor.visit_seq(LenientSeq(Vec::new().into_iter())),
            other => visitor.visit_seq(LenientSeq(vec![other].into_iter())),
        }
    }

    fn deserialize_tuple<V: Visitor<'de>>(self, _len: usize, visitor: V) -> Result<V::Value, E> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(self, _name: &'static str, _len: usize, visitor: V) -> Result<V::Value, E> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, E> {
        match self.0 {
            Value::Object(o) => visitor.visit_map(LenientMap::new(o)),
            Value::Null => visitor.visit_map(LenientMap::new(Map::new())),
            other => other.deserialize_map(visitor),
        }
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, E> {
        self.deserialize_map(visitor)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, E> {
        self.0.deserialize_enum(name, variants, visitor)
    }

    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, E> {
        self.0.deserialize_identifier(visitor)
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, E> {
        visitor.visit_unit()
    }
}

struct LenientSeq(std::vec::IntoIter<Value>);

impl<'de> SeqAccess<'de> for LenientSeq {
    type Error = E;
    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>, E> {
        match self.0.next() {
            Some(v) => seed.deserialize(Lenient(v)).map(Some),
            None => Ok(None),
        }
    }
    fn size_hint(&self) -> Option<usize> {
        Some(self.0.len())
    }
}

struct LenientMap {
    iter: serde_json::map::IntoIter,
    value: Option<Value>,
}

impl LenientMap {
    fn new(map: Map<String, Value>) -> Self {
        LenientMap { iter: map.into_iter(), value: None }
    }
}

impl<'de> MapAccess<'de> for LenientMap {
    type Error = E;
    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, E> {
        match self.iter.next() {
            Some((k, v)) => {
                self.value = Some(v);
                let de: de::value::StringDeserializer<E> = k.into_deserializer();
                seed.deserialize(de).map(Some)
            }
            None => Ok(None),
        }
    }
    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, E> {
        let v = self.value.take().unwrap_or(Value::Null);
        seed.deserialize(Lenient(v))
    }
}

/// Captures the variant names of a serde enum (used for `#[is_enum]` messages).
pub(crate) struct VariantsCapture;

#[derive(Debug)]
pub(crate) struct CaptureError(pub Option<&'static [&'static str]>);
impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "capture")
    }
}
impl std::error::Error for CaptureError {}
impl de::Error for CaptureError {
    fn custom<T: std::fmt::Display>(_msg: T) -> Self {
        CaptureError(None)
    }
}

impl<'de> de::Deserializer<'de> for VariantsCapture {
    type Error = CaptureError;
    fn deserialize_any<V: Visitor<'de>>(self, _visitor: V) -> Result<V::Value, CaptureError> {
        Err(CaptureError(None))
    }
    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        variants: &'static [&'static str],
        _visitor: V,
    ) -> Result<V::Value, CaptureError> {
        Err(CaptureError(Some(variants)))
    }
    forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct identifier ignored_any
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::json;

    #[derive(Deserialize, Debug, PartialEq)]
    struct Q {
        page: u32,
        active: bool,
        name: String,
        tags: Vec<String>,
        limit: Option<i64>,
    }

    #[test]
    fn lenient_parsing() {
        let v = json!({ "page": "2", "active": "true", "name": "123", "tags": "a", "limit": "10" });
        let q: Q = from_value(v).unwrap();
        assert_eq!(q, Q { page: 2, active: true, name: "123".into(), tags: vec!["a".into()], limit: Some(10) });
        let n: i64 = from_value(json!("42")).unwrap();
        assert_eq!(n, 42);
    }
}
