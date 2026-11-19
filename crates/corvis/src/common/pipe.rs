//! Pipes (`PipeTransform`) and the built-in pipes: `ParseIntPipe`, `ParseFloatPipe`,
//! `ParseBoolPipe`, `ParseUUIDPipe`, `ParseArrayPipe`, `ParseEnumPipe`, `DefaultValuePipe`.
//! `ValidationPipe` lives in [`crate::validation`].

use async_trait::async_trait;
use http::StatusCode;
use serde::Serialize;
use serde_json::Value;

use crate::http::HttpException;
use crate::validation::{ValidationError, ValidatorOptions};
use crate::z::js;

/// Where a handler argument comes from (`'body' | 'query' | 'param' | 'custom'`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    Body,
    Query,
    Param,
    Headers,
    Ip,
    Custom,
    /// `@UploadedFile()`.
    File,
    /// `@UploadedFiles()`.
    Files,
}

impl ParamKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ParamKind::Body => "body",
            ParamKind::Query => "query",
            ParamKind::Param => "param",
            ParamKind::Headers => "headers",
            ParamKind::Ip => "ip",
            ParamKind::Custom => "custom",
            ParamKind::File => "file",
            ParamKind::Files => "files",
        }
    }
}

pub type ValidatorFn = fn(&mut Value, &ValidatorOptions) -> Vec<ValidationError>;

/// The declared type of the argument (`metatype`).
#[derive(Clone, Copy)]
pub struct Metatype {
    pub name: &'static str,
    /// Present when the type derives `Validate`.
    pub validator: Option<ValidatorFn>,
    /// Present when the type has a zod schema (`zod_dto!`).
    pub zod: Option<fn() -> &'static crate::z::Schema>,
}

impl std::fmt::Debug for Metatype {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

/// Metadata about the argument being processed by a pipe.
#[derive(Debug, Clone, Copy)]
pub struct ArgumentMetadata {
    pub kind: ParamKind,
    pub data: Option<&'static str>,
    pub metatype: Metatype,
}

/// A pipe transforms or validates handler arguments. Values travel as JSON values;
/// the final value is deserialized into the declared parameter type.
#[async_trait]
pub trait PipeTransform: Send + Sync + 'static {
    async fn transform(&self, value: Value, metadata: &ArgumentMetadata) -> Result<Value, HttpException>;
}

fn fail(status: StatusCode, msg: &str) -> HttpException {
    if status == StatusCode::BAD_REQUEST {
        crate::http::BadRequestException::new(msg)
    } else {
        HttpException::with_error(msg, status.canonical_reason().unwrap_or("Error"), status)
    }
}

macro_rules! pipe_options {
    ($name:ident) => {
        impl $name {
            pub fn new() -> Self {
                Self::default()
            }
            /// Status code used when validation fails (default 400).
            pub fn error_http_status_code(mut self, status: u16) -> Self {
                self.status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST);
                self
            }
            /// Let `null`/missing values pass through untouched.
            pub fn optional(mut self) -> Self {
                self.optional = true;
                self
            }
        }
        impl Default for $name {
            fn default() -> Self {
                $name { status: StatusCode::BAD_REQUEST, optional: false }
            }
        }
    };
}

const NUMERIC_EXPECTED: &str = "Validation failed (numeric string is expected)";
const ARRAY_EXPECTED: &str = "Validation failed (parsable array expected)";

/// Parses a numeric string into an integer: the text must be `-?\d+` (no sign `+`, no white space, no exponent), as
/// the original requires. The numeral is read exactly, where the original's `parseInt` rounds one beyond 2^53 to the
/// nearest double (`"9007199254740993"` is `9007199254740993` here, `9007199254740992` there). An integer outside
/// `i64` is refused with the same 400 (the original would return it; Corvis cannot represent it).
#[derive(Debug, Clone)]
pub struct ParseIntPipe {
    status: StatusCode,
    optional: bool,
}
pipe_options!(ParseIntPipe);

