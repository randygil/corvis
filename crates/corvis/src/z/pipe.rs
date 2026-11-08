//! Zod integration for handlers: `createZodDto` ([`ZodDto`] / [`zod_dto!`](crate::zod_dto)),
//! `ZodValidationPipe` / `createZodValidationPipe`, `ZodValidationException`,
//! `ZodSerializationException`, `ZodSchemaDeclarationException`, `ZodSerializerDto`,
//! `ZodSerializerInterceptor` / `createZodSerializerInterceptor`, `ZodGuard` and `validate`.

use std::sync::{Arc, OnceLock, RwLock};

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{Direction, ParseOptions, Schema, ZodError, ZodSchema};
use crate::common::context::ExecutionContext;
use crate::common::interceptor::{CallHandler, CorvisInterceptor};
use crate::common::pipe::{ArgumentMetadata, ParamKind, PipeTransform};
use crate::common::reflector::Reflector;
use crate::http::{Body, HandlerResult, HttpException, HttpStatus};

/// A DTO with an attached schema (`createZodDto(schema, { codec })`).
/// Implement it with [`zod_dto!`](crate::zod_dto).
pub trait ZodDto {
    fn schema() -> &'static Schema;
    /// `createZodDto(schema, { codec: true })`: responses are serialized with `encode`.
    fn codec() -> bool {
        false
    }
    /// `Dto.create(input)`: `schema.parse(input)`, deserialized into the DTO type.
    fn create(input: &Value) -> Result<Self, ZodError>
    where
        Self: serde::de::DeserializeOwned + Sized,
    {
        Self::schema().parse_as(input)
    }
}

/// `ZodValidationException` (extends `BadRequestException`). Its body is
/// `{ "statusCode": 400, "message": "Validation failed", "errors": [...issues] }`.
pub struct ZodValidationException;

impl ZodValidationException {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(error: ZodError) -> HttpException {
        HttpException::new(json!({ "statusCode": 400, "message": "Validation failed", "errors": error.to_json() }), HttpStatus::BAD_REQUEST)
            .with_cause(error)
    }

    /// `exception.getZodError()`: the [`ZodError`] behind an exception built by
    /// [`ZodValidationException::new`] or [`ZodSerializationException::new`].
    pub fn get_zod_error(exception: &HttpException) -> Option<&ZodError> {
        exception.downcast_cause::<ZodError>()
    }
}

/// `ZodSerializationException` (extends `InternalServerErrorException`). Its body is
/// `{ "message": "Internal Server Error", "statusCode": 500 }`; the [`ZodError`] is kept as the cause.
pub struct ZodSerializationException;

impl ZodSerializationException {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(error: ZodError) -> HttpException {
        internal_server_error().with_cause(error)
    }

    /// `exception.getZodError()`.
    pub fn get_zod_error(exception: &HttpException) -> Option<&ZodError> {
        exception.downcast_cause::<ZodError>()
    }
}

/// `ZodSchemaDeclarationException` (extends `InternalServerErrorException`): thrown by a
/// pipe with `strictSchemaDeclaration` when the parameter type has no schema.
pub struct ZodSchemaDeclarationException;

impl ZodSchemaDeclarationException {
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> HttpException {
        internal_server_error()
    }
}

/// `new InternalServerErrorException()`: `{ "message": "Internal Server Error", "statusCode": 500 }`.
fn internal_server_error() -> HttpException {
    crate::http::InternalServerErrorException::empty()
}

/// `createValidationException(error)`.
pub type CreateValidationException = Arc<dyn Fn(ZodError) -> HttpException + Send + Sync>;

/// Options of `createZodValidationPipe({ createValidationException, strictSchemaDeclaration })`.
#[derive(Clone, Default)]
pub struct ZodValidationPipeOptions {
    pub create_validation_exception: Option<CreateValidationException>,
    pub strict_schema_declaration: bool,
}

