//! Pipes (`PipeTransform`) and the built-in pipes: `ParseIntPipe`, `ParseFloatPipe`,
//! `ParseBoolPipe`, `ParseUUIDPipe`, `ParseArrayPipe`, `ParseEnumPipe`, `DefaultValuePipe`.
//! `ValidationPipe` lives in [`crate::validation`].

use async_trait::async_trait;
use http::StatusCode;
use serde::Serialize;
use serde_json::Value;

use crate::http::HttpException;
use crate::validation::{ValidationError, ValidatorOptions};

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

/// Parses a numeric string into an integer.
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
        match &value {
            Value::Number(n) if n.is_i64() || n.is_u64() => Ok(value),
            Value::String(s) => {
                let t = s.trim();
                let valid = !t.is_empty() && t.trim_start_matches(['-', '+']).chars().all(|c| c.is_ascii_digit());
                match (valid, t.parse::<i64>()) {
                    (true, Ok(n)) => Ok(Value::from(n)),
                    _ => Err(fail(self.status, "Validation failed (numeric string is expected)")),
                }
            }
            _ => Err(fail(self.status, "Validation failed (numeric string is expected)")),
        }
    }
}

/// Parses a numeric string into a float.
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
        match &value {
            Value::Number(_) => Ok(value),
            Value::String(s) => match s.trim().parse::<f64>() {
                Ok(n) if n.is_finite() => Ok(serde_json::Number::from_f64(n).map(Value::Number).unwrap_or(Value::Null)),
                _ => Err(fail(self.status, "Validation failed (numeric string is expected)")),
            },
            _ => Err(fail(self.status, "Validation failed (numeric string is expected)")),
        }
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

/// Splits a string (`"1,2,3"`) into an array. Items can optionally be parsed as numbers.
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
        if value.is_null() {
            return if self.optional { Ok(value) } else { Err(fail(self.status, "Validation failed (parsable array expected)")) };
        }
        let items: Vec<Value> = match value {
            Value::Array(a) => a,
            Value::String(s) => {
                if s.is_empty() {
                    vec![]
                } else {
                    s.split(self.separator.as_str()).map(|p| Value::String(p.trim().to_string())).collect()
                }
            }
            _ => return Err(fail(self.status, "Validation failed (parsable array expected)")),
        };
        if !self.numbers {
            return Ok(Value::Array(items));
        }
        let mut out = Vec::with_capacity(items.len());
        for i in items {
            match i {
                Value::Number(_) => out.push(i),
                Value::String(s) => match s.parse::<f64>() {
                    Ok(n) => out.push(
                        if n.fract() == 0.0 { Value::from(n as i64) } else { serde_json::Number::from_f64(n).map(Value::Number).unwrap_or(Value::Null) },
                    ),
                    Err(_) => return Err(fail(self.status, "Validation failed (parsable array expected)")),
                },
                _ => return Err(fail(self.status, "Validation failed (parsable array expected)")),
            }
        }
        Ok(Value::Array(out))
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
    /// Uses the variants of a serde enum.
    pub fn of<T: serde::de::DeserializeOwned>() -> Self {
        Self::new(crate::validation::rules::enum_variants::<T>().iter().copied())
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

/// Supplies a default when the value is missing (`new DefaultValuePipe(1)`).
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
        match &value {
            Value::Null => Ok(self.default.clone()),
            Value::String(s) if s.is_empty() => Ok(self.default.clone()),
            Value::Number(n) if n.as_f64().map(|f| f.is_nan()).unwrap_or(false) => Ok(self.default.clone()),
            _ => Ok(value),
        }
    }
}

crate::default_injectable!(ParseIntPipe, ParseFloatPipe, ParseBoolPipe, ParseUUIDPipe, ParseArrayPipe);