#[async_trait]
impl PipeTransform for ParseIntPipe {
    async fn transform(&self, value: Value, _m: &ArgumentMetadata) -> Result<Value, HttpException> {
        if self.optional && value.is_null() {
            return Ok(value);
        }
        let n = match &value {
            // A JSON integer as it is; any other number by its JavaScript text (`5.0` is `"5"`, `1.5` is refused).
            Value::Number(n) if n.is_i64() || n.is_u64() => n.as_i64(),
            Value::Number(_) => decimal_integer(&js::to_js_string(&value)),
            Value::String(s) => decimal_integer(s),
            _ => None,
        };
        n.map(Value::from).ok_or_else(|| fail(self.status, NUMERIC_EXPECTED))
    }
}

/// `-?\d+` read exactly; `None` for any other text or an integer outside `i64`.
fn decimal_integer(text: &str) -> Option<i64> {
    let digits = text.strip_prefix('-').unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// Parses a numeric string into a float, as JavaScript's `parseFloat`, when the whole value is a finite number for
/// `Number()` (`" 1.5 "` is `1.5`, `"0x10"` is `0`; `"1.5abc"`, `"Infinity"` and `""` are refused).
#[derive(Debug, Clone)]
pub struct ParseFloatPipe {
    status: StatusCode,
    optional: bool,
}
pipe_options!(ParseFloatPipe);

#[async_trait]
impl PipeTransform for ParseFloatPipe {
    async fn transform(&self, value: Value, _m: &ArgumentMetadata) -> Result<Value, HttpException> {
        if self.optional && value.is_null() {
            return Ok(value);
        }
        let n = js::parse_float(&js::to_js_string(&value));
        if n.is_nan() || !js::to_number(Some(&value)).is_finite() {
            return Err(fail(self.status, NUMERIC_EXPECTED));
        }
        Ok(js::num_json(n))
    }
}

/// Parses `"true"` / `"false"` into a boolean.
#[derive(Debug, Clone)]
pub struct ParseBoolPipe {
    status: StatusCode,
    optional: bool,
}
pipe_options!(ParseBoolPipe);

#[async_trait]
impl PipeTransform for ParseBoolPipe {
    async fn transform(&self, value: Value, _m: &ArgumentMetadata) -> Result<Value, HttpException> {
        if self.optional && value.is_null() {
            return Ok(value);
        }
        match &value {
            Value::Bool(_) => Ok(value),
            Value::String(s) if s == "true" => Ok(Value::Bool(true)),
            Value::String(s) if s == "false" => Ok(Value::Bool(false)),
            _ => Err(fail(self.status, "Validation failed (boolean string is expected)")),
        }
    }
}

/// Validates that the value is a UUID (`ParseUUIDPipe`).
#[derive(Debug, Clone)]
pub struct ParseUUIDPipe {
    status: StatusCode,
    optional: bool,
    version: Option<u8>,
}
pub type ParseUuidPipe = ParseUUIDPipe;

impl ParseUUIDPipe {
    pub fn new() -> Self {
        Self::default()
    }
    /// Status code used when validation fails (default 400).
    pub fn error_http_status_code(mut self, status: u16) -> Self {
        self.status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST);
        self
    }
    /// Let `null`/missing values pass through untouched.
    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }
    /// `new ParseUUIDPipe({ version: '4' })`: only accept UUIDs of that version
    /// (3, 4, 5 or 7). Message: `Validation failed (uuid v 4 is expected)`.
    pub fn version(mut self, version: u8) -> Self {
        self.version = Some(version);
        self
    }

    fn matches(&self, s: &str) -> bool {
        let b = s.as_bytes();
        if !crate::validation::rules::is_uuid(s) {
            return false;
        }
        let variant_ok = matches!(b[19].to_ascii_lowercase(), b'8' | b'9' | b'a' | b'b');
        match self.version {
            None => true,
            Some(3) => b[14] == b'3',
            Some(v @ (4 | 5 | 7)) => b[14] == b'0' + v && variant_ok,
            Some(_) => false,
        }
    }
}

