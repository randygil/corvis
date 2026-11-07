//! `FileInterceptor`, `FilesInterceptor`, `FileFieldsInterceptor`, `AnyFilesInterceptor` and
//! `NoFilesInterceptor` of the Express platform, on top of multer's `makeMiddleware`.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Map, Value};

use super::busboy::{self, Event};
use super::{
    FileInfo, Limits, MULTER_MODULE_OPTIONS, MulterError, MulterErrorCode, MulterOptions, MultipartData,
    StorageEngine, append_field::append_field, is_multipart, transform_busboy_error, transform_multer_error,
};
use crate::common::context::ExecutionContext;
use crate::common::interceptor::{CallHandler, CorvisInterceptor};
use crate::http::{HandlerResult, HttpException, Request};

/// `{ name, maxCount }` of `FileFieldsInterceptor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MulterField {
    pub name: String,
    pub max_count: Option<u64>,
}

impl MulterField {
    pub fn new(name: impl Into<String>) -> Self {
        MulterField { name: name.into(), max_count: None }
    }
    pub fn max_count(mut self, n: u64) -> Self {
        self.max_count = Some(n);
        self
    }
}

impl From<&str> for MulterField {
    fn from(name: &str) -> Self {
        MulterField::new(name)
    }
}

impl From<(&str, u64)> for MulterField {
    fn from((name, n): (&str, u64)) -> Self {
        MulterField::new(name).max_count(n)
    }
}

/// multer's `fileStrategy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Strategy {
    None,
    Value,
    Array,
    Object,
}

#[derive(Clone)]
struct Setup {
    /// `None` = `any()` (no per-field accounting).
    fields: Option<Vec<MulterField>>,
    strategy: Strategy,
}

/// Runs multer on the request and stores `req.body` / `req.file` / `req.files`.
async fn run(ctx: &ExecutionContext, local: &MulterOptions, setup: &Setup) -> Result<(), HttpException> {
    let req = ctx.request();
    if !is_multipart(req) {
        return Ok(());
    }
    let module_opts = ctx
        .module_ref()
        .and_then(|m| m.get_token_in_context::<MulterOptions>(MULTER_MODULE_OPTIONS).ok());
    let options = local.merged_over(module_opts.as_deref());
    let data = process(req, &options, setup).await?;
    req.insert(data);
    Ok(())
}

struct PendingFile {
    info: FileInfo,
    included: bool,
    slot: Option<usize>,
}

/// A placeholder of `FileAppender`: `None` while the file is being stored.
struct Appender {
    strategy: Strategy,
    /// Array strategy: placeholders in insertion order.
    list: Vec<Option<Option<Map<String, Value>>>>,
    /// Object strategy: field -> placeholders.
    by_field: Vec<(String, Vec<usize>)>,
    single: Option<Map<String, Value>>,
}

impl Appender {
    fn insert(&mut self, field: &str) -> usize {
        let idx = self.list.len();
        self.list.push(Some(None));
        if self.strategy == Strategy::Object {
            match self.by_field.iter_mut().find(|(k, _)| k == field) {
                Some((_, v)) => v.push(idx),
                None => self.by_field.push((field.to_string(), vec![idx])),
            }
        }
        idx
    }
    fn remove(&mut self, idx: usize) {
        self.list[idx] = None;
        if self.strategy == Strategy::Object {
            for (_, v) in self.by_field.iter_mut() {
                v.retain(|i| *i != idx);
            }
            self.by_field.retain(|(_, v)| !v.is_empty());
        }
    }
    fn replace(&mut self, idx: usize, file: Map<String, Value>) {
        if self.strategy == Strategy::Value {
            self.single = Some(file);
            return;
        }
        self.list[idx] = Some(Some(file));
    }
    fn placeholder(&self, idx: usize, field: &str) -> Value {
        match &self.list[idx] {
            Some(Some(f)) => Value::Object(f.clone()),
            // A placeholder that never got its file keeps `{ fieldname }`.
            _ => {
                let mut m = Map::new();
                m.insert("fieldname".into(), Value::String(field.to_string()));
                Value::Object(m)
            }
        }
    }
}

