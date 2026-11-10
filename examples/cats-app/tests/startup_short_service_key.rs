//! The application refuses to start with a `SERVICE_API_KEY` shorter than 32 bytes, and the error does not repeat it.

#[corvis::test]
async fn refuses_to_start_with_a_short_service_key() {
    // SAFETY: this test binary has a single test and sets the environment before the app reads it.
    unsafe {
        std::env::set_var("JWT_SECRET", "cats-test-secret-0123456789abcdef");
        std::env::set_var("SERVICE_API_KEY", "hunter2-too-short");
    }
    let err = cats_app::create_app().await.err().expect("the application started with a short service key");
    let text = format!("{err:#}");
    assert!(text.contains("SERVICE_API_KEY") && text.contains("at least 32 bytes"), "{text}");
    assert!(!text.contains("hunter2"), "the secret must not be in the error: {text}");
}