impl Default for ParseUUIDPipe {
    fn default() -> Self {
        ParseUUIDPipe { status: StatusCode::BAD_REQUEST, optional: false, version: None }
    }
}

#[async_trait]
impl PipeTransform for ParseUUIDPipe {
    async fn transform(&self, value: Value, _m: &ArgumentMetadata) -> Result<Value, HttpException> {
        if self.optional && value.is_null() {
            return Ok(value);
        }
        match &value {
            Value::String(s) if self.matches(s) => Ok(value),
            Value::String(_) => {
                let v = self.version.map(|v| format!(" v {v}")).unwrap_or_default();
                Err(fail(self.status, &format!("Validation failed (uuid{v} is expected)")))
            }
            _ => Err(fail(self.status, "The value passed as UUID is not a string")),
        }
    }
}

/// Splits a string (`"1,2,3"`) into an array: the string is trimmed and split, the items are not trimmed. A missing
/// or empty value is refused. With [`numbers`](Self::numbers) every item is read as JavaScript's `Number()` reads it
/// (`" 2"` is `2`, `"0x10"` is `16`); an empty item, a non-number and an infinite number (which Corvis cannot
/// represent) are refused with `[index] item must be a number`.
#[derive(Debug, Clone)]
pub struct ParseArrayPipe {
    separator: String,
    numbers: bool,
    optional: bool,
    status: StatusCode,
}

impl Default for ParseArrayPipe {
    fn default() -> Self {
        ParseArrayPipe { separator: ",".into(), numbers: false, optional: false, status: StatusCode::BAD_REQUEST }
    }
}

impl ParseArrayPipe {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn separator(mut self, sep: impl Into<String>) -> Self {
        self.separator = sep.into();
        self
    }
    /// `items: Number`.
    pub fn numbers(mut self) -> Self {
        self.numbers = true;
        self
    }
    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }
}

#[async_trait]
impl PipeTransform for ParseArrayPipe {
    async fn transform(&self, value: Value, _m: &ArgumentMetadata) -> Result<Value, HttpException> {
        let falsy = match &value {
            Value::Null => true,
            Value::Bool(b) => !b,
            Value::Number(n) => js::json_f64(n) == 0.0,
            Value::String(s) => s.is_empty(),
            Value::Array(_) | Value::Object(_) => false,
        };
        if falsy && !self.optional {
            return Err(fail(self.status, ARRAY_EXPECTED));
        }
        let items: Vec<Value> = match value {
            Value::Null => return Ok(value),
            Value::Array(a) => a,
            Value::String(s) => js::js_trim(&s).split(self.separator.as_str()).map(|p| Value::String(p.to_string())).collect(),
            _ => return Err(fail(self.status, ARRAY_EXPECTED)),
        };
        if !self.numbers {
            return Ok(Value::Array(items));
        }
        items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                // `+item`, with `''` and `null` as NaN.
                let n = match item {
                    Value::Null => f64::NAN,
                    Value::String(s) if s.is_empty() => f64::NAN,
                    other => js::to_number(Some(other)),
                };
                if n.is_finite() { Ok(js::num_json(n)) } else { Err(fail(self.status, &format!("[{i}] item must be a number"))) }
            })
            .collect::<Result<_, _>>()
            .map(Value::Array)
    }
}

/// Validates that the value is one of the given enum values.
#[derive(Debug, Clone)]
pub struct ParseEnumPipe {
    values: Vec<String>,
    optional: bool,
    status: StatusCode,
}