async fn process(req: &Request, options: &MulterOptions, setup: &Setup) -> Result<MultipartData, HttpException> {
    let storage = options.resolve_storage();
    let limits = options.limits.clone().unwrap_or_default();
    let preserve_path = options.preserve_path.unwrap_or(false);
    let def_param_charset = options.def_param_charset.clone().unwrap_or_else(|| "latin1".into());

    let boundary = busboy::boundary_of(req.headers().get(http::header::CONTENT_TYPE).map(|v| v.as_bytes()))
        .map_err(|e| match e {
            busboy::ConfigError::BoundaryNotFound => transform_busboy_error(busboy::BOUNDARY_NOT_FOUND),
            other => HttpException::internal(super::BusboyError(other.message())),
        })?;
    let cfg = busboy::Config { limits: &limits, preserve_path, def_param_charset: &def_param_charset };
    let events = busboy::parse(req.raw_body(), &boundary, &cfg);

    let mut files_left: HashMap<String, u64> = HashMap::new();
    if let Some(fields) = &setup.fields {
        for f in fields {
            files_left.insert(f.name.clone(), f.max_count.unwrap_or(u64::MAX));
        }
    }
    let mut body = Map::new();
    let mut appender = Appender { strategy: setup.strategy, list: Vec::new(), by_field: Vec::new(), single: None };
    let mut pending: HashMap<usize, PendingFile> = HashMap::new();
    let mut uploaded: Vec<Map<String, Value>> = Vec::new();
    let mut error: Option<HttpException> = None;

    for ev in events {
        let result: Result<(), HttpException> = match ev {
            Event::Field { name, value, value_truncated } => {
                handle_field(&mut body, name, value, value_truncated, &limits).map_err(transform_multer_error)
            }
            Event::FileStart { id, name, filename, encoding, mime_type } => {
                let Some(fieldname) = name else {
                    error = Some(transform_multer_error(MulterError::new(MulterErrorCode::MissingFieldName, None)));
                    break;
                };
                // Parts without a file name are not files for multer.
                let Some(filename) = filename.filter(|f| !f.is_empty()) else { continue };
                if let Some(max) = limits.field_name_size {
                    if fieldname.encode_utf16().count() as u64 > max {
                        error = Some(transform_multer_error(MulterError::new(MulterErrorCode::LimitFieldKey, None)));
                        break;
                    }
                }
                let info = FileInfo { fieldname: fieldname.clone(), originalname: filename, encoding, mimetype: mime_type };
                let slot = appender.insert(&fieldname);
                let filter = if setup.fields.is_some() {
                    let left = files_left.get(&fieldname).copied().unwrap_or(0);
                    if left == 0 {
                        Err(transform_multer_error(MulterError::new(MulterErrorCode::LimitUnexpectedFile, Some(fieldname))))
                    } else {
                        if left != u64::MAX {
                            files_left.insert(fieldname.clone(), left - 1);
                        }
                        apply_filter(options, req, &info)
                    }
                } else {
                    apply_filter(options, req, &info)
                };
                match filter {
                    Err(e) => {
                        appender.remove(slot);
                        Err(e)
                    }
                    Ok(false) => {
                        appender.remove(slot);
                        pending.insert(id, PendingFile { info, included: false, slot: None });
                        Ok(())
                    }
                    Ok(true) => {
                        pending.insert(id, PendingFile { info, included: true, slot: Some(slot) });
                        Ok(())
                    }
                }
            }
            Event::FileLimit { id } => match pending.get(&id) {
                Some(p) if p.included => Err(transform_multer_error(MulterError::new(
                    MulterErrorCode::LimitFileSize,
                    Some(p.info.fieldname.clone()),
                ))),
                _ => Ok(()),
            },
            Event::FileEnd { id, data } => match pending.remove(&id) {
                Some(PendingFile { info, included: true, slot: Some(slot) }) => {
                    match storage.handle_file(req, &info, data).await {
                        Ok(stored) => {
                            let mut file = Map::new();
                            file.insert("fieldname".into(), Value::String(info.fieldname.clone()));
                            file.insert("originalname".into(), Value::String(info.originalname.clone()));
                            file.insert("encoding".into(), Value::String(info.encoding.clone()));
                            file.insert("mimetype".into(), Value::String(info.mimetype.clone()));
                            file.extend(stored);
                            appender.replace(slot, file.clone());
                            uploaded.push(file);
                            Ok(())
                        }
                        Err(e) => {
                            appender.remove(slot);
                            Err(e)
                        }
                    }
                }
                _ => Ok(()),
            },
            Event::FilesLimit => Err(transform_multer_error(MulterError::new(MulterErrorCode::LimitFileCount, None))),
            Event::FieldsLimit => Err(transform_multer_error(MulterError::new(MulterErrorCode::LimitFieldCount, None))),
            Event::PartsLimit => Err(transform_multer_error(MulterError::new(MulterErrorCode::LimitPartCount, None))),
            Event::Error(msg) => Err(transform_busboy_error(msg)),
        };
        if let Err(e) = result {
            error = Some(e);
            break;
        }
    }

    if let Some(err) = error {
        // `removeUploadedFiles`: undo what the storage already wrote.
        for mut f in uploaded {
            let _ = storage.remove_file(req, &mut f).await;
        }
        return Err(err);
    }

    // `req.body` is a plain JS object: array-index keys enumerate first.
    let body = crate::z::js::js_key_order(Value::Object(body));
    let mut data = MultipartData { body: Arc::new(body), file: None, files: None };
    match setup.strategy {
        Strategy::None => {}
        Strategy::Value => data.file = appender.single.take().map(|f| Arc::new(Value::Object(f))),
        Strategy::Array => {
            let list: Vec<Value> = (0..appender.list.len())
                .filter(|i| appender.list[*i].is_some())
                .map(|i| appender.placeholder(i, ""))
                .collect();
            data.files = Some(Arc::new(Value::Array(list)));
        }
        Strategy::Object => {
            let mut obj = Map::new();
            for (field, idxs) in &appender.by_field {
                obj.insert(field.clone(), Value::Array(idxs.iter().map(|i| appender.placeholder(*i, field)).collect()));
            }
            data.files = Some(Arc::new(Value::Object(obj)));
        }
    }
    Ok(data)
}

