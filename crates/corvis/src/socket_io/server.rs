//! The Socket.IO server: the namespaces on top of an Engine.IO server.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock, Weak};

use super::client::{self, ClientShared};
use super::engine::{EngineOptionsError, EngineServer};
use super::namespace::IoNamespace;
use super::options::{IoOptions, IoOptionsError};
use super::parser::MAX_NAMESPACE_LEN;
use super::socket::{DisconnectReason, IoSocket};
use super::warn::{WARN_INTERVAL, WarnLimiter};
use crate::http_adapter::HttpServer;

/// Why a namespace could not be registered.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NamespaceNameError {
    /// The name is longer than 256 bytes.
    #[error("the namespace name is {len} bytes long; the most is 256")]
    TooLong { len: usize },
    /// The name has a `,`, the delimiter of the packet header, so no packet could address it.
    #[error("the namespace name {name:?} contains a comma")]
    Comma { name: String },
    /// The name has a control character.
    #[error("the namespace name {name:?} contains a control character")]
    ControlCharacter { name: String },
}

pub(crate) struct ServerInner {
    pub(crate) options: Arc<IoOptions>,
    pub(crate) engine: EngineServer,
    /// Rate limit of the lines a client can provoke.
    pub(crate) warnings: Arc<WarnLimiter>,
    namespaces: RwLock<HashMap<String, IoNamespace>>,
    closed: AtomicBool,
}

impl ServerInner {
    pub(crate) fn namespace(&self, name: &str) -> Option<IoNamespace> {
        self.namespaces.read().unwrap_or_else(|e| e.into_inner()).get(name).cloned()
    }

    /// After a close: the engine refuses handshakes, and anything that still reaches the callback is closed. The
    /// callback no longer holds the server.
    pub(crate) fn release(&self) {
        self.engine.on_connection(|socket| socket.close());
    }

    /// Reports `reason` to every connected socket (without a DISCONNECT packet) and removes them from their
    /// namespaces. Idempotent: a socket reports once.
    pub(crate) fn disconnect_sockets(&self, reason: DisconnectReason) {
        let namespaces: Vec<IoNamespace> = self.namespaces.read().unwrap_or_else(|e| e.into_inner()).values().cloned().collect();
        for namespace in namespaces {
            for socket in namespace.sockets() {
                socket.disconnect_with(reason, false);
            }
        }
    }
}

/// The Socket.IO server: `new Server(httpServer)` in Node. Cheap to clone; all clones are the same server.
///
/// ```no_run
/// use corvis::prelude::*;
/// use corvis::socket_io::{IoOptions, IoServer};
///
/// # async fn demo(app: &CorvisApplication) -> Result<(), Box<dyn std::error::Error>> {
/// let io = IoServer::new(IoOptions::new())?;
/// io.main().on_connection(|socket| {
///     socket.on("echo", |_socket, args, ack| async move {
///         if let Some(ack) = ack {
///             let _ = ack.send(args);
///         }
///     });
/// });
/// io.attach(&app.get_http_server())?;
/// # Ok(()) }
/// ```
#[derive(Clone)]
pub struct IoServer {
    pub(crate) inner: Arc<ServerInner>,
}

impl std::fmt::Debug for IoServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IoServer").finish_non_exhaustive()
    }
}

/// Checks a namespace name and adds the leading `/` Node adds.
fn normalize(name: &str) -> Result<String, NamespaceNameError> {
    let name = if name.starts_with('/') { name.to_string() } else { format!("/{name}") };
    if name.len() > MAX_NAMESPACE_LEN {
        return Err(NamespaceNameError::TooLong { len: name.len() });
    }
    if name.contains(',') {
        return Err(NamespaceNameError::Comma { name });
    }
    if name.chars().any(char::is_control) {
        return Err(NamespaceNameError::ControlCharacter { name });
    }
    Ok(name)
}

impl IoServer {
    /// Creates a server, refusing invalid options with an error that names the option. The main namespace `/` exists
    /// from the start.
    pub fn new(options: IoOptions) -> Result<IoServer, IoOptionsError> {
        options.validate()?;
        let engine = EngineServer::new(options.effective_engine_options())?;
        let options = Arc::new(options);
        let mut namespaces = HashMap::new();
        namespaces.insert("/".to_string(), IoNamespace::new("/".to_string(), options.clone()));
        let inner = Arc::new(ServerInner {
            options,
            engine: engine.clone(),
            warnings: Arc::new(WarnLimiter::new(WARN_INTERVAL)),
            namespaces: RwLock::new(namespaces),
            closed: AtomicBool::new(false),
        });
        let weak: Weak<ServerInner> = Arc::downgrade(&inner);
        // The connection callback keeps the server alive, as `new Server(httpServer)` is kept by its HTTP server: an
        // application may drop its `IoServer` handle after attaching it. `close`, and the application's shutdown,
        // replace the callback, which breaks the cycle.
        let strong = inner.clone();
        engine.on_connection(move |socket| {
            let messages = socket.messages_held();
            let shared = ClientShared::new(socket, strong.options.clone(), &strong);
            tokio::spawn(client::run(strong.clone(), shared, messages));
        });
        // The application's shutdown closes the engine: the sockets hear `server shutting down` first.
        engine.on_closing(move || {
            if let Some(server) = weak.upgrade() {
                server.disconnect_sockets(DisconnectReason::ServerShuttingDown);
                server.release();
            }
        });
        Ok(IoServer { inner })
    }

