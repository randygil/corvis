---
status: documented
---

# File upload and streaming

`multipart/form-data` uploads are processed with file interceptors, in the style of `multer`
on Express: an interceptor reads the body, saves each file with a *storage* and makes the files
available to the `#[uploaded_file]` and `#[uploaded_files]` parameters. Everything lives in
`corvis::multer`, and the most used types are re-exported at the root of `corvis`.

## One file

```rust
use corvis::prelude::*;
use corvis::{FileInterceptor, UploadedFile};

#[controller("files")]
pub struct FilesController;

#[routes]
impl FilesController {
    #[post("avatar")]
    #[use_interceptors(FileInterceptor::new("file"))]
    fn avatar(&self, #[uploaded_file] file: Option<UploadedFile>, #[body] body: Value) -> Value {
        json!({
            "name": file.as_ref().map(|f| f.originalname.clone()),
            "size": file.as_ref().map(|f| f.size),
            "fields": body,
        })
    }
}
```

- `FileInterceptor::new("file")` accepts one file in the `file` field. A file in another field
  responds `400 Unexpected field - <field>`.
- `#[uploaded_file] file: Option<UploadedFile>` receives `None` if no file arrived. With
  `file: UploadedFile` (without `Option`) the file is required.
- `#[body]` receives the text fields of the form. The keys `tags[]`, `user[name]` or `items[0]`
  and repeated keys are turned into arrays and objects, as with `append-field`.

`UploadedFile` has the fields of a multer file: `fieldname`, `originalname`, `encoding`,
`mimetype`, `size` and, depending on the storage, `buffer` (memory) or `destination`, `filename` and `path`
(disk). `file.bytes()` returns the content when it is in memory. If the handler returns an `UploadedFile`, it is
serialized with the same key order as multer and `buffer` as `{ "type": "Buffer", "data": [...] }`.

## Multiple files

| Interceptor | Parameter |
|---|---|
| `FilesInterceptor::new("photos").max_count(12)` | `#[uploaded_files] files: Vec<UploadedFile>` |
| `FileFieldsInterceptor::new([MulterField::new("avatar").max_count(1), MulterField::new("docs")])` | `#[uploaded_files] files: HashMap<String, Vec<UploadedFile>>` |
| `AnyFilesInterceptor::new()` | `#[uploaded_files] files: Vec<UploadedFile>` |
| `NoFilesInterceptor::new()` | Text fields only; any file responds `400` |

```rust
use std::collections::HashMap;
use corvis::{FileFieldsInterceptor, MulterField};

#[post("profile")]
#[use_interceptors(FileFieldsInterceptor::new([MulterField::new("avatar").max_count(1), MulterField::new("docs")]))]
fn profile(&self, #[uploaded_files] files: HashMap<String, Vec<UploadedFile>>) -> Value {
    json!({ "fields": files.keys().collect::<Vec<_>>() })
}
```

## Limits and options

Every interceptor accepts `.limits(..)`, `.storage(..)` and `.options(MulterOptions)`:

```rust
use corvis::multer::{DiskStorageOptions, Limits, MulterOptions, disk_storage};

#[post(":id/evidence")]
#[use_interceptors(FileInterceptor::new("file").options(
    MulterOptions::new()
        .storage(disk_storage(
            DiskStorageOptions::new()
                .destination("./uploads")
                .filename(|_req, file| Ok(format!("{}-{}", chrono::Utc::now().timestamp_millis(), file.originalname))),
        ))
        .limits(Limits::new().file_size(5 * 1024 * 1024).files(1).fields(20).parts(25)),
))]
async fn upload(&self, #[param("id")] id: String, #[uploaded_file] file: UploadedFile) -> HttpResult<Value> {
    Ok(json!({ "id": id, "path": file.path }))
}
```

(The example uses the `chrono` crate for the name; any name generator works.)

