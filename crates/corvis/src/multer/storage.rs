//! Storage engines: `memoryStorage()`, `diskStorage({ destination, filename })` and custom ones.

use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use serde_json::{Map, Value, json};

use super::FileInfo;
use super::buffer::buffer_to_value;
use crate::http::{HttpException, Request};

/// A multer storage engine (`_handleFile` / `_removeFile`).
///
/// `handle_file` receives the whole file and returns the keys multer merges into the file object
/// (`{ ...file, ...info }`), e.g. `{ "buffer": ..., "size": 3 }` or
/// `{ "destination", "filename", "path", "size" }`. Use [`buffer_to_value`] to return a buffer.
/// `remove_file` undoes it when the upload is aborted (it receives the merged file object).
#[async_trait]
pub trait StorageEngine: Send + Sync + 'static {
    async fn handle_file(&self, req: &Request, file: &FileInfo, data: Bytes) -> Result<Map<String, Value>, HttpException>;
    async fn remove_file(&self, req: &Request, file: &mut Map<String, Value>) -> Result<(), HttpException>;
}

/// `memoryStorage()`: keeps the file in `file.buffer`.
#[derive(Debug, Clone, Copy, Default)]
pub struct MemoryStorage;

/// `multer.memoryStorage()`.
pub fn memory_storage() -> MemoryStorage {
    MemoryStorage
}

#[async_trait]
impl StorageEngine for MemoryStorage {
    async fn handle_file(&self, req: &Request, _file: &FileInfo, data: Bytes) -> Result<Map<String, Value>, HttpException> {
        let size = data.len();
        let mut m = Map::new();
        m.insert("buffer".into(), buffer_to_value(req, data));
        m.insert("size".into(), json!(size));
        Ok(m)
    }

    async fn remove_file(&self, _req: &Request, file: &mut Map<String, Value>) -> Result<(), HttpException> {
        file.remove("buffer");
        Ok(())
    }
}

type PathFn = Arc<dyn Fn(&Request, &FileInfo) -> Result<String, HttpException> + Send + Sync>;

enum Destination {
    Fixed(String),
    Dynamic(PathFn),
    TmpDir,
}

/// Options of `diskStorage({ destination, filename })`.
#[derive(Default)]
pub struct DiskStorageOptions {
    destination: Option<Destination>,
    filename: Option<PathFn>,
}

impl DiskStorageOptions {
    pub fn new() -> Self {
        Self::default()
    }
    /// `destination: 'uploads'` (the folder is created when the storage is built, like multer).
    pub fn destination(mut self, dir: impl Into<String>) -> Self {
        self.destination = Some(Destination::Fixed(dir.into()));
        self
    }
    /// `destination: (req, file, cb) => cb(null, dir)`. The folder must exist (multer does not create it).
    pub fn destination_fn(mut self, f: impl Fn(&Request, &FileInfo) -> Result<String, HttpException> + Send + Sync + 'static) -> Self {
        self.destination = Some(Destination::Dynamic(Arc::new(f)));
        self
    }
    /// `filename: (req, file, cb) => cb(null, name)`. Default: 16 random bytes in hex, without extension.
    pub fn filename(mut self, f: impl Fn(&Request, &FileInfo) -> Result<String, HttpException> + Send + Sync + 'static) -> Self {
        self.filename = Some(Arc::new(f));
        self
    }
}

/// `diskStorage(...)`: writes the file to `destination/filename`.
pub struct DiskStorage {
    destination: Destination,
    filename: Option<PathFn>,
}

/// `multer.diskStorage({ destination, filename })`.
pub fn disk_storage(options: DiskStorageOptions) -> DiskStorage {
    let destination = options.destination.unwrap_or(Destination::TmpDir);
    if let Destination::Fixed(d) = &destination {
        // `fs.mkdirSync(opts.destination, { recursive: true })`
        let _ = std::fs::create_dir_all(d);
    }
    DiskStorage { destination, filename: options.filename }
}

