//! The process-wide rustls provider is the one `TlsOptions` uses. Its own binary: the provider is process-wide and can
//! be installed once.
#![cfg(feature = "tls")]

mod tls_common;

use corvis::ServerOptions;
use corvis::prelude::*;
use corvis::tls::rustls::CipherSuite;
use corvis::tls::rustls::crypto::{CryptoProvider, ring};
use tls_common::*;

#[controller("t")]
pub struct TController;

#[routes]
impl TController {
    #[get("ok")]
    fn ok(&self) -> Value {
        json!({ "ok": true })
    }
}

#[module(controllers = [TController])]
pub struct AppModule;

#[corvis::test]
async fn the_installed_provider_decides_the_cipher_suite() {
    let provider = CryptoProvider { cipher_suites: vec![ring::cipher_suite::TLS13_AES_128_GCM_SHA256], ..ring::default_provider() };
    provider.install_default().expect("no provider was installed before");

    let pki = pki();
    let tls = options(&pki);
    let mut app = CorvisFactory::create::<AppModule>().await.unwrap();
    app.set_server_options(ServerOptions::new());
    let addr = app.listen_tls_in_background("127.0.0.1:0", tls).await.unwrap();

    // The client offers every ring suite; the server only knows one.
    for _ in 0..3 {
        let client = connect(addr, &pki, H1).await;
        let suite = client.get_ref().1.negotiated_cipher_suite().expect("a suite was negotiated");
        assert_eq!(suite.suite(), CipherSuite::TLS13_AES_128_GCM_SHA256);
    }
    let r = https_get(addr, &pki, "/t/ok", false).await;
    assert_eq!(r.status, 200);
}