impl ZodValidationPipeOptions {
    pub fn new() -> Self {
        Self::default()
    }
    /// `createValidationException: (error) => new BadRequestException(...)`.
    pub fn create_validation_exception<F: Fn(ZodError) -> HttpException + Send + Sync + 'static>(mut self, f: F) -> Self {
        self.create_validation_exception = Some(Arc::new(f));
        self
    }
    /// `strictSchemaDeclaration: true`.
    pub fn strict_schema_declaration(mut self, yes: bool) -> Self {
        self.strict_schema_declaration = yes;
        self
    }
}

/// Runs `f`, turning a panic (zod *throws* a plain `Error`: unmergable intersections, a transform
/// during `encode`, a panicking transform, ...) into `Err(None)`.
fn catch_thrown(f: impl FnOnce() -> Result<Value, ZodError>) -> Result<Value, Option<ZodError>> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(r) => r.map_err(Some),
        Err(_) => Err(None),
    }
}

/// `validate(value, schemaOrDto, createValidationException?)`: `schema.parse(value)`, turning
/// the error into a [`ZodValidationException`] (or the custom exception).
///
/// An error thrown while parsing (a panic here) also becomes a validation
/// exception: its body has no `errors`, and a custom `createValidationException` receives an
/// empty [`ZodError`].
pub fn validate(value: &Value, schema: &Schema, create: Option<&CreateValidationException>) -> Result<Value, HttpException> {
    validate_opt(Some(value), schema, create)
}

/// [`validate`] where `None` is JavaScript `undefined`.
pub fn validate_opt(value: Option<&Value>, schema: &Schema, create: Option<&CreateValidationException>) -> Result<Value, HttpException> {
    catch_thrown(|| schema.parse_opt(value, &ParseOptions::new())).map_err(|e| match (create, e) {
        (Some(f), e) => f(e.unwrap_or_else(|| ZodError::new(Vec::new()))),
        (None, Some(e)) => ZodValidationException::new(e),
        (None, None) => HttpException::new(json!({ "statusCode": 400, "message": "Validation failed" }), HttpStatus::BAD_REQUEST),
    })
}

/// Validates handler arguments with a zod [`Schema`] (`ZodValidationPipe`).
///
/// - `ZodValidationPipe::new(schema)`: `new ZodValidationPipe(schemaOrDto)`.
/// - `ZodValidationPipe::auto()` / `ZodValidationPipe` as a type: uses the schema of the
///   parameter type registered with [`zod_dto!`](crate::zod_dto); other types pass through
///   (or fail with [`ZodSchemaDeclarationException`] with `strictSchemaDeclaration`).
/// - [`create_zod_validation_pipe`]: `createZodValidationPipe({ ... })`.
///
/// The parsed output (defaults applied, unknown keys stripped, transforms run) replaces
/// the raw value before it is deserialized into the parameter type. Custom parameter
/// decorators are skipped.
#[derive(Clone, Default)]
pub struct ZodValidationPipe {
    schema: Option<Schema>,
    options: ZodValidationPipeOptions,
}

impl ZodValidationPipe {
    pub fn new(schema: impl Into<Schema>) -> Self {
        ZodValidationPipe { schema: Some(schema.into()), options: ZodValidationPipeOptions::default() }
    }

    /// `new ZodValidationPipe(MyDto)`.
    pub fn dto<T: ZodDto>() -> Self {
        Self::new(T::schema().clone())
    }

    pub fn auto() -> Self {
        ZodValidationPipe::default()
    }

    /// Same pipe with the options of `createZodValidationPipe`.
    pub fn with_options(mut self, options: ZodValidationPipeOptions) -> Self {
        self.options = options;
        self
    }
}

/// `createZodValidationPipe({ createValidationException, strictSchemaDeclaration })`: returns
/// a pipe that uses the parameter type's schema (`app.useGlobalPipes(...)` / `APP_PIPE`).
pub fn create_zod_validation_pipe(options: ZodValidationPipeOptions) -> ZodValidationPipe {
    ZodValidationPipe::auto().with_options(options)
}

