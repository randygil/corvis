//! Socket.IO arguments: JSON with binary values at any depth.
//!
//! `serde_json::Value` cannot hold bytes, so the arguments of an event are [`IoValue`]s: a JSON tree whose leaves may
//! be [`Binary`]. A handler that carries no binary payload never has to name them: `Args::get::<T>(i)` deserializes
//! an argument into any `serde` type, `IntoArgs` accepts tuples of `Serialize` values, and `IoValue` converts from
//! and to `serde_json::Value` (`TryFrom`: a value nested deeper than [`MAX_DEPTH`] is refused, and an `IoValue` with
//! binary data cannot become a `Value`).
//!
//! Every recursion in this layer is bounded by [`MAX_DEPTH`] (127 levels: `serde_json`'s parser refuses the 128th).

use bytes::Bytes;
use indexmap::IndexMap;
use serde::de::{self, DeserializeSeed, IntoDeserializer, Visitor};
use serde::{Deserialize, Serialize, ser};
use serde_json::{Number, Value};

/// The deepest nesting of arrays and objects accepted anywhere in this layer (decoding, placeholder substitution,
/// conversion, encoding). Node has no limit.
pub const MAX_DEPTH: usize = 127;

/// The name `Binary` serializes under (the technique `serde_json::RawValue` uses): a serializer that knows it turns
/// the field into a binary leaf; any other serializer sees a byte sequence.
pub(crate) const BINARY_MARKER: &str = "$corvis::socket_io::Binary";

/// What one decoded JSON value costs in memory, whatever it holds: a node, the longest of `IoValue`'s variants
/// rounded up. An empty array costs as much as a small number.
pub(crate) const NODE_COST: usize = 48;
/// What one object entry costs besides its key text and its value.
pub(crate) const ENTRY_COST: usize = 64;

/// An error converting to or from an [`IoValue`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IoValueError {
    /// A `serde` error raised by the type being converted.
    #[error("{0}")]
    Custom(String),
    /// The value is nested deeper than 127 levels.
    #[error("the value is nested deeper than 127 levels")]
    TooDeep,
    /// The value holds binary data, which JSON cannot carry.
    #[error("the value contains binary data")]
    ContainsBinary,
    /// A map key that is not a string (or a number or boolean, written as text).
    #[error("a map key must be a string")]
    KeyNotString,
}

impl ser::Error for IoValueError {
    fn custom<T: std::fmt::Display>(msg: T) -> Self {
        IoValueError::Custom(msg.to_string())
    }
}

impl de::Error for IoValueError {
    fn custom<T: std::fmt::Display>(msg: T) -> Self {
        IoValueError::Custom(msg.to_string())
    }
}

/// Binary data in an event: a `Buffer` in Node, `Bytes` here, so cloning it (or a placeholder repeated many times)
/// never copies the bytes.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Binary(Bytes);

impl Binary {
    pub fn new(bytes: impl Into<Bytes>) -> Self {
        Binary(bytes.into())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The shared buffer (a cheap clone).
    pub fn bytes(&self) -> Bytes {
        self.0.clone()
    }

    pub fn into_bytes(self) -> Bytes {
        self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for Binary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Binary({} bytes)", self.0.len())
    }
}

impl std::ops::Deref for Binary {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.0
    }
}

impl From<Bytes> for Binary {
    fn from(v: Bytes) -> Self {
        Binary(v)
    }
}
impl From<Vec<u8>> for Binary {
    fn from(v: Vec<u8>) -> Self {
        Binary(Bytes::from(v))
    }
}
impl From<&[u8]> for Binary {
    fn from(v: &[u8]) -> Self {
        Binary(Bytes::copy_from_slice(v))
    }
}
impl From<Binary> for Bytes {
    fn from(v: Binary) -> Self {
        v.0
    }
}

struct BytesRef<'a>(&'a [u8]);

impl Serialize for BytesRef<'_> {
    fn serialize<S: ser::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(self.0)
    }
}

impl Serialize for Binary {
    fn serialize<S: ser::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_newtype_struct(BINARY_MARKER, &BytesRef(&self.0))
    }
}

struct BytesVisitor;

impl<'de> Visitor<'de> for BytesVisitor {
    type Value = Binary;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("binary data")
    }
    fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<Binary, E> {
        Ok(Binary::from(v))
    }
    fn visit_byte_buf<E: de::Error>(self, v: Vec<u8>) -> Result<Binary, E> {
        Ok(Binary::from(v))
    }
    fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Binary, A::Error> {
        let mut out = Vec::new();
        while let Some(byte) = seq.next_element::<u8>()? {
            out.push(byte);
        }
        Ok(Binary::from(out))
    }
    fn visit_newtype_struct<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<Binary, D::Error> {
        deserializer.deserialize_bytes(BytesVisitor)
    }
}

impl<'de> Deserialize<'de> for Binary {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_newtype_struct(BINARY_MARKER, BytesVisitor)
    }
}

/// A JSON tree whose leaves may be [`Binary`]: one argument of an event or acknowledgement. Objects keep their
/// insertion order, as `JSON.stringify` does.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum IoValue {
    #[default]
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<IoValue>),
    Object(Box<IndexMap<String, IoValue>>),
    Binary(Binary),
}

impl IoValue {
    /// An object from `(key, value)` pairs.
    pub fn object<K: Into<String>>(entries: impl IntoIterator<Item = (K, IoValue)>) -> IoValue {
        IoValue::Object(Box::new(entries.into_iter().map(|(k, v)| (k.into(), v)).collect()))
    }

