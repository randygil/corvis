//! File uploads (`multipart/form-data`) with the API and semantics of the Express platform
//! + `multer` 2 + `busboy`:
//!
//! - Interceptors: [`FileInterceptor`], [`FilesInterceptor`], [`FileFieldsInterceptor`],
//!   [`AnyFilesInterceptor`] and [`NoFilesInterceptor`].
//! - Parameters: `#[uploaded_file] file: Option<UploadedFile>` (`@UploadedFile()`) and
//!   `#[uploaded_files] files: Vec<UploadedFile>` / `HashMap<String, Vec<UploadedFile>>`
//!   (`@UploadedFiles()`); `#[body]` receives the text fields (`req.body`).
//! - Storage: [`memory_storage`] (default, `file.buffer`) and [`disk_storage`] (`dest`,
//!   `destination`/`filename` callbacks), or your own [`StorageEngine`].
//! - Options: [`MulterOptions`] (`dest`, `storage`, `limits`, `file_filter`, `preserve_path`,
//!   `def_param_charset`) and module defaults with [`MulterModule::register`].
//! - Errors: the same `MulterError` codes and the same HTTP mapping as the Express
//!   platform's `transformException` (`413 File too large`, `400 Unexpected field - avatar`,
//!   `400 Multipart: Unexpected end of form`, ...).
//!
//! ```ignore
//! #[post("upload")]
//! #[use_interceptors(FileInterceptor::new("file").options(
//!     MulterOptions::new().limits(Limits::new().file_size(5 * 1024 * 1024)),
//! ))]
//! async fn upload(&self, #[uploaded_file(ParseFilePipe::new()
//!     .validator(MaxFileSizeValidator::new(1000))
//!     .validator(FileTypeValidator::new("image/jpeg")))] file: UploadedFile) -> Value { ... }
//! ```

mod append_field;
mod buffer;
mod busboy;
mod file;
mod interceptors;
mod storage;

use std::sync::Arc;

use serde_json::Value;

use crate::di::{DynamicModule, Provider, ResolveCtx, Token};
use crate::http::{HttpException, Request};

pub use buffer::{buffer_from_value, buffer_to_value};
pub use file::{FileInfo, UploadedFile};
pub use interceptors::{
    AnyFilesInterceptor, FileFieldsInterceptor, FileInterceptor, FilesInterceptor, MulterField, NoFilesInterceptor,
};
pub use storage::{
    DiskStorage, DiskStorageOptions, MemoryStorage, StorageEngine, disk_storage, memory_storage, node_path_join,
};

/// Injection token of the module-wide options (`MULTER_MODULE_OPTIONS`).
pub const MULTER_MODULE_OPTIONS: &str = "MULTER_MODULE_OPTIONS";

/// Default maximum size of a multipart body read by a file interceptor (10 MiB). See
/// [`MulterOptions::body_limit`].
pub const DEFAULT_MULTIPART_BODY_LIMIT: u64 = 10 * 1024 * 1024;

/// `limits` of multer / busboy. `None` = busboy's default (`field_size` = 1 MiB, the rest unlimited).
///
/// They are enforced while the body is read from the socket: the upload is aborted as soon as a
/// limit is hit, without reading the rest. The whole body is also capped by
/// [`MulterOptions::body_limit`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Limits {
    /// Max field name size in UTF-16 units (multer checks it only when set).
    pub field_name_size: Option<u64>,
    /// Max field value size in bytes (default 1 MiB).
    pub field_size: Option<u64>,
    /// Max number of non-file fields.
    pub fields: Option<u64>,
    /// Max file size in bytes. As in busboy, a file that *reaches* the limit is rejected.
    pub file_size: Option<u64>,
    /// Max number of file fields.
    pub files: Option<u64>,
    /// Max number of parts (fields + files).
    pub parts: Option<u64>,
    /// Max nesting depth of field names (`a[b][c]`), multer 2.
    pub field_nesting_depth: Option<u64>,
}

impl Limits {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn field_name_size(mut self, v: u64) -> Self {
        self.field_name_size = Some(v);
        self
    }
    pub fn field_size(mut self, v: u64) -> Self {
        self.field_size = Some(v);
        self
    }
    pub fn fields(mut self, v: u64) -> Self {
        self.fields = Some(v);
        self
    }
    pub fn file_size(mut self, v: u64) -> Self {
        self.file_size = Some(v);
        self
    }
    pub fn files(mut self, v: u64) -> Self {
        self.files = Some(v);
        self
    }
    pub fn parts(mut self, v: u64) -> Self {
        self.parts = Some(v);
        self
    }
    pub fn field_nesting_depth(mut self, v: u64) -> Self {
        self.field_nesting_depth = Some(v);
        self
    }
}

