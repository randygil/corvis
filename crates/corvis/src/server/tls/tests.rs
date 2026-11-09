use std::sync::Arc;

use rcgen::{CertificateParams, KeyPair};

use super::*;

struct Pem {
    cert: String,
    key: String,
}

fn pem(name: &str) -> Pem {
    let key = KeyPair::generate().unwrap();
    let cert = CertificateParams::new(vec![name.to_string()]).unwrap().self_signed(&key).unwrap();
    Pem { cert: cert.pem(), key: key.serialize_pem() }
}

fn err(r: Result<TlsOptions, TlsError>) -> TlsError {
    r.expect_err("an error")
}

#[test]
fn valid_pem_builds_options() {
    let p = pem("localhost");
    assert!(TlsOptions::from_pem(p.cert.as_bytes(), p.key.as_bytes()).is_ok());
}

#[test]
fn a_chain_keeps_every_certificate() {
    let (a, b) = (pem("a.test"), pem("b.test"));
    let chain = format!("{}{}", a.cert, b.cert);
    assert!(TlsOptions::from_pem(chain.as_bytes(), a.key.as_bytes()).is_ok());
}

#[test]
fn a_missing_file_names_its_path() {
    let p = pem("localhost");
    let dir = std::env::temp_dir().join(format!("corvis-tls-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cert = dir.join("cert.pem");
    std::fs::write(&cert, &p.cert).unwrap();
    let missing = dir.join("missing-key.pem");
    let e = err(TlsOptions::from_pem_files(&cert, &missing));
    assert!(matches!(e, TlsError::Io { .. }));
    assert!(e.to_string().contains("missing-key.pem"), "{e}");
    let ok = dir.join("key.pem");
    std::fs::write(&ok, &p.key).unwrap();
    assert!(TlsOptions::from_pem_files(&cert, &ok).is_ok());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn no_certificate() {
    let p = pem("localhost");
    assert!(matches!(err(TlsOptions::from_pem(b"", p.key.as_bytes())), TlsError::NoCertificate));
    assert!(matches!(err(TlsOptions::from_pem(p.key.as_bytes(), p.key.as_bytes())), TlsError::NoCertificate));
}

#[test]
fn no_private_key() {
    let p = pem("localhost");
    assert!(matches!(err(TlsOptions::from_pem(p.cert.as_bytes(), b"")), TlsError::NoPrivateKey));
    assert!(matches!(err(TlsOptions::from_pem(p.cert.as_bytes(), p.cert.as_bytes())), TlsError::NoPrivateKey));
}

#[test]
fn more_than_one_private_key() {
    let (a, b) = (pem("a.test"), pem("b.test"));
    let keys = format!("{}{}", a.key, b.key);
    match err(TlsOptions::from_pem(a.cert.as_bytes(), keys.as_bytes())) {
        TlsError::MultiplePrivateKeys { count } => assert_eq!(count, 2),
        e => panic!("{e}"),
    }
}

#[test]
fn an_encrypted_key_says_it_must_be_decrypted() {
    let p = pem("localhost");
    let encrypted =
        "-----BEGIN ENCRYPTED PRIVATE KEY-----\nMIIBvTBXBgkqhkiG9w0BBQ0wSjApBgkqhkiG9w0BBQwwHAQI\n-----END ENCRYPTED PRIVATE KEY-----\n";
    let e = err(TlsOptions::from_pem(p.cert.as_bytes(), encrypted.as_bytes()));
    assert!(matches!(e, TlsError::EncryptedPrivateKey));
    assert!(e.to_string().contains("decrypt"), "{e}");
}

#[test]
fn malformed_pem() {
    let p = pem("localhost");
    let broken = "-----BEGIN CERTIFICATE-----\n!!!not base64!!!\n-----END CERTIFICATE-----\n";
    assert!(matches!(err(TlsOptions::from_pem(broken.as_bytes(), p.key.as_bytes())), TlsError::InvalidPem));
}

#[test]
fn a_key_of_another_certificate_is_rejected_without_echoing_the_key() {
    let (a, b) = (pem("a.test"), pem("b.test"));
    let e = err(TlsOptions::from_pem(a.cert.as_bytes(), b.key.as_bytes()));
    assert!(matches!(e, TlsError::Rustls(_)), "{e}");
    let text = format!("{e} {e:?}");
    for line in b.key.lines().filter(|l| !l.starts_with("-----")) {
        assert!(!text.contains(line), "the error contains key material: {text}");
    }
}

#[test]
fn errors_never_contain_the_text_of_the_key() {
    let p = pem("localhost");
    for e in [err(TlsOptions::from_pem(p.cert.as_bytes(), b"-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n"))] {
        let text = format!("{e} {e:?}");
        for line in p.key.lines().filter(|l| !l.starts_with("-----")) {
            assert!(!text.contains(line), "{text}");
        }
    }
}

#[test]
fn early_data_is_refused_even_when_the_configuration_allows_it() {
    let p = pem("localhost");
    let built = TlsOptions::from_pem(p.cert.as_bytes(), p.key.as_bytes()).unwrap();
    let mut config = (*built.server_config()).clone();
    config.max_early_data_size = 16384;
    let options = TlsOptions::from_server_config(Arc::new(config));
    assert_eq!(options.server_config().max_early_data_size, 0);
}

#[test]
fn alpn_lists() {
    let p = pem("localhost");
    let base = TlsOptions::from_pem(p.cert.as_bytes(), p.key.as_bytes()).unwrap();
    assert_eq!(base.server_config().alpn_protocols, [b"h2".to_vec(), b"http/1.1".to_vec()]);
    assert_eq!(base.clone().http2(false).server_config().alpn_protocols, [b"http/1.1".to_vec()]);

    let mut config = (*base.server_config()).clone();
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec(), b"custom".to_vec()];
    let custom = TlsOptions::from_server_config(Arc::new(config));
    assert_eq!(custom.server_config().alpn_protocols.len(), 3, "a configured list is kept");
    assert_eq!(custom.http2(false).server_config().alpn_protocols, [b"http/1.1".to_vec(), b"custom".to_vec()]);
}

#[test]
fn the_handshake_timeout_defaults_to_ten_seconds() {
    let p = pem("localhost");
    let o = TlsOptions::from_pem(p.cert.as_bytes(), p.key.as_bytes()).unwrap();
    assert_eq!(o.settings().handshake_timeout, Duration::from_secs(10));
    assert_eq!(o.handshake_timeout(Duration::from_millis(5)).settings().handshake_timeout, Duration::from_millis(5));
}
