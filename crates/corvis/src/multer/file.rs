//! `Express.Multer.File`.

use bytes::Bytes;
use serde::de::Error as _;
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use super::buffer::buffer_from_value;

/// What multer knows about a file before storing it (the `file` argument of `fileFilter`,
/// `destination` and `filename`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    /// Form field name.
    pub fieldname: String,
    /// File name on the client (without path unless `preserve_path`).
    pub originalname: String,
    /// `Content-Transfer-Encoding` of the part (`7bit` by default).
    pub encoding: String,
    /// `Content-Type` of the part.
    pub mimetype: String,
}

/// An uploaded file (`Express.Multer.File`).
///
/// - Memory storage: `buffer` and `size`.
/// - Disk storage: `destination`, `filename`, `path` and `size`.
/// - Custom storage engines may add more keys (`extra`).
///
/// It serializes like the object multer builds (same key order); `buffer` is written as Node's
/// `Buffer#toJSON()` (`{ "type": "Buffer", "data": [...] }`), which is what is sent if a
/// handler returns the file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UploadedFile {
    pub fieldname: String,
    pub originalname: String,
    pub encoding: String,
    pub mimetype: String,
    pub size: u64,
    pub buffer: Option<Bytes>,
    pub destination: Option<String>,
    pub filename: Option<String>,
    pub path: Option<String>,
    /// Other keys returned by a custom storage engine, in order.
    pub extra: Map<String, Value>,
    /// Original key order of the storage info (multer: `{ ...file, ...info }`).
    pub(crate) order: Vec<String>,
}

const BASE: [&str; 4] = ["fieldname", "originalname", "encoding", "mimetype"];

impl UploadedFile {
    /// The contents (memory storage) or `None` (disk storage: read `path`).
    pub fn bytes(&self) -> Option<&Bytes> {
        self.buffer.as_ref()
    }

    fn info_keys(&self) -> Vec<String> {
        if !self.order.is_empty() {
            return self.order.clone();
        }
        let mut keys = Vec::new();
        if self.buffer.is_some() {
            keys.push("buffer".to_string());
        }
        if self.destination.is_some() {
            keys.push("destination".into());
        }
        if self.filename.is_some() {
            keys.push("filename".into());
        }
        if self.path.is_some() {
            keys.push("path".into());
        }
        keys.push("size".into());
        keys.extend(self.extra.keys().cloned());
        keys
    }
}

impl Serialize for UploadedFile {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let keys = self.info_keys();
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("fieldname", &self.fieldname)?;
        m.serialize_entry("originalname", &self.originalname)?;
        m.serialize_entry("encoding", &self.encoding)?;
        m.serialize_entry("mimetype", &self.mimetype)?;
        for k in keys {
            match k.as_str() {
                "buffer" => {
                    if let Some(b) = &self.buffer {
                        m.serialize_entry("buffer", &NodeBuffer(b))?;
                    }
                }
                "destination" => m.serialize_entry("destination", &self.destination)?,
                "filename" => m.serialize_entry("filename", &self.filename)?,
                "path" => m.serialize_entry("path", &self.path)?,
                "size" => m.serialize_entry("size", &self.size)?,
                other => {
                    if let Some(v) = self.extra.get(other) {
                        m.serialize_entry(other, v)?;
                    }
                }
            }
        }
        m.end()
    }
}

struct NodeBuffer<'a>(&'a Bytes);

impl Serialize for NodeBuffer<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(Some(2))?;
        m.serialize_entry("type", "Buffer")?;
        m.serialize_entry("data", &self.0[..])?;
        m.end()
    }
}

impl<'de> Deserialize<'de> for UploadedFile {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        UploadedFile::from_value(&v).map_err(D::Error::custom)
    }
}

impl UploadedFile {
    /// Builds the file from the JSON multer puts on the request.
    pub fn from_value(v: &Value) -> Result<Self, String> {
        let Value::Object(o) = v else { return Err(format!("invalid type: expected an uploaded file, got {v}")) };
        let s = |k: &str| o.get(k).and_then(|x| x.as_str()).map(String::from);
        let mut f = UploadedFile {
            fieldname: s("fieldname").unwrap_or_default(),
            originalname: s("originalname").unwrap_or_default(),
            encoding: s("encoding").unwrap_or_default(),
            mimetype: s("mimetype").unwrap_or_default(),
            size: o.get("size").and_then(|x| x.as_f64()).map(|x| x as u64).unwrap_or(0),
            ..Default::default()
        };
        for (k, val) in o {
            if BASE.contains(&k.as_str()) {
                continue;
            }
            f.order.push(k.clone());
            match k.as_str() {
                "buffer" => f.buffer = buffer_from_value(val),
                "destination" => f.destination = val.as_str().map(String::from),
                "filename" => f.filename = val.as_str().map(String::from),
                "path" => f.path = val.as_str().map(String::from),
                "size" => {}
                _ => {
                    f.extra.insert(k.clone(), val.clone());
                }
            }
        }
        Ok(f)
    }
}