/// `fileFilter(req, file, cb)`: `Ok(true)` accepts the file, `Ok(false)` skips it silently and
/// `Err(e)` aborts the upload with `e` (`cb(new BadRequestException(...))`).
pub type FileFilter = Arc<dyn Fn(&Request, &FileInfo) -> Result<bool, HttpException> + Send + Sync>;

/// Options of `multer(...)` / `FileInterceptor(field, options)` / `MulterModule.register(options)`.
#[derive(Clone, Default)]
pub struct MulterOptions {
    pub dest: Option<String>,
    pub storage: Option<Arc<dyn StorageEngine>>,
    pub limits: Option<Limits>,
    pub preserve_path: Option<bool>,
    pub def_param_charset: Option<String>,
    pub file_filter: Option<FileFilter>,
    /// Maximum size of the whole multipart body (see [`MulterOptions::body_limit`]).
    pub body_limit: Option<u64>,
    /// Remove the stored files when a later pipe or the handler fails (default `true`).
    pub remove_files_on_error: Option<bool>,
}

impl std::fmt::Debug for MulterOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MulterOptions")
            .field("dest", &self.dest)
            .field("storage", &self.storage.as_ref().map(|_| "StorageEngine"))
            .field("limits", &self.limits)
            .field("preserve_path", &self.preserve_path)
            .field("def_param_charset", &self.def_param_charset)
            .field("file_filter", &self.file_filter.as_ref().map(|_| "fn"))
            .field("body_limit", &self.body_limit)
            .field("remove_files_on_error", &self.remove_files_on_error)
            .finish()
    }
}

impl MulterOptions {
    pub fn new() -> Self {
        Self::default()
    }
    /// `{ dest: 'uploads' }`: disk storage with random file names in that folder (created if missing).
    pub fn dest(mut self, dest: impl Into<String>) -> Self {
        self.dest = Some(dest.into());
        self
    }
    /// `{ storage: diskStorage({...}) }` / `memoryStorage()` / a custom engine.
    pub fn storage(mut self, storage: impl StorageEngine) -> Self {
        self.storage = Some(Arc::new(storage));
        self
    }
    pub fn storage_arc(mut self, storage: Arc<dyn StorageEngine>) -> Self {
        self.storage = Some(storage);
        self
    }
    pub fn limits(mut self, limits: Limits) -> Self {
        self.limits = Some(limits);
        self
    }
    /// Keep the full path sent by the client in `originalname` (`preservePath`).
    ///
    /// The path is the client's: never use it as is to build a path on disk. The disk storage
    /// refuses names that leave its destination (`../`, absolute paths), but a custom storage
    /// engine must check it itself.
    pub fn preserve_path(mut self, v: bool) -> Self {
        self.preserve_path = Some(v);
        self
    }
    /// Charset of non-extended header parameters such as `filename` (default `latin1`, like multer).
    pub fn def_param_charset(mut self, charset: impl Into<String>) -> Self {
        self.def_param_charset = Some(charset.into());
        self
    }
    pub fn file_filter(mut self, f: impl Fn(&Request, &FileInfo) -> Result<bool, HttpException> + Send + Sync + 'static) -> Self {
        self.file_filter = Some(Arc::new(f));
        self
    }
    /// Maximum size in bytes of the whole multipart body (all parts, headers and boundaries).
    /// A bigger body is answered with `413 request entity too large` as soon as the limit is
    /// crossed (or before reading, if `Content-Length` already exceeds it).
    ///
    /// Default: [`DEFAULT_MULTIPART_BODY_LIMIT`] (10 MiB), or `file_size * files + 1 MiB` when
    /// `limits.file_size` asks for more. multer has no such limit; Corvis keeps each file in
    /// memory until the storage engine receives it, so the body must be bounded.
    pub fn body_limit(mut self, bytes: u64) -> Self {
        self.body_limit = Some(bytes);
        self
    }
    /// `false` keeps the files the storage engine already wrote when a pipe (`ParseFilePipe`) or
    /// the handler fails afterwards. By default they are removed (`StorageEngine::remove_file`),
    /// so rejected uploads do not pile up on disk.
    pub fn remove_files_on_error(mut self, remove: bool) -> Self {
        self.remove_files_on_error = Some(remove);
        self
    }

