//! `Reflector`: reads metadata attached with `#[set_metadata(key, value)]`.

use serde::de::DeserializeOwned;
use serde_json::Value;

use super::context::Target;

/// Helper to read custom metadata from handlers and classes. Injected like any provider:
/// `reflector: Arc<Reflector>`.
#[derive(Debug, Default, Clone, Copy)]
pub struct Reflector;

impl Reflector {
    /// `reflector.get(KEY, context.getHandler())`.
    pub fn get<T: DeserializeOwned>(&self, key: &str, target: &Target) -> Option<T> {
        target.metadata.get(key).and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    /// `reflector.getAllAndOverride(KEY, [context.getHandler(), context.getClass()])`:
    /// returns the first defined value.
    pub fn get_all_and_override<T: DeserializeOwned>(&self, key: &str, targets: &[&Target]) -> Option<T> {
        targets.iter().find_map(|t| self.get::<T>(key, t))
    }

    /// `reflector.getAllAndMerge(KEY, [...])`: concatenates arrays (and collects scalars).
    pub fn get_all_and_merge<T: DeserializeOwned>(&self, key: &str, targets: &[&Target]) -> Vec<T> {
        let mut out = Vec::new();
        for t in targets {
            match t.metadata.get(key) {
                Some(Value::Array(items)) => {
                    out.extend(items.iter().filter_map(|i| serde_json::from_value(i.clone()).ok()));
                }
                Some(v) => {
                    if let Ok(x) = serde_json::from_value(v.clone()) {
                        out.push(x);
                    }
                }
                None => {}
            }
        }
        out
    }

    /// `reflector.getAll(KEY, [...])`.
    pub fn get_all<T: DeserializeOwned>(&self, key: &str, targets: &[&Target]) -> Vec<Option<T>> {
        targets.iter().map(|t| self.get::<T>(key, t)).collect()
    }
}
