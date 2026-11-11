//! What a fetched socket shares with the local one, and what `Debug` prints of it (never a credential).

use super::super::*;
use crate::socket_io::adapter::SocketInfo;

#[tokio::test(start_paused = true)]
async fn fetch_sockets_shares_the_handshake_of_a_local_socket() {
    let io = server(options());
    let conn = io.conn();
    let id = conn.join(r#"0{"token":"SECRET-AUTH-VALUE"}"#).await;
    let socket = io.main().socket(&id).unwrap();
    let fetched = io.fetch_sockets().await.unwrap();
    assert_eq!(fetched.len(), 1);
    let info = fetched[0].handshake();
    assert!(std::ptr::eq(info.auth(), socket.handshake().auth()), "the auth object is shared, not copied");
    assert_eq!(info.auth()["token"], "SECRET-AUTH-VALUE");
    // It still serializes as plain data and reads back equal.
    let wire = serde_json::to_string(fetched[0].info()).unwrap();
    let back: SocketInfo = serde_json::from_str(&wire).unwrap();
    assert_eq!(&back, fetched[0].info());
    assert_eq!(back.handshake.auth()["token"], "SECRET-AUTH-VALUE");
}

#[tokio::test(start_paused = true)]
async fn debug_output_hides_credentials() {
    let io = server(options());
    let conn = io.conn();
    let id = conn.join(r#"0{"token":"SECRET-AUTH-VALUE"}"#).await;
    let fetched = io.fetch_sockets().await.unwrap();
    for text in [format!("{:?}", fetched[0]), format!("{:?}", fetched[0].info()), format!("{:?}", fetched[0].handshake())] {
        assert!(!text.contains("SECRET-AUTH-VALUE"), "{text}");
        assert!(text.contains(&id) || !text.contains("RemoteSocket") || text.contains("id"), "{text}");
    }
    assert!(format!("{:?}", fetched[0].handshake()).contains("token"), "the key is shown");
    // A handshake described by another process.
    let remote = HandshakeInfo::new(
        json!({ "token": "SECRET-AUTH-VALUE" }),
        vec![
            ("cookie".into(), "session=SECRET-COOKIE".into()),
            ("Authorization".into(), "Bearer SECRET-BEARER".into()),
            ("proxy-authorization".into(), "Basic SECRET-PROXY".into()),
            ("x-api-key".into(), "SECRET-APIKEY".into()),
            ("X-Auth-Token".into(), "SECRET-TOKEN".into()),
            ("x-custom-credential".into(), "SECRET-CUSTOM".into()),
            ("proxy-authorization ".into(), "SECRET-SPACE".into()),
            ("user-agent".into(), "tests".into()),
        ],
        "/socket.io/?EIO=4&token=SECRET-QUERY".into(),
        "10.0.0.1".into(),
        1,
    );
    let text = format!("{remote:?}");
    for secret in [
        "SECRET-AUTH-VALUE",
        "SECRET-COOKIE",
        "SECRET-BEARER",
        "SECRET-PROXY",
        "SECRET-QUERY",
        "SECRET-APIKEY",
        "SECRET-TOKEN",
        "SECRET-CUSTOM",
        "SECRET-SPACE",
    ] {
        assert!(!text.contains(secret), "{secret} in {text}");
    }
    for shown in ["cookie", "Authorization", "user-agent: tests", "/socket.io/", "10.0.0.1"] {
        assert!(text.contains(shown), "{shown} missing from {text}");
    }
}
