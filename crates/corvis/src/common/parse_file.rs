//! `ParseFilePipe`, `ParseFilePipeBuilder`, `FileValidator`, `MaxFileSizeValidator` and
//! `FileTypeValidator`.
//!
//! The pipes see files as the JSON multer builds (`{ fieldname, originalname, mimetype, size, buffer, ... }`);
//! `buffer` is resolved with [`crate::multer::buffer_from_value`].

use std::sync::Arc;

use async_trait::async_trait;
use http::StatusCode;
use regex::Regex;
use serde_json::{Value, json};

use super::pipe::{ArgumentMetadata, PipeTransform};
use crate::http::HttpException;

/// `FileValidator`: `isValid(file)` + `buildErrorMessage(file)`. `file` is the file JSON
/// (or, for `FileFieldsInterceptor`, the whole `{ field: [...] }` object).
#[async_trait]
pub trait FileValidator: Send + Sync + 'static {
    async fn is_valid(&self, file: &Value) -> bool;
    fn build_error_message(&self, file: &Value) -> String;
}

type ErrorMessageFn = Arc<dyn Fn(&Value, &Value) -> String + Send + Sync>;

#[derive(Clone)]
enum ErrorMessage {
    Text(String),
    /// `errorMessage: ({ file, config }) => string`.
    Fn(ErrorMessageFn),
}

type SizeMessage = Result<String, Arc<dyn Fn(u64) -> String + Send + Sync>>;

/// `new MaxFileSizeValidator({ maxSize, message?, errorMessage? })`. Valid when `file.size < maxSize`.
#[derive(Clone)]
pub struct MaxFileSizeValidator {
    max_size: u64,
    message: Option<SizeMessage>,
    error_message: Option<ErrorMessage>,
}

impl MaxFileSizeValidator {
    pub fn new(max_size: u64) -> Self {
        MaxFileSizeValidator { max_size, message: None, error_message: None }
    }
    /// `message: 'text'` (legacy option).
    pub fn message(mut self, m: impl Into<String>) -> Self {
        self.message = Some(Ok(m.into()));
        self
    }
    /// `message: (maxSize) => 'text'`.
    pub fn message_fn(mut self, f: impl Fn(u64) -> String + Send + Sync + 'static) -> Self {
        self.message = Some(Err(Arc::new(f)));
        self
    }
    /// `errorMessage: 'text'` (takes precedence over `message`).
    pub fn error_message(mut self, m: impl Into<String>) -> Self {
        self.error_message = Some(ErrorMessage::Text(m.into()));
        self
    }
    /// `errorMessage: ({ file, config }) => 'text'`; `config` is `{ maxSize }`.
    pub fn error_message_fn(mut self, f: impl Fn(&Value, &Value) -> String + Send + Sync + 'static) -> Self {
        self.error_message = Some(ErrorMessage::Fn(Arc::new(f)));
        self
    }
    pub fn max_size(&self) -> u64 {
        self.max_size
    }
}

#[async_trait]
impl FileValidator for MaxFileSizeValidator {
    async fn is_valid(&self, file: &Value) -> bool {
        if file.is_null() {
            return true;
        }
        match file.get("size") {
            Some(s) => s.as_f64().is_some_and(|s| s < self.max_size as f64),
            None => false,
        }
    }

    fn build_error_message(&self, file: &Value) -> String {
        if let Some(e) = &self.error_message {
            return match e {
                ErrorMessage::Text(t) => t.clone(),
                ErrorMessage::Fn(f) => f(file, &json!({ "maxSize": self.max_size })),
            };
        }
        if let Some(m) = &self.message {
            return match m {
                Ok(t) => t.clone(),
                Err(f) => f(self.max_size),
            };
        }
        match file.get("size").and_then(|s| s.as_f64()).filter(|s| *s != 0.0) {
            Some(size) => format!(
                "Validation failed (current file size is {}, expected size is less than {})",
                crate::z::js::num_to_string(size),
                self.max_size
            ),
            None => format!("Validation failed (expected size is less than {})", self.max_size),
        }
    }
}

/// The `fileType` option: a string (used as a regular expression, like `String#match`) or a regex.
#[derive(Clone, Debug)]
pub struct FileTypePattern {
    source: String,
    regex: Option<Regex>,
}

impl FileTypePattern {
    fn matches(&self, mime: &str) -> bool {
        self.regex.as_ref().is_some_and(|r| r.is_match(mime))
    }
}

impl From<&str> for FileTypePattern {
    fn from(s: &str) -> Self {
        FileTypePattern { source: s.to_string(), regex: Regex::new(s).ok() }
    }
}

impl From<String> for FileTypePattern {
    fn from(s: String) -> Self {
        s.as_str().into()
    }
}

