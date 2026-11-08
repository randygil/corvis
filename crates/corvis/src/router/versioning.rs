//! API versioning (`app.enableVersioning({ type: VersioningType.URI })`).

use super::controller::VersionValue;
use crate::http::Request;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersioningType {
    /// `/v1/cats`
    Uri,
    /// `Custom-Header: 1`
    Header,
    /// `Accept: application/json;v=2`
    MediaType,
}

/// Options for `app.enable_versioning(...)`.
#[derive(Debug, Clone)]
pub struct VersioningOptions {
    pub kind: VersioningType,
    /// URI prefix (default `"v"`).
    pub prefix: String,
    /// Header name for `VersioningType::Header`.
    pub header: String,
    /// Media type key for `VersioningType::MediaType` (default `"v="`).
    pub key: String,
    pub default_version: Option<VersionValue>,
}

impl VersioningOptions {
    pub fn uri() -> Self {
        VersioningOptions {
            kind: VersioningType::Uri,
            prefix: "v".into(),
            header: String::new(),
            key: String::new(),
            default_version: None,
        }
    }
    pub fn header(name: impl Into<String>) -> Self {
        VersioningOptions {
            kind: VersioningType::Header,
            prefix: String::new(),
            header: name.into(),
            key: String::new(),
            default_version: None,
        }
    }
    pub fn media_type(key: impl Into<String>) -> Self {
        VersioningOptions {
            kind: VersioningType::MediaType,
            prefix: String::new(),
            header: String::new(),
            key: key.into(),
            default_version: None,
        }
    }
    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }
    pub fn default_version(mut self, v: impl Into<VersionValue>) -> Self {
        self.default_version = Some(v.into());
        self
    }

    /// Extracts the requested version from the request (header / media type strategies).
    pub(crate) fn requested_version(&self, req: &Request) -> Option<String> {
        match self.kind {
            VersioningType::Uri => None,
            VersioningType::Header => req.header(&self.header).map(|s| s.trim().to_string()),
            VersioningType::MediaType => {
                let accept = req.header("accept")?;
                accept.split(';').map(|p| p.trim()).find_map(|p| p.strip_prefix(self.key.as_str()).map(|v| v.to_string()))
            }
        }
    }
}