fn tmpdir() -> String {
    // `os.tmpdir()` strips the trailing separator.
    let t = std::env::temp_dir().to_string_lossy().into_owned();
    let trimmed = t.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() || trimmed.ends_with(':') { t } else { trimmed.to_string() }
}

#[async_trait]
impl StorageEngine for DiskStorage {
    async fn handle_file(&self, req: &Request, file: &FileInfo, data: Bytes) -> Result<Map<String, Value>, HttpException> {
        let destination = match &self.destination {
            Destination::Fixed(d) => d.clone(),
            Destination::Dynamic(f) => f(req, file)?,
            Destination::TmpDir => tmpdir(),
        };
        let filename = match &self.filename {
            Some(f) => f(req, file)?,
            None => {
                let raw: [u8; 16] = rand::random();
                raw.iter().map(|b| format!("{b:02x}")).collect()
            }
        };
        let final_path = node_path_join(&destination, &filename);
        tokio::fs::write(&final_path, &data).await.map_err(HttpException::internal)?;
        let mut m = Map::new();
        m.insert("destination".into(), json!(destination));
        m.insert("filename".into(), json!(filename));
        m.insert("path".into(), json!(final_path));
        m.insert("size".into(), json!(data.len()));
        Ok(m)
    }

    async fn remove_file(&self, _req: &Request, file: &mut Map<String, Value>) -> Result<(), HttpException> {
        let path = file.get("path").and_then(|p| p.as_str()).map(String::from);
        file.remove("destination");
        file.remove("filename");
        file.remove("path");
        if let Some(p) = path {
            tokio::fs::remove_file(&p).await.map_err(HttpException::internal)?;
        }
        Ok(())
    }
}

/// Node's `path.join(a, b)` for the current platform (`path.win32` on Windows, `path.posix` elsewhere):
/// joins with the platform separator and normalizes `.`, `..` and repeated separators.
pub fn node_path_join(a: &str, b: &str) -> String {
    let joined = match (a.is_empty(), b.is_empty()) {
        (true, true) => return ".".into(),
        (true, false) => b.to_string(),
        (false, true) => a.to_string(),
        (false, false) => format!("{a}/{b}"),
    };
    normalize(&joined, cfg!(windows))
}

fn normalize(path: &str, windows: bool) -> String {
    let sep = if windows { '\\' } else { '/' };
    let is_sep = |c: char| c == '/' || (windows && c == '\\');
    let mut rest = path;
    let mut prefix = String::new();
    if windows {
        let b = rest.as_bytes();
        if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
            prefix = rest[..2].to_string();
            rest = &rest[2..];
        }
    }
    let absolute = rest.starts_with(is_sep);
    let trailing = rest.ends_with(is_sep);
    let mut parts: Vec<&str> = Vec::new();
    for seg in rest.split(is_sep) {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|p| *p != "..") {
                    parts.pop();
                } else if !absolute {
                    parts.push("..");
                }
            }
            s => parts.push(s),
        }
    }
    let mut out = prefix;
    if absolute {
        out.push(sep);
    }
    out.push_str(&parts.join(&sep.to_string()));
    if parts.is_empty() && !absolute {
        out.push('.');
    }
    if trailing && !out.ends_with(sep) {
        out.push(sep);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::normalize;

    #[test]
    fn path_join_matches_node() {
        assert_eq!(normalize("./uploads/123.png", false), "uploads/123.png");
        assert_eq!(normalize("./uploads/123.png", true), "uploads\\123.png");
        assert_eq!(normalize("/tmp//x/../y", false), "/tmp/y");
        assert_eq!(normalize("C:\\tmp/./a", true), "C:\\tmp\\a");
        assert_eq!(normalize("../a/b", false), "../a/b");
        assert_eq!(normalize("./", false), "./");
        assert_eq!(normalize(".", false), ".");
    }
}
