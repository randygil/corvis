---
status: unavailable
---

# File storage

Corvis does not have a file storage layer with interchangeable drivers (local disk, S3 or other services) behind a single API.

## Alternative

What does exist covers the case of saving and serving files on disk:

- **Receiving files**: `FileInterceptor` and the other interceptors in `corvis::multer` accept `multipart/form-data`. With `disk_storage(DiskStorageOptions::new().destination(..).filename(..))` each file is written to disk, and with `memory_storage()` it stays in memory. For another destination (a bucket, for example), implement the `StorageEngine` trait (`handle_file` and `remove_file`). See [File upload and streaming](/http/file-upload).
- **Sending files**: `StreamableFile::from_path("report.pdf").await?` reads the file in chunks and returns it from a handler.
- **Serving a folder**: `corvis::serve_static(root)` or `ServeStaticModule::for_root(...)` (see [Static files](/recipes/serve-static)).
