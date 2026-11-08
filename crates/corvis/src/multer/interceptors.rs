//! `FileInterceptor`, `FilesInterceptor`, `FileFieldsInterceptor`, `AnyFilesInterceptor` and
//! `NoFilesInterceptor` of the Express platform, on top of multer's `makeMiddleware`.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Map, Value};

use super::busboy::{self, Event};
use super::{
    FileInfo, Limits, MULTER_MODULE_OPTIONS, MulterError, MulterErrorCode, MulterOptions, MultipartData,
    BodySource, StorageEngine, append_field::{ArrayBudget, append_field}, is_multipart, transform_busboy_error,
    transform_multer_error,
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

/// Files written by the storage engine, removed if a later pipe or the handler fails.
struct Cleanup {
    storage: Arc<dyn StorageEngine>,
    files: Vec<Map<String, Value>>,
}

impl Cleanup {
    async fn run(self, req: &Request) {
        for mut f in self.files {
            let _ = self.storage.remove_file(req, &mut f).await;
        }
    }
}

/// Runs multer on the request and stores `req.body` / `req.file` / `req.files`.
async fn run(ctx: &ExecutionContext, local: &MulterOptions, setup: &Setup) -> Result<Option<Cleanup>, HttpException> {
    let req = ctx.request();
    if !is_multipart(req) {
        return Ok(None);
    }
    let module_opts = ctx
        .module_ref()
        .and_then(|m| m.get_token_in_context::<MulterOptions>(MULTER_MODULE_OPTIONS).ok());
    let options = local.merged_over(module_opts.as_deref());
    let (data, cleanup) = process(req, &options, setup).await?;
    req.insert(data);
    Ok(cleanup.filter(|c| !c.files.is_empty() && options.remove_files_on_error.unwrap_or(true)))
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

/// State of one upload while its events are handled.
struct Upload<'a> {
    req: &'a Request,
    options: &'a MulterOptions,
    setup: &'a Setup,
    limits: &'a Limits,
    storage: Arc<dyn StorageEngine>,
    files_left: HashMap<String, u64>,
    body: Map<String, Value>,
    budget: ArrayBudget,
    appender: Appender,
    pending: HashMap<usize, PendingFile>,
    uploaded: Vec<Map<String, Value>>,
}

impl Upload<'_> {
    /// Handles one busboy event. `Ok(Some(id))` asks the parser to drop the data of file `id`.
    async fn on_event(&mut self, ev: Event) -> Result<Option<usize>, HttpException> {
        let limits = self.limits;
        match ev {
            Event::Field { name, value, value_truncated } => {
                handle_field(&mut self.body, &mut self.budget, name, value, value_truncated, limits)
                    .map_err(transform_multer_error)?;
                Ok(None)
            }
            Event::FileStart { id, name, filename, encoding, mime_type } => {
                let Some(fieldname) = name else {
                    return Err(transform_multer_error(MulterError::new(MulterErrorCode::MissingFieldName, None)));
                };
                // Parts without a file name are not files for multer.
                let Some(filename) = filename.filter(|f| !f.is_empty()) else { return Ok(Some(id)) };
                if let Some(max) = limits.field_name_size
                    && fieldname.encode_utf16().count() as u64 > max {
                        return Err(transform_multer_error(MulterError::new(MulterErrorCode::LimitFieldKey, None)));
                    }
                let info = FileInfo { fieldname: fieldname.clone(), originalname: filename, encoding, mimetype: mime_type };
                let slot = self.appender.insert(&fieldname);
                let filter = if self.setup.fields.is_some() {
                    let left = self.files_left.get(&fieldname).copied().unwrap_or(0);
                    if left == 0 {
                        Err(transform_multer_error(MulterError::new(MulterErrorCode::LimitUnexpectedFile, Some(fieldname))))
                    } else {
                        if left != u64::MAX {
                            self.files_left.insert(fieldname.clone(), left - 1);
                        }
                        apply_filter(self.options, self.req, &info)
                    }
                } else {
                    apply_filter(self.options, self.req, &info)
                };
                match filter {
                    Err(e) => {
                        self.appender.remove(slot);
                        Err(e)
                    }
                    Ok(false) => {
                        // `fileStream.resume()`: the contents are skipped.
                        self.appender.remove(slot);
                        self.pending.insert(id, PendingFile { info, included: false, slot: None });
                        Ok(Some(id))
                    }
                    Ok(true) => {
                        self.pending.insert(id, PendingFile { info, included: true, slot: Some(slot) });
                        Ok(None)
                    }
                }
            }
            Event::FileLimit { id } => match self.pending.get(&id) {
                Some(p) if p.included => Err(transform_multer_error(MulterError::new(
                    MulterErrorCode::LimitFileSize,
                    Some(p.info.fieldname.clone()),
                ))),
                _ => Ok(None),
            },
            Event::FileEnd { id, data } => match self.pending.remove(&id) {
                Some(PendingFile { info, included: true, slot: Some(slot) }) => {
                    match self.storage.handle_file(self.req, &info, data).await {
                        Ok(stored) => {
                            let mut file = Map::new();
                            file.insert("fieldname".into(), Value::String(info.fieldname.clone()));
                            file.insert("originalname".into(), Value::String(info.originalname.clone()));
                            file.insert("encoding".into(), Value::String(info.encoding.clone()));
                            file.insert("mimetype".into(), Value::String(info.mimetype.clone()));
                            file.extend(stored);
                            self.appender.replace(slot, file.clone());
                            self.uploaded.push(file);
                            Ok(None)
                        }
                        Err(e) => {
                            self.appender.remove(slot);
                            Err(e)
                        }
                    }
                }
                _ => Ok(None),
            },
            Event::FilesLimit => Err(transform_multer_error(MulterError::new(MulterErrorCode::LimitFileCount, None))),
            Event::FieldsLimit => Err(transform_multer_error(MulterError::new(MulterErrorCode::LimitFieldCount, None))),
            Event::PartsLimit => Err(transform_multer_error(MulterError::new(MulterErrorCode::LimitPartCount, None))),
            Event::Error(msg) => Err(transform_busboy_error(msg)),
        }
    }
}

