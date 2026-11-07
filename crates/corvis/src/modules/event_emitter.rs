//! Events: `EventEmitterModule.forRoot()`, `EventEmitter2` and `@OnEvent()`.
//!
//! ```ignore
//! #[injectable]
//! pub struct OrderCreatedListener;
//!
//! #[handlers]
//! impl OrderCreatedListener {
//!     #[on_event("order.created")]
//!     async fn handle(&self, event: OrderCreatedEvent) { /* ... */ }
//! }
//!
//! // somewhere else
//! self.event_emitter.emit("order.created", OrderCreatedEvent { .. });
//! ```

use std::any::Any;
use std::future::Future;
use std::sync::{Arc, RwLock};

use futures::FutureExt;

use crate::di::{DynamicModule, Module, ModuleDefinition, Provider, Token};
use crate::discovery::{EventCall, Payload};
use crate::logger::Logger;

/// Options for `EventEmitterModule::for_root`.
#[derive(Debug, Clone)]
pub struct EventEmitterOptions {
    /// Enable `*` (one segment) and `**` (any number of segments) patterns.
    pub wildcard: bool,
    pub delimiter: String,
    /// Register the module globally (default `true`).
    pub global: bool,
}

impl Default for EventEmitterOptions {
    fn default() -> Self {
        EventEmitterOptions { wildcard: false, delimiter: ".".into(), global: true }
    }
}

struct Listener {
    pattern: String,
    call: EventCall,
}

struct Inner {
    options: EventEmitterOptions,
    listeners: RwLock<Vec<Listener>>,
}

/// The event emitter (`EventEmitter2`). Inject it with `event_emitter: Arc<EventEmitter>`.
#[derive(Clone)]
pub struct EventEmitter {
    inner: Arc<Inner>,
}

impl Default for EventEmitter {
    fn default() -> Self {
        EventEmitter::new(EventEmitterOptions::default())
    }
}

impl EventEmitter {
    pub fn new(options: EventEmitterOptions) -> Self {
        EventEmitter { inner: Arc::new(Inner { options, listeners: RwLock::new(Vec::new()) }) }
    }

    fn matches(&self, pattern: &str, event: &str) -> bool {
        if pattern == event {
            return true;
        }
        if !self.inner.options.wildcard {
            return false;
        }
        let d = self.inner.options.delimiter.as_str();
        let p: Vec<&str> = pattern.split(d).collect();
        let e: Vec<&str> = event.split(d).collect();
        fn rec(p: &[&str], e: &[&str]) -> bool {
            match p.first() {
                None => e.is_empty(),
                Some(&"**") => (0..=e.len()).any(|i| rec(&p[1..], &e[i..])),
                Some(&"*") => !e.is_empty() && rec(&p[1..], &e[1..]),
                Some(s) => e.first() == Some(s) && rec(&p[1..], &e[1..]),
            }
        }
        rec(&p, &e)
    }

    fn listeners_for(&self, event: &str) -> Vec<EventCall> {
        self.inner
            .listeners
            .read()
            .unwrap()
            .iter()
            .filter(|l| self.matches(&l.pattern, event))
            .map(|l| l.call.clone())
            .collect()
    }

    /// `emitter.emit(event, payload)` – listeners run concurrently in the background.
    /// Returns `true` if the event had listeners.
    pub fn emit<T: Send + Sync + 'static>(&self, event: &str, payload: T) -> bool {
        let listeners = self.listeners_for(event);
        let payload: Payload = Arc::new(payload);
        let has = !listeners.is_empty();
        for l in listeners {
            let payload = payload.clone();
            let event = event.to_string();
            tokio::spawn(async move {
                if let Err(e) = l(payload).await {
                    Logger::new("EventEmitter").error(format!("Error in listener for \"{event}\": {e}"));
                }
            });
        }
        has
    }

    /// `emitter.emitAsync(event, payload)` – waits for every listener.
    pub async fn emit_async<T: Send + Sync + 'static>(&self, event: &str, payload: T) -> Vec<crate::Result<()>> {
        let payload: Payload = Arc::new(payload);
        let futs = self.listeners_for(event).into_iter().map(|l| l(payload.clone()));
        futures::future::join_all(futs).await
    }

    /// `emitter.on(event, listener)`. The listener receives the payload as `&dyn Any`.
    pub fn on<F, Fut>(&self, event: &str, listener: F)
    where
        F: Fn(Payload) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = crate::Result<()>> + Send + 'static,
    {
        let listener = Arc::new(listener);
        let call: EventCall = Arc::new(move |p| {
            let l = listener.clone();
            async move { l(p).await }.boxed()
        });
        self.add_listener(event, call);
    }

    pub(crate) fn add_listener(&self, event: &str, call: EventCall) {
        self.inner.listeners.write().unwrap().push(Listener { pattern: event.to_string(), call });
    }

    /// Number of listeners registered for `event`.
    pub fn listener_count(&self, event: &str) -> usize {
        self.listeners_for(event).len()
    }

    /// Removes every listener of `event`.
    pub fn remove_all_listeners(&self, event: &str) {
        self.inner.listeners.write().unwrap().retain(|l| l.pattern != event);
    }
}

/// Downcasts an event payload (used by generated code).
pub fn payload_as<T: Clone + 'static>(payload: &(dyn Any + Send + Sync)) -> Option<T> {
    payload.downcast_ref::<T>().cloned()
}

/// `EventEmitterModule`.
#[derive(Default)]
pub struct EventEmitterModule;
crate::default_injectable!(EventEmitterModule);

impl Module for EventEmitterModule {
    fn definition() -> ModuleDefinition {
        ModuleDefinition::new::<EventEmitterModule>("EventEmitterModule").module_provider(Provider::class::<EventEmitterModule>())
    }
}

impl EventEmitterModule {
    /// `EventEmitterModule.forRoot()`.
    pub fn for_root(options: EventEmitterOptions) -> DynamicModule {
        let global = options.global;
        DynamicModule::new::<EventEmitterModule>()
            .provider(Provider::use_value(Token::of::<EventEmitter>(), EventEmitter::new(options)))
            .export(Token::of::<EventEmitter>())
            .global(global)
    }
}