    /// The body limit that applies with these options and `limits`.
    pub(crate) fn effective_body_limit(&self, limits: &Limits) -> u64 {
        if let Some(b) = self.body_limit {
            return b;
        }
        let from_files = limits
            .file_size
            .map(|f| f.saturating_mul(limits.files.unwrap_or(1).max(1)).saturating_add(1024 * 1024))
            .unwrap_or(0);
        DEFAULT_MULTIPART_BODY_LIMIT.max(from_files)
    }

    /// `{ ...moduleOptions, ...localOptions }`.
    pub(crate) fn merged_over(&self, base: Option<&MulterOptions>) -> MulterOptions {
        let Some(base) = base else { return self.clone() };
        MulterOptions {
            dest: self.dest.clone().or_else(|| base.dest.clone()),
            storage: self.storage.clone().or_else(|| base.storage.clone()),
            limits: self.limits.clone().or_else(|| base.limits.clone()),
            preserve_path: self.preserve_path.or(base.preserve_path),
            def_param_charset: self.def_param_charset.clone().or_else(|| base.def_param_charset.clone()),
            file_filter: self.file_filter.clone().or_else(|| base.file_filter.clone()),
            body_limit: self.body_limit.or(base.body_limit),
            remove_files_on_error: self.remove_files_on_error.or(base.remove_files_on_error),
        }
    }

    /// The storage engine multer would pick: `storage`, else `diskStorage({ destination: dest })`, else memory.
    pub(crate) fn resolve_storage(&self) -> Arc<dyn StorageEngine> {
        if let Some(s) = &self.storage {
            return s.clone();
        }
        if let Some(d) = &self.dest {
            return Arc::new(disk_storage(DiskStorageOptions::new().destination(d.clone())));
        }
        Arc::new(memory_storage())
    }
}

/// `MulterModule`: default options for every file interceptor of the importing module.
#[derive(Default)]
pub struct MulterModule;
crate::default_injectable!(MulterModule);

impl crate::di::Module for MulterModule {
    fn definition() -> crate::di::ModuleDefinition {
        crate::di::ModuleDefinition::new::<MulterModule>("MulterModule").module_provider(Provider::class::<MulterModule>())
    }
}

impl MulterModule {
    /// `MulterModule.register({ dest: './upload' })`.
    pub fn register(options: MulterOptions) -> DynamicModule {
        DynamicModule::new::<MulterModule>()
            .provider(Provider::use_value(Token::from(MULTER_MODULE_OPTIONS), options))
            .export(Token::from(MULTER_MODULE_OPTIONS))
    }

    /// `MulterModule.registerAsync({ useFactory, inject })`.
    pub fn register_async<F, Fut>(factory: F) -> DynamicModule
    where
        F: Fn(ResolveCtx) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = crate::Result<MulterOptions>> + Send + 'static,
    {
        let factory = Arc::new(factory);
        DynamicModule::new::<MulterModule>()
            .provider(Provider::use_factory(Token::from(MULTER_MODULE_OPTIONS), move |ctx| {
                let f = factory.clone();
                async move { f(ctx).await }
            }))
            .export(Token::from(MULTER_MODULE_OPTIONS))
    }
}

/// Codes of `MulterError` (multer `lib/multer-error.js`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MulterErrorCode {
    LimitPartCount,
    LimitFileSize,
    LimitFileCount,
    LimitFieldKey,
    LimitFieldValue,
    LimitFieldCount,
    LimitUnexpectedFile,
    MissingFieldName,
    LimitFieldNesting,
}

impl MulterErrorCode {
    /// `'LIMIT_FILE_SIZE'`, ...
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::LimitPartCount => "LIMIT_PART_COUNT",
            Self::LimitFileSize => "LIMIT_FILE_SIZE",
            Self::LimitFileCount => "LIMIT_FILE_COUNT",
            Self::LimitFieldKey => "LIMIT_FIELD_KEY",
            Self::LimitFieldValue => "LIMIT_FIELD_VALUE",
            Self::LimitFieldCount => "LIMIT_FIELD_COUNT",
            Self::LimitUnexpectedFile => "LIMIT_UNEXPECTED_FILE",
            Self::MissingFieldName => "MISSING_FIELD_NAME",
            Self::LimitFieldNesting => "LIMIT_FIELD_NESTING",
        }
    }
    pub fn message(&self) -> &'static str {
        match self {
            Self::LimitPartCount => "Too many parts",
            Self::LimitFileSize => "File too large",
            Self::LimitFileCount => "Too many files",
            Self::LimitFieldKey => "Field name too long",
            Self::LimitFieldValue => "Field value too long",
            Self::LimitFieldCount => "Too many fields",
            Self::LimitUnexpectedFile => "Unexpected field",
            Self::MissingFieldName => "Field name missing",
            Self::LimitFieldNesting => "Field name nesting too deep",
        }
    }
}