| `Limits` | Effect when exceeded |
|---|---|
| `.file_size(bytes)` | `413 File too large` (also when the size is **reached**, as in busboy) |
| `.files(n)` | `400 Too many files` |
| `.fields(n)`, `.parts(n)` | `400` with the multer message |
| `.field_size(bytes)` | `400 Field value too long - <field>` (1 MiB by default) |
| `.field_name_size(bytes)` | Maximum length of a field name |
| `.field_nesting_depth(n)` | Bracket depth in field names |

The other multipart errors use the same bodies as the reference app: `400 Field name missing`,
`400 Multipart: Boundary not found`, `400 Multipart: Unexpected end of form` and
`400 Multipart: Malformed part header`. The ones it does not translate (`Unsupported content type`,
`Field name nesting too deep`, write failures) are a `500` whose cause is `MulterError` or
`BusboyError`. If the upload fails, the files already written to disk are deleted. As in busboy,
`filename` is decoded as latin1 unless it arrives as `filename*=UTF-8''...` or `def_param_charset("utf8")` is set.

Other `MulterOptions` options:

- `.dest("uploads")`: shortcut to save to disk with random names.
- `.file_filter(|req, file| ...)`: `Ok(true)` accepts, `Ok(false)` skips the file and `Err(e)` aborts
  the upload with that error.
- `.preserve_path(true)` and `.def_param_charset("utf8")`: as in multer.
- `.body_limit(bytes)`: total cap for the multipart body (10 MiB by default, or
  `file_size × files + 1 MiB` if the `limits` ask for more). Exceeding it gives `413 request entity too large`,
  before reading anything if the `Content-Length` already exceeds it. multer has no such cap; Corvis needs it
  because each file is kept in memory until the storage receives it. If you upload several large files in the
  same request, raise it explicitly.
- `.remove_files_on_error(false)`: keeps the files already written on disk if a
  pipe or the handler fails afterwards (by default they are deleted; multer leaves them).

`server.request_timeout` (300 s by default) also bounds the reading of the multipart body and responds `408`.

Without `limits` there is no cap on parts, fields or files beyond `body_limit` (and the 1 MiB `field_size`).
On public routes, set the ones the route needs, for example
`Limits::new().file_size(5 * 1024 * 1024).files(1).fields(20).parts(25)`.

Field names are bounded too: `x[1000000]=v` does not allocate an array of a million `null`s. Each request
can create at most 1000 array holes, and an index that needs more turns that array into an object
(`{ "1000000": "v" }`), as `qs` does. Names with more than 32 bracket levels (`a[b][b]...`) are stored as a
literal key, so a small body cannot build a value deep enough to overflow the stack.
`limits.field_nesting_depth(n)` changes that cap (up to 256) and also responds with an error, as multer does.

### Per-module options

`MulterModule::register(MulterOptions::new()...)` (or `register_async`) defines the default
options for the interceptors of the controllers in the module that imports it.

## Storage

- `memory_storage()` (default): the content stays in `file.buffer`.
- `disk_storage(DiskStorageOptions::new().destination(..).filename(..))`: writes to disk.
  `destination_fn` decides the folder per file. The name returned by `filename` is validated and, if
  it escapes `destination` (`..`, absolute paths, drives, NTFS streams, reserved Windows
  names), the upload responds `400 Invalid file name`. On Windows it also rejects an NTFS alternate
  stream (`name.txt:stream`), a reserved device (`CON`, `NUL`, `COM1`, `nul.txt`...), the characters
  `<>"|?*` and names ending in a dot or space. Even so, do not use `file.originalname` as is
  as the name on disk: generate the name on the server.
- With `preserve_path(true)`, `originalname` keeps the path the client sends (`../../x`, `C:\...`). The
  disk storage rejects names that leave the destination, but a custom `StorageEngine` must validate it itself.
- Orphan files: if `ParseFilePipe` or the handler reject the request, the files already written are deleted
  (see `remove_files_on_error`). To avoid writing invalid files at all, use `limits.file_size` and
  `file_filter`, which act before the storage.