    pub fn is_null(&self) -> bool {
        matches!(self, IoValue::Null)
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            IoValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            IoValue::Number(n) => n.as_i64(),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            IoValue::Number(n) => n.as_u64(),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            IoValue::Number(n) => n.as_f64(),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            IoValue::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[IoValue]> {
        match self {
            IoValue::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&IndexMap<String, IoValue>> {
        match self {
            IoValue::Object(o) => Some(o),
            _ => None,
        }
    }

    pub fn as_binary(&self) -> Option<&Binary> {
        match self {
            IoValue::Binary(b) => Some(b),
            _ => None,
        }
    }

    /// The member `key` of an object.
    pub fn get(&self, key: &str) -> Option<&IoValue> {
        self.as_object().and_then(|o| o.get(key))
    }

    /// `true` when a [`Binary`] is anywhere in the value.
    pub fn contains_binary(&self) -> bool {
        let mut stack = vec![self];
        while let Some(v) = stack.pop() {
            match v {
                IoValue::Binary(_) => return true,
                IoValue::Array(a) => stack.extend(a.iter()),
                IoValue::Object(o) => stack.extend(o.values()),
                _ => {}
            }
        }
        false
    }

    /// What the value costs in memory, with the model of `IoOptions::max_decoded_bytes`: [`NODE_COST`] per value,
    /// the text of strings and keys, [`ENTRY_COST`] per object entry. Binary data is charged where it arrives, as
    /// an attachment, so a placeholder repeated many times costs one node each and no bytes.
    pub(crate) fn cost(&self) -> usize {
        let mut total = 0usize;
        let mut stack = vec![self];
        while let Some(v) = stack.pop() {
            total = total.saturating_add(NODE_COST);
            match v {
                IoValue::String(s) => total = total.saturating_add(s.len()),
                IoValue::Array(a) => stack.extend(a.iter()),
                IoValue::Object(o) => {
                    for (k, v) in o.iter() {
                        total = total.saturating_add(ENTRY_COST).saturating_add(k.len());
                        stack.push(v);
                    }
                }
                _ => {}
            }
        }
        total
    }

    /// Converts any `Serialize` value, turning [`Binary`] fields into binary leaves. A value nested deeper than
    /// [`MAX_DEPTH`] fails.
    pub fn from_serialize<T: Serialize + ?Sized>(value: &T) -> Result<IoValue, IoValueError> {
        value.serialize(IoSerializer { depth: 0 })
    }

    /// Deserializes the value into any `serde` type; a field of type [`Binary`] takes a binary leaf (the bytes are
    /// copied once).
    pub fn deserialize_into<'de, T: Deserialize<'de>>(&'de self) -> Result<T, IoValueError> {
        T::deserialize(self)
    }

    /// The same value as plain JSON; fails when it holds binary data.
    pub fn to_json(&self) -> Result<Value, IoValueError> {
        to_json(self, 0)
    }
}

fn to_json(v: &IoValue, depth: usize) -> Result<Value, IoValueError> {
    Ok(match v {
        IoValue::Null => Value::Null,
        IoValue::Bool(b) => Value::Bool(*b),
        IoValue::Number(n) => Value::Number(n.clone()),
        IoValue::String(s) => Value::String(s.clone()),
        IoValue::Binary(_) => return Err(IoValueError::ContainsBinary),
        IoValue::Array(a) => {
            if depth + 1 > MAX_DEPTH {
                return Err(IoValueError::TooDeep);
            }
            Value::Array(a.iter().map(|x| to_json(x, depth + 1)).collect::<Result<_, _>>()?)
        }
        IoValue::Object(o) => {
            if depth + 1 > MAX_DEPTH {
                return Err(IoValueError::TooDeep);
            }
            let mut map = serde_json::Map::new();
            for (k, x) in o.iter() {
                map.insert(k.clone(), to_json(x, depth + 1)?);
            }
            Value::Object(map)
        }
    })
}

fn from_json(v: Value, depth: usize) -> Result<IoValue, IoValueError> {
    Ok(match v {
        Value::Null => IoValue::Null,
        Value::Bool(b) => IoValue::Bool(b),
        Value::Number(n) => IoValue::Number(n),
        Value::String(s) => IoValue::String(s),
        Value::Array(a) => {
            if depth + 1 > MAX_DEPTH {
                return Err(IoValueError::TooDeep);
            }
            IoValue::Array(a.into_iter().map(|x| from_json(x, depth + 1)).collect::<Result<_, _>>()?)
        }
        Value::Object(o) => {
            if depth + 1 > MAX_DEPTH {
                return Err(IoValueError::TooDeep);
            }
            let mut map = IndexMap::new();
            for (k, x) in o {
                map.insert(k, from_json(x, depth + 1)?);
            }
            IoValue::Object(Box::new(map))
        }
    })
}

impl TryFrom<Value> for IoValue {
    type Error = IoValueError;
    fn try_from(v: Value) -> Result<Self, Self::Error> {
        from_json(v, 0)
    }
}

impl TryFrom<&Value> for IoValue {
    type Error = IoValueError;
    fn try_from(v: &Value) -> Result<Self, Self::Error> {
        from_json(v.clone(), 0)
    }
}

impl TryFrom<IoValue> for Value {
    type Error = IoValueError;
    fn try_from(v: IoValue) -> Result<Self, Self::Error> {
        v.to_json()
    }
}

impl TryFrom<&IoValue> for Value {
    type Error = IoValueError;
    fn try_from(v: &IoValue) -> Result<Self, Self::Error> {
        v.to_json()
    }
}

impl From<bool> for IoValue {
    fn from(v: bool) -> Self {
        IoValue::Bool(v)
    }
}
impl From<&str> for IoValue {
    fn from(v: &str) -> Self {
        IoValue::String(v.to_string())
    }
}
impl From<String> for IoValue {
    fn from(v: String) -> Self {
        IoValue::String(v)
    }
}
impl From<Binary> for IoValue {
    fn from(v: Binary) -> Self {
        IoValue::Binary(v)
    }
}
impl From<Bytes> for IoValue {
    fn from(v: Bytes) -> Self {
        IoValue::Binary(Binary(v))
    }
}
impl From<Vec<IoValue>> for IoValue {
    fn from(v: Vec<IoValue>) -> Self {
        IoValue::Array(v)
    }
}
macro_rules! from_number {
    ($($t:ty),*) => {$(
        impl From<$t> for IoValue {
            fn from(v: $t) -> Self { IoValue::Number(Number::from(v)) }
        }
    )*};
}
from_number!(i8, i16, i32, i64, u8, u16, u32, u64, usize, isize);

impl From<f64> for IoValue {
    /// A number; `NaN` and the infinities have no JSON form and become `null`, as `JSON.stringify` writes them.
    fn from(v: f64) -> Self {
        Number::from_f64(v).map(IoValue::Number).unwrap_or(IoValue::Null)
    }
}

impl Serialize for IoValue {
    fn serialize<S: ser::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use ser::{SerializeMap, SerializeSeq};
        match self {
            IoValue::Null => serializer.serialize_unit(),
            IoValue::Bool(b) => serializer.serialize_bool(*b),
            IoValue::Number(n) => n.serialize(serializer),
            IoValue::String(s) => serializer.serialize_str(s),
            IoValue::Binary(b) => b.serialize(serializer),
            IoValue::Array(a) => {
                let mut seq = serializer.serialize_seq(Some(a.len()))?;
                for x in a {
                    seq.serialize_element(x)?;
                }
                seq.end()
            }
            IoValue::Object(o) => {
                let mut map = serializer.serialize_map(Some(o.len()))?;
                for (k, v) in o.iter() {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for IoValue {
    /// Builds the value with a plain visitor: no special keys, so it does not depend on which `serde_json` features
    /// the build enables. The source deserializer bounds the depth (`serde_json` refuses the 128th level); packets received
    /// from clients do not come through here, the decoder has its own bounded reader.
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = IoValue;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("any JSON value")
            }
            fn visit_bool<E>(self, v: bool) -> Result<IoValue, E> {
                Ok(IoValue::Bool(v))
            }
            fn visit_i64<E>(self, v: i64) -> Result<IoValue, E> {
                Ok(IoValue::from(v))
            }
            fn visit_u64<E>(self, v: u64) -> Result<IoValue, E> {
                Ok(IoValue::from(v))
            }
            fn visit_f64<E>(self, v: f64) -> Result<IoValue, E> {
                Ok(IoValue::from(v))
            }
            fn visit_str<E>(self, v: &str) -> Result<IoValue, E> {
                Ok(IoValue::String(v.to_owned()))
            }
            fn visit_string<E>(self, v: String) -> Result<IoValue, E> {
                Ok(IoValue::String(v))
            }
            fn visit_bytes<E>(self, v: &[u8]) -> Result<IoValue, E> {
                Ok(IoValue::Binary(Binary::from(v)))
            }
            fn visit_byte_buf<E>(self, v: Vec<u8>) -> Result<IoValue, E> {
                Ok(IoValue::Binary(Binary::from(v)))
            }
            fn visit_unit<E>(self) -> Result<IoValue, E> {
                Ok(IoValue::Null)
            }
            fn visit_none<E>(self) -> Result<IoValue, E> {
                Ok(IoValue::Null)
            }
            fn visit_some<D: de::Deserializer<'de>>(self, d: D) -> Result<IoValue, D::Error> {
                IoValue::deserialize(d)
            }
            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<IoValue, A::Error> {
                let mut items = Vec::new();
                while let Some(item) = seq.next_element::<IoValue>()? {
                    items.push(item);
                }
                Ok(IoValue::Array(items))
            }
            fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<IoValue, A::Error> {
                let mut out = IndexMap::new();
                while let Some((k, v)) = map.next_entry::<String, IoValue>()? {
                    out.insert(k, v);
                }
                Ok(IoValue::Object(Box::new(out)))
            }
        }
        deserializer.deserialize_any(V)
    }
}

// ------------------------------------------------------------------------------------------------ serializer

struct IoSerializer {
    /// Containers enclosing the value being serialized.
    depth: usize,
}

impl IoSerializer {
    fn child(&self) -> Result<usize, IoValueError> {
        if self.depth + 1 > MAX_DEPTH { Err(IoValueError::TooDeep) } else { Ok(self.depth + 1) }
    }
}

struct SeqBuilder {
    depth: usize,
    items: Vec<IoValue>,
    /// `Some(variant)` for a tuple variant, wrapped as `{variant: [..]}`.
    variant: Option<&'static str>,
}

struct MapBuilder {
    depth: usize,
    entries: IndexMap<String, IoValue>,
    key: Option<String>,
    variant: Option<&'static str>,
}

fn wrap_variant(variant: Option<&'static str>, value: IoValue) -> IoValue {
    match variant {
        Some(name) => {
            let mut map = IndexMap::new();
            map.insert(name.to_string(), value);
            IoValue::Object(Box::new(map))
        }
        None => value,
    }
}

impl ser::Serializer for IoSerializer {
    type Ok = IoValue;
    type Error = IoValueError;
    type SerializeSeq = SeqBuilder;
    type SerializeTuple = SeqBuilder;
    type SerializeTupleStruct = SeqBuilder;
    type SerializeTupleVariant = SeqBuilder;
    type SerializeMap = MapBuilder;
    type SerializeStruct = MapBuilder;
    type SerializeStructVariant = MapBuilder;

    fn serialize_bool(self, v: bool) -> Result<IoValue, IoValueError> {
        Ok(IoValue::Bool(v))
    }
    fn serialize_i8(self, v: i8) -> Result<IoValue, IoValueError> {
        Ok(IoValue::from(v))
    }
    fn serialize_i16(self, v: i16) -> Result<IoValue, IoValueError> {
        Ok(IoValue::from(v))
    }
    fn serialize_i32(self, v: i32) -> Result<IoValue, IoValueError> {
        Ok(IoValue::from(v))
    }
    fn serialize_i64(self, v: i64) -> Result<IoValue, IoValueError> {
        Ok(IoValue::from(v))
    }
    fn serialize_u8(self, v: u8) -> Result<IoValue, IoValueError> {
        Ok(IoValue::from(v))
    }
    fn serialize_u16(self, v: u16) -> Result<IoValue, IoValueError> {
        Ok(IoValue::from(v))
    }
    fn serialize_u32(self, v: u32) -> Result<IoValue, IoValueError> {
        Ok(IoValue::from(v))
    }
    fn serialize_u64(self, v: u64) -> Result<IoValue, IoValueError> {
        Ok(IoValue::from(v))
    }
    fn serialize_f32(self, v: f32) -> Result<IoValue, IoValueError> {
        Ok(IoValue::from(f64::from(v)))
    }
    fn serialize_f64(self, v: f64) -> Result<IoValue, IoValueError> {
        Ok(IoValue::from(v))
    }
    fn serialize_char(self, v: char) -> Result<IoValue, IoValueError> {
        Ok(IoValue::String(v.to_string()))
    }
    fn serialize_str(self, v: &str) -> Result<IoValue, IoValueError> {
        Ok(IoValue::String(v.to_string()))
    }
    fn serialize_bytes(self, v: &[u8]) -> Result<IoValue, IoValueError> {
        // As `serde_json` does for a serializer that has no byte type.
        Ok(IoValue::Array(v.iter().map(|b| IoValue::from(*b)).collect()))
    }
    fn serialize_none(self) -> Result<IoValue, IoValueError> {
        Ok(IoValue::Null)
    }
    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<IoValue, IoValueError> {
        value.serialize(self)
    }
    fn serialize_unit(self) -> Result<IoValue, IoValueError> {
        Ok(IoValue::Null)
    }
    fn serialize_unit_struct(self, _name: &'static str) -> Result<IoValue, IoValueError> {
        Ok(IoValue::Null)
    }
    fn serialize_unit_variant(self, _name: &'static str, _index: u32, variant: &'static str) -> Result<IoValue, IoValueError> {
        Ok(IoValue::String(variant.to_string()))
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(self, name: &'static str, value: &T) -> Result<IoValue, IoValueError> {
        if name == BINARY_MARKER {
            return value.serialize(BinaryCapture);
        }
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<IoValue, IoValueError> {
        let depth = self.child()?;
        Ok(wrap_variant(Some(variant), value.serialize(IoSerializer { depth })?))
    }
    fn serialize_seq(self, len: Option<usize>) -> Result<SeqBuilder, IoValueError> {
        let depth = self.child()?;
        // The hint comes from the value being serialized, which is application data, but never reserve blindly.
        Ok(SeqBuilder { depth, items: Vec::with_capacity(len.unwrap_or(0).min(1024)), variant: None })
    }
    fn serialize_tuple(self, len: usize) -> Result<SeqBuilder, IoValueError> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_struct(self, _name: &'static str, len: usize) -> Result<SeqBuilder, IoValueError> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<SeqBuilder, IoValueError> {
        let mut seq = self.serialize_seq(Some(len))?;
        // The wrapping object is one more level.
        if seq.depth + 1 > MAX_DEPTH {
            return Err(IoValueError::TooDeep);
        }
        seq.variant = Some(variant);
        Ok(seq)
    }
    fn serialize_map(self, _len: Option<usize>) -> Result<MapBuilder, IoValueError> {
        let depth = self.child()?;
        Ok(MapBuilder { depth, entries: IndexMap::new(), key: None, variant: None })
    }
    fn serialize_struct(self, _name: &'static str, _len: usize) -> Result<MapBuilder, IoValueError> {
        self.serialize_map(None)
    }
    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<MapBuilder, IoValueError> {
        let mut map = self.serialize_map(None)?;
        if map.depth + 1 > MAX_DEPTH {
            return Err(IoValueError::TooDeep);
        }
        map.variant = Some(variant);
        Ok(map)
    }
}

impl SeqBuilder {
    fn push<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), IoValueError> {
        self.items.push(value.serialize(IoSerializer { depth: self.depth })?);
        Ok(())
    }
    fn finish(self) -> Result<IoValue, IoValueError> {
        Ok(wrap_variant(self.variant, IoValue::Array(self.items)))
    }
}

impl ser::SerializeSeq for SeqBuilder {
    type Ok = IoValue;
    type Error = IoValueError;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), IoValueError> {
        self.push(value)
    }
    fn end(self) -> Result<IoValue, IoValueError> {
        self.finish()
    }
}
impl ser::SerializeTuple for SeqBuilder {
    type Ok = IoValue;
    type Error = IoValueError;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), IoValueError> {
        self.push(value)
    }
    fn end(self) -> Result<IoValue, IoValueError> {
        self.finish()
    }
}
impl ser::SerializeTupleStruct for SeqBuilder {
    type Ok = IoValue;
    type Error = IoValueError;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), IoValueError> {
        self.push(value)
    }
    fn end(self) -> Result<IoValue, IoValueError> {
        self.finish()
    }
}
impl ser::SerializeTupleVariant for SeqBuilder {
    type Ok = IoValue;
    type Error = IoValueError;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), IoValueError> {
        self.push(value)
    }
    fn end(self) -> Result<IoValue, IoValueError> {
        self.finish()
    }
}