/// `MulterError` (`err.code`, `err.field`, `err.message`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MulterError {
    pub code: MulterErrorCode,
    pub field: Option<String>,
}

impl MulterError {
    pub fn new(code: MulterErrorCode, field: Option<String>) -> Self {
        MulterError { code, field }
    }
    pub fn message(&self) -> &'static str {
        self.code.message()
    }
}

impl std::fmt::Display for MulterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for MulterError {}

/// Errors raised by busboy (`Malformed part header`, `Unexpected end of form`,
/// `Unsupported content type: ...`, ...). Available as the cause of 500 errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusboyError(pub String);

impl std::fmt::Display for BusboyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BusboyError {}

/// `transformException` of the Express platform for multer errors.
pub(crate) fn transform_multer_error(e: MulterError) -> HttpException {
    use crate::http::{BadRequestException, PayloadTooLargeException};
    use MulterErrorCode::*;
    match e.code {
        LimitFileSize => PayloadTooLargeException::new(e.message()),
        LimitFileCount | LimitFieldKey | LimitFieldValue | LimitFieldCount | LimitUnexpectedFile | LimitPartCount
        | MissingFieldName => match &e.field {
            Some(f) => BadRequestException::new(format!("{} - {}", e.message(), f)),
            None => BadRequestException::new(e.message()),
        },
        LimitFieldNesting => HttpException::internal(e),
    }
}

pub(crate) fn transform_busboy_error(msg: &str) -> HttpException {
    use crate::http::BadRequestException;
    match msg {
        busboy::BOUNDARY_NOT_FOUND => BadRequestException::new(msg),
        busboy::MALFORMED_PART_HEADER | busboy::UNEXPECTED_END_OF_FORM | busboy::UNEXPECTED_END_OF_FILE => {
            BadRequestException::new(format!("Multipart: {msg}"))
        }
        _ => HttpException::internal(BusboyError(msg.to_string())),
    }
}

/// What multer leaves on the request (`req.body`, `req.file`, `req.files`).
#[derive(Clone, Debug, Default)]
pub struct MultipartData {
    pub body: Arc<Value>,
    pub file: Option<Arc<Value>>,
    pub files: Option<Arc<Value>>,
}

/// Accessors for what the file interceptors put on the request.
pub trait MulterRequestExt {
    /// `req.file` (single file interceptor), as the JSON multer builds.
    fn file(&self) -> Option<UploadedFile>;
    /// `req.files` (array or object of arrays, depending on the interceptor).
    fn files_value(&self) -> Option<Value>;
    /// `req.body` built from the text fields of a multipart request.
    fn multipart_body(&self) -> Option<Value>;
}

impl MulterRequestExt for Request {
    fn file(&self) -> Option<UploadedFile> {
        let d = self.get::<MultipartData>()?;
        serde_json::from_value((*d.file?).clone()).ok()
    }
    fn files_value(&self) -> Option<Value> {
        self.get::<MultipartData>().and_then(|d| d.files.map(|f| (*f).clone()))
    }
    fn multipart_body(&self) -> Option<Value> {
        self.get::<MultipartData>().map(|d| (*d.body).clone())
    }
}

/// Value of `@UploadedFile(key)` / `@UploadedFiles()` before pipes (`req[key || 'file']`, `req.files`).
pub(crate) fn uploaded_param(req: &Request, files: bool, key: Option<&str>) -> Value {
    let Some(d) = req.get::<MultipartData>() else { return Value::Null };
    let key = if files { "files" } else { key.unwrap_or("file") };
    match key {
        "file" => d.file.map(|v| (*v).clone()).unwrap_or(Value::Null),
        "files" => d.files.map(|v| (*v).clone()).unwrap_or(Value::Null),
        "body" => (*d.body).clone(),
        _ => Value::Null,
    }
}

/// `req.body` after a file interceptor ran (`None` = not a multipart request).
pub(crate) fn body_override(req: &Request) -> Option<Value> {
    req.get::<MultipartData>().map(|d| (*d.body).clone())
}