/// Reads the body from the socket chunk by chunk, feeding busboy and handling its events as
/// they come: a limit, a rejected file or a malformed part stops the read right away.
async fn read_events(
    up: &mut Upload<'_>,
    source: &mut BodySource,
    parser: &mut busboy::StreamParser<'_>,
    body_limit: u64,
) -> Result<(), HttpException> {
    let mut total: u64 = 0;
    loop {
        let Some(chunk) = source.next().await? else {
            for ev in parser.finish() {
                if let Some(id) = up.on_event(ev).await? {
                    parser.discard_file(id);
                }
            }
            return Ok(());
        };
        total += chunk.len() as u64;
        if total > body_limit {
            return Err(crate::server::entity_too_large_error());
        }
        if parser.is_complete() {
            // Closing boundary seen: the epilogue is read (so the connection stays usable)
            // but ignored, like busboy.
            continue;
        }
        parser.push(chunk);
        for ev in parser.take_events() {
            if let Some(id) = up.on_event(ev).await? {
                parser.discard_file(id);
            }
        }
    }
}

async fn process(req: &Request, options: &MulterOptions, setup: &Setup) -> Result<(MultipartData, Option<Cleanup>), HttpException> {
    let storage = options.resolve_storage();
    let limits = options.limits.clone().unwrap_or_default();
    let preserve_path = options.preserve_path.unwrap_or(false);
    let def_param_charset = options.def_param_charset.clone().unwrap_or_else(|| "latin1".into());

    let boundary = busboy::boundary_of(req.headers().get(http::header::CONTENT_TYPE).map(|v| v.as_bytes()))
        .map_err(|e| match e {
            busboy::ConfigError::BoundaryNotFound => transform_busboy_error(busboy::BOUNDARY_NOT_FOUND),
            other => HttpException::internal(super::BusboyError(other.message())),
        })?;
    let body_limit = options.effective_body_limit(&limits);
    let mut source = BodySource::of(req);
    if crate::server::content_length(req.headers()).is_some_and(|n| n > body_limit) {
        // A declared length over the limit is rejected before reading anything.
        source.give_back(req);
        return Err(crate::server::entity_too_large_error());
    }
    let cfg = busboy::Config { limits: &limits, preserve_path, def_param_charset: &def_param_charset };
    let mut parser = busboy::StreamParser::new(&boundary, &cfg);

    let mut files_left: HashMap<String, u64> = HashMap::new();
    if let Some(fields) = &setup.fields {
        for f in fields {
            files_left.insert(f.name.clone(), f.max_count.unwrap_or(u64::MAX));
        }
    }
    let mut up = Upload {
        req,
        options,
        setup,
        limits: &limits,
        storage: storage.clone(),
        files_left,
        body: Map::new(),
        budget: ArrayBudget::default(),
        appender: Appender { strategy: setup.strategy, list: Vec::new(), by_field: Vec::new(), single: None },
        pending: HashMap::new(),
        uploaded: Vec::new(),
    };

    let result = read_events(&mut up, &mut source, &mut parser, body_limit).await;
    source.give_back(req);
    if let Err(err) = result {
        // `removeUploadedFiles`: undo what the storage already wrote.
        for mut f in std::mem::take(&mut up.uploaded) {
            let _ = storage.remove_file(req, &mut f).await;
        }
        return Err(err);
    }
    let Upload { body, mut appender, uploaded, .. } = up;
    let cleanup = Some(Cleanup { storage, files: uploaded });

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
    Ok((data, cleanup))
}

fn apply_filter(options: &MulterOptions, req: &Request, info: &FileInfo) -> Result<bool, HttpException> {
    match &options.file_filter {
        Some(f) => f(req, info),
        None => Ok(true),
    }
}

fn handle_field(
    body: &mut Map<String, Value>,
    budget: &mut ArrayBudget,
    name: Option<String>,
    value: String,
    value_truncated: bool,
    limits: &Limits,
) -> Result<(), MulterError> {
    let Some(name) = name else { return Err(MulterError::new(MulterErrorCode::MissingFieldName, None)) };
    if value_truncated {
        return Err(MulterError::new(MulterErrorCode::LimitFieldValue, Some(name)));
    }
    if let Some(max) = limits.field_name_size
        && name.encode_utf16().count() as u64 > max {
            return Err(MulterError::new(MulterErrorCode::LimitFieldKey, None));
        }
    if let Some(depth) = limits.field_nesting_depth
        && name.matches('[').count() as u64 > depth {
            return Err(MulterError::new(MulterErrorCode::LimitFieldNesting, Some(name)));
        }
    let max_depth = limits
        .field_nesting_depth
        .map_or(super::append_field::DEFAULT_MAX_FIELD_DEPTH, |d| usize::try_from(d).unwrap_or(usize::MAX));
    append_field(body, budget, &name, Value::String(value), max_depth);
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
                let cleanup = run(ctx, &self.options, &self.setup()).await?;
                let result = next.handle().await;
                // A pipe (`ParseFilePipe`) or the handler rejected the request: the files the
                // storage already wrote are removed, so rejected uploads do not stay on disk.
                if let (true, Some(cleanup)) = (result.is_err(), cleanup) {
                    cleanup.run(ctx.request()).await;
                }
                result
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