- A custom storage implements the `StorageEngine` trait (`handle_file` and `remove_file`).

## Validating the file with `ParseFilePipe`

`#[uploaded_file(...)]` accepts pipes. `ParseFilePipe` runs validators on the file:

```rust
use corvis::{FileTypeValidator, MaxFileSizeValidator, ParseFilePipe};

#[post("validated")]
#[use_interceptors(FileInterceptor::new("file"))]
fn validated(
    &self,
    #[uploaded_file(ParseFilePipe::new()
        .validator(MaxFileSizeValidator::new(1_000_000))
        .validator(FileTypeValidator::new("image/png")))]
    file: UploadedFile,
) -> Value {
    json!({ "ok": file.originalname })
}
```

- `MaxFileSizeValidator::new(bytes)` with `.message(..)` or `.error_message(..)`.
- `FileTypeValidator::new("image/(png|jpeg)")` detects the real type from the file signature
  (*magic numbers*, as the `file-type` package does): images, PDF, ZIP and Office, audio, video, fonts and
  archives, among others. For text types without a signature use `.fallback_to_mimetype(true)` or
  `.skip_magic_numbers_validation(true)`.
- Inside the pipes the file travels as JSON; `corvis::multer::buffer_from_value(&v["buffer"])` recovers the
  bytes in a custom pipe or validator. From a later guard or interceptor, `req.file()` and `req.files_value()`
  (trait `MulterRequestExt`) give `req.file` / `req.files`.
- `ParseFilePipe` accepts `.file_is_required(false)`, `.error_http_status_code(422)` and
  `.exception_factory(..)`.
- `ParseFilePipeBuilder::new().add_max_size_validator(..).add_file_type_validator(..).build(ParseFilePipe::new())`
  is the *builder* form.
- A custom validator implements `FileValidator` (`is_valid` and `build_error_message`).

::: warning The signature does not guarantee the file is harmless
`FileTypeValidator` only looks at the start of the file: a polyglot file (a GIF header followed by HTML)
can pass as an image, and `fallback_to_mimetype(true)` trusts the `mimetype` the client sends. When
serving uploads, use a fixed `Content-Type`, `Content-Disposition: attachment` and
`X-Content-Type-Options: nosniff`, and avoid `fallback_to_mimetype` for sensitive types.
:::

## When the body is read

The interceptor only acts if the request carries `Content-Type: multipart/*`; otherwise it does nothing.
The multipart body is not read by the server but by the interceptor, after the guards and only on the
routes that use it: a request to a route without an interceptor, a nonexistent route or one rejected by a guard is
answered without reading the body. Reading is *streaming* and the limits cut the upload as soon as they are
reached.

Each file, however, is accumulated in memory until its part ends and is handed whole to the
storage: there is no direct streaming to disk. That is why `body_limit` exists.

To process the multipart by hand, `corvis::multer::read_body(&req, limit).await?` returns the
complete body; `req.raw_body()` is empty on those requests.

## Sending files to the client

For the opposite direction, returning a file, use `StreamableFile`:

```rust
#[get(":id/pdf")]
async fn pdf(&self, #[param("id", ParseUUIDPipe)] id: String) -> HttpResult<StreamableFile> {
    let file = StreamableFile::from_path(format!("./reports/{id}.pdf"))
        .await
        .map_err(|_| NotFoundException::new("Report not found"))?;
    Ok(file.content_type("application/pdf").filename(format!("{id}.pdf")))
}
```

`from_path` reads the file from disk in chunks. There are more forms in
[Streaming responses](/http/streaming-responses).

## Testing

The test client builds the `multipart/form-data` (`attach` accepts text or bytes):

```rust
let r = request(&app)
    .post("/files/avatar")
    .field("title", "Hello")
    .attach("file", "content", "notes.txt")
    .send()
    .await;
```