/// `type-is`' `is(req, ['multipart'])`: the request has a body and a `multipart/*` content type.
pub fn is_multipart(req: &Request) -> bool {
    let has_body = req.headers().contains_key(http::header::TRANSFER_ENCODING)
        || req.header("content-length").is_some_and(|v| v.trim().parse::<f64>().is_ok());
    if !has_body {
        return false;
    }
    let Some(ct) = req.header("content-type") else { return false };
    let main = ct.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    match main.split_once('/') {
        Some((t, s)) => t == "multipart" && !s.is_empty() && !s.contains(char::is_whitespace),
        None => false,
    }
}

/// Reads the whole body of a request whose body the server did not read up front (a
/// `multipart/*` body: only file interceptors read it, streaming) and returns it, failing with
/// `413` past `limit` bytes, `408` past the server's `request_timeout` and `400` if the client
/// aborts. For other requests it returns `req.raw_body()`.
///
/// Use it in a handler or middleware that parses a multipart body by itself. The body can be
/// read only once: afterwards file interceptors see an empty body.
pub async fn read_body(req: &Request, limit: u64) -> Result<bytes::Bytes, HttpException> {
    let mut source = BodySource::of(req);
    if source.is_stream() && crate::server::content_length(req.headers()).is_some_and(|n| n > limit) {
        source.give_back(req);
        return Err(crate::server::entity_too_large_error());
    }
    let mut out = bytes::BytesMut::new();
    loop {
        let chunk = match source.next().await {
            Ok(Some(c)) => c,
            Ok(None) => break,
            Err(e) => {
                source.give_back(req);
                return Err(e);
            }
        };
        if (out.len() + chunk.len()) as u64 > limit {
            source.give_back(req);
            return Err(crate::server::entity_too_large_error());
        }
        if !source.is_stream() {
            return Ok(chunk);
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out.freeze())
}

/// Where a multipart body comes from: the socket (read on demand) or the request buffer
/// (requests built in memory, e.g. by `corvis::testing`).
pub(crate) enum BodySource {
    Stream { body: hyper::body::Incoming, deadline: Option<tokio::time::Instant> },
    Buffered(Option<bytes::Bytes>),
}

impl BodySource {
    pub(crate) fn of(req: &Request) -> Self {
        match crate::server::PendingBodySlot::take(req) {
            Some(p) => BodySource::Stream { body: p.body, deadline: p.deadline },
            None => BodySource::Buffered(Some(req.raw_body().clone()).filter(|b| !b.is_empty())),
        }
    }

    pub(crate) fn is_stream(&self) -> bool {
        matches!(self, BodySource::Stream { .. })
    }

    /// Puts an unfinished socket body back on the request (after a failed read), so the server
    /// drains a bounded part of it once the error response is sent.
    pub(crate) fn give_back(self, req: &Request) {
        if let (BodySource::Stream { body, deadline }, Some(slot)) = (self, req.get::<crate::server::PendingBodySlot>()) {
            let mut guard = slot.0.lock().unwrap_or_else(|e| e.into_inner());
            *guard = Some(crate::server::PendingBody { body, deadline });
        }
    }

    /// The next chunk of the body (`None` at the end).
    pub(crate) async fn next(&mut self) -> Result<Option<bytes::Bytes>, HttpException> {
        use http_body_util::BodyExt;
        match self {
            BodySource::Buffered(b) => Ok(b.take()),
            BodySource::Stream { body, deadline } => loop {
                let frame = match deadline {
                    Some(d) => tokio::time::timeout_at(*d, body.frame())
                        .await
                        .map_err(|_| crate::server::request_timeout_error())?,
                    None => body.frame().await,
                };
                match frame {
                    None => return Ok(None),
                    Some(Err(_)) => return Err(crate::server::request_aborted_error()),
                    Some(Ok(f)) => {
                        if let Ok(data) = f.into_data() {
                            if !data.is_empty() {
                                return Ok(Some(data));
                            }
                        }
                    }
                }
            },
        }
    }
}

/// `true` for `Content-Type: multipart/*`: the server leaves these bodies unread (see
/// [`read_body`] and the file interceptors).
pub(crate) fn is_multipart_content_type(headers: &http::HeaderMap) -> bool {
    headers
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.trim_start().to_ascii_lowercase().starts_with("multipart/"))
        .unwrap_or(false)
}