fn apply_filter(options: &MulterOptions, req: &Request, info: &FileInfo) -> Result<bool, HttpException> {
    match &options.file_filter {
        Some(f) => f(req, info),
        None => Ok(true),
    }
}

fn handle_field(
    body: &mut Map<String, Value>,
    name: Option<String>,
    value: String,
    value_truncated: bool,
    limits: &Limits,
) -> Result<(), MulterError> {
    let Some(name) = name else { return Err(MulterError::new(MulterErrorCode::MissingFieldName, None)) };
    if value_truncated {
        return Err(MulterError::new(MulterErrorCode::LimitFieldValue, Some(name)));
    }
    if let Some(max) = limits.field_name_size {
        if name.encode_utf16().count() as u64 > max {
            return Err(MulterError::new(MulterErrorCode::LimitFieldKey, None));
        }
    }
    if let Some(depth) = limits.field_nesting_depth {
        if name.matches('[').count() as u64 > depth {
            return Err(MulterError::new(MulterErrorCode::LimitFieldNesting, Some(name)));
        }
    }
    append_field(body, &name, Value::String(value));
    Ok(())
}

macro_rules! interceptor_options {
    ($t:ty) => {
        impl $t {
            /// Local multer options (`FileInterceptor(field, localOptions)`); they override the
            /// `MulterModule` options key by key.
            pub fn options(mut self, options: MulterOptions) -> Self {
                self.options = options;
                self
            }
            /// Shortcut for `.options(MulterOptions::new().storage(storage))`.
            pub fn storage(mut self, storage: impl StorageEngine) -> Self {
                self.options.storage = Some(Arc::new(storage));
                self
            }
            /// Shortcut for `.options(MulterOptions::new().limits(limits))`.
            pub fn limits(mut self, limits: Limits) -> Self {
                self.options.limits = Some(limits);
                self
            }
        }

        #[async_trait]
        impl CorvisInterceptor for $t {
            async fn intercept(&self, ctx: &ExecutionContext, next: CallHandler) -> HandlerResult {
                run(ctx, &self.options, &self.setup()).await?;
                next.handle().await
            }
        }
    };
}

