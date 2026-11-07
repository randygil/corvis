//! Server-Sent Events (`@Sse()` + `Observable<MessageEvent>`).
//!
//! ```ignore
//! #[sse("events")]
//! fn events(&self) -> Sse {
//!     let stream = IntervalStream::new(tokio::time::interval(Duration::from_secs(1)))
//!         .map(|_| MessageEvent::new(json!({ "hello": "world" })));
//!     Sse::new(stream)
//! }
//! ```

use bytes::Bytes;
use futures::{Stream, StreamExt};
use serde::Serialize;
use serde_json::Value;

use super::response::{HandlerResult, IntoResponse, Response};

/// A server-sent event (`MessageEvent`).
#[derive(Debug, Clone, Default)]
pub struct MessageEvent {
    pub data: Value,
    pub id: Option<String>,
    pub event: Option<String>,
    pub retry: Option<u64>,
}

impl MessageEvent {
    pub fn new(data: impl Serialize) -> Self {
        MessageEvent { data: serde_json::to_value(data).unwrap_or(Value::Null), ..Default::default() }
    }
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }
    /// The event type (`type` of `MessageEvent`).
    pub fn event(mut self, event: impl Into<String>) -> Self {
        self.event = Some(event.into());
        self
    }
    pub fn retry(mut self, ms: u64) -> Self {
        self.retry = Some(ms);
        self
    }

    pub fn to_frame(&self) -> String {
        let mut out = String::new();
        if let Some(e) = &self.event {
            out.push_str(&format!("event: {e}\n"));
        }
        if let Some(id) = &self.id {
            out.push_str(&format!("id: {id}\n"));
        }
        if let Some(r) = self.retry {
            out.push_str(&format!("retry: {r}\n"));
        }
        let data = match &self.data {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        for line in data.split('\n') {
            out.push_str(&format!("data: {line}\n"));
        }
        out.push('\n');
        out
    }
}

/// An SSE response wrapping a stream of [`MessageEvent`]s.
pub struct Sse {
    stream: std::pin::Pin<Box<dyn Stream<Item = MessageEvent> + Send>>,
}

impl Sse {
    pub fn new(stream: impl Stream<Item = MessageEvent> + Send + 'static) -> Self {
        Sse { stream: Box::pin(stream) }
    }
}

impl IntoResponse for Sse {
    fn into_response(self) -> HandlerResult {
        let s = self.stream.map(|e| Ok::<_, std::io::Error>(Bytes::from(e.to_frame())));
        let mut r = Response::stream(Box::pin(s), "text/event-stream");
        r.set_header("cache-control", "no-cache");
        r.set_header("connection", "keep-alive");
        r.set_header("x-accel-buffering", "no");
        r.set_status(200);
        Ok(r)
    }
}