    /// `io.attach(httpServer)`: from now on the server takes every request under the Engine.IO path (`/socket.io/`
    /// by default). Fails when another attached server's path overlaps this one.
    pub fn attach(&self, server: &HttpServer) -> Result<(), EngineOptionsError> {
        self.inner.engine.attach(server)
    }

    /// `io.of(name)`: the namespace with this name, registered if it was not. A name without a leading `/` gets one.
    /// Fails for a name of more than 256 bytes, with a `,` or with a control character.
    pub fn of(&self, name: &str) -> Result<IoNamespace, NamespaceNameError> {
        let name = normalize(name)?;
        if let Some(existing) = self.inner.namespace(&name) {
            return Ok(existing);
        }
        let mut namespaces = self.inner.namespaces.write().unwrap_or_else(|e| e.into_inner());
        let namespace = namespaces.entry(name.clone()).or_insert_with(|| IoNamespace::new(name, self.inner.options.clone()));
        Ok(namespace.clone())
    }

    /// The main namespace `/`.
    pub fn main(&self) -> IoNamespace {
        self.inner.namespace("/").expect("the main namespace always exists")
    }

    /// `io.on("connection", fn)` of the main namespace.
    pub fn on_connection(&self, f: impl Fn(IoSocket) + Send + Sync + 'static) -> &Self {
        self.main().on_connection(f);
        self
    }

    /// `io.use(fn)`: connect middleware of the main namespace (see [`IoNamespace::use_middleware`]).
    pub fn use_middleware<F, Fut>(&self, f: F) -> &Self
    where
        F: Fn(IoSocket) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), super::namespace::ConnectError>> + Send + 'static,
    {
        self.main().use_middleware(f);
        self
    }

    /// `io.close()`: every connected socket reports `server shutting down` and fails its pending acknowledgements,
    /// then the Engine.IO sessions close and new handshakes are refused with `503`. Idempotent. The application's
    /// shutdown does the same through the Engine.IO server's hook.
    pub fn close(&self) {
        self.inner.closed.store(true, Ordering::SeqCst);
        self.inner.disconnect_sockets(DisconnectReason::ServerShuttingDown);
        self.inner.release();
        self.inner.engine.close();
    }

    /// The Engine.IO server underneath. Unstable and hidden: for tests and for the layers built on this one.
    #[doc(hidden)]
    pub fn engine(&self) -> &EngineServer {
        &self.inner.engine
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_get_a_leading_slash_and_are_validated() {
        assert_eq!(normalize("admin").unwrap(), "/admin");
        assert_eq!(normalize("/admin").unwrap(), "/admin");
        assert_eq!(normalize("").unwrap(), "/");
        assert_eq!(normalize(&"a".repeat(255)).unwrap().len(), 256);
        assert_eq!(normalize(&"a".repeat(256)), Err(NamespaceNameError::TooLong { len: 257 }));
        assert_eq!(normalize("/a,b"), Err(NamespaceNameError::Comma { name: "/a,b".into() }));
        assert!(matches!(normalize("/a\nb"), Err(NamespaceNameError::ControlCharacter { .. })));
        assert!(matches!(normalize("/a\u{0}"), Err(NamespaceNameError::ControlCharacter { .. })));
        assert!(matches!(normalize("/a\u{7f}"), Err(NamespaceNameError::ControlCharacter { .. })));
        assert!(normalize("/a b/é").is_ok());
        assert!(normalize("/a,b").unwrap_err().to_string().contains("/a,b"));
    }

    #[test]
    fn the_main_namespace_exists_and_of_returns_the_same_namespace() {
        let io = IoServer::new(IoOptions::new()).unwrap();
        assert_eq!(io.main().name(), "/");
        let a = io.of("chat").unwrap();
        let b = io.of("/chat").unwrap();
        assert!(Arc::ptr_eq(&a.inner, &b.inner));
        assert!(io.inner.namespace("/missing").is_none());
        assert!(Arc::ptr_eq(&io.of("/").unwrap().inner, &io.main().inner));
    }

    #[test]
    fn invalid_options_are_refused_by_name() {
        let err = IoServer::new(IoOptions::new().max_inflight(0)).unwrap_err();
        assert!(err.to_string().contains("max_inflight"));
        let err = IoServer::new(IoOptions::new().connect_timeout(std::time::Duration::ZERO)).unwrap_err();
        assert!(err.to_string().contains("connect_timeout"));
    }
}
