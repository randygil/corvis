//! Namespaces: a name, the connect middleware, the connection handlers and the sockets that joined.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use futures::future::BoxFuture;
use serde::Serialize;

use super::adapter::{Adapter, LocalNamespace};
use super::options::IoOptions;
use super::socket::IoSocket;
use super::value::IoValue;

pub(crate) type Middleware = Arc<dyn Fn(IoSocket) -> BoxFuture<'static, Result<(), ConnectError>> + Send + Sync>;
pub(crate) type ConnectionHandler = Arc<dyn Fn(IoSocket) + Send + Sync>;

/// Why connect middleware refused a connection: it becomes the CONNECT_ERROR packet the client receives,
/// `{"message": <message>}` plus `"data"` when the error carries data (what `err.message` and `err.data` are in the
/// client's `connect_error` handler). Put nothing in it that the client should not see.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectError {
    pub(crate) message: String,
    pub(crate) data: Option<IoValue>,
}

impl ConnectError {
    pub fn new(message: impl Into<String>) -> Self {
        ConnectError { message: message.into(), data: None }
    }

    /// Attaches data (`err.data` in the client). A value that cannot be written as JSON (binary data, a nesting
    /// deeper than 128 levels, a serializer error) is left out.
    pub fn data(mut self, data: impl Serialize) -> Self {
        self.data = IoValue::from_serialize(&data).ok().filter(|v| !v.contains_binary());
        self
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    /// What the client is told when the middleware panicked or did not finish in time.
    pub(crate) fn internal() -> Self {
        ConnectError::new("Internal server error")
    }
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ConnectError {}

impl From<&str> for ConnectError {
    fn from(message: &str) -> Self {
        ConnectError::new(message)
    }
}

impl From<String> for ConnectError {
    fn from(message: String) -> Self {
        ConnectError::new(message)
    }
}

/// A namespace of an [`IoServer`](super::IoServer): `io.of("/admin")` in Node. Cheap to clone; all clones are the same
/// namespace. Namespaces are registered by the application; a client cannot create one.
#[derive(Clone)]
pub struct IoNamespace {
    pub(crate) inner: Arc<NamespaceInner>,
}

pub(crate) struct NamespaceInner {
    pub(crate) name: String,
    pub(crate) options: Arc<IoOptions>,
    pub(crate) adapter: Arc<dyn Adapter>,
    /// The server-wide room for the answers held by calls with acknowledgements.
    ack_budget: Arc<super::adapter::AckBudget>,
    /// `Adapter::init` ran (it runs before the first cluster operation).
    initialized: tokio::sync::OnceCell<()>,
    middleware: RwLock<Vec<Middleware>>,
    connection: RwLock<Vec<ConnectionHandler>>,
    sockets: RwLock<HashMap<String, IoSocket>>,
    /// The id of the next server-side acknowledgement: a counter per namespace starting at 0, as Node's `_ids`.
    next_ack_id: AtomicU64,
}

impl std::fmt::Debug for IoNamespace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IoNamespace").field("name", &self.inner.name).finish()
    }
}

impl IoNamespace {
    pub(crate) fn new(name: String, options: Arc<IoOptions>, ack_budget: Arc<super::adapter::AckBudget>) -> IoNamespace {
        let inner = Arc::new_cyclic(|weak| NamespaceInner {
            name,
            // The factory gets a handle that does not keep the namespace alive, so an adapter cannot make a cycle.
            adapter: options.adapter.build(LocalNamespace::new(weak.clone())),
            options,
            ack_budget,
            initialized: tokio::sync::OnceCell::new(),
            middleware: RwLock::new(Vec::new()),
            connection: RwLock::new(Vec::new()),
            sockets: RwLock::new(HashMap::new()),
            next_ack_id: AtomicU64::new(0),
        });
        IoNamespace { inner }
    }

    /// The adapter of this namespace. Unstable and hidden: for tests and for the layers built on this one.
    #[doc(hidden)]
    pub fn adapter(&self) -> &Arc<dyn Adapter> {
        &self.inner.adapter
    }

    /// The adapter, after its `init` ran (once, before the first cluster operation).
    /// Bounded by `adapter_timeout`: an adapter whose `init` never returns fails the call with `Timeout`.
    pub(crate) async fn ready_adapter(&self) -> Result<&Arc<dyn Adapter>, super::adapter::AdapterError> {
        self.bounded(async { self.inner.initialized.get_or_try_init(|| self.inner.adapter.init()).await.map(|_| ()) }).await?;
        Ok(&self.inner.adapter)
    }