impl MapBuilder {
    fn finish(self) -> Result<IoValue, IoValueError> {
        Ok(wrap_variant(self.variant, IoValue::Object(Box::new(self.entries))))
    }
    fn field<T: Serialize + ?Sized>(&mut self, key: &str, value: &T) -> Result<(), IoValueError> {
        let v = value.serialize(IoSerializer { depth: self.depth })?;
        self.entries.insert(key.to_string(), v);
        Ok(())
    }
}

impl ser::SerializeMap for MapBuilder {
    type Ok = IoValue;
    type Error = IoValueError;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), IoValueError> {
        let key = match key.serialize(IoSerializer { depth: self.depth })? {
            IoValue::String(s) => s,
            IoValue::Number(n) => n.to_string(),
            IoValue::Bool(b) => b.to_string(),
            _ => return Err(IoValueError::KeyNotString),
        };
        self.key = Some(key);
        Ok(())
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), IoValueError> {
        let key = self.key.take().ok_or(IoValueError::KeyNotString)?;
        self.field(&key, value)
    }
    fn end(self) -> Result<IoValue, IoValueError> {
        self.finish()
    }
}
impl ser::SerializeStruct for MapBuilder {
    type Ok = IoValue;
    type Error = IoValueError;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, key: &'static str, value: &T) -> Result<(), IoValueError> {
        self.field(key, value)
    }
    fn end(self) -> Result<IoValue, IoValueError> {
        self.finish()
    }
}
impl ser::SerializeStructVariant for MapBuilder {
    type Ok = IoValue;
    type Error = IoValueError;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, key: &'static str, value: &T) -> Result<(), IoValueError> {
        self.field(key, value)
    }
    fn end(self) -> Result<IoValue, IoValueError> {
        self.finish()
    }
}

