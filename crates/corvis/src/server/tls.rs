//! TLS termination for the built-in server (`tls` feature): [`TlsOptions`] and [`TlsError`].
//!
//! ```no_run
//! # async fn run(app: corvis::application::CorvisApplication) -> corvis::Result<()> {
//! use corvis::tls::TlsOptions;
//!
//! let tls = TlsOptions::from_pem_files("cert.pem", "key.pem")?;
//! app.listen_tls("0.0.0.0:3443", tls).await
//! # }
//! ```
//!
//! The options are built before the application starts, so a missing file or a key that does not match the
//! certificate fails at the call site, never in the middle of `listen_tls`.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::crypto::{CryptoProvider, ring};
use tokio_rustls::rustls::pki_types::pem::{PemObject, SectionKind};
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs1KeyDer, PrivatePkcs8KeyDer, PrivateSec1KeyDer};

/// The rustls version Corvis uses, to build a [`ServerConfig`](tokio_rustls::rustls::ServerConfig) for
/// [`TlsOptions::from_server_config`] without a version mismatch.
pub use tokio_rustls::rustls;

const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Why [`TlsOptions`] could not be built. Messages name files, never their contents.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TlsError {
    /// A certificate or key file could not be read.
    #[error("cannot read {}: {source}", path.display())]
    Io {
        /// The file.
        path: PathBuf,
        /// What the operating system said.
        source: std::io::Error,
    },
    /// The certificate PEM has no `CERTIFICATE` block.
    #[error("no certificate found in the PEM data")]
    NoCertificate,
    /// The key PEM has no `PRIVATE KEY`, `RSA PRIVATE KEY` or `EC PRIVATE KEY` block.
    #[error("no private key found in the PEM data")]
    NoPrivateKey,
    /// The key PEM has more than one private key.
    #[error("found {count} private keys in the PEM data, expected exactly one")]
    MultiplePrivateKeys {
        /// How many.
        count: usize,
    },
    /// The key is encrypted with a passphrase.
    #[error("the private key is encrypted: decrypt it first, for example with `openssl pkcs8 -in key.pem -out key-decrypted.pem`")]
    EncryptedPrivateKey,
    /// The PEM data is malformed.
    #[error("the PEM data is not valid")]
    InvalidPem,
    /// rustls rejected the key (unsupported type) or the key does not belong to the certificate.
    #[error("TLS configuration rejected: {0}")]
    Rustls(#[from] rustls::Error),
}

/// TLS settings of a listener, for `listen_tls` and `listen_tls_in_background`.
///
/// Defaults: TLS 1.2 and 1.3, rustls' default cipher suites, no client certificates, one certificate for every server
/// name, ALPN `h2` and `http/1.1`, 0-RTT early data refused, and a handshake limited to 10 seconds.
#[derive(Clone)]
pub struct TlsOptions {
    config: Arc<ServerConfig>,
    handshake_timeout: Duration,
    http2: bool,
}

impl fmt::Debug for TlsOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TlsOptions").field("handshake_timeout", &self.handshake_timeout).field("http2", &self.http2).finish_non_exhaustive()
    }
}

impl TlsOptions {
    /// Reads a PEM certificate chain (leaf first) and one private key (PKCS#8, PKCS#1 or SEC1) from two files.
    pub fn from_pem_files(cert_chain: impl AsRef<Path>, key: impl AsRef<Path>) -> Result<Self, TlsError> {
        let read = |p: &Path| std::fs::read(p).map_err(|source| TlsError::Io { path: p.to_path_buf(), source });
        let cert = read(cert_chain.as_ref())?;
        let key = read(key.as_ref())?;
        Self::from_pem(&cert, &key)
    }

    /// Like [`from_pem_files`](Self::from_pem_files), from the bytes of the PEM data.
    pub fn from_pem(cert_chain: &[u8], key: &[u8]) -> Result<Self, TlsError> {
        let mut certs = Vec::new();
        for section in <(SectionKind, Vec<u8>)>::pem_slice_iter(cert_chain) {
            let (kind, der) = section.map_err(|_| TlsError::InvalidPem)?;
            if kind == SectionKind::Certificate {
                certs.push(CertificateDer::from(der));
            }
        }
        if certs.is_empty() {
            return Err(TlsError::NoCertificate);
        }
        if contains(key, b"ENCRYPTED PRIVATE KEY") || contains(key, b"ENCRYPTED") && contains(key, b"Proc-Type:") {
            return Err(TlsError::EncryptedPrivateKey);
        }
        let mut keys = Vec::new();
        for section in <(SectionKind, Vec<u8>)>::pem_slice_iter(key) {
            let (kind, der) = section.map_err(|_| TlsError::InvalidPem)?;
            match kind {
                SectionKind::PrivateKey => keys.push(PrivateKeyDer::from(PrivatePkcs8KeyDer::from(der))),
                SectionKind::RsaPrivateKey => keys.push(PrivateKeyDer::from(PrivatePkcs1KeyDer::from(der))),
                SectionKind::EcPrivateKey => keys.push(PrivateKeyDer::from(PrivateSec1KeyDer::from(der))),
                _ => {}
            }
        }
        let count = keys.len();
        let key = match keys.pop() {
            None => return Err(TlsError::NoPrivateKey),
            Some(k) if count == 1 => k,
            Some(_) => return Err(TlsError::MultiplePrivateKeys { count }),
        };
        let provider = CryptoProvider::get_default().cloned().unwrap_or_else(|| Arc::new(ring::default_provider()));
        let config = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_single_cert(certs, key)?;
        Ok(Self::from_server_config(Arc::new(config)))
    }

    /// Uses a ready rustls configuration: SNI-based certificates, client certificate verifiers, a certificate resolver
    /// that reloads files, TLS 1.3 only. Corvis applies its own hardening to a copy (0-RTT early data off, ALPN).
    pub fn from_server_config(config: Arc<ServerConfig>) -> Self {
        TlsOptions { config, handshake_timeout: DEFAULT_HANDSHAKE_TIMEOUT, http2: true }
    }

    /// Longest a client may take to finish the TLS handshake (default 10 seconds, so a client that stalls cannot
    /// hold a connection slot). It cannot be turned off.
    pub fn handshake_timeout(mut self, timeout: Duration) -> Self {
        self.handshake_timeout = timeout;
        self
    }

    /// Whether ALPN offers `h2` (default `true`). With `false` only `http/1.1` is offered.
    pub fn http2(mut self, enabled: bool) -> Self {
        self.http2 = enabled;
        self
    }

    /// The configuration the listener uses: the one given, with early data refused and the ALPN list set.
    pub fn server_config(&self) -> Arc<ServerConfig> {
        let mut config = (*self.config).clone();
        config.max_early_data_size = 0;
        if config.alpn_protocols.is_empty() {
            config.alpn_protocols = if self.http2 { vec![b"h2".to_vec(), b"http/1.1".to_vec()] } else { vec![b"http/1.1".to_vec()] };
        } else if !self.http2 {
            config.alpn_protocols.retain(|p| p != b"h2");
        }
        Arc::new(config)
    }

    pub(crate) fn settings(&self) -> TlsSettings {
        TlsSettings { acceptor: TlsAcceptor::from(self.server_config()), handshake_timeout: self.handshake_timeout }
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// What the accept loop needs of a TLS listener.
#[derive(Clone)]
pub(crate) struct TlsSettings {
    pub(crate) acceptor: TlsAcceptor,
    pub(crate) handshake_timeout: Duration,
}

#[cfg(test)]
mod tests;