#[async_trait]
impl PipeTransform for ZodValidationPipe {
    async fn transform(&self, value: Value, metadata: &ArgumentMetadata) -> Result<Value, HttpException> {
        if metadata.kind == ParamKind::Custom {
            return Ok(value);
        }
        let create = self.options.create_validation_exception.as_ref();
        // Express gives `undefined` for a missing body, query value, route param or header
        // (and for a missing key of `@Body('key')`); the raw value is then `null` here.
        let input = (!value.is_null()).then_some(&value);
        if let Some(s) = &self.schema {
            return validate_opt(input, s, create);
        }
        match metadata.metatype.zod {
            Some(f) => validate_opt(input, f(), create),
            None if self.options.strict_schema_declaration => Err(ZodSchemaDeclarationException::new()),
            None => Ok(value),
        }
    }
}

crate::default_injectable!(ZodValidationPipe);

// ---------------------------------------------------------------------------
// Guard (deprecated in favour of the pipe, kept for parity).
// ---------------------------------------------------------------------------

/// The request part a [`ZodGuard`] validates (`'body' | 'query' | 'params'`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZodGuardSource {
    Body,
    Query,
    Params,
}

/// `ZodGuard` / `UseZodGuard(source, schemaOrDto)` / `createZodGuard({ createValidationException })`:
/// validates `request[source]` and lets the request through (the parsed value is discarded).
/// Deprecated in favour of [`ZodValidationPipe`].
///
/// ```ignore
/// #[use_guards(z::ZodGuard::new(z::ZodGuardSource::Body, schema))]
/// ```
#[derive(Clone)]
pub struct ZodGuard {
    source: ZodGuardSource,
    schema: Schema,
    create_validation_exception: Option<CreateValidationException>,
}

impl ZodGuard {
    pub fn new(source: ZodGuardSource, schema: impl Into<Schema>) -> Self {
        ZodGuard { source, schema: schema.into(), create_validation_exception: None }
    }
    /// `new ZodGuard(source, MyDto)`.
    pub fn dto<T: ZodDto>(source: ZodGuardSource) -> Self {
        Self::new(source, T::schema().clone())
    }
    /// `createZodGuard({ createValidationException })`.
    pub fn create_validation_exception<F: Fn(ZodError) -> HttpException + Send + Sync + 'static>(mut self, f: F) -> Self {
        self.create_validation_exception = Some(Arc::new(f));
        self
    }
}

#[async_trait]
impl crate::common::guard::CanActivate for ZodGuard {
    async fn can_activate(&self, context: &ExecutionContext) -> Result<bool, HttpException> {
        let req = context.switch_to_http().get_request();
        let data = match self.source {
            ZodGuardSource::Body => req.body().clone(),
            ZodGuardSource::Query => Value::Object(req.query().clone()),
            ZodGuardSource::Params => Value::Object(req.params().iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect()),
        };
        let input = (!data.is_null()).then_some(&data);
        validate_opt(input, &self.schema, self.create_validation_exception.as_ref())?;
        Ok(true)
    }
}

// ---------------------------------------------------------------------------
// Serialization.
// ---------------------------------------------------------------------------

/// Metadata key used by [`ZodSerializerDto`] (`ZodSerializerDtoOptions`).
pub const ZOD_SERIALIZER_DTO_OPTIONS: &str = "ZOD_SERIALIZER_DTO_OPTIONS";

#[derive(Clone)]
struct SerializerEntry {
    schema: Schema,
    /// `Some(codec)` for a DTO, `None` for a plain schema.
    dto_codec: Option<bool>,
}

fn serializer_table() -> &'static RwLock<Vec<SerializerEntry>> {
    static TABLE: OnceLock<RwLock<Vec<SerializerEntry>>> = OnceLock::new();
    TABLE.get_or_init(|| RwLock::new(Vec::new()))
}

fn register_serializer(entry: SerializerEntry, array: bool) -> Value {
    let mut table = serializer_table().write().unwrap_or_else(|e| e.into_inner());
    let id = table.len();
    table.push(entry);
    json!({ "zodSerializer": id, "array": array })
}