/// Accepts only the bytes a `Binary` serializes inside its marker.
struct BinaryCapture;

macro_rules! capture_refuses {
    ($($name:ident($($arg:ty),*))*) => {$(
        fn $name(self, $(_: $arg),*) -> Result<IoValue, IoValueError> {
            Err(IoValueError::Custom("expected binary data".into()))
        }
    )*};
}

impl ser::Serializer for BinaryCapture {
    type Ok = IoValue;
    type Error = IoValueError;
    type SerializeSeq = ser::Impossible<IoValue, IoValueError>;
    type SerializeTuple = ser::Impossible<IoValue, IoValueError>;
    type SerializeTupleStruct = ser::Impossible<IoValue, IoValueError>;
    type SerializeTupleVariant = ser::Impossible<IoValue, IoValueError>;
    type SerializeMap = ser::Impossible<IoValue, IoValueError>;
    type SerializeStruct = ser::Impossible<IoValue, IoValueError>;
    type SerializeStructVariant = ser::Impossible<IoValue, IoValueError>;

    fn serialize_bytes(self, v: &[u8]) -> Result<IoValue, IoValueError> {
        Ok(IoValue::Binary(Binary::from(v)))
    }
    capture_refuses! {
        serialize_bool(bool) serialize_i8(i8) serialize_i16(i16) serialize_i32(i32) serialize_i64(i64)
        serialize_u8(u8) serialize_u16(u16) serialize_u32(u32) serialize_u64(u64) serialize_f32(f32) serialize_f64(f64)
        serialize_char(char) serialize_str(&str) serialize_none() serialize_unit()
        serialize_unit_struct(&'static str) serialize_unit_variant(&'static str, u32, &'static str)
    }
    fn serialize_some<T: Serialize + ?Sized>(self, _: &T) -> Result<IoValue, IoValueError> {
        Err(IoValueError::Custom("expected binary data".into()))
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(self, _: &'static str, _: &T) -> Result<IoValue, IoValueError> {
        Err(IoValueError::Custom("expected binary data".into()))
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<IoValue, IoValueError> {
        Err(IoValueError::Custom("expected binary data".into()))
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, IoValueError> {
        Err(IoValueError::Custom("expected binary data".into()))
    }
    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, IoValueError> {
        Err(IoValueError::Custom("expected binary data".into()))
    }
    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeTupleStruct, IoValueError> {
        Err(IoValueError::Custom("expected binary data".into()))
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, IoValueError> {
        Err(IoValueError::Custom("expected binary data".into()))
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, IoValueError> {
        Err(IoValueError::Custom("expected binary data".into()))
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeStruct, IoValueError> {
        Err(IoValueError::Custom("expected binary data".into()))
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, IoValueError> {
        Err(IoValueError::Custom("expected binary data".into()))
    }
}

// ---------------------------------------------------------------------------------------------- deserializer

struct SeqDe<'de> {
    iter: std::slice::Iter<'de, IoValue>,
}

impl<'de> de::SeqAccess<'de> for SeqDe<'de> {
    type Error = IoValueError;
    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>, IoValueError> {
        match self.iter.next() {
            Some(v) => seed.deserialize(v).map(Some),
            None => Ok(None),
        }
    }
    fn size_hint(&self) -> Option<usize> {
        Some(self.iter.len())
    }
}

struct BytesSeqDe<'de> {
    iter: std::slice::Iter<'de, u8>,
}

impl<'de> de::SeqAccess<'de> for BytesSeqDe<'de> {
    type Error = IoValueError;
    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>, IoValueError> {
        match self.iter.next() {
            Some(b) => seed.deserialize((*b).into_deserializer()).map(Some),
            None => Ok(None),
        }
    }
}

struct MapDe<'de> {
    iter: indexmap::map::Iter<'de, String, IoValue>,
    value: Option<&'de IoValue>,
}

impl<'de> de::MapAccess<'de> for MapDe<'de> {
    type Error = IoValueError;
    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, IoValueError> {
        match self.iter.next() {
            Some((k, v)) => {
                self.value = Some(v);
                seed.deserialize(de::value::BorrowedStrDeserializer::new(k.as_str())).map(Some)
            }
            None => Ok(None),
        }
    }
    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, IoValueError> {
        match self.value.take() {
            Some(v) => seed.deserialize(v),
            None => Err(de::Error::custom("a value without a key")),
        }
    }
    fn size_hint(&self) -> Option<usize> {
        Some(self.iter.len())
    }
}

struct EnumDe<'de> {
    variant: &'de str,
    value: Option<&'de IoValue>,
}

impl<'de> de::EnumAccess<'de> for EnumDe<'de> {
    type Error = IoValueError;
    type Variant = VariantDe<'de>;
    fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, VariantDe<'de>), IoValueError> {
        let name = seed.deserialize(de::value::BorrowedStrDeserializer::<IoValueError>::new(self.variant))?;
        Ok((name, VariantDe { value: self.value }))
    }
}