impl From<Regex> for FileTypePattern {
    /// A regex prints as `/source/` in messages, like a JS `RegExp`.
    fn from(r: Regex) -> Self {
        FileTypePattern { source: format!("/{}/", r.as_str()), regex: Some(r) }
    }
}

/// `new FileTypeValidator({ fileType, skipMagicNumbersValidation?, fallbackToMimetype?, errorMessage? })`.
///
/// Validates the real type of the content by its magic number ([`detect_mime`](super::file_type::detect_mime));
/// with `skip_magic_numbers_validation` it only checks `file.mimetype`.
#[derive(Clone)]
pub struct FileTypeValidator {
    file_type: FileTypePattern,
    skip_magic_numbers_validation: bool,
    fallback_to_mimetype: bool,
    error_message: Option<ErrorMessage>,
}

impl FileTypeValidator {
    pub fn new(file_type: impl Into<FileTypePattern>) -> Self {
        FileTypeValidator {
            file_type: file_type.into(),
            skip_magic_numbers_validation: false,
            fallback_to_mimetype: false,
            error_message: None,
        }
    }
    pub fn skip_magic_numbers_validation(mut self, v: bool) -> Self {
        self.skip_magic_numbers_validation = v;
        self
    }
    pub fn fallback_to_mimetype(mut self, v: bool) -> Self {
        self.fallback_to_mimetype = v;
        self
    }
    pub fn error_message(mut self, m: impl Into<String>) -> Self {
        self.error_message = Some(ErrorMessage::Text(m.into()));
        self
    }
    /// `errorMessage: ({ file, config }) => 'text'`; `config` is `{ fileType, skipMagicNumbersValidation, fallbackToMimetype }`.
    pub fn error_message_fn(mut self, f: impl Fn(&Value, &Value) -> String + Send + Sync + 'static) -> Self {
        self.error_message = Some(ErrorMessage::Fn(Arc::new(f)));
        self
    }
}

#[async_trait]
impl FileValidator for FileTypeValidator {
    async fn is_valid(&self, file: &Value) -> bool {
        let mimetype = file.get("mimetype").map(|m| m.as_str().unwrap_or_default().to_string());
        let mime_ok = |m: &Option<String>| m.as_deref().is_some_and(|m| self.file_type.matches(m));
        if self.skip_magic_numbers_validation {
            return mime_ok(&mimetype);
        }
        if mimetype.is_none() {
            return false;
        }
        let buffer = file.get("buffer").and_then(crate::multer::buffer_from_value);
        let Some(buffer) = buffer else {
            return self.fallback_to_mimetype && mime_ok(&mimetype);
        };
        match super::file_type::detect_mime(&buffer) {
            Some(detected) => self.file_type.matches(detected),
            None => self.fallback_to_mimetype && mime_ok(&mimetype),
        }
    }

    fn build_error_message(&self, file: &Value) -> String {
        if let Some(e) = &self.error_message {
            return match e {
                ErrorMessage::Text(t) => t.clone(),
                ErrorMessage::Fn(f) => f(
                    file,
                    &json!({
                        "fileType": self.file_type.source,
                        "skipMagicNumbersValidation": self.skip_magic_numbers_validation,
                        "fallbackToMimetype": self.fallback_to_mimetype,
                    }),
                ),
            };
        }
        let mimetype = file.get("mimetype").and_then(|m| m.as_str()).filter(|m| !m.is_empty());
        let has_buffer = file.get("buffer").is_some_and(|b| !b.is_null());
        let expected = &self.file_type.source;
        if let Some(m) = mimetype {
            if !has_buffer && !self.fallback_to_mimetype && !self.skip_magic_numbers_validation {
                return format!(
                    "Validation failed (file buffer is not available; file type validation could not be performed; expected type is {expected})"
                );
            }
            let base = format!("Validation failed (current file type is {m}, expected type is {expected})");
            if self.fallback_to_mimetype {
                return format!("{base} - magic number detection failed, used mimetype fallback");
            }
            return base;
        }
        format!("Validation failed (expected type is {expected})")
    }
}

type ExceptionFactory = Arc<dyn Fn(String) -> HttpException + Send + Sync>;

/// `new ParseFilePipe({ validators, errorHttpStatusCode, exceptionFactory, fileIsRequired })`.
#[derive(Clone)]
pub struct ParseFilePipe {
    validators: Vec<Arc<dyn FileValidator>>,
    exception_factory: ExceptionFactory,
    file_is_required: bool,
}

impl Default for ParseFilePipe {
    fn default() -> Self {
        ParseFilePipe {
            validators: Vec::new(),
            exception_factory: Arc::new(|m| http_error_by_code(StatusCode::BAD_REQUEST, m)),
            file_is_required: true,
        }
    }
}