/// `@ZodSerializerDto(...)`: builds the metadata value that [`ZodSerializerInterceptor`] reads.
/// Attach it with `#[set_metadata(z::ZOD_SERIALIZER_DTO_OPTIONS, ZodSerializerDto::dto::<UserDto>())]`.
///
/// - `ZodSerializerDto::dto::<UserDto>()`: `@ZodSerializerDto(UserDto)`.
/// - `ZodSerializerDto::dto_array::<UserDto>()`: `@ZodSerializerDto([UserDto])`.
/// - `ZodSerializerDto::schema(s)` / `schema_array(s)`: a plain schema.
///
/// `ZodResponse({ type })` is the same as `ZodSerializerDto(type)` plus the OpenAPI response
/// and `@HttpCode(status)`.
pub struct ZodSerializerDto;

impl ZodSerializerDto {
    pub fn dto<T: ZodDto>() -> Value {
        register_serializer(SerializerEntry { schema: T::schema().clone(), dto_codec: Some(T::codec()) }, false)
    }
    pub fn dto_array<T: ZodDto>() -> Value {
        register_serializer(SerializerEntry { schema: T::schema().clone(), dto_codec: Some(T::codec()) }, true)
    }
    pub fn schema(schema: impl Into<Schema>) -> Value {
        register_serializer(SerializerEntry { schema: schema.into(), dto_codec: None }, false)
    }
    pub fn schema_array(schema: impl Into<Schema>) -> Value {
        register_serializer(SerializerEntry { schema: schema.into(), dto_codec: None }, true)
    }

    /// Serializes `value` like the interceptor does for the given metadata value.
    pub fn serialize(meta: &Value, value: &Value, report_input: Option<bool>) -> Result<Value, HttpException> {
        let Some(id) = meta.get("zodSerializer").and_then(Value::as_u64) else { return Ok(value.clone()) };
        let entry = {
            let table = serializer_table().read().unwrap_or_else(|e| e.into_inner());
            match table.get(id as usize) {
                Some(e) => e.clone(),
                None => return Ok(value.clone()),
            }
        };
        let schema =
            if meta.get("array").and_then(Value::as_bool).unwrap_or(false) { Schema::new(entry.schema.array()) } else { entry.schema };
        let mut opts = ParseOptions::new();
        if let Some(r) = report_input {
            opts = opts.report_input(r);
        }
        if entry.dto_codec == Some(true) {
            opts = opts.direction(Direction::Backward);
        }
        catch_thrown(|| schema.parse_with(value, &opts)).map_err(|e| match e {
            Some(e) => ZodSerializationException::new(e),
            None => internal_server_error(),
        })
    }
}

/// `ZodSerializerInterceptor` / `createZodSerializerInterceptor({ reportInput })`: parses
/// (or `encode`s, for codec DTOs) the handler result with the schema set by
/// [`ZodSerializerDto`] on the handler or the controller. Failures become a
/// [`ZodSerializationException`] (500). Non-JSON bodies (files, streams) pass through.
#[derive(Clone, Copy, Default)]
pub struct ZodSerializerInterceptor {
    pub report_input: Option<bool>,
}

impl ZodSerializerInterceptor {
    pub fn new() -> Self {
        Self::default()
    }
    /// `createZodSerializerInterceptor({ reportInput })`.
    pub fn create(report_input: Option<bool>) -> Self {
        ZodSerializerInterceptor { report_input }
    }
}

#[async_trait]
impl CorvisInterceptor for ZodSerializerInterceptor {
    async fn intercept(&self, context: &ExecutionContext, next: CallHandler) -> HandlerResult {
        let meta: Option<Value> = Reflector.get_all_and_override(ZOD_SERIALIZER_DTO_OPTIONS, &[context.get_handler(), context.get_class()]);
        let response = next.handle().await?;
        let Some(meta) = meta else { return Ok(response) };
        // An empty result is validated as `null`, like the original validates `null`/`undefined`.
        let body = match response.body() {
            Body::Json(v) => v.clone(),
            Body::Empty => Value::Null,
            _ => return Ok(response),
        };
        let out = ZodSerializerDto::serialize(&meta, &body, self.report_input)?;
        Ok(response.map_data(|_| out))
    }
}

crate::default_injectable!(ZodSerializerInterceptor);
