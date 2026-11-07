//! Validation: the `Validate` trait (derived with `#[derive(Validate)]` and
//! class-validator-like attributes) and the `ValidationPipe`.
//!
//! ```ignore
//! #[derive(Deserialize, Validate)]
//! pub struct CreateCatDto {
//!     #[is_string]
//!     #[is_not_empty]
//!     pub name: String,
//!     #[is_int]
//!     #[min(0)]
//!     pub age: i32,
//!     #[is_optional]
//!     #[is_email]
//!     pub owner_email: Option<String>,
//! }
//! ```

pub mod rules;

use async_trait::async_trait;
use http::StatusCode;
use serde::Serialize;
use serde_json::Value;

use crate::common::pipe::{ArgumentMetadata, ParamKind, PipeTransform};
use crate::http::HttpException;

/// Options passed to the generated validators.
#[derive(Debug, Clone, Default)]
pub struct ValidatorOptions {
    /// Strip properties that do not have any decorators.
    pub whitelist: bool,
    /// Instead of stripping non-whitelisted properties, throw an error.
    pub forbid_non_whitelisted: bool,
    /// Skip validation of properties that are `null` or missing.
    pub skip_missing_properties: bool,
    /// Convert numeric / boolean strings (query and route params) before validating.
    pub coerce_primitives: bool,
    /// Stop validating a property after its first failing constraint.
    pub stop_at_first_error: bool,
}

/// A validation error for one property (class-validator `ValidationError`).
#[derive(Debug, Clone, Serialize)]
pub struct ValidationError {
    pub property: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    /// `(constraint name, message)` pairs, e.g. `("isEmail", "email must be an email")`.
    pub constraints: Vec<(String, String)>,
    pub children: Vec<ValidationError>,
}

impl ValidationError {
    pub fn new(property: impl Into<String>) -> Self {
        ValidationError { property: property.into(), value: None, constraints: Vec::new(), children: Vec::new() }
    }

    /// The flattened, human readable messages, as `ValidationPipe` reports them.
    pub fn messages(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.collect("", &mut out);
        out
    }

    fn collect(&self, parent: &str, out: &mut Vec<String>) {
        for (_, msg) in &self.constraints {
            if parent.is_empty() {
                out.push(msg.clone());
            } else {
                out.push(format!("{parent}.{msg}"));
            }
        }
        let path = if parent.is_empty() { self.property.clone() } else { format!("{parent}.{}", self.property) };
        for child in &self.children {
            child.collect(&path, out);
        }
    }
}

/// Flattens a list of validation errors into messages.
pub fn flatten_errors(errors: &[ValidationError]) -> Vec<String> {
    errors.iter().flat_map(|e| e.messages()).collect()
}

/// Implemented by `#[derive(Validate)]`.
pub trait Validate {
    /// Validates (and possibly normalizes: whitelist, coercion) a raw JSON value.
    fn validate_value(value: &mut Value, options: &ValidatorOptions) -> Vec<ValidationError>;

    /// Property names known to the DTO (used by `whitelist`).
    fn properties() -> &'static [&'static str];
}

/// Validates an already constructed DTO (`validate(object)` in class-validator).
pub fn validate<T: Validate + Serialize>(value: &T) -> Vec<ValidationError> {
    let mut v = serde_json::to_value(value).unwrap_or(Value::Null);
    T::validate_value(&mut v, &ValidatorOptions::default())
}

/// Options for [`ValidationPipe`] (`new ValidationPipe({ whitelist: true, ... })`).
#[derive(Clone)]
pub struct ValidationPipeOptions {
    pub whitelist: bool,
    pub forbid_non_whitelisted: bool,
    pub skip_missing_properties: bool,
    pub disable_error_messages: bool,
    pub stop_at_first_error: bool,
    pub error_http_status_code: u16,
    /// Always on in corvis (values are deserialized into the declared Rust types);
    /// kept for API familiarity.
    pub transform: bool,
    pub exception_factory: Option<fn(Vec<ValidationError>) -> HttpException>,
}

impl Default for ValidationPipeOptions {
    fn default() -> Self {
        ValidationPipeOptions {
            whitelist: false,
            forbid_non_whitelisted: false,
            skip_missing_properties: false,
            disable_error_messages: false,
            stop_at_first_error: false,
            error_http_status_code: 400,
            transform: true,
            exception_factory: None,
        }
    }
}

/// The built-in `ValidationPipe`: validates every argument whose type derives `Validate`.
///
/// ```ignore
/// app.use_global_pipes(ValidationPipe::new().whitelist(true));
/// ```
#[derive(Clone, Default)]
pub struct ValidationPipe {
    options: ValidationPipeOptions,
}

impl ValidationPipe {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_options(options: ValidationPipeOptions) -> Self {
        ValidationPipe { options }
    }
    pub fn whitelist(mut self, v: bool) -> Self {
        self.options.whitelist = v;
        self
    }
    pub fn forbid_non_whitelisted(mut self, v: bool) -> Self {
        self.options.forbid_non_whitelisted = v;
        self
    }
    pub fn transform(mut self, v: bool) -> Self {
        self.options.transform = v;
        self
    }
    pub fn skip_missing_properties(mut self, v: bool) -> Self {
        self.options.skip_missing_properties = v;
        self
    }
    pub fn disable_error_messages(mut self, v: bool) -> Self {
        self.options.disable_error_messages = v;
        self
    }
    pub fn stop_at_first_error(mut self, v: bool) -> Self {
        self.options.stop_at_first_error = v;
        self
    }
    pub fn error_http_status_code(mut self, status: u16) -> Self {
        self.options.error_http_status_code = status;
        self
    }
    pub fn exception_factory(mut self, f: fn(Vec<ValidationError>) -> HttpException) -> Self {
        self.options.exception_factory = Some(f);
        self
    }

    fn create_exception(&self, errors: Vec<ValidationError>) -> HttpException {
        if let Some(f) = self.options.exception_factory {
            return f(errors);
        }
        let status = StatusCode::from_u16(self.options.error_http_status_code).unwrap_or(StatusCode::BAD_REQUEST);
        let error = status.canonical_reason().unwrap_or("Bad Request");
        if self.options.disable_error_messages {
            return HttpException::with_error((), error, status);
        }
        HttpException::with_error(flatten_errors(&errors), error, status)
    }
}

#[async_trait]
impl PipeTransform for ValidationPipe {
    async fn transform(&self, mut value: Value, metadata: &ArgumentMetadata) -> Result<Value, HttpException> {
        if metadata.kind == ParamKind::Custom {
            return Ok(value);
        }
        let Some(validator) = metadata.metatype.validator else {
            return Ok(value);
        };
        let opts = ValidatorOptions {
            whitelist: self.options.whitelist,
            forbid_non_whitelisted: self.options.forbid_non_whitelisted,
            skip_missing_properties: self.options.skip_missing_properties,
            coerce_primitives: metadata.kind != ParamKind::Body,
            stop_at_first_error: self.options.stop_at_first_error,
        };
        let errors = validator(&mut value, &opts);
        if errors.is_empty() { Ok(value) } else { Err(self.create_exception(errors)) }
    }
}

crate::default_injectable!(ValidationPipe);