/// `FileInterceptor(fieldName, options)`: one file in `req.file` (`multer.single`).
#[derive(Clone)]
pub struct FileInterceptor {
    field: String,
    options: MulterOptions,
}

impl FileInterceptor {
    pub fn new(field_name: impl Into<String>) -> Self {
        FileInterceptor { field: field_name.into(), options: MulterOptions::default() }
    }
    fn setup(&self) -> Setup {
        Setup { fields: Some(vec![MulterField::new(self.field.clone()).max_count(1)]), strategy: Strategy::Value }
    }
}
interceptor_options!(FileInterceptor);

/// `FilesInterceptor(fieldName, maxCount?, options)`: an array of files in `req.files` (`multer.array`).
#[derive(Clone)]
pub struct FilesInterceptor {
    field: String,
    max_count: Option<u64>,
    options: MulterOptions,
}

impl FilesInterceptor {
    pub fn new(field_name: impl Into<String>) -> Self {
        FilesInterceptor { field: field_name.into(), max_count: None, options: MulterOptions::default() }
    }
    pub fn max_count(mut self, n: u64) -> Self {
        self.max_count = Some(n);
        self
    }
    fn setup(&self) -> Setup {
        let mut f = MulterField::new(self.field.clone());
        f.max_count = self.max_count;
        Setup { fields: Some(vec![f]), strategy: Strategy::Array }
    }
}
interceptor_options!(FilesInterceptor);

/// `FileFieldsInterceptor([{ name, maxCount }], options)`: `req.files` is `{ field: File[] }` (`multer.fields`).
#[derive(Clone)]
pub struct FileFieldsInterceptor {
    fields: Vec<MulterField>,
    options: MulterOptions,
}

impl FileFieldsInterceptor {
    pub fn new<F: Into<MulterField>>(fields: impl IntoIterator<Item = F>) -> Self {
        FileFieldsInterceptor { fields: fields.into_iter().map(Into::into).collect(), options: MulterOptions::default() }
    }
    fn setup(&self) -> Setup {
        Setup { fields: Some(self.fields.clone()), strategy: Strategy::Object }
    }
}
interceptor_options!(FileFieldsInterceptor);

/// `AnyFilesInterceptor(options)`: every file, in `req.files` (`multer.any`).
#[derive(Clone, Default)]
pub struct AnyFilesInterceptor {
    options: MulterOptions,
}

impl AnyFilesInterceptor {
    pub fn new() -> Self {
        Self::default()
    }
    fn setup(&self) -> Setup {
        Setup { fields: None, strategy: Strategy::Array }
    }
}
interceptor_options!(AnyFilesInterceptor);

/// `NoFilesInterceptor(options)`: only text fields; any file is `400 Unexpected field - <name>` (`multer.none`).
#[derive(Clone, Default)]
pub struct NoFilesInterceptor {
    options: MulterOptions,
}

impl NoFilesInterceptor {
    pub fn new() -> Self {
        Self::default()
    }
    fn setup(&self) -> Setup {
        Setup { fields: Some(Vec::new()), strategy: Strategy::None }
    }
}
interceptor_options!(NoFilesInterceptor);