impl ParseEnumPipe {
    pub fn new<I: IntoIterator<Item = S>, S: Into<String>>(values: I) -> Self {
        ParseEnumPipe { values: values.into_iter().map(Into::into).collect(), optional: false, status: StatusCode::BAD_REQUEST }
    }
    /// Uses the variants of a serde enum (unit variants, `rename_all` respected).
    ///
    /// # Panics
    ///
    /// When the variant names of `T` cannot be read: an untagged, internally or adjacently tagged enum, or a type that
    /// is not an enum (the message names `T`), instead of building a pipe that refuses every value.
    pub fn of<T: serde::de::DeserializeOwned>() -> Self {
        Self::new(crate::validation::rules::readable_enum_variants::<T>("ParseEnumPipe::of").iter().copied())
    }
    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }
}

#[async_trait]
impl PipeTransform for ParseEnumPipe {
    async fn transform(&self, value: Value, _m: &ArgumentMetadata) -> Result<Value, HttpException> {
        if self.optional && value.is_null() {
            return Ok(value);
        }
        match &value {
            Value::String(s) if self.values.iter().any(|v| v == s) => Ok(value),
            _ => Err(fail(self.status, "Validation failed (enum string is expected)")),
        }
    }
}

/// Supplies a default when the value is missing or `null` (`new DefaultValuePipe(1)`); an empty string is kept.
#[derive(Debug, Clone)]
pub struct DefaultValuePipe {
    default: Value,
}

impl DefaultValuePipe {
    pub fn new(default: impl Serialize) -> Self {
        DefaultValuePipe { default: serde_json::to_value(default).unwrap_or(Value::Null) }
    }
}

#[async_trait]
impl PipeTransform for DefaultValuePipe {
    async fn transform(&self, value: Value, _m: &ArgumentMetadata) -> Result<Value, HttpException> {
        // JSON has no NaN, so the original's `isNil(value) || isNaN(value)` is `null` here.
        Ok(if value.is_null() { self.default.clone() } else { value })
    }
}

crate::default_injectable!(ParseIntPipe, ParseFloatPipe, ParseBoolPipe, ParseUUIDPipe, ParseArrayPipe);

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(dead_code)]
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Untagged {
        A(String),
        B(i64),
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "lowercase")]
    enum Role {
        Admin,
        Member,
    }

    fn query_meta() -> ArgumentMetadata {
        ArgumentMetadata { kind: ParamKind::Query, data: None, metatype: Metatype { name: "String", validator: None, zod: None } }
    }

    /// An id beyond 2^53 is the id the client sent, not the nearest double (`parseInt` rounds it).
    #[tokio::test]
    async fn parse_int_pipe_reads_integers_beyond_2_pow_53_exactly() {
        let pipe = ParseIntPipe::new();
        for text in ["9007199254740993", "1234567890123456789", "-9007199254740993", "9223372036854775807", "-9223372036854775808"] {
            let want: i64 = text.parse().unwrap();
            assert_eq!(pipe.transform(Value::from(text), &query_meta()).await.unwrap(), Value::from(want), "{text}");
            assert_eq!(pipe.transform(Value::from(want), &query_meta()).await.unwrap(), Value::from(want), "{want} as a JSON number");
        }
        for refused in [Value::from("9223372036854775808"), Value::from("-9223372036854775809"), Value::from(u64::MAX)] {
            let e = pipe.transform(refused.clone(), &query_meta()).await.unwrap_err();
            assert_eq!(e.get_response()["message"], NUMERIC_EXPECTED, "{refused}");
        }
    }

    #[test]
    #[should_panic(expected = "Untagged")]
    fn parse_enum_pipe_of_an_untagged_enum_panics_when_built() {
        let _ = ParseEnumPipe::of::<Untagged>();
    }

    #[tokio::test]
    async fn parse_enum_pipe_of_a_plain_enum_accepts_its_variants() {
        let _ = (Role::Admin, Role::Member);
        let pipe = ParseEnumPipe::of::<Role>();
        let meta = query_meta();
        assert_eq!(pipe.transform(Value::from("admin"), &meta).await.unwrap(), "admin");
        let e = pipe.transform(Value::from("root"), &meta).await.unwrap_err();
        assert_eq!(e.get_status(), 400);
        assert_eq!(e.get_response()["message"], "Validation failed (enum string is expected)");
    }
}
