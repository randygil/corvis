//! `file.buffer` inside the JSON values that travel through pipes.
//!
//! Handler arguments go through pipes as `serde_json::Value`. Copying a whole upload into a JSON
//! array would be very expensive, so the value carries a handle
//! (`{ "type": "Buffer", "handle": "<id>" }`) to bytes kept in a registry while the request is
//! alive. [`UploadedFile`](super::UploadedFile) resolves it when it is deserialized, and
//! [`buffer_from_value`] does it for custom pipes and validators. Node's `Buffer#toJSON` form
//! (`{ "type": "Buffer", "data": [..] }`) is accepted too.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use bytes::Bytes;
use serde_json::{Value, json};

use crate::http::Request;

static REGISTRY: LazyLock<Mutex<HashMap<String, Bytes>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static COUNTER: AtomicU64 = AtomicU64::new(1);

/// Removes the request's buffers from the registry when the request is dropped.
struct Lease {
    ids: Mutex<Vec<String>>,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let ids = std::mem::take(&mut *self.ids.lock().unwrap());
        if ids.is_empty() {
            return;
        }
        let mut reg = REGISTRY.lock().unwrap();
        for id in ids {
            reg.remove(&id);
        }
    }
}

#[derive(Clone)]
struct LeaseHandle(Arc<Lease>);

/// Registers `bytes` for the lifetime of `req` and returns the JSON handle that represents
/// `file.buffer` (use it in custom [`StorageEngine`](super::StorageEngine)s).
pub fn buffer_to_value(req: &Request, bytes: Bytes) -> Value {
    let lease = match req.get::<LeaseHandle>() {
        Some(l) => l,
        None => {
            let l = LeaseHandle(Arc::new(Lease { ids: Mutex::new(Vec::new()) }));
            req.insert(l.clone());
            l
        }
    };
    let id = format!("{:x}-{:016x}", COUNTER.fetch_add(1, Ordering::Relaxed), rand::random::<u64>());
    REGISTRY.lock().unwrap().insert(id.clone(), bytes);
    lease.0.ids.lock().unwrap().push(id.clone());
    json!({ "type": "Buffer", "handle": id })
}

/// The bytes of a `file.buffer` value (a registry handle or Node's `{ type: 'Buffer', data }`).
pub fn buffer_from_value(v: &Value) -> Option<Bytes> {
    let o = v.as_object()?;
    if o.get("type").and_then(|t| t.as_str()) != Some("Buffer") {
        return None;
    }
    if let Some(id) = o.get("handle").and_then(|h| h.as_str()) {
        return REGISTRY.lock().unwrap().get(id).cloned();
    }
    let data = o.get("data")?.as_array()?;
    let mut out = Vec::with_capacity(data.len());
    for b in data {
        out.push(u8::try_from(b.as_u64()?).ok()?);
    }
    Some(Bytes::from(out))
}