struct VariantDe<'de> {
    value: Option<&'de IoValue>,
}

impl<'de> de::VariantAccess<'de> for VariantDe<'de> {
    type Error = IoValueError;
    fn unit_variant(self) -> Result<(), IoValueError> {
        match self.value {
            None | Some(IoValue::Null) => Ok(()),
            Some(_) => Err(de::Error::custom("expected a unit variant")),
        }
    }
    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, IoValueError> {
        match self.value {
            Some(v) => seed.deserialize(v),
            None => Err(de::Error::custom("expected a newtype variant")),
        }
    }
    fn tuple_variant<V: Visitor<'de>>(self, _len: usize, visitor: V) -> Result<V::Value, IoValueError> {
        match self.value {
            Some(IoValue::Array(a)) => visitor.visit_seq(SeqDe { iter: a.iter() }),
            _ => Err(de::Error::custom("expected a tuple variant")),
        }
    }
    fn struct_variant<V: Visitor<'de>>(self, _fields: &'static [&'static str], visitor: V) -> Result<V::Value, IoValueError> {
        match self.value {
            Some(IoValue::Object(o)) => visitor.visit_map(MapDe { iter: o.iter(), value: None }),
            _ => Err(de::Error::custom("expected a struct variant")),
        }
    }
}

impl<'de> de::Deserializer<'de> for &'de IoValue {
    type Error = IoValueError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, IoValueError> {
        match self {
            IoValue::Null => visitor.visit_unit(),
            IoValue::Bool(b) => visitor.visit_bool(*b),
            IoValue::Number(n) => {
                if let Some(u) = n.as_u64() {
                    visitor.visit_u64(u)
                } else if let Some(i) = n.as_i64() {
                    visitor.visit_i64(i)
                } else {
                    visitor.visit_f64(n.as_f64().unwrap_or(0.0))
                }
            }
            IoValue::String(s) => visitor.visit_borrowed_str(s),
            IoValue::Array(a) => visitor.visit_seq(SeqDe { iter: a.iter() }),
            IoValue::Object(o) => visitor.visit_map(MapDe { iter: o.iter(), value: None }),
            IoValue::Binary(b) => visitor.visit_borrowed_bytes(b.as_bytes()),
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, IoValueError> {
        match self {
            IoValue::Null => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(self, name: &'static str, visitor: V) -> Result<V::Value, IoValueError> {
        if name == BINARY_MARKER {
            return match self {
                IoValue::Binary(b) => visitor.visit_newtype_struct(de::value::BorrowedBytesDeserializer::<IoValueError>::new(b.as_bytes())),
                _ => Err(de::Error::custom("expected binary data")),
            };
        }
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, IoValueError> {
        match self {
            IoValue::Binary(b) => visitor.visit_seq(BytesSeqDe { iter: b.as_bytes().iter() }),
            _ => self.deserialize_any(visitor),
        }
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, IoValueError> {
        match self {
            IoValue::String(s) => visitor.visit_enum(EnumDe { variant: s, value: None }),
            IoValue::Object(o) if o.len() == 1 => {
                let (k, v) = o.iter().next().expect("one entry");
                visitor.visit_enum(EnumDe { variant: k, value: Some(v) })
            }
            _ => Err(de::Error::custom("expected a string or an object with one key for an enum")),
        }
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf unit unit_struct
        tuple tuple_struct map struct identifier ignored_any
    }
}

// ------------------------------------------------------------------------------------------------------ args

/// The arguments of one event or acknowledgement received from a client (the values after the event name).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Args(Vec<IoValue>);

impl Args {
    pub fn new(values: Vec<IoValue>) -> Self {
        Args(values)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The argument at `index`.
    pub fn value(&self, index: usize) -> Option<&IoValue> {
        self.0.get(index)
    }

    pub fn as_slice(&self) -> &[IoValue] {
        &self.0
    }

    pub fn iter(&self) -> std::slice::Iter<'_, IoValue> {
        self.0.iter()
    }

    pub fn into_vec(self) -> Vec<IoValue> {
        self.0
    }

    /// Deserializes the argument at `index` into `T`. A missing argument fails with an error naming the index.
    pub fn get<'de, T: Deserialize<'de>>(&'de self, index: usize) -> Result<T, IoValueError> {
        match self.0.get(index) {
            Some(v) => v.deserialize_into(),
            None => Err(IoValueError::Custom(format!("argument {index} is missing ({} received)", self.0.len()))),
        }
    }

    /// Deserializes all the arguments as one sequence: a tuple `(A, B)` or a `Vec<T>`.
    pub fn deserialize_into<'de, T: Deserialize<'de>>(&'de self) -> Result<T, IoValueError> {
        T::deserialize(de::value::SeqDeserializer::<_, IoValueError>::new(self.0.iter()))
    }

    /// The arguments as plain JSON values; fails when one holds binary data.
    pub fn to_json(&self) -> Result<Vec<Value>, IoValueError> {
        self.0.iter().map(IoValue::to_json).collect()
    }
}

impl From<Vec<IoValue>> for Args {
    fn from(v: Vec<IoValue>) -> Self {
        Args(v)
    }
}

impl<'a> IntoIterator for &'a Args {
    type Item = &'a IoValue;
    type IntoIter = std::slice::Iter<'a, IoValue>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl IntoIterator for Args {
    type Item = IoValue;
    type IntoIter = std::vec::IntoIter<IoValue>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'de> IntoDeserializer<'de, IoValueError> for &'de IoValue {
    type Deserializer = &'de IoValue;
    fn into_deserializer(self) -> &'de IoValue {
        self
    }
}

/// What can be given as the arguments of an emit: `()`, a tuple of up to eight `Serialize` values (each becomes one
/// argument), a `Vec<IoValue>`, [`Args`], or a single [`IoValue`] or `serde_json::Value` (one argument).
///
/// ```ignore
/// socket.emit("update", (1, "two", serde_json::json!({ "three": 3 })))?;
/// socket.emit("ready", ())?;
/// ```
pub trait IntoArgs: Send {
    fn into_args(self) -> Result<Vec<IoValue>, IoValueError>;
}

impl IntoArgs for () {
    fn into_args(self) -> Result<Vec<IoValue>, IoValueError> {
        Ok(Vec::new())
    }
}

impl IntoArgs for Vec<IoValue> {
    fn into_args(self) -> Result<Vec<IoValue>, IoValueError> {
        Ok(self)
    }
}

impl IntoArgs for Args {
    fn into_args(self) -> Result<Vec<IoValue>, IoValueError> {
        Ok(self.0)
    }
}

impl IntoArgs for IoValue {
    fn into_args(self) -> Result<Vec<IoValue>, IoValueError> {
        Ok(vec![self])
    }
}

impl IntoArgs for Value {
    fn into_args(self) -> Result<Vec<IoValue>, IoValueError> {
        Ok(vec![IoValue::try_from(self)?])
    }
}

macro_rules! tuple_args {
    ($(($($name:ident),+))+) => {$(
        impl<$($name: Serialize + Send),+> IntoArgs for ($($name,)+) {
            #[allow(non_snake_case)]
            fn into_args(self) -> Result<Vec<IoValue>, IoValueError> {
                let ($($name,)+) = self;
                Ok(vec![$(IoValue::from_serialize(&$name)?),+])
            }
        }
    )+};
}

tuple_args! {
    (A)
    (A, B)
    (A, B, C)
    (A, B, C, D)
    (A, B, C, D, E)
    (A, B, C, D, E, F)
    (A, B, C, D, E, F, G)
    (A, B, C, D, E, F, G, H)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct Upload {
        name: String,
        data: Binary,
        parts: Vec<Binary>,
        note: Option<String>,
    }

    #[test]
    fn a_node_is_never_costlier_than_the_model_says() {
        assert!(std::mem::size_of::<IoValue>() <= NODE_COST, "IoValue is {} bytes", std::mem::size_of::<IoValue>());
    }

    #[test]
    fn a_struct_with_binary_fields_goes_both_ways() {
        let upload = Upload {
            name: "a.bin".into(),
            data: Binary::from(&b"\x00\x01\x02"[..]),
            parts: vec![Binary::from(vec![9u8; 4]), Binary::from(Vec::new())],
            note: None,
        };
        let value = IoValue::from_serialize(&upload).unwrap();
        assert_eq!(value.get("name").and_then(IoValue::as_str), Some("a.bin"));
        assert_eq!(value.get("data").and_then(IoValue::as_binary).map(Binary::len), Some(3));
        assert!(value.contains_binary());
        assert!(value.get("note").unwrap().is_null());
        let back: Upload = value.deserialize_into().unwrap();
        assert_eq!(back, upload);
        // With any other serializer a Binary is a byte sequence.
        assert_eq!(serde_json::to_value(&upload.data).unwrap(), json!([0, 1, 2]));
        let from_json: Binary = serde_json::from_value(json!([7, 8])).unwrap();
        assert_eq!(from_json.as_bytes(), &[7, 8]);
    }

    #[test]
    fn tuples_become_arguments_and_arguments_deserialize_back() {
        let args = (1, "two", json!({ "three": 3 })).into_args().unwrap();
        assert_eq!(args.len(), 3);
        let args = Args::new(args);
        assert_eq!(args.get::<i64>(0).unwrap(), 1);
        assert_eq!(args.get::<String>(1).unwrap(), "two");
        assert_eq!(args.get::<Value>(2).unwrap(), json!({ "three": 3 }));
        let (a, b, c): (u8, String, serde_json::Map<String, Value>) = args.deserialize_into().unwrap();
        assert_eq!((a, b.as_str()), (1, "two"));
        assert_eq!(c["three"], 3);
        let err = args.get::<i64>(5).unwrap_err();
        assert!(err.to_string().contains("argument 5"), "{err}");
        assert_eq!(().into_args().unwrap(), Vec::<IoValue>::new());
    }

    #[test]
    fn objects_keep_their_insertion_order() {
        let value = IoValue::try_from(json!({ "z": 1, "a": 2, "m": 3 })).unwrap();
        let keys: Vec<&str> = value.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, ["z", "a", "m"]);
        assert_eq!(value.to_json().unwrap().to_string(), r#"{"z":1,"a":2,"m":3}"#);
    }

    #[test]
    fn json_conversions_refuse_binary_and_depth() {
        let bin = IoValue::from(Bytes::from_static(b"x"));
        assert_eq!(bin.to_json(), Err(IoValueError::ContainsBinary));
        assert_eq!(Value::try_from(IoValue::Array(vec![bin])), Err(IoValueError::ContainsBinary));
        let mut deep = json!(1);
        for _ in 0..127 {
            deep = json!([deep]);
        }
        assert!(IoValue::try_from(deep.clone()).is_ok(), "127 levels are allowed");
        let too_deep = json!([deep]);
        assert_eq!(IoValue::try_from(too_deep.clone()), Err(IoValueError::TooDeep));
        // The same bound on the way out and on serialization.
        let mut nested = IoValue::Null;
        for _ in 0..129 {
            nested = IoValue::Array(vec![nested]);
        }
        assert_eq!(nested.to_json(), Err(IoValueError::TooDeep));
        assert_eq!(IoValue::from_serialize(&too_deep), Err(IoValueError::TooDeep));
    }

    #[test]
    fn serialization_of_a_deeply_recursive_type_stops_at_the_bound() {
        #[derive(Serialize)]
        struct Chain(Vec<Chain>);
        let mut chain = Chain(Vec::new());
        for _ in 0..200 {
            chain = Chain(vec![chain]);
        }
        assert_eq!(IoValue::from_serialize(&chain), Err(IoValueError::TooDeep));
    }

    #[test]
    fn enums_and_maps_round_trip() {
        #[derive(Debug, Serialize, Deserialize, PartialEq)]
        enum Shape {
            Dot,
            Circle(f64),
            Rect { w: u32, h: u32 },
            Pair(i8, i8),
        }
        for shape in [Shape::Dot, Shape::Circle(1.5), Shape::Rect { w: 2, h: 3 }, Shape::Pair(1, -1)] {
            let value = IoValue::from_serialize(&shape).unwrap();
            assert_eq!(value.deserialize_into::<Shape>().unwrap(), shape);
        }
        let mut map = std::collections::BTreeMap::new();
        map.insert(1u8, "a".to_string());
        let value = IoValue::from_serialize(&map).unwrap();
        assert_eq!(value.get("1").and_then(IoValue::as_str), Some("a"));
        let bad: std::collections::BTreeMap<Vec<u8>, u8> = std::collections::BTreeMap::from([(vec![1], 1)]);
        assert_eq!(IoValue::from_serialize(&bad), Err(IoValueError::KeyNotString));
    }

    #[test]
    fn a_binary_leaf_deserializes_into_bytes_and_seqs() {
        let value = IoValue::from(Bytes::from_static(&[1, 2, 3]));
        assert_eq!(value.deserialize_into::<Vec<u8>>().unwrap(), vec![1, 2, 3]);
        assert_eq!(value.deserialize_into::<Binary>().unwrap().as_bytes(), &[1, 2, 3]);
        assert!(IoValue::from("text").deserialize_into::<Binary>().is_err());
        assert_eq!(IoValue::from(f64::NAN), IoValue::Null);
    }

    #[test]
    fn the_cost_counts_empty_values_and_keys() {
        assert_eq!(IoValue::Null.cost(), NODE_COST);
        assert_eq!(IoValue::Array(Vec::new()).cost(), NODE_COST);
        let many = IoValue::Array(vec![IoValue::Array(Vec::new()); 1000]);
        assert_eq!(many.cost(), 1001 * NODE_COST);
        let obj = IoValue::object([("key", IoValue::from("value"))]);
        assert_eq!(obj.cost(), NODE_COST + ENTRY_COST + 3 + NODE_COST + 5);
    }
}
