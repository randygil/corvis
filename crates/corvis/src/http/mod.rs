//! HTTP primitives: requests, responses, exceptions and status codes.

pub mod body_parser;
#[cfg(feature = "compression")]
pub mod compression;
pub mod exception;
pub mod express;
pub mod json_parse;
pub mod proxy_addr;
mod redact;
pub mod request;
pub mod res;
pub mod response;
pub mod serve_static;
pub mod sse;
pub mod status;

pub use exception::*;
pub use express::{EtagMode, HttpSettings};
pub use proxy_addr::TrustProxy;
pub use request::Request;
pub use res::{CookieOptions, Res, ResMode, SendBody};
pub use response::StreamableFileOptions;
pub use response::{Body, ByteStream, Cookie, HandlerResult, Html, IntoResponse, Json, Redirect, Response, StreamableFile};
pub use serve_static::{Dotfiles, ServeStatic, serve_static};
pub use sse::{MessageEvent, Sse};
pub use status::HttpStatus;

pub use http::{HeaderMap, HeaderValue, Method, StatusCode};

/// HTTP methods usable in route decorators (`RequestMethod`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RequestMethod {
    Get,
    Post,
    Put,
    Delete,
    Patch,
    All,
    Options,
    Head,
    Search,
}

impl RequestMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            RequestMethod::Get => "GET",
            RequestMethod::Post => "POST",
            RequestMethod::Put => "PUT",
            RequestMethod::Delete => "DELETE",
            RequestMethod::Patch => "PATCH",
            RequestMethod::All => "ALL",
            RequestMethod::Options => "OPTIONS",
            RequestMethod::Head => "HEAD",
            RequestMethod::Search => "SEARCH",
        }
    }

    pub fn to_http(&self) -> Option<Method> {
        match self {
            RequestMethod::All => None,
            RequestMethod::Search => Method::from_bytes(b"SEARCH").ok(),
            other => Method::from_bytes(other.as_str().as_bytes()).ok(),
        }
    }

    pub fn matches(&self, m: &Method) -> bool {
        match self {
            RequestMethod::All => true,
            other => other.as_str() == m.as_str(),
        }
    }
}