    /// Runs a call of the adapter with the `adapter_timeout` deadline: an adapter that never answers fails it with
    /// `AdapterError::Timeout`, and the call is dropped.
    pub(crate) async fn bounded<T>(
        &self,
        call: impl Future<Output = Result<T, super::adapter::AdapterError>>,
    ) -> Result<T, super::adapter::AdapterError> {
        match tokio::time::timeout(self.inner.options.adapter_timeout, call).await {
            Ok(result) => result,
            Err(_) => Err(super::adapter::AdapterError::Timeout),
        }
    }

    /// The namespace name, with its leading `/`.
    pub fn name(&self) -> &str {
        &self.inner.name
    }

    /// `namespace.use(fn)`: adds a function that runs, in registration order, before a socket connects. The first
    /// error stops the chain and is sent as CONNECT_ERROR; the socket is not added to the namespace and connection
    /// handlers do not run for it. The socket handed to middleware is not connected yet: `emit` fails with
    /// `NotConnected`, but `handshake()` (with the client's `auth`), `data` and `insert` work, which is where
    /// authentication attaches the user. A panic, and a function that runs longer than
    /// [`IoOptions::middleware_timeout`](super::IoOptions::middleware_timeout), is answered with
    /// `Internal server error`.
    ///
    /// ```
    /// # use corvis::socket_io::{ConnectError, IoOptions, IoServer};
    /// # let io = IoServer::new(IoOptions::new()).unwrap();
    /// io.of("/admin").unwrap().use_middleware(|socket| async move {
    ///     match socket.handshake().auth().get("token").and_then(|t| t.as_str()) {
    ///         Some("secret") => Ok(()),
    ///         _ => Err(ConnectError::new("not authorized").data(serde_json::json!({ "content": "retry later" }))),
    ///     }
    /// });
    /// ```
    pub fn use_middleware<F, Fut>(&self, f: F) -> &Self
    where
        F: Fn(IoSocket) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), ConnectError>> + Send + 'static,
    {
        self.inner.middleware.write().unwrap_or_else(|e| e.into_inner()).push(Arc::new(move |socket| Box::pin(f(socket))));
        self
    }

    /// `namespace.on("connection", fn)`: adds a function that runs for every socket that connected, after the
    /// CONNECT packet was queued and before the first packet of the client is handled, so `on` registered here never
    /// misses an event. It is synchronous: spawn a task for anything slow. A panic is logged and the socket stays.
    pub fn on_connection(&self, f: impl Fn(IoSocket) + Send + Sync + 'static) -> &Self {
        self.inner.connection.write().unwrap_or_else(|e| e.into_inner()).push(Arc::new(f));
        self
    }

    /// The sockets connected to this namespace.
    pub fn sockets(&self) -> Vec<IoSocket> {
        self.inner.sockets.read().unwrap_or_else(|e| e.into_inner()).values().cloned().collect()
    }

    /// The number of sockets connected to this namespace.
    pub fn socket_count(&self) -> usize {
        self.inner.sockets.read().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// The connected socket with this id.
    pub fn socket(&self, id: &str) -> Option<IoSocket> {
        self.inner.sockets.read().unwrap_or_else(|e| e.into_inner()).get(id).cloned()
    }

    pub(crate) fn middleware(&self) -> Vec<Middleware> {
        self.inner.middleware.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub(crate) fn connection_handlers(&self) -> Vec<ConnectionHandler> {
        self.inner.connection.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub(crate) fn add_socket(&self, socket: &IoSocket) {
        self.inner.sockets.write().unwrap_or_else(|e| e.into_inner()).insert(socket.id().to_string(), socket.clone());
    }

    pub(crate) fn remove_socket(&self, id: &str) {
        self.inner.sockets.write().unwrap_or_else(|e| e.into_inner()).remove(id);
    }

    pub(crate) fn next_ack_id(&self) -> u64 {
        self.inner.next_ack_id.fetch_add(1, Ordering::Relaxed)
    }

    pub(crate) fn ack_budget(&self) -> &Arc<super::adapter::AckBudget> {
        &self.inner.ack_budget
    }

    pub(crate) fn options(&self) -> &Arc<IoOptions> {
        &self.inner.options
    }
}

impl NamespaceInner {
    pub(crate) fn socket(&self, id: &str) -> Option<IoSocket> {
        self.sockets.read().unwrap_or_else(|e| e.into_inner()).get(id).cloned()
    }

    /// The sockets with these ids under one read lock.
    pub(crate) fn sockets_by_ids<'a>(&self, ids: impl IntoIterator<Item = &'a str>) -> Vec<IoSocket> {
        let sockets = self.sockets.read().unwrap_or_else(|e| e.into_inner());
        ids.into_iter().filter_map(|id| sockets.get(id).cloned()).collect()
    }

    pub(crate) fn next_ack_id(&self) -> u64 {
        self.next_ack_id.fetch_add(1, Ordering::Relaxed)
    }
}