impl ParseFilePipe {
    pub fn new() -> Self {
        Self::default()
    }
    /// Adds a validator (`validators: [...]`).
    pub fn validator(mut self, v: impl FileValidator) -> Self {
        self.validators.push(Arc::new(v));
        self
    }
    pub fn validator_arc(mut self, v: Arc<dyn FileValidator>) -> Self {
        self.validators.push(v);
        self
    }
    /// `errorHttpStatusCode` (default 400): the error is the built-in exception of that status.
    pub fn error_http_status_code(mut self, status: u16) -> Self {
        let status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST);
        self.exception_factory = Arc::new(move |m| http_error_by_code(status, m));
        self
    }
    /// `exceptionFactory: (error) => new MyException(error)`.
    pub fn exception_factory(mut self, f: impl Fn(String) -> HttpException + Send + Sync + 'static) -> Self {
        self.exception_factory = Arc::new(f);
        self
    }
    /// `fileIsRequired` (default `true`): `File is required` when nothing was uploaded.
    pub fn file_is_required(mut self, v: bool) -> Self {
        self.file_is_required = v;
        self
    }
    /// `getValidators()`.
    pub fn get_validators(&self) -> &[Arc<dyn FileValidator>] {
        &self.validators
    }

    fn there_are_no_files_in(value: &Value) -> bool {
        match value {
            Value::Null => true,
            Value::Array(a) => a.is_empty(),
            Value::Object(o) => o.is_empty(),
            _ => false,
        }
    }

    async fn validate_or_throw(&self, file: &Value, validator: &Arc<dyn FileValidator>) -> Result<(), HttpException> {
        if !validator.is_valid(file).await {
            return Err((self.exception_factory)(validator.build_error_message(file)));
        }
        Ok(())
    }
}

#[async_trait]
impl PipeTransform for ParseFilePipe {
    async fn transform(&self, value: Value, _metadata: &ArgumentMetadata) -> Result<Value, HttpException> {
        let no_files = Self::there_are_no_files_in(&value);
        if no_files && self.file_is_required {
            return Err((self.exception_factory)("File is required".into()));
        }
        if !no_files && !self.validators.is_empty() {
            match &value {
                // `Promise.all(files.map(validate))`: validators advance in lockstep, so the first
                // failure is the earliest validator, then the earliest file.
                Value::Array(files) => {
                    for v in &self.validators {
                        for f in files {
                            self.validate_or_throw(f, v).await?;
                        }
                    }
                }
                other => {
                    for v in &self.validators {
                        self.validate_or_throw(other, v).await?;
                    }
                }
            }
        }
        Ok(value)
    }
}

crate::default_injectable!(ParseFilePipe);

/// `new ParseFilePipeBuilder().addMaxSizeValidator(...).addFileTypeValidator(...).build(options)`.
#[derive(Default)]
pub struct ParseFilePipeBuilder {
    validators: Vec<Arc<dyn FileValidator>>,
}

impl ParseFilePipeBuilder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add_max_size_validator(self, v: MaxFileSizeValidator) -> Self {
        self.add_validator(v)
    }
    pub fn add_file_type_validator(self, v: FileTypeValidator) -> Self {
        self.add_validator(v)
    }
    pub fn add_validator(mut self, v: impl FileValidator) -> Self {
        self.validators.push(Arc::new(v));
        self
    }
    /// `build({ errorHttpStatusCode, exceptionFactory, fileIsRequired })`: pass a configured
    /// `ParseFilePipe` (its validators are replaced by the builder's).
    pub fn build(self, options: ParseFilePipe) -> ParseFilePipe {
        ParseFilePipe { validators: self.validators, ..options }
    }
}

/// `HttpErrorByCode[status](message)`: the built-in exception of that status.
pub fn http_error_by_code(status: StatusCode, message: String) -> HttpException {
    use crate::http::*;
    match status.as_u16() {
        400 => BadRequestException::new(message),
        401 => UnauthorizedException::new(message),
        403 => ForbiddenException::new(message),
        404 => NotFoundException::new(message),
        405 => MethodNotAllowedException::new(message),
        406 => NotAcceptableException::new(message),
        408 => RequestTimeoutException::new(message),
        409 => ConflictException::new(message),
        410 => GoneException::new(message),
        412 => PreconditionFailedException::new(message),
        413 => PayloadTooLargeException::new(message),
        415 => UnsupportedMediaTypeException::new(message),
        418 => ImATeapotException::new(message),
        421 => MisdirectedException::new(message),
        422 => UnprocessableEntityException::new(message),
        500 => InternalServerErrorException::new(message),
        501 => NotImplementedException::new(message),
        502 => BadGatewayException::new(message),
        503 => ServiceUnavailableException::new(message),
        504 => GatewayTimeoutException::new(message),
        505 => HttpVersionNotSupportedException::new(message),
        _ => HttpException::new(message, status),
    }
}
